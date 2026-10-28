//! Lag compensation: a short per-tick history of every hitbox, so hit validation can rewind
//! targets to where the shooter saw them.
//!
//! Each [`shared::input::PlayerInput`] carries `view_tick` / `view_frac`: the (fractional)
//! server tick the client was displaying remote entities at (its interpolation time). When the
//! server processes a shot from that input it calls [`LagCompHistory::rewind`] with
//! `input.view_time_ticks()`; the history interpolates between the two recorded ticks around
//! that time (exactly what the client rendered, since it interpolates between snapshots of the
//! same ticks) and clamps the rewind to `NetTuning::lag_comp_max_ms` so high-ping players can't
//! shoot arbitrarily far into the past.
//!
//! Frames hold [`Hitbox`]es of players today; enemies and other shootable entities just add
//! their hitboxes in `SimWorld::hitboxes` ([`HitTarget::Entity`]).

use std::collections::VecDeque;

use glam::Vec3;
use shared::sim::{HitPart, HitTarget, Hitbox, HitboxHistory};

/// All hitboxes after simulating one tick.
#[derive(Clone, Debug, PartialEq)]
pub struct HitboxFrame {
    pub tick: u32,
    pub hitboxes: Vec<Hitbox>,
}

/// Result of a rewind.
#[derive(Clone, Debug, PartialEq)]
pub struct Rewound {
    /// The time (ticks) actually used after clamping.
    pub time_ticks: f64,
    pub hitboxes: Vec<Hitbox>,
}

impl Rewound {
    /// Nearest hit along a ray among the rewound hitboxes, skipping `ignore` (the shooter).
    pub fn ray_hit(
        &self,
        origin: Vec3,
        dir: Vec3,
        max_dist: f32,
        ignore: Option<HitTarget>,
    ) -> Option<(HitTarget, f32, HitPart)> {
        self.hitboxes
            .iter()
            .filter(|h| Some(h.target) != ignore)
            .filter_map(|h| {
                h.ray_hit(origin, dir, max_dist)
                    .map(|(t, p)| (h.target, t, p))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
}

/// Ring buffer of recent hitbox frames.
pub struct LagCompHistory {
    tick_rate: u32,
    capacity: usize,
    frames: VecDeque<HitboxFrame>,
}

impl LagCompHistory {
    /// Keeps `history_ms` of frames at `tick_rate`.
    pub fn new(tick_rate: u32, history_ms: f32) -> Self {
        let capacity =
            ((history_ms.max(0.0) / 1000.0 * tick_rate as f32).ceil() as usize + 2).max(2);
        Self {
            tick_rate: tick_rate.max(1),
            capacity,
            frames: VecDeque::with_capacity(capacity),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn latest_tick(&self) -> Option<u32> {
        self.frames.back().map(|f| f.tick)
    }

    pub fn oldest_tick(&self) -> Option<u32> {
        self.frames.front().map(|f| f.tick)
    }

    pub fn frames(&self) -> impl Iterator<Item = &HitboxFrame> {
        self.frames.iter()
    }

    /// Records the hitboxes after simulating `tick`. Ticks must increase; a repeated or older
    /// tick replaces the history from that point (e.g. after a level restart).
    pub fn record(&mut self, tick: u32, hitboxes: Vec<Hitbox>) {
        while self.frames.back().is_some_and(|f| f.tick >= tick) {
            self.frames.pop_back();
        }
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(HitboxFrame { tick, hitboxes });
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }

    /// Clamps a requested view time to what may be rewound: not later than the newest frame,
    /// not earlier than `max_rewind_ms` before it (nor than the oldest frame).
    pub fn clamp_time(&self, view_time_ticks: f64, max_rewind_ms: f32) -> Option<f64> {
        let latest = self.latest_tick()? as f64;
        let oldest = self.oldest_tick()? as f64;
        let max_rewind = max_rewind_ms.max(0.0) as f64 / 1000.0 * self.tick_rate as f64;
        let earliest = (latest - max_rewind).max(oldest);
        if view_time_ticks.is_nan() {
            return Some(latest);
        }
        Some(view_time_ticks.clamp(earliest, latest))
    }

    /// Hitboxes as they were at `view_time_ticks` (clamped, interpolated between ticks).
    pub fn rewind(&self, view_time_ticks: f64, max_rewind_ms: f32) -> Rewound {
        let Some(t) = self.clamp_time(view_time_ticks, max_rewind_ms) else {
            return Rewound {
                time_ticks: view_time_ticks,
                hitboxes: Vec::new(),
            };
        };
        let (a, b, frac) = self.bracket(t);
        let hitboxes = match b {
            None => a.hitboxes.clone(),
            Some(b) => {
                let span = (b.tick - a.tick) as f32;
                a.hitboxes
                    .iter()
                    .enumerate()
                    .map(|(i, ha)| {
                        // Same order in consecutive frames, unless the set changed.
                        let hb = b
                            .hitboxes
                            .get(i)
                            .filter(|hb| hb.target == ha.target)
                            .or_else(|| b.hitboxes.iter().find(|hb| hb.target == ha.target));
                        match hb {
                            Some(hb) => ha.blend(hb, frac, span),
                            None => ha.clone(),
                        }
                    })
                    .collect()
            }
        };
        Rewound {
            time_ticks: t,
            hitboxes,
        }
    }

    /// One target's hitbox at `view_time_ticks`, if it existed then.
    pub fn rewind_target(
        &self,
        target: HitTarget,
        view_time_ticks: f64,
        max_rewind_ms: f32,
    ) -> Option<Hitbox> {
        let t = self.clamp_time(view_time_ticks, max_rewind_ms)?;
        let (a, b, frac) = self.bracket(t);
        let ha = a.hitboxes.iter().find(|h| h.target == target)?;
        let Some(b) = b else {
            return Some(ha.clone());
        };
        match b.hitboxes.iter().find(|h| h.target == target) {
            Some(hb) => Some(ha.blend(hb, frac, (b.tick - a.tick) as f32)),
            None => Some(ha.clone()),
        }
    }

    /// Frames around `t` (which must be within the recorded range) and the blend factor.
    fn bracket(&self, t: f64) -> (&HitboxFrame, Option<&HitboxFrame>, f32) {
        // First frame with tick > t.
        let idx = self.frames.partition_point(|f| (f.tick as f64) <= t);
        let a = &self.frames[idx.saturating_sub(1)];
        match self.frames.get(idx) {
            Some(b) if idx > 0 => {
                let span = (b.tick - a.tick) as f64;
                let frac = ((t - a.tick as f64) / span).clamp(0.0, 1.0) as f32;
                (a, Some(b), frac)
            }
            _ => (a, None, 0.0),
        }
    }
}

impl HitboxHistory for LagCompHistory {
    fn rewind_hitboxes(&self, view_time_ticks: f64, max_rewind_ms: f32) -> Vec<Hitbox> {
        self.rewind(view_time_ticks, max_rewind_ms).hitboxes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::sim::{HitShape, PlayerId};

    fn player_at(id: u8, x: f32) -> Hitbox {
        Hitbox {
            target: HitTarget::Player(PlayerId(id)),
            shapes: vec![HitShape::capsule(
                HitPart::Body,
                Vec3::new(x, 0.5, 0.0),
                Vec3::new(x, 1.3, 0.0),
                0.4,
            )],
        }
    }

    /// 60 Hz, one frame per tick, player 0 moving +1 m per tick along x.
    fn history() -> LagCompHistory {
        let mut h = LagCompHistory::new(60, 500.0);
        for tick in 1..=40u32 {
            let mut boxes = vec![player_at(0, tick as f32)];
            if tick >= 35 {
                boxes.push(player_at(1, 100.0));
            }
            h.record(tick, boxes);
        }
        h
    }

    #[test]
    fn interpolates_between_ticks() {
        let h = history();
        let r = h.rewind(30.25, 250.0);
        assert_eq!(r.time_ticks, 30.25);
        let x = r.hitboxes[0].shapes[0].a.x;
        assert!((x - 30.25).abs() < 1e-4, "{x}");
        let exact = h.rewind(33.0, 250.0);
        assert!((exact.hitboxes[0].shapes[0].a.x - 33.0).abs() < 1e-6);
    }

    #[test]
    fn clamps_to_max_rewind_and_present() {
        let h = history();
        // 250 ms at 60 Hz = 15 ticks before the newest (40).
        let r = h.rewind(2.0, 250.0);
        assert!((r.time_ticks - 25.0).abs() < 1e-9, "{}", r.time_ticks);
        assert!((r.hitboxes[0].shapes[0].a.x - 25.0).abs() < 1e-4);
        let r = h.rewind(1000.0, 250.0);
        assert_eq!(r.time_ticks, 40.0);
        assert_eq!(h.rewind(f64::NAN, 250.0).time_ticks, 40.0);
    }

    #[test]
    fn capacity_bounds_history() {
        let h = history();
        // 500 ms at 60 Hz = 30 ticks (+2).
        assert_eq!(h.len(), 32);
        assert_eq!(h.oldest_tick(), Some(9));
        // A rewind past the history clamps to the oldest frame even with a huge max.
        assert_eq!(h.rewind(0.0, 10_000.0).time_ticks, 9.0);
    }

    #[test]
    fn targets_only_exist_while_recorded() {
        let h = history();
        assert_eq!(h.rewind(30.0, 250.0).hitboxes.len(), 1);
        assert_eq!(h.rewind(36.0, 250.0).hitboxes.len(), 2);
        let target = HitTarget::Player(PlayerId(1));
        assert!(h.rewind_target(target, 30.0, 250.0).is_none());
        assert!(h.rewind_target(target, 37.5, 250.0).is_some());
    }

    #[test]
    fn rewound_ray_hits_where_the_shooter_saw_the_target() {
        let h = history();
        // The target is at x=40 now, but the shooter saw it at x=30.
        let shooter = HitTarget::Player(PlayerId(9));
        let origin = Vec3::new(30.0, 1.0, -10.0);
        let r = h.rewind(30.0, 250.0);
        let (target, dist, part) = r.ray_hit(origin, Vec3::Z, 100.0, Some(shooter)).unwrap();
        assert_eq!(target, HitTarget::Player(PlayerId(0)));
        assert_eq!(part, HitPart::Body);
        assert!((dist - 9.6).abs() < 1e-3);
        let now = h.rewind(40.0, 250.0);
        assert!(now.ray_hit(origin, Vec3::Z, 100.0, Some(shooter)).is_none());
    }

    #[test]
    fn a_teleport_is_rewound_as_clients_show_it() {
        // Player 0 jumps 20 m between ticks 5 and 6 (a respawn, a blink): clients show it at
        // the old place until tick 6, so a rewind never puts it halfway.
        let mut h = LagCompHistory::new(60, 500.0);
        h.record(4, vec![player_at(0, 0.0)]);
        h.record(5, vec![player_at(0, 0.5)]);
        h.record(6, vec![player_at(0, 20.5)]);
        let x = |t: f64| h.rewind(t, 500.0).hitboxes[0].shapes[0].a.x;
        assert!((x(4.5) - 0.25).abs() < 1e-5, "walking: interpolated");
        assert_eq!(x(5.5), 0.5);
        assert_eq!(x(5.99), 0.5);
        assert_eq!(x(6.0), 20.5);
        let target = HitTarget::Player(PlayerId(0));
        assert_eq!(
            h.rewind_target(target, 5.5, 500.0).unwrap().shapes[0].a.x,
            0.5
        );
    }

    #[test]
    fn re_recording_a_tick_replaces_it() {
        let mut h = LagCompHistory::new(60, 100.0);
        h.record(5, vec![player_at(0, 1.0)]);
        h.record(6, vec![player_at(0, 2.0)]);
        h.record(6, vec![player_at(0, 3.0)]);
        assert_eq!(h.len(), 2);
        assert_eq!(h.rewind(6.0, 100.0).hitboxes[0].shapes[0].a.x, 3.0);
    }
}
