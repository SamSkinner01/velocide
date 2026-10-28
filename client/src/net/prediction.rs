//! Client-side prediction and server reconciliation for the local player.
//!
//! Every fixed tick the game's input is applied immediately with the same
//! [`shared::sim::player_move_step`] the server runs, and kept (with the predicted state after
//! it) until the server acknowledges it. When a snapshot arrives with the authoritative
//! [`MoveState`] after input `acked_seq`:
//!
//! 1. acknowledged inputs are dropped;
//! 2. if the authoritative state equals what we predicted for `acked_seq` (bit-exact — the
//!    movement code is deterministic), nothing happens;
//! 3. otherwise (server-side impulse, teleport, lost input...) we rewind to the server state and
//!    **replay** the remaining unacknowledged inputs. The visual jump is hidden by a render
//!    offset that decays exponentially (`NetTuning::correction_smoothing` per second) unless
//!    it is larger than `NetTuning::correction_snap_distance`, in which case we snap.

use std::collections::VecDeque;

use glam::Vec3;
use shared::input::PlayerInput;
use shared::movement::{MoveEnv, MoveEvents, MoveState};
use shared::sim::player_move_step;
use shared::tuning::{MovementTuning, NetTuning};

/// Inputs kept for replay at most (about 8 s at 60 Hz).
pub const MAX_PREDICTION_HISTORY: usize = 512;

#[derive(Clone, Copy, Debug)]
struct Predicted {
    input: PlayerInput,
    after: MoveState,
}

/// Statistics for the debug overlay.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PredictionStats {
    /// Reconciliations where the server disagreed with our prediction.
    pub corrections: u64,
    /// Corrections larger than the snap distance (not smoothed).
    pub snaps: u64,
    /// Position error (m) of the last correction at the acknowledged input.
    pub last_error: f32,
    pub max_error: f32,
    pub total_error: f64,
    /// Inputs re-simulated during corrections.
    pub replayed_inputs: u64,
    /// How far the predicted current position moved in the last / largest correction (m):
    /// the visual error hidden by smoothing.
    pub last_shift: f32,
    pub max_shift: f32,
}

impl PredictionStats {
    pub fn average_error(&self) -> f32 {
        if self.corrections == 0 {
            0.0
        } else {
            (self.total_error / self.corrections as f64) as f32
        }
    }
}

/// Details of one correction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Correction {
    /// Position error at the acknowledged input (m).
    pub error: f32,
    /// How far the predicted *current* position moved.
    pub shift: Vec3,
    pub snapped: bool,
    pub replayed: usize,
}

/// Predicted local player.
#[derive(Clone, Debug)]
pub struct Prediction {
    acked_seq: u32,
    acked_state: MoveState,
    history: VecDeque<Predicted>,
    current: MoveState,
    prev_position: Vec3,
    render_offset: Vec3,
    stats: PredictionStats,
}

impl Prediction {
    /// Starts from an authoritative state (after input `seq`).
    pub fn new(seq: u32, state: MoveState) -> Self {
        Self {
            acked_seq: seq,
            acked_state: state,
            history: VecDeque::new(),
            current: state,
            prev_position: state.position,
            render_offset: Vec3::ZERO,
            stats: PredictionStats::default(),
        }
    }

    /// Latest predicted state.
    pub fn state(&self) -> &MoveState {
        &self.current
    }

    /// Last input acknowledged by the server.
    pub fn acked_seq(&self) -> u32 {
        self.acked_seq
    }

    /// The last authoritative state received.
    pub fn server_state(&self) -> &MoveState {
        &self.acked_state
    }

    /// Inputs not yet acknowledged, oldest first.
    pub fn unacked(&self) -> impl DoubleEndedIterator<Item = &PlayerInput> + ExactSizeIterator {
        self.history.iter().map(|p| &p.input)
    }

    pub fn unacked_len(&self) -> usize {
        self.history.len()
    }

    pub fn stats(&self) -> &PredictionStats {
        &self.stats
    }

    /// Current visual correction offset (decays to zero).
    pub fn render_offset(&self) -> Vec3 {
        self.render_offset
    }

    /// Position to draw, interpolated between the last two ticks (`alpha` from the fixed
    /// timestep) plus the correction offset.
    pub fn render_position(&self, alpha: f32) -> Vec3 {
        self.prev_position
            .lerp(self.current.position, alpha.clamp(0.0, 1.0))
            + self.render_offset
    }

    /// Predicts one tick.
    pub fn apply(
        &mut self,
        input: PlayerInput,
        env: &MoveEnv,
        tuning: &MovementTuning,
        dt: f32,
    ) -> MoveEvents {
        self.prev_position = self.current.position;
        let events = player_move_step(&mut self.current, &input, env, tuning, dt);
        self.history.push_back(Predicted {
            input,
            after: self.current,
        });
        while self.history.len() > MAX_PREDICTION_HISTORY {
            self.history.pop_front();
        }
        events
    }

    /// Applies an authoritative state. Returns the correction made, if the server disagreed.
    #[allow(clippy::too_many_arguments)]
    pub fn reconcile(
        &mut self,
        acked_seq: u32,
        server: &MoveState,
        env: &MoveEnv,
        tuning: &MovementTuning,
        net: &NetTuning,
        dt: f32,
    ) -> Option<Correction> {
        if acked_seq < self.acked_seq {
            return None; // older than what we already have
        }
        let predicted = if acked_seq == self.acked_seq {
            Some(self.acked_state)
        } else {
            self.history
                .iter()
                .find(|p| p.input.seq == acked_seq)
                .map(|p| p.after)
        };
        while self
            .history
            .front()
            .is_some_and(|p| p.input.seq <= acked_seq)
        {
            self.history.pop_front();
        }
        self.acked_seq = acked_seq;
        self.acked_state = *server;

        if predicted.as_ref() == Some(server) {
            return None;
        }

        // Rewind and replay.
        let mut state = *server;
        for p in self.history.iter_mut() {
            player_move_step(&mut state, &p.input, env, tuning, dt);
            p.after = state;
        }
        let shift = state.position - self.current.position;
        self.current = state;
        // Keep the drawn position continuous: shift the previous tick too and hide the jump
        // in the decaying render offset.
        self.prev_position += shift;
        self.render_offset -= shift;
        let snapped = !self.render_offset.is_finite()
            || self.render_offset.length() > net.correction_snap_distance;
        if snapped {
            self.render_offset = Vec3::ZERO;
            self.prev_position = self.current.position;
        }

        let error = predicted.map_or(shift.length(), |p| p.position.distance(server.position));
        let error = if error.is_finite() { error } else { f32::MAX };
        self.stats.corrections += 1;
        self.stats.snaps += snapped as u64;
        self.stats.last_error = error;
        self.stats.max_error = self.stats.max_error.max(error);
        self.stats.total_error += error as f64;
        self.stats.replayed_inputs += self.history.len() as u64;
        self.stats.last_shift = shift.length();
        self.stats.max_shift = self.stats.max_shift.max(shift.length());
        Some(Correction {
            error,
            shift,
            snapped,
            replayed: self.history.len(),
        })
    }

    /// Decays the visual correction offset; call once per rendered frame.
    pub fn decay_smoothing(&mut self, frame_dt: f32, net: &NetTuning) {
        let k = (-net.correction_smoothing.max(0.0) * frame_dt.max(0.0)).exp();
        self.render_offset *= k;
        if self.render_offset.length_squared() < 1e-8 {
            self.render_offset = Vec3::ZERO;
        }
    }

    /// Drops the correction smoothing and the tick interpolation, so the view stands at the
    /// predicted position at once: a teleport the player meant (the Blink Arrow's step) snaps
    /// instead of sliding there.
    pub fn snap_view(&mut self) {
        self.render_offset = Vec3::ZERO;
        self.prev_position = self.current.position;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::sim::{BUILTIN_LEVEL, SimWorld, builtin_level};
    use shared::tuning::Tuning;
    use std::sync::Arc;

    fn world() -> SimWorld {
        SimWorld::new(
            BUILTIN_LEVEL,
            Arc::new(builtin_level(BUILTIN_LEVEL).unwrap()),
            60,
        )
    }

    fn input(seq: u32) -> PlayerInput {
        PlayerInput {
            seq,
            forward: 127,
            yaw: seq as f32 * 0.01,
            ..Default::default()
        }
    }

    /// Runs `n` inputs through an authoritative copy, optionally with an impulse after
    /// `impulse_at`, returning the states after each input.
    fn server_states(n: u32, start: MoveState, impulse_at: Option<u32>) -> Vec<MoveState> {
        let w = world();
        let t = Tuning::default();
        let mut s = start;
        (1..=n)
            .map(|seq| {
                player_move_step(&mut s, &input(seq), &w.move_env(), &t.movement, w.dt());
                if impulse_at == Some(seq) {
                    s.velocity += Vec3::new(0.0, 8.0, 3.0);
                }
                s
            })
            .collect()
    }

    #[test]
    fn exact_prediction_needs_no_correction() {
        let w = world();
        let t = Tuning::default();
        let start = MoveState::spawn(Vec3::ZERO);
        let server = server_states(60, start, None);
        let mut p = Prediction::new(0, start);
        for seq in 1..=60 {
            p.apply(input(seq), &w.move_env(), &t.movement, w.dt());
            if seq % 6 == 0 && seq > 12 {
                let ack = seq - 10;
                let c = p.reconcile(
                    ack,
                    &server[ack as usize - 1],
                    &w.move_env(),
                    &t.movement,
                    &t.net,
                    w.dt(),
                );
                assert!(c.is_none());
            }
        }
        assert_eq!(p.stats().corrections, 0);
        assert_eq!(*p.state(), server[59]);
    }

    #[test]
    fn impulse_is_corrected_and_smoothed() {
        let w = world();
        let t = Tuning::default();
        let start = MoveState::spawn(Vec3::ZERO);
        let server = server_states(60, start, Some(20));
        let mut p = Prediction::new(0, start);
        for seq in 1..=28 {
            p.apply(input(seq), &w.move_env(), &t.movement, w.dt());
        }
        let before = p.render_position(1.0);
        let c = p
            .reconcile(22, &server[21], &w.move_env(), &t.movement, &t.net, w.dt())
            .expect("correction");
        assert!(c.error > 0.0);
        assert!(!c.snapped, "{c:?}");
        assert_eq!(c.replayed, 6);
        // The replayed state matches what the server will compute.
        assert_eq!(*p.state(), server[27]);
        // Drawn position didn't jump.
        assert!((p.render_position(1.0) - before).length() < 1e-4);
        // Offset decays.
        for _ in 0..120 {
            p.decay_smoothing(1.0 / 60.0, &t.net);
        }
        assert!(p.render_offset().length() < 1e-3);
        // Later acks agree again.
        for seq in 29..=60 {
            p.apply(input(seq), &w.move_env(), &t.movement, w.dt());
        }
        assert!(
            p.reconcile(50, &server[49], &w.move_env(), &t.movement, &t.net, w.dt())
                .is_none()
        );
        assert_eq!(*p.state(), server[59]);
        assert_eq!(p.stats().corrections, 1);
    }

    #[test]
    fn big_errors_snap() {
        let w = world();
        let t = Tuning::default();
        let start = MoveState::spawn(Vec3::ZERO);
        let mut p = Prediction::new(0, start);
        for seq in 1..=10 {
            p.apply(input(seq), &w.move_env(), &t.movement, w.dt());
        }
        let teleported = MoveState::spawn(Vec3::new(40.0, 0.0, 40.0));
        let c = p
            .reconcile(10, &teleported, &w.move_env(), &t.movement, &t.net, w.dt())
            .unwrap();
        assert!(c.snapped);
        assert_eq!(p.render_offset(), Vec3::ZERO);
        assert_eq!(p.render_position(0.0), teleported.position);
    }

    #[test]
    fn a_meant_step_snaps_the_view_even_when_short() {
        // A Blink Arrow's step under the snap distance would be smoothed like a correction;
        // `snap_view` puts the view there at once.
        let w = world();
        let t = Tuning::default();
        let start = MoveState::spawn(Vec3::ZERO);
        let mut p = Prediction::new(0, start);
        for seq in 1..=10 {
            p.apply(input(seq), &w.move_env(), &t.movement, w.dt());
        }
        let stepped = MoveState::spawn(Vec3::new(1.5, 0.0, -1.0));
        let c = p
            .reconcile(10, &stepped, &w.move_env(), &t.movement, &t.net, w.dt())
            .unwrap();
        assert!(!c.snapped && p.render_offset().length() > 1.0);
        p.snap_view();
        assert_eq!(p.render_offset(), Vec3::ZERO);
        for alpha in [0.0, 0.5, 1.0] {
            assert_eq!(p.render_position(alpha), stepped.position);
        }
    }

    #[test]
    fn stale_acks_are_ignored() {
        let w = world();
        let t = Tuning::default();
        let mut p = Prediction::new(5, MoveState::spawn(Vec3::ZERO));
        assert!(
            p.reconcile(
                3,
                &MoveState::spawn(Vec3::ONE),
                &w.move_env(),
                &t.movement,
                &t.net,
                w.dt()
            )
            .is_none()
        );
        assert_eq!(p.acked_seq(), 5);
    }
}
