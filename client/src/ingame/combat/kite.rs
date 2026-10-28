//! The Kite (docs/rework.md §1, Explosive, area **client-b**): an ember kite steered with the
//! crosshair while ALT is held, bursting on contact.
//!
//! * **In flight** ([`kite_transform`], [`draw`]): `ModelId::Kite` nose first along its
//!   flight, banking into its turns (the roll follows how fast its heading swings), its ember
//!   heart pulsing; `Sfx::KiteLaunch` as it leaves, `Sfx::KiteFlutter` while it flies.
//! * **The steering trail** ([`Trail`], [`trail_effects`]): a ribbon of its path over the last
//!   half second in the ability's rose, brighter while it is being steered, embers falling off
//!   it; the turns stay drawn in the air so its owner and its targets both see where it is
//!   going.
//! * **The burst** is its explosion (`Fx::Explosion` with the Kite's attack): the arsenal's
//!   blast in rose and ember.

use std::collections::VecDeque;

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::DrawItem;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::super::effects::{Effects, FxTex, Particle};
use super::Rng;
use super::fx::{Beams, FxCtx};
use crate::models::gear::{KITE_COLOR, KITE_HEART};
use crate::models::{ModelId, ModelLibrary, Tint};

/// Drawn this much larger than life.
pub const SCALE: f32 = 1.4;
/// The ribbon samples a point this often and keeps this many.
const SAMPLE: f32 = 0.025;
const POINTS: usize = 22;
/// The most it banks (radians).
const MAX_BANK: f32 = 1.0;

fn rose(k: f32, a: f32) -> Vec4 {
    (KITE_COLOR.truncate() * k).extend(a)
}

/// The kite's root at `position` flying along `velocity`, rolled `bank` radians (+ = right
/// wing down, turning right).
pub fn kite_transform(position: Vec3, velocity: Vec3, bank: f32) -> Mat4 {
    let dir = velocity.normalize_or(Vec3::NEG_Z);
    let face = Quat::from_rotation_arc(Vec3::NEG_Z, dir);
    // Keep the sail level about the flight before the bank (no twist from the arc).
    let up = face * Vec3::Y;
    let level_up = (Vec3::Y - dir * dir.dot(Vec3::Y)).normalize_or(up);
    let twist = Quat::from_rotation_arc(up, level_up);
    let bank = if bank.is_finite() {
        bank.clamp(-MAX_BANK, MAX_BANK)
    } else {
        0.0
    };
    let roll = Quat::from_axis_angle(dir, bank);
    Mat4::from_scale_rotation_translation(Vec3::splat(SCALE), roll * twist * face, position)
}

/// Draws the kite (`pulse` 0..1: its heart's flicker).
pub fn draw(
    lib: &ModelLibrary,
    position: Vec3,
    velocity: Vec3,
    bank: f32,
    pulse: f32,
    draws: &mut Vec<DrawItem>,
) {
    let mut pose = lib.get(ModelId::Kite).rest_pose();
    let g = 1.2 + 0.8 * pulse.clamp(0.0, 1.0);
    pose.glow = Vec4::new(g, g * 0.85, g * 0.75, 1.0);
    lib.get(ModelId::Kite).draw(
        &pose,
        kite_transform(position, velocity, bank),
        &Tint::default(),
        draws,
    );
}

/// The kite's recent path and how it is banking (keep one per kite in flight).
#[derive(Clone, Debug, Default)]
pub struct Trail {
    points: VecDeque<Vec3>,
    since: f32,
    heading: Option<Vec3>,
    /// Smoothed bank (radians).
    pub bank: f32,
    /// Seconds to the next flutter.
    flutter: f32,
}

impl Trail {
    /// Follows the kite at `position` flying `velocity` for `dt` seconds.
    pub fn follow(&mut self, position: Vec3, velocity: Vec3, dt: f32) {
        self.since += dt;
        if self.points.is_empty() || self.since >= SAMPLE {
            self.since = 0.0;
            self.points.push_front(position);
            self.points.truncate(POINTS);
        } else if let Some(p) = self.points.front_mut() {
            *p = position;
        }
        // Bank into the turn: the heading's swing about the vertical (+ = turning right).
        let flat = Vec3::new(velocity.x, 0.0, velocity.z).normalize_or(Vec3::ZERO);
        if let Some(h) = self.heading
            && dt > 0.0
            && flat != Vec3::ZERO
        {
            let turn = h.cross(flat).y.clamp(-1.0, 1.0).asin() / dt;
            let target = (-turn * 0.35).clamp(-MAX_BANK, MAX_BANK);
            self.bank += (target - self.bank) * (1.0 - (-8.0 * dt).exp());
        }
        if flat != Vec3::ZERO {
            self.heading = Some(flat);
        }
    }

    /// Whether the flutter is due (`dt` passed), restarting its timer.
    pub fn flutter_due(&mut self, dt: f32) -> bool {
        self.flutter -= dt;
        if self.flutter <= 0.0 {
            self.flutter = 0.55;
            true
        } else {
            false
        }
    }

    pub fn points(&self) -> impl Iterator<Item = &Vec3> {
        self.points.iter()
    }
}

/// The ribbon behind a kite this frame and the embers it sheds (`steered`: held on the
/// crosshair, the ribbon brighter).
pub fn trail_effects(
    effects: &mut Effects,
    beams: &mut Beams,
    rng: &mut Rng,
    trail: &Trail,
    heart: Vec3,
    steered: bool,
    dt: f32,
) {
    let pts: Vec<Vec3> = std::iter::once(heart)
        .chain(trail.points().copied())
        .collect();
    let n = pts.len().max(2) as f32;
    let k = if steered { 1.0 } else { 0.6 };
    for (i, w) in pts.windows(2).enumerate() {
        let fade = 1.0 - i as f32 / n;
        beams.frame_beam(w[0], w[1], 0.09 * fade + 0.02, rose(1.6 * k, 0.75 * fade));
        beams.frame_beam(w[0], w[1], 0.03, Vec4::new(1.8, 1.4, 1.0, 0.6 * fade * k));
    }
    if rng.unit() < dt * 40.0 {
        effects.spawn(Particle {
            position: heart + rng.dir() * 0.1,
            velocity: Vec3::new(0.0, -1.0, 0.0) + rng.dir() * 0.5,
            gravity: 2.0,
            drag: 1.0,
            max_life: rng.range(0.4, 0.7),
            size: Vec2::splat(0.06),
            end_scale: 0.4,
            color: Vec4::new(1.0, 0.65, 0.3, 1.0),
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
    effects.billboard(heart, 0.7, rose(1.3, 0.55), FxTex::Glow, 0.0);
}

/// Your steered kite's aim this frame: a small turning rose diamond where your crosshair meets
/// the level (up to 60 m out), the point the kite is turning towards.
pub fn steer_marker(
    beams: &mut Beams,
    world: Option<&engine::physics::CollisionWorld>,
    eye: Vec3,
    look: Vec3,
    time: f64,
) {
    let look = look.normalize_or(Vec3::NEG_Z);
    let reach = 60.0;
    let at = world
        .and_then(|w| w.cast_ray(eye, look, reach, engine::physics::MASK_SHOTS))
        .map_or(eye + look * reach, |h| h.point - look * 0.1);
    let r = 0.12 * eye.distance(at).max(2.0) / 10.0 + 0.1;
    let spin = (time * 2.0) as f32;
    let up = (Vec3::Y - look * look.dot(Vec3::Y)).normalize_or(Vec3::X);
    let right = look.cross(up).normalize_or(Vec3::X);
    let corner = |k: usize| {
        let a = spin + k as f32 * std::f32::consts::FRAC_PI_2;
        at + (right * a.cos() + up * a.sin()) * r
    };
    for k in 0..4 {
        beams.frame_beam(corner(k), corner(k + 1), 0.025 * r / 0.2, rose(1.8, 0.85));
    }
}

/// Where the heart of a kite drawn with [`kite_transform`] is.
pub fn heart(position: Vec3, velocity: Vec3, bank: f32) -> Vec3 {
    kite_transform(position, velocity, bank).transform_point3(KITE_HEART)
}

/// What the kite's burst adds to the arsenal's blast (`arsenal::Blast::Kite`): a rose shock
/// ring and scraps of burning paper flung out.
pub fn burst_scraps(ctx: &mut FxCtx, at: Vec3, radius: f32) {
    ctx.beams.ring(
        at + Vec3::Y * 0.2,
        Vec3::Y,
        0.5,
        radius * 1.1,
        0.12,
        rose(1.8, 0.9),
        0.35,
    );
    for k in 0..18 {
        let a = k as f32 / 18.0 * std::f32::consts::TAU;
        let d = Vec3::new(a.cos(), 0.25 + 0.3 * ctx.rng.unit(), a.sin());
        ctx.effects.spawn(Particle {
            position: at,
            velocity: d * radius * ctx.rng.range(1.4, 2.0),
            gravity: 4.0,
            drag: 1.4,
            max_life: ctx.rng.range(0.5, 0.9),
            size: Vec2::splat(if k % 3 == 0 { 0.16 } else { 0.08 }),
            end_scale: 0.5,
            color: if k % 3 == 0 {
                rose(1.4, 1.0)
            } else {
                Vec4::new(1.6, 0.9, 0.4, 1.0)
            },
            texture: if k % 3 == 0 {
                FxTex::Shard
            } else {
                FxTex::Spark
            },
            additive: true,
            rotation: ctx.rng.angle(),
            spin: ctx.rng.range(-8.0, 8.0),
            ..Default::default()
        });
    }
}

/// Launched at `at`: a puff of embers, the sound.
pub fn launch(effects: &mut Effects, audio: &mut Audio, at: Vec3, own: bool) {
    effects.glow(at, rose(1.4, 0.8), 0.9, 0.15);
    if own {
        audio.play_2d(Sfx::KiteLaunch);
    } else {
        audio.play(Sfx::KiteLaunch, PlayParams::at(at));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kite_flies_nose_first_and_banks_into_turns() {
        let m = kite_transform(Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), 0.0);
        let nose = m.transform_vector3(Vec3::NEG_Z).normalize();
        assert!(nose.x > 0.99, "{nose}");
        let up = m.transform_vector3(Vec3::Y).normalize();
        assert!(up.y > 0.99, "level: {up}");
        // Turning right (heading swinging from -Z towards +X): it banks right.
        let mut t = Trail::default();
        for i in 0..30 {
            let a = i as f32 * 0.05;
            t.follow(
                Vec3::ZERO,
                Vec3::new(a.sin(), 0.0, -a.cos()) * 16.0,
                1.0 / 60.0,
            );
        }
        assert!(t.bank > 0.2, "{}", t.bank);
        assert!(t.points().count() >= 2);
        assert!(kite_transform(Vec3::ZERO, Vec3::NAN, f32::NAN).is_finite());
    }
}
