//! Local weapon prediction: the client runs the same pure `shared::sim::step_weapons` on every
//! input it predicts, so firing is instant (muzzle flash, viewmodel kick, sound, tracers)
//! instead of a round trip late. The authoritative `OwnState::combat.weapons` re-bases the
//! prediction whenever a newer snapshot arrives (replaying the unacknowledged inputs, without
//! effects), exactly like movement reconciliation.
//!
//! **Press-frame feedback** (`docs/feel.md`): inputs are simulated at fixed ticks, so a click
//! between two ticks used to wait for the next one (up to a tick, 16.7 ms at 60 Hz: more than
//! two frames at 144 Hz) before anything showed. [`WeaponPredictor::lookahead`] runs the next
//! tick's `step_weapons` on a copy of the state on the frame the button changes, so the shot is
//! seen and heard on that frame; the tick that really fires it then finds it already played
//! ([`PredictedShot::played`]) and doesn't play it twice. The prediction itself (the state, what
//! is sent) is untouched.

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
    /// What [`Self::lookahead`] already played for the next tick.
    preplayed: Option<Vec<FireIntent>>,
}

/// One predicted input's intents.
#[derive(Clone, Debug)]
pub struct PredictedShot {
    pub input: PlayerInput,
    pub intents: Vec<FireIntent>,
    /// The predicted eye when it fired, before its input's movement (projectiles launch from
    /// there, `shared::sim::projectiles::LaunchView`; so do their stand-ins).
    pub eye: Option<glam::Vec3>,
    /// Its effects were already played on the press frame ([`WeaponPredictor::lookahead`]).
    pub played: bool,
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
        let mut shot = self.apply(&input, alive, &net.tuning().combat, net.tick_rate());
        // Its projectiles leave from where the input's movement started (in the stance after
        // it), as on the server.
        shot.eye = net.prediction().map(|p| {
            let now = p.state();
            let back = p.previous_state().position - now.position;
            let back = if back.length() <= shared::sim::TELEPORT_PER_TICK {
                back
            } else {
                glam::Vec3::ZERO
            };
            now.eye_position(&net.tuning().movement) + back
        });
        Some(shot)
    }

    /// Applies one predicted input (`after_tick` without the network).
    fn apply(
        &mut self,
        input: &PlayerInput,
        alive: bool,
        t: &CombatTuning,
        tick_rate: u32,
    ) -> PredictedShot {
        let intents = step_weapons(&mut self.state, input, alive, t, tick_rate);
        self.applied_seq = input.seq;
        let played = self.take_preplayed(&intents);
        PredictedShot {
            input: *input,
            intents,
            eye: None,
            played,
        }
    }

    /// Press-frame feedback: what the next tick will fire if its input is `input` (built now,
    /// with the presses latched so far), played now instead of at that tick. Only looks when a
    /// fire / punch / throw button changed since the last tick; `None` if it fires nothing or
    /// was already played. The predicted state is not touched.
    pub fn lookahead(
        &mut self,
        input: &PlayerInput,
        alive: bool,
        t: &CombatTuning,
        tick_rate: u32,
    ) -> Option<PredictedShot> {
        use shared::input::Buttons;
        let mask = (Buttons::FIRE | Buttons::ALT_FIRE | Buttons::PUNCH | Buttons::THROW).0;
        if self.based_on.is_none()
            || self.preplayed.is_some()
            || input.buttons.0 & mask == self.state.prev_buttons.0 & mask
        {
            return None;
        }
        let mut next = self.state;
        let intents = step_weapons(&mut next, input, alive, t, tick_rate);
        if intents.is_empty() {
            return None;
        }
        self.preplayed = Some(intents.clone());
        Some(PredictedShot {
            input: *input,
            intents,
            eye: None,
            played: false,
        })
    }

    /// Whether the tick that just fired `intents` was played ahead by [`Self::lookahead`]
    /// (the same intents: then don't play them again). Forgets the lookahead either way.
    fn take_preplayed(&mut self, intents: &[FireIntent]) -> bool {
        match self.preplayed.take() {
            Some(pre) => !intents.is_empty() && same_intents(&pre, intents),
            None => false,
        }
    }

    /// Discharge charge fraction for the viewmodel glow and the HUD gauge.
    pub fn charge(&self, t: &CombatTuning, tick_rate: u32) -> f32 {
        self.state.charge_fraction(t, tick_rate)
    }
}

/// The same shots (kind, weapon, ability; charges within a draw tick).
fn same_intents(a: &[FireIntent], b: &[FireIntent]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.attack() == y.attack() && x.weapon() == y.weapon() && {
                let charge = |i: &FireIntent| match i {
                    FireIntent::Primary { charge, .. } | FireIntent::Ability { charge, .. } => {
                        *charge
                    }
                    _ => 0.0,
                };
                (charge(x) - charge(y)).abs() < 0.05
            }
        })
}

/// Directions of the rays a hitscan intent fires (client-side, for tracers only: the server's
/// spread is authoritative for damage). A scattergun's blast is exactly the server's: its fixed
/// pattern's jitter is seeded by the input (`scatter::pellet_dirs`).
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
    let blast = |w| shared::sim::scatter::blast_dirs(t, w, input).unwrap_or_default();
    let charge = match intent {
        FireIntent::Primary { charge, .. } | FireIntent::Ability { charge, .. } => *charge,
        _ => 0.0,
    };
    match attack {
        Attack::Quill | Attack::Brand | Attack::Arc | Attack::Lamplighter => vec![aim],
        // The Lancet's bloom (`charge`) widens its needles' spread.
        Attack::Lancet => cone(1, t.lancet.bloom_degrees * charge * 2.0),
        Attack::Blast => blast(WeaponKind::Bellows),
        Attack::Thresher => blast(WeaponKind::Thresher),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::time::FixedTimestep;
    use shared::input::Buttons;

    /// A predictor already based on an authoritative state (default loadout: the Quill).
    fn based() -> WeaponPredictor {
        WeaponPredictor {
            based_on: Some((0, 0)),
            ..WeaponPredictor::default()
        }
    }

    /// Frames from a click to the frame its shot is first shown, at `fps` with 60 Hz ticks, for
    /// a click landing at `phase` (0..1) of a tick; and how many times it was shown.
    fn click_to_flash(fps: f64, phase: f64, lookahead: bool) -> (u32, u32) {
        let t = CombatTuning::default();
        let mut p = based();
        let mut ts = FixedTimestep::new(60.0);
        ts.advance(phase / 60.0);
        let press = 4;
        let (mut shown, mut plays) = (None, 0);
        let (mut seq, mut latched) = (0, false);
        for frame in 0..30u32 {
            // A quick click: down on the press frame only (latched until the next tick).
            let down = frame == press;
            latched |= down;
            let buttons = if down || latched {
                Buttons::FIRE
            } else {
                Buttons::NONE
            };
            let ticks = ts.advance(1.0 / fps);
            for _ in 0..ticks {
                seq += 1;
                let input = PlayerInput {
                    seq,
                    buttons: if latched {
                        Buttons::FIRE
                    } else {
                        Buttons::NONE
                    },
                    ..Default::default()
                };
                latched = false;
                let shot = p.apply(&input, true, &t, 60);
                if !shot.intents.is_empty() && !shot.played {
                    plays += 1;
                    shown.get_or_insert(frame);
                }
            }
            if ticks == 0 && lookahead {
                let next = PlayerInput {
                    seq: seq + 1,
                    buttons,
                    ..Default::default()
                };
                if p.lookahead(&next, true, &t, 60).is_some() {
                    plays += 1;
                    shown.get_or_insert(frame);
                }
            }
        }
        (shown.map_or(99, |f| f - press), plays)
    }

    #[test]
    fn a_click_between_ticks_shows_on_its_own_frame() {
        // Before (tick-only feedback) vs after (press-frame lookahead), clicks at 48 phases
        // of a tick, at 144 and 240 frames per second.
        for fps in [144.0, 240.0] {
            let (mut before, mut after) = (0u32, 0u32);
            let n = 48;
            for i in 0..n {
                let phase = (i as f64 + 0.5) / n as f64;
                let (b, plays_b) = click_to_flash(fps, phase, false);
                let (a, plays_a) = click_to_flash(fps, phase, true);
                assert_eq!((plays_b, plays_a), (1, 1), "one shot shown per click");
                assert_eq!(a, 0, "{fps} fps, phase {phase}: shown on the click's frame");
                before += b;
                after += a;
            }
            let mean = before as f64 / n as f64;
            // A tick is 2.4 frames at 144 fps, 4 at 240: the shot waited 0.75 / 1.5 frames on
            // average (~5–6 ms, up to a whole tick) before (docs/feel.md).
            eprintln!("{fps} fps: {mean:.2} frames from click to flash before, 0 now");
            assert!(mean > 0.6, "{fps} fps: {mean} frames before");
            assert_eq!(after, 0);
        }
    }

    #[test]
    fn a_lookahead_that_does_not_happen_is_played_by_its_tick() {
        let t = CombatTuning::default();
        let mut p = based();
        let fire = PlayerInput {
            seq: 1,
            buttons: Buttons::FIRE,
            ..Default::default()
        };
        assert!(p.lookahead(&fire, true, &t, 60).is_some());
        // Looking again before the tick plays nothing new.
        assert!(p.lookahead(&fire, true, &t, 60).is_none());
        // The tick fires the same shot: already played.
        let shot = p.apply(&fire, true, &t, 60);
        assert!(!shot.intents.is_empty() && shot.played);
        // Held, the button no longer changes: no lookahead.
        let held = PlayerInput { seq: 2, ..fire };
        assert!(p.lookahead(&held, true, &t, 60).is_none());
        // A punch looked ahead, but the tick's input punched nothing (the press was
        // undone): the next real shot is played normally.
        let punch = PlayerInput {
            seq: 3,
            buttons: Buttons::PUNCH,
            ..Default::default()
        };
        let mut q = based();
        assert!(q.lookahead(&punch, true, &t, 60).is_some());
        let none = q.apply(
            &PlayerInput {
                seq: 3,
                ..Default::default()
            },
            true,
            &t,
            60,
        );
        assert!(none.intents.is_empty() && !none.played);
        let fired = q.apply(&PlayerInput { seq: 4, ..fire }, true, &t, 60);
        assert!(!fired.intents.is_empty() && !fired.played);
    }
}
