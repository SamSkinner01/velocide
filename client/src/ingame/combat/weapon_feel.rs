//! Local weapon prediction: the client runs the same pure `shared::sim::step_weapons` on every
//! input it predicts, so firing is instant (muzzle flash, viewmodel kick, sound, tracers)
//! instead of a round trip late. The authoritative `OwnState::combat.weapons` re-bases the
//! prediction whenever a newer snapshot arrives (replaying the unacknowledged inputs, without
//! effects), exactly like movement reconciliation.

use glam::Vec3;
use shared::input::{PlayerInput, look_dir};
use shared::sim::{FireIntent, WeaponState, step_weapons};
use shared::tuning::CombatTuning;

use crate::net::NetClient;

/// Predicted weapon state of the local player.
#[derive(Default)]
pub struct WeaponPredictor {
    state: WeaponState,
    /// Last input applied to `state`.
    applied_seq: u32,
    /// (tick, seq) of the authoritative state we last re-based on.
    based_on: Option<(u32, u32)>,
    /// Re-bases whose replay disagreed with what we had predicted (cooldown / sticky / prism
    /// differences decided by the server).
    pub mismatches: u64,
}

/// One predicted input's intents.
pub struct PredictedShot {
    pub input: PlayerInput,
    pub intents: Vec<FireIntent>,
}

impl WeaponPredictor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> &WeaponState {
        &self.state
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Re-bases on the newest authoritative weapon state if there is one we haven't used.
    pub fn sync(&mut self, net: &NetClient) {
        let (Some(own), Some(pred)) = (net.own_combat(), net.prediction()) else {
            return;
        };
        if self.based_on == Some((own.tick, own.seq)) {
            return;
        }
        let first = self.based_on.is_none();
        self.based_on = Some((own.tick, own.seq));
        let t = &net.tuning().combat;
        let rate = net.tick_rate();
        let mut state = own.combat.weapons;
        let mut last = own.seq;
        for input in pred.unacked() {
            if input.seq <= own.seq || (!first && input.seq > self.applied_seq) {
                continue;
            }
            let alive = !own.dead_at(input.seq);
            step_weapons(&mut state, input, alive, t, rate);
            last = input.seq;
        }
        if !first && state != self.state {
            self.mismatches += 1;
        }
        self.state = state;
        if first {
            self.applied_seq = last;
        }
    }

    /// After `NetClient::tick`: applies the input just predicted and returns what it fired.
    pub fn after_tick(&mut self, net: &NetClient) -> Option<PredictedShot> {
        self.sync(net);
        self.based_on?;
        let input = *net.prediction()?.unacked().last()?;
        if input.seq <= self.applied_seq {
            return None;
        }
        let alive = !net.own_combat().is_some_and(|c| c.dead_at(input.seq));
        let intents = step_weapons(
            &mut self.state,
            &input,
            alive,
            &net.tuning().combat,
            net.tick_rate(),
        );
        self.applied_seq = input.seq;
        Some(PredictedShot { input, intents })
    }

    /// Overcharge fraction for the viewmodel glow.
    pub fn charge(&self, t: &CombatTuning, tick_rate: u32) -> f32 {
        self.state.charge_fraction(t, tick_rate)
    }
}

/// Directions of the rays a hitscan intent fires (client-side, for tracers only: the server's
/// spread is authoritative for damage).
pub fn hitscan_dirs(
    intent: &FireIntent,
    input: &PlayerInput,
    t: &CombatTuning,
    rng: &mut super::Rng,
) -> Vec<Vec3> {
    let aim = look_dir(input.yaw, input.pitch);
    let Some(attack) = intent.attack() else {
        return Vec::new();
    };
    use shared::sim::Attack;
    match attack {
        Attack::Lancet | Attack::Overcharge | Attack::Slug => vec![aim],
        Attack::Fan => {
            let n = t.lancet.fan_shots.max(1);
            (0..n)
                .map(|i| {
                    let f = if n == 1 {
                        0.0
                    } else {
                        i as f32 / (n - 1) as f32 * 2.0 - 1.0
                    };
                    glam::Quat::from_rotation_y((t.lancet.fan_spread_degrees * f).to_radians())
                        * aim
                })
                .collect()
        }
        Attack::Pellet => {
            let half = (t.thresher.cone_degrees * 0.5).to_radians().tan();
            let (right, up) = shared::sim::weapons::aim_basis(aim);
            (0..t.thresher.pellets.max(1))
                .map(|_| {
                    let r = rng.unit().sqrt() * half;
                    let a = rng.unit() * std::f32::consts::TAU;
                    (aim + right * (r * a.cos()) + up * (r * a.sin())).normalize_or(aim)
                })
                .collect()
        }
        _ => Vec::new(),
    }
}
