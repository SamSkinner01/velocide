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
    /// Re-bases whose replay disagreed with what we had predicted (should stay 0: nothing
    /// server-side changes the weapon state).
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

    /// Discharge charge fraction for the viewmodel glow and the HUD gauge.
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
    use shared::loadout::WeaponKind;
    use shared::sim::Attack;
    let weapon = intent.weapon();
    // Everything the bow looses is an arrow (a projectile), the Brand included.
    if weapon == Some(WeaponKind::Longwick) {
        return Vec::new();
    }
    let (right, up) = shared::sim::weapons::aim_basis(aim);
    let mut cone = |count: u32, degrees: f32| -> Vec<Vec3> {
        let half = (degrees * 0.5).to_radians().tan();
        (0..count.max(1))
            .map(|_| {
                let r = rng.unit().sqrt() * half;
                let a = rng.unit() * std::f32::consts::TAU;
                (aim + right * (r * a.cos()) + up * (r * a.sin())).normalize_or(aim)
            })
            .collect()
    };
    let charge = match intent {
        FireIntent::Primary { charge, .. } | FireIntent::Ability { charge, .. } => *charge,
        _ => 0.0,
    };
    match attack {
        Attack::Quill
        | Attack::Brand
        | Attack::Splinter
        | Attack::Arc
        | Attack::Skewer
        | Attack::Slug => vec![aim],
        // The Lancet's bloom (`charge`) widens its needles' spread.
        Attack::Lancet => cone(1, t.lancet.bloom_degrees * charge * 2.0),
        Attack::Flurry => cone(1, 1.0),
        Attack::Blast => cone(t.bellows.pellets, t.bellows.cone_degrees),
        Attack::BothBarrels => cone(t.bellows.both_pellets, t.bellows.both_cone_degrees),
        Attack::Thresher => cone(t.thresher.pellets, t.thresher.cone_degrees),
        // Ten of the weapon's own shots at once: rays for hitscan weapons (rivets fly).
        Attack::Volley
            if !matches!(
                weapon,
                Some(WeaponKind::Riveter | WeaponKind::Kiln | WeaponKind::Tumbler)
            ) =>
        {
            cone(10, 6.0)
        }
        _ => Vec::new(),
    }
}
