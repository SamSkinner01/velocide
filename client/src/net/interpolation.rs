//! Snapshot buffer and entity interpolation.
//!
//! Decoded snapshot frames are kept by tick. They serve two purposes: delta baselines (the
//! server encodes against the newest tick we acknowledged) and the interpolation buffer.
//! Remote entities are drawn at a render time slightly in the past
//! ([`super::NetClient::render_tick`]), interpolated between the two frames around it; if
//! frames are late we extrapolate along the velocity for at most `max_extrapolation_ms`.

use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use shared::protocol::{EntityState, NetFrame, NetId};

/// Frames older than this (ticks behind the newest) are discarded. Must cover the server's
/// baseline age limit (`server::MAX_BASELINE_AGE_TICKS`).
pub const FRAME_RETENTION_TICKS: u32 = 180;
/// Entities moving faster than this between two frames (m per tick) are treated as teleported
/// and not interpolated.
pub const TELEPORT_SPEED: f32 = 3.0;

/// How a sample was produced.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SampleInfo {
    /// Ticks of the frames used (`to` is `None` when extrapolating / clamping).
    pub from: Option<u32>,
    pub to: Option<u32>,
    /// Ticks extrapolated past the newest frame.
    pub extrapolated_ticks: f64,
}

/// Decoded frames by tick.
#[derive(Clone, Debug)]
pub struct SnapshotBuffer {
    frames: BTreeMap<u32, Arc<NetFrame>>,
    /// Frames more than this many ticks behind the newest are dropped.
    retention: u32,
}

impl Default for SnapshotBuffer {
    fn default() -> Self {
        Self::with_retention(FRAME_RETENTION_TICKS)
    }
}

impl SnapshotBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// A buffer keeping `ticks` of history instead of [`FRAME_RETENTION_TICKS`] (the play of
    /// the game's recordings, `ingame::replay`; `u32::MAX` = prune only with `retain_from`).
    pub fn with_retention(ticks: u32) -> Self {
        Self {
            frames: BTreeMap::new(),
            retention: ticks,
        }
    }

    /// Stores a frame and drops frames more than the retention (default
    /// [`FRAME_RETENTION_TICKS`]) behind the newest. A frame that old itself is refused
    /// (returns false): late or replayed snapshots can't grow the buffer.
    pub fn insert(&mut self, tick: u32, frame: Arc<NetFrame>) -> bool {
        if self.is_stale(tick) {
            return false;
        }
        self.frames.insert(tick, frame);
        self.prune();
        true
    }

    /// Is `tick` too far behind the newest frame to be kept?
    pub fn is_stale(&self, tick: u32) -> bool {
        self.latest()
            .is_some_and(|(latest, _)| tick < latest.saturating_sub(self.retention))
    }

    /// Frames from tick `from` to `to` (inclusive), oldest first.
    pub fn range(&self, from: u32, to: u32) -> impl Iterator<Item = (u32, &Arc<NetFrame>)> {
        let to = to.max(from);
        self.frames.range(from..=to).map(|(t, f)| (*t, f))
    }

    pub fn oldest(&self) -> Option<(u32, &Arc<NetFrame>)> {
        self.frames.iter().next().map(|(t, f)| (*t, f))
    }

    /// Drops every frame older than `tick`.
    pub fn retain_from(&mut self, tick: u32) {
        if self.oldest().is_some_and(|(oldest, _)| oldest < tick) {
            self.frames = self.frames.split_off(&tick);
        }
    }

    pub fn get(&self, tick: u32) -> Option<&Arc<NetFrame>> {
        self.frames.get(&tick)
    }

    pub fn contains(&self, tick: u32) -> bool {
        self.frames.contains_key(&tick)
    }

    pub fn latest(&self) -> Option<(u32, &Arc<NetFrame>)> {
        self.frames.iter().next_back().map(|(t, f)| (*t, f))
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }

    /// Drops frames more than the retention behind the newest.
    pub fn prune(&mut self) {
        if let Some((latest, _)) = self.latest() {
            let min = latest.saturating_sub(self.retention);
            if self.oldest().is_some_and(|(oldest, _)| oldest < min) {
                self.frames = self.frames.split_off(&min);
            }
        }
    }

    /// Entity states at `render_tick` (fractional), skipping `exclude`. `tick_dt` is seconds
    /// per tick (for extrapolation).
    pub fn sample(
        &self,
        render_tick: f64,
        max_extrapolation_ticks: f64,
        tick_dt: f32,
        exclude: Option<NetId>,
        out: &mut Vec<EntityState>,
    ) -> SampleInfo {
        out.clear();
        let mut info = SampleInfo::default();
        let before = self
            .frames
            .range(..=floor_tick(render_tick))
            .next_back()
            .filter(|(t, _)| **t as f64 <= render_tick);
        let after = self
            .frames
            .range(floor_tick(render_tick)..)
            .find(|(t, _)| **t as f64 > render_tick);

        match (before, after) {
            (Some((&ta, a)), Some((&tb, b))) => {
                info.from = Some(ta);
                info.to = Some(tb);
                let span = (tb - ta) as f64;
                let t = ((render_tick - ta as f64) / span).clamp(0.0, 1.0) as f32;
                for (id, ea) in &a.entities {
                    if Some(*id) == exclude {
                        continue;
                    }
                    let sa = ea.dequantize(*id);
                    let state = match b.entities.get(id) {
                        Some(eb) => {
                            let sb = eb.dequantize(*id);
                            let speed = sa.position.distance(sb.position) / span as f32;
                            if speed > TELEPORT_SPEED || sa.kind != sb.kind {
                                if t < 1.0 { sa } else { sb }
                            } else {
                                lerp_state(&sa, &sb, t)
                            }
                        }
                        // Removed in `b`: keep showing it until then.
                        None => sa,
                    };
                    out.push(state);
                }
            }
            (Some((&ta, a)), None) => {
                info.from = Some(ta);
                let ahead = (render_tick - ta as f64).max(0.0);
                let extra = ahead.min(max_extrapolation_ticks.max(0.0));
                info.extrapolated_ticks = extra;
                let secs = extra as f32 * tick_dt;
                for (id, e) in &a.entities {
                    if Some(*id) == exclude {
                        continue;
                    }
                    let mut s = e.dequantize(*id);
                    s.position += s.velocity * secs;
                    out.push(s);
                }
            }
            (None, Some((&tb, b))) => {
                info.to = Some(tb);
                out.extend(b.states().filter(|s| Some(s.id) != exclude));
            }
            (None, None) => {}
        }
        info
    }
}

fn floor_tick(t: f64) -> u32 {
    if t <= 0.0 {
        0
    } else if t >= u32::MAX as f64 {
        u32::MAX
    } else {
        t.floor() as u32
    }
}

/// Shortest-arc angle interpolation.
pub fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let mut d = (b - a).rem_euclid(TAU);
    if d > PI {
        d -= TAU;
    }
    a + d * t
}

/// Interpolates continuous fields; discrete fields come from the nearer frame.
pub fn lerp_state(a: &EntityState, b: &EntityState, t: f32) -> EntityState {
    let near = if t < 0.5 { a } else { b };
    EntityState {
        position: a.position.lerp(b.position, t),
        velocity: a.velocity.lerp(b.velocity, t),
        yaw: lerp_angle(a.yaw, b.yaw, t),
        pitch: a.pitch + (b.pitch - a.pitch) * t,
        ..*near
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use shared::protocol::kind;

    fn frame(x: f32, yaw: f32) -> Arc<NetFrame> {
        let mut f = NetFrame::new();
        f.insert(&EntityState {
            id: NetId(1),
            kind: kind::PLAYER,
            position: Vec3::new(x, 0.0, 0.0),
            velocity: Vec3::new(30.0, 0.0, 0.0),
            yaw,
            ..Default::default()
        });
        Arc::new(f)
    }

    #[test]
    fn interpolates_and_extrapolates() {
        let mut buf = SnapshotBuffer::new();
        buf.insert(10, frame(0.0, 0.1));
        buf.insert(12, frame(1.0, TAU - 0.1));
        let mut out = Vec::new();
        let info = buf.sample(11.0, 6.0, 1.0 / 60.0, None, &mut out);
        assert_eq!((info.from, info.to), (Some(10), Some(12)));
        assert!((out[0].position.x - 0.5).abs() < 0.01);
        // Yaw goes the short way through 0.
        assert!(
            out[0].yaw.abs() < 0.01 || (out[0].yaw - TAU).abs() < 0.01,
            "{}",
            out[0].yaw
        );

        // 3 ticks past the newest frame at 30 m/s = 1.5 m.
        let info = buf.sample(15.0, 6.0, 1.0 / 60.0, None, &mut out);
        assert_eq!(info.extrapolated_ticks, 3.0);
        assert!((out[0].position.x - 2.5).abs() < 0.01);
        // Extrapolation is capped.
        let info = buf.sample(100.0, 6.0, 1.0 / 60.0, None, &mut out);
        assert_eq!(info.extrapolated_ticks, 6.0);
        assert!((out[0].position.x - 4.0).abs() < 0.01);

        buf.sample(11.0, 6.0, 1.0 / 60.0, Some(NetId(1)), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn old_frames_are_refused_and_pruned_on_every_insert() {
        let mut buf = SnapshotBuffer::new();
        assert!(buf.insert(5000, frame(0.0, 0.0)));
        // Late / replayed snapshots used to be stored (pruning only ran when a newer tick
        // arrived), so the buffer could grow without bound.
        for tick in (0..5000).rev() {
            buf.insert(tick, frame(0.0, 0.0));
            assert!(buf.len() <= FRAME_RETENTION_TICKS as usize + 1);
        }
        assert_eq!(buf.len(), FRAME_RETENTION_TICKS as usize + 1);
        assert!(buf.is_stale(5000 - FRAME_RETENTION_TICKS - 1));
        assert!(!buf.insert(5000 - FRAME_RETENTION_TICKS - 1, frame(0.0, 0.0)));
        assert!(buf.contains(5000 - FRAME_RETENTION_TICKS));
        // A newer frame prunes right away.
        assert!(buf.insert(5100, frame(0.0, 0.0)));
        // 4920..=5000 and 5100.
        assert_eq!(buf.len(), FRAME_RETENTION_TICKS as usize + 1 - 100 + 1);
        assert!(!buf.contains(5100 - FRAME_RETENTION_TICKS - 1));
    }

    #[test]
    fn teleports_are_not_interpolated() {
        let mut buf = SnapshotBuffer::new();
        buf.insert(10, frame(0.0, 0.0));
        buf.insert(12, frame(50.0, 0.0));
        let mut out = Vec::new();
        buf.sample(11.0, 6.0, 1.0 / 60.0, None, &mut out);
        assert_eq!(out[0].position.x, 0.0);
    }

    #[test]
    fn prune_keeps_recent_frames() {
        let mut buf = SnapshotBuffer::new();
        for t in 0..400 {
            buf.insert(t, frame(0.0, 0.0));
        }
        buf.prune();
        assert_eq!(buf.len(), FRAME_RETENTION_TICKS as usize + 1);
        assert!(buf.get(399).is_some() && buf.get(399 - FRAME_RETENTION_TICKS).is_some());
    }
}
