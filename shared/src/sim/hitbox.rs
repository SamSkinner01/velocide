//! Hitboxes: spheres and capsules attached to players and (later) enemies. Used for hit
//! detection and recorded per tick by the server's lag compensation history.

use glam::Vec3;

use super::PlayerId;
use crate::protocol::NetId;

/// Who a hitbox belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HitTarget {
    Player(PlayerId),
    /// Any replicated gameplay entity (enemies, prisms...).
    Entity(NetId),
}

/// Which part of the target a shape covers (damage multipliers are applied per part).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HitPart {
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

/// All hit shapes of one target at one point in time.
#[derive(Clone, Debug, PartialEq)]
pub struct Hitbox {
    pub target: HitTarget,
    pub shapes: Vec<HitShape>,
}

impl Hitbox {
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
}
