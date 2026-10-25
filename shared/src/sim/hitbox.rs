//! Hitboxes: spheres and capsules attached to players and enemies. Used for hit detection and
//! recorded per tick by the server's lag compensation history.
//!
//! **Hit assist**: attacks owned by players test against shapes inflated by [`HitPads`]
//! (`combat.hit_assist`, see [`crate::sim::SimWorld::hit_pads`]); the recorded / rewound shapes
//! stay plain and the padding is added at test time. A ray that hits the plain shapes keeps
//! that result (the padding never turns a headshot into a body hit); only a near miss falls
//! back to the padded shapes.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use super::PlayerId;
use crate::protocol::NetId;

/// Who a hitbox belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HitTarget {
    Player(PlayerId),
    /// Any replicated gameplay entity (enemies, prisms...).
    Entity(NetId),
}

impl HitTarget {
    pub fn player(self) -> Option<PlayerId> {
        match self {
            HitTarget::Player(p) => Some(p),
            HitTarget::Entity(_) => None,
        }
    }

    pub fn entity(self) -> Option<NetId> {
        match self {
            HitTarget::Entity(id) => Some(id),
            HitTarget::Player(_) => None,
        }
    }
}

/// Which part of the target a shape covers (damage multipliers are applied per part).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HitPart {
    #[default]
    Body,
    Head,
    /// Enemy weak point by index into its definition.
    Weak(u8),
}

/// A capsule from `a` to `b` with `radius` (a sphere when `a == b`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitShape {
    pub part: HitPart,
    pub a: Vec3,
    pub b: Vec3,
    pub radius: f32,
}

impl HitShape {
    pub fn sphere(part: HitPart, center: Vec3, radius: f32) -> Self {
        Self {
            part,
            a: center,
            b: center,
            radius,
        }
    }

    pub fn capsule(part: HitPart, a: Vec3, b: Vec3, radius: f32) -> Self {
        Self { part, a, b, radius }
    }

    pub fn center(&self) -> Vec3 {
        (self.a + self.b) * 0.5
    }

    pub fn lerp(&self, other: &HitShape, t: f32) -> HitShape {
        HitShape {
            part: self.part,
            a: self.a.lerp(other.a, t),
            b: self.b.lerp(other.b, t),
            radius: self.radius + (other.radius - self.radius) * t,
        }
    }

    /// Distance along the ray (`dir` normalised) to the first hit within `max_dist`. A ray
    /// starting inside the shape hits at distance 0.
    pub fn ray_hit(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<f32> {
        if self.distance_to_point(origin) <= 0.0 {
            return Some(0.0);
        }
        let t = if (self.b - self.a).length_squared() < 1e-12 {
            ray_sphere(origin, dir, self.a, self.radius)
        } else {
            ray_capsule(origin, dir, self.a, self.b, self.radius)
        }?;
        (0.0..=max_dist).contains(&t).then_some(t)
    }

    /// Signed distance from `p` to the shape surface (negative inside).
    pub fn distance_to_point(&self, p: Vec3) -> f32 {
        let ab = self.b - self.a;
        let len2 = ab.length_squared();
        let t = if len2 < 1e-12 {
            0.0
        } else {
            ((p - self.a).dot(ab) / len2).clamp(0.0, 1.0)
        };
        (p - (self.a + ab * t)).length() - self.radius
    }
}

fn ray_sphere(origin: Vec3, dir: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let oc = origin - center;
    let b = oc.dot(dir);
    let c = oc.length_squared() - radius * radius;
    let h = b * b - c;
    if h < 0.0 {
        return None;
    }
    let t = -b - h.sqrt();
    (t >= 0.0).then_some(t)
}

fn ray_capsule(origin: Vec3, dir: Vec3, pa: Vec3, pb: Vec3, radius: f32) -> Option<f32> {
    let ba = pb - pa;
    let oa = origin - pa;
    let baba = ba.dot(ba);
    let bard = ba.dot(dir);
    let baoa = ba.dot(oa);
    let rdoa = dir.dot(oa);
    let oaoa = oa.dot(oa);
    let a = baba - bard * bard;
    let b = baba * rdoa - baoa * bard;
    let c = baba * oaoa - baoa * baoa - radius * radius * baba;
    let h = b * b - a * c;
    let mut best: Option<f32> = None;
    if a.abs() > 1e-9 && h >= 0.0 {
        let t = (-b - h.sqrt()) / a;
        let y = baoa + t * bard;
        if y > 0.0 && y < baba && t >= 0.0 {
            best = Some(t);
        }
    }
    // End caps (also covers rays parallel to the axis).
    for cap in [pa, pb] {
        if let Some(t) = ray_sphere(origin, dir, cap, radius) {
            best = Some(best.map_or(t, |b: f32| b.min(t)));
        }
    }
    best
}

/// Extra radius for forgiving hit tests: `body` for [`HitPart::Body`] shapes, `weak` for heads
/// and weak points. [`HitPads::NONE`] = the plain shapes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HitPads {
    pub body: f32,
    pub weak: f32,
}

impl HitPads {
    pub const NONE: HitPads = HitPads {
        body: 0.0,
        weak: 0.0,
    };

    pub fn is_none(self) -> bool {
        self.body <= 0.0 && self.weak <= 0.0
    }

    /// Padding of a shape covering `part`.
    pub fn for_part(self, part: HitPart) -> f32 {
        match part {
            HitPart::Body => self.body,
            HitPart::Head | HitPart::Weak(_) => self.weak,
        }
    }
}

/// All hit shapes of one target at one point in time.
#[derive(Clone, Debug, PartialEq)]
pub struct Hitbox {
    pub target: HitTarget,
    pub shapes: Vec<HitShape>,
}

impl Hitbox {
    /// The same shapes inflated by `pads`.
    pub fn padded(&self, pads: HitPads) -> Hitbox {
        Hitbox {
            target: self.target,
            shapes: self
                .shapes
                .iter()
                .map(|s| HitShape {
                    radius: s.radius + pads.for_part(s.part).max(0.0),
                    ..*s
                })
                .collect(),
        }
    }

    /// [`Hitbox::ray_hit`] with hit assist: the plain shapes first, then (a near miss) the
    /// shapes inflated by `pads`.
    pub fn ray_hit_padded(
        &self,
        origin: Vec3,
        dir: Vec3,
        max_dist: f32,
        pads: HitPads,
    ) -> Option<(f32, HitPart)> {
        let plain = self.ray_hit(origin, dir, max_dist);
        if plain.is_some() || pads.is_none() {
            return plain;
        }
        self.padded(pads).ray_hit(origin, dir, max_dist)
    }

    /// [`Hitbox::sweep_sphere`] against the shapes inflated by `pads`.
    pub fn sweep_sphere_padded(
        &self,
        origin: Vec3,
        motion: Vec3,
        radius: f32,
        pads: HitPads,
    ) -> Option<(f32, HitPart)> {
        if pads.is_none() {
            return self.sweep_sphere(origin, motion, radius);
        }
        self.padded(pads).sweep_sphere(origin, motion, radius)
    }

    /// [`Hitbox::distance_to_point`] to the shapes inflated by `pads`.
    pub fn distance_to_point_padded(&self, p: Vec3, pads: HitPads) -> Option<(f32, HitPart)> {
        self.shapes
            .iter()
            .map(|s| (s.distance_to_point(p) - pads.for_part(s.part).max(0.0), s.part))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// Interpolates towards `other` (same target). Falls back to `self` if the shape layouts
    /// differ (e.g. a stance change that swapped shapes).
    pub fn lerp(&self, other: &Hitbox, t: f32) -> Hitbox {
        if self.shapes.len() != other.shapes.len() {
            return self.clone();
        }
        Hitbox {
            target: self.target,
            shapes: self
                .shapes
                .iter()
                .zip(&other.shapes)
                .map(|(a, b)| a.lerp(b, t))
                .collect(),
        }
    }

    /// Nearest hit along a ray: (distance, part).
    pub fn ray_hit(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<(f32, HitPart)> {
        self.shapes
            .iter()
            .filter_map(|s| s.ray_hit(origin, dir, max_dist).map(|t| (t, s.part)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// First contact of a sphere of `radius` moving from `origin` by `motion`: (fraction of
    /// the motion in `0..=1`, part). A sphere already touching hits at 0.
    pub fn sweep_sphere(&self, origin: Vec3, motion: Vec3, radius: f32) -> Option<(f32, HitPart)> {
        let len = motion.length();
        let dir = if len > 1e-6 { motion / len } else { Vec3::X };
        self.shapes
            .iter()
            .filter_map(|s| {
                let inflated = HitShape {
                    radius: s.radius + radius,
                    ..*s
                };
                inflated
                    .ray_hit(origin, dir, len)
                    .map(|t| (if len > 1e-6 { t / len } else { 0.0 }, s.part))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// Signed distance from `p` to the nearest shape surface and that shape's part.
    pub fn distance_to_point(&self, p: Vec3) -> Option<(f32, HitPart)> {
        self.shapes
            .iter()
            .map(|s| (s.distance_to_point(p), s.part))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// Centre of the first `Body` shape (or of the first shape): the point to aim at.
    pub fn center(&self) -> Option<Vec3> {
        self.shapes
            .iter()
            .find(|s| s.part == HitPart::Body)
            .or(self.shapes.first())
            .map(HitShape::center)
    }

    /// Centre of the shape covering `part`, if any.
    pub fn part_center(&self, part: HitPart) -> Option<Vec3> {
        self.shapes
            .iter()
            .find(|s| s.part == part)
            .map(HitShape::center)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_capsule_body_and_caps() {
        let s = HitShape::capsule(
            HitPart::Body,
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
            0.5,
        );
        // Straight at the body.
        let t = s
            .ray_hit(Vec3::new(-5.0, 1.0, 0.0), Vec3::X, 100.0)
            .unwrap();
        assert!((t - 4.5).abs() < 1e-4, "{t}");
        // From above onto the top cap.
        let t = s
            .ray_hit(Vec3::new(0.0, 10.0, 0.0), Vec3::NEG_Y, 100.0)
            .unwrap();
        assert!((t - 7.5).abs() < 1e-4, "{t}");
        // Miss.
        assert!(
            s.ray_hit(Vec3::new(-5.0, 1.0, 2.0), Vec3::X, 100.0)
                .is_none()
        );
        // Behind the origin.
        assert!(
            s.ray_hit(Vec3::new(5.0, 1.0, 0.0), Vec3::X, 100.0)
                .is_none()
        );
        // Out of range.
        assert!(s.ray_hit(Vec3::new(-5.0, 1.0, 0.0), Vec3::X, 3.0).is_none());
        // Inside.
        assert_eq!(s.ray_hit(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 3.0), Some(0.0));
    }

    #[test]
    fn ray_hits_sphere() {
        let s = HitShape::sphere(HitPart::Head, Vec3::new(0.0, 0.0, -10.0), 1.0);
        let t = s.ray_hit(Vec3::ZERO, Vec3::NEG_Z, 100.0).unwrap();
        assert!((t - 9.0).abs() < 1e-4);
    }

    #[test]
    fn padding_catches_near_misses_without_stealing_hits() {
        let h = Hitbox {
            target: HitTarget::Player(PlayerId(1)),
            shapes: vec![
                HitShape::capsule(HitPart::Body, Vec3::ZERO, Vec3::new(0.0, 1.2, 0.0), 0.4),
                HitShape::sphere(HitPart::Head, Vec3::new(0.0, 1.6, 0.0), 0.25),
            ],
        };
        let pads = HitPads {
            body: 0.2,
            weak: 0.1,
        };
        // 0.15 m past the body's edge: a miss, caught by the 0.2 m pad.
        let origin = Vec3::new(0.55, 0.6, 10.0);
        assert!(h.ray_hit(origin, Vec3::NEG_Z, 100.0).is_none());
        let (_, part) = h.ray_hit_padded(origin, Vec3::NEG_Z, 100.0, pads).unwrap();
        assert_eq!(part, HitPart::Body);
        // Beyond the pad: still a miss.
        let far = Vec3::new(0.65, 0.6, 10.0);
        assert!(h.ray_hit_padded(far, Vec3::NEG_Z, 100.0, pads).is_none());
        // A plain head hit stays a head hit (the padded body reaches this high too).
        let head = Vec3::new(0.0, 1.7, 10.0);
        assert_eq!(h.ray_hit(head, Vec3::NEG_Z, 100.0).unwrap().1, HitPart::Head);
        assert_eq!(
            h.ray_hit_padded(head, Vec3::NEG_Z, 100.0, pads).unwrap().1,
            HitPart::Head
        );
        // No pads: identical to the plain test.
        assert!(h.ray_hit_padded(origin, Vec3::NEG_Z, 100.0, HitPads::NONE).is_none());
        // Distances and sweeps shrink by the pad.
        let p = Vec3::new(1.0, 0.6, 0.0);
        let (plain, _) = h.distance_to_point(p).unwrap();
        let (padded, _) = h.distance_to_point_padded(p, pads).unwrap();
        assert!((plain - padded - 0.2).abs() < 1e-5);
        let motion = Vec3::new(0.0, 0.0, -20.0);
        assert!(h.sweep_sphere(origin, motion, 0.05).is_none());
        assert!(h.sweep_sphere_padded(origin, motion, 0.05, pads).is_some());
    }
}
