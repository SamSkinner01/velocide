//! Capsule collision for the character controller: collide-and-slide, step-up, ground probing
//! and snapping, depenetration and wall probing.
//!
//! Everything here is a pure function of its arguments (plus the static world), so the
//! controller stays deterministic. All sweeps are continuous (`CollisionWorld::cast_capsule`),
//! so nothing tunnels regardless of speed.
//!
//! GJK-based sweeps are occasionally imprecise when they start almost touching a surface (the
//! time of impact can overshoot by centimetres, normals can be off). Every sweep result is
//! therefore validated with a closest-point query at the stopping position ([`Ctx::sweep`]),
//! which also gives the exact surface normal.

use engine::physics::{CollisionWorld, ContactInfo, MASK_MOVEMENT};
use glam::Vec3;

/// Distance the capsule is kept away from surfaces by every sweep.
pub const SKIN: f32 = 0.01;
/// How far below the feet we look for ground when not snapping down steps.
pub const GROUND_PROBE: f32 = 0.05;
/// After touching a surface the capsule is pushed this far back out along its normal, so the
/// next (tangential) sweep can't report a zero-distance hit from floating-point noise.
const NUDGE: f32 = 1e-4;
const MAX_SLIDE_ITERATIONS: usize = 6;
const MAX_PLANES: usize = 6;
/// Surfaces whose normal points down more than this are ceilings.
pub const CEILING_NORMAL_Y: f32 = -0.7;
/// Surfaces with `|normal.y|` below this count as walls (wall jumps, wall cling).
pub const WALL_NORMAL_Y: f32 = 0.35;
/// Contacts closer than this are pushed out by [`depenetrate`]. Besides real penetration this
/// catches *exact* contact (e.g. spawning with the feet exactly on the floor), from which
/// sweeps are unreliable.
const TOUCH_DISTANCE: f32 = SKIN * 0.25;
/// Contact normals are undefined at exactly zero distance, so contact probes use a capsule
/// this much thinner (and subtract it from the reported distance).
const PROBE_SHRINK: f32 = 0.005;

/// Capsule dimensions for one stance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub height: f32,
    pub radius: f32,
}

impl Body {
    pub fn center(&self, feet: Vec3) -> Vec3 {
        feet + Vec3::Y * (self.height * 0.5)
    }

    pub fn half_height(&self) -> f32 {
        (self.height * 0.5 - self.radius).max(0.0)
    }
}

/// The world plus the walkable-slope threshold.
pub struct Ctx<'a> {
    pub world: &'a CollisionWorld,
    /// Minimum `normal.y` of walkable ground (cos of the max slope).
    pub min_walk_y: f32,
}

/// A validated sweep hit.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    /// Fraction of the motion travelled.
    pub fraction: f32,
    /// Feet position where the capsule stopped (about `SKIN` from the surface).
    pub pos: Vec3,
    /// Exact surface normal (closest-point normal at the stopping position).
    pub normal: Vec3,
    /// Contact point on the capsule.
    pub point: Vec3,
}

/// What happened during a [`slide_move`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SlideResult {
    /// Hit something that isn't walkable ground and isn't a ceiling.
    pub blocked: bool,
    pub hit_ceiling: bool,
    /// Largest velocity component removed by a collision this move (for impact sounds).
    pub impact_speed: f32,
    /// Vertical offset applied by a step-up (0 if none).
    pub stepped: f32,
}

impl Ctx<'_> {
    /// Sweeps the capsule from feet position `feet` along `motion`.
    pub fn sweep(&self, body: Body, feet: Vec3, motion: Vec3) -> Option<Hit> {
        let raw = self.world.cast_capsule(
            body.center(feet),
            body.half_height(),
            body.radius,
            motion,
            SKIN,
            MASK_MOVEMENT,
        )?;
        let mut hit = Hit {
            fraction: raw.fraction,
            pos: feet + motion * raw.fraction,
            normal: raw.normal,
            point: raw.point,
        };
        if let Some(c) = self
            .contacts(body, hit.pos, SKIN * 2.0)
            .iter()
            .find(|c| c.collider == raw.collider)
        {
            hit.normal = c.normal;
            if c.distance < SKIN {
                // Stopped inside the skin band (the sweep started there, or its time of impact
                // overshot): restore exactly the skin distance. Resting inside the band would
                // make every later sweep with the slightest component into the surface report
                // a zero-distance hit and eat the motion.
                hit.pos += c.normal * (SKIN - c.distance);
            }
        }
        Some(hit)
    }

    /// Contacts within `prediction` of the capsule, probed with a slightly thinner capsule so
    /// normals stay reliable even at exact contact. Distances are those of the real capsule.
    pub fn contacts(&self, body: Body, feet: Vec3, prediction: f32) -> Vec<ContactInfo> {
        let mut contacts = self.world.capsule_contacts(
            body.center(feet),
            body.half_height(),
            body.radius - PROBE_SHRINK,
            prediction + PROBE_SHRINK,
            MASK_MOVEMENT,
        );
        for c in &mut contacts {
            c.distance -= PROBE_SHRINK;
        }
        contacts
    }

    pub fn walkable(&self, normal: Vec3) -> bool {
        normal.y >= self.min_walk_y
    }
}

fn horizontal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// Re-aims a motion so it runs along a ground plane with the same horizontal displacement
/// (walking up or down a slope doesn't change horizontal speed).
pub fn along_ground(motion: Vec3, normal: Vec3) -> Vec3 {
    if normal.y < 0.1 {
        return motion;
    }
    let y = -(normal.x * motion.x + normal.z * motion.z) / normal.y;
    Vec3::new(motion.x, y, motion.z)
}

/// Removes the component of `v` going into the plane (keeps components moving away).
fn clip(v: Vec3, n: Vec3) -> Vec3 {
    let d = v.dot(n);
    if d < 0.0 { v - n * d } else { v }
}

/// Clips `v` so it doesn't go into any of `planes` (Quake-style: one plane, else the crease
/// of two planes, else stop).
fn clip_planes(v: Vec3, planes: &[Vec3]) -> Vec3 {
    let tolerance = -1e-5 * (v.length() + 1.0);
    'planes: for (i, &p) in planes.iter().enumerate() {
        let c = clip(v, p);
        for (j, &q) in planes.iter().enumerate() {
            if i != j && c.dot(q) < tolerance {
                continue 'planes;
            }
        }
        return c;
    }
    for i in 0..planes.len() {
        for j in i + 1..planes.len() {
            let dir = planes[i].cross(planes[j]).normalize_or_zero();
            if dir == Vec3::ZERO {
                continue;
            }
            let c = dir * v.dot(dir);
            if planes.iter().all(|&q| c.dot(q) >= tolerance) {
                return c;
            }
        }
    }
    if planes.is_empty() { v } else { Vec3::ZERO }
}

/// Collide-and-slide. Moves `pos` by `motion`, sliding along whatever it hits, and clips `vel`
/// against the surfaces touched.
///
/// With `grounded`, walls are treated as vertical (no climbing them or being pushed into the
/// floor) and walkable surfaces are followed keeping the horizontal motion. An airborne move
/// that lands on walkable ground keeps its horizontal velocity and continues as grounded.
pub fn slide_move(
    ctx: &Ctx,
    body: Body,
    pos: &mut Vec3,
    vel: &mut Vec3,
    motion: Vec3,
    grounded: bool,
) -> SlideResult {
    let mut res = SlideResult::default();
    let mut grounded = grounded;
    let mut planes = [Vec3::ZERO; MAX_PLANES];
    let mut plane_count = 0;
    let mut remaining = motion;
    for _ in 0..MAX_SLIDE_ITERATIONS {
        if remaining.length_squared() < 1e-12 {
            break;
        }
        let Some(hit) = ctx.sweep(body, *pos, remaining) else {
            *pos += remaining;
            break;
        };
        *pos = hit.pos;
        let left = remaining * (1.0 - hit.fraction);
        let n = hit.normal;

        if ctx.walkable(n) && (grounded || vel.y <= 0.0) {
            // Grounded: a bump or the start of a ramp, follow it. Airborne: a landing; keep
            // all horizontal speed (no loss landing on ramps) and run along the ground for
            // the rest of the move.
            if !grounded {
                vel.y = 0.0;
                grounded = true;
            }
            *pos += n * NUDGE;
            remaining = clip_planes(along_ground(left, n), &planes[..plane_count]);
            continue;
        }

        let is_ceiling = n.y < CEILING_NORMAL_Y;
        if is_ceiling && vel.y > 0.0 {
            res.hit_ceiling = true;
        }
        let plane = if grounded && !is_ceiling {
            horizontal(n).normalize_or(n)
        } else {
            n
        };
        if !is_ceiling && !ctx.walkable(n) {
            res.blocked = true;
        }
        *pos += plane * NUDGE;
        if plane_count < MAX_PLANES {
            planes[plane_count] = plane;
            plane_count += 1;
        }
        let into = vel.dot(plane);
        if into < 0.0 {
            res.impact_speed = res.impact_speed.max(-into);
        }
        *vel = clip_planes(*vel, &planes[..plane_count]);
        remaining = clip_planes(left, &planes[..plane_count]);
    }
    res
}

/// Ground movement with automatic step-up: tries the plain slide, and if that was blocked by
/// a wall also tries "up `step` → across → down"; keeps whichever got further horizontally
/// (and only if the stepped move ends on walkable ground no more than `step` higher).
pub fn step_slide_move(
    ctx: &Ctx,
    body: Body,
    pos: &mut Vec3,
    vel: &mut Vec3,
    motion: Vec3,
    step: f32,
) -> SlideResult {
    let start = *pos;
    let start_vel = *vel;
    let plain = slide_move(ctx, body, pos, vel, motion, true);
    if !plain.blocked || step <= 0.0 {
        return plain;
    }

    let mut p = ctx
        .sweep(body, start, Vec3::Y * step)
        .map_or(start + Vec3::Y * step, |h| h.pos);
    let up = p.y - start.y;
    if up < 0.02 {
        return plain;
    }
    let mut v = start_vel;
    let across = slide_move(ctx, body, &mut p, &mut v, horizontal(motion), true);
    let Some(down) = ctx.sweep(body, p, Vec3::NEG_Y * (up + GROUND_PROBE)) else {
        return plain;
    };
    // Landing on the rim of the step is fine (a slow walk into a step may not carry the
    // capsule fully over it this tick), but the *surface* must be within step height: the
    // rim of a higher ledge must not count, or the capsule could perch on it and climb.
    let Some((_, surface_y)) = ground_under(ctx, &down) else {
        return plain;
    };
    let p = down.pos;
    if p.y - start.y > step + 1e-3 || surface_y - start.y > step + SKIN * 2.0 {
        return plain;
    }
    let gained = horizontal(p - start).length_squared();
    let plain_gained = horizontal(*pos - start).length_squared();
    if gained <= plain_gained + 1e-6 {
        return plain;
    }
    *pos = p;
    *vel = v;
    SlideResult {
        stepped: p.y - start.y,
        ..across
    }
}

/// Moves the capsule down onto walkable ground within `max_drop`. Returns the ground normal,
/// or `None` (without moving) if there is no walkable ground that close.
///
/// Resting on the rim of a step or ledge counts as ground (see [`ground_under`]), so running
/// down stairs stays grounded while rolling over each rim. If the way down is blocked by a
/// steep surface (standing at the foot of a steep slope), the snap slides along it like
/// gravity would, so it still settles onto the ground below.
pub fn snap_to_ground(ctx: &Ctx, body: Body, pos: &mut Vec3, max_drop: f32) -> Option<Vec3> {
    let mut p = *pos;
    let mut motion = Vec3::NEG_Y * max_drop;
    for _ in 0..3 {
        let hit = ctx.sweep(body, p, motion)?;
        p = hit.pos;
        if let Some((n, _)) = ground_under(ctx, &hit) {
            *pos = p;
            return Some(n);
        }
        if hit.normal.y < CEILING_NORMAL_Y {
            return None;
        }
        motion = clip(motion * (1.0 - hit.fraction), hit.normal);
        p += hit.normal * NUDGE;
        if motion.y > -1e-6 {
            return None;
        }
    }
    None
}

/// The walkable ground a downward sweep landed on: `(normal, surface height)`, or `None` if
/// it isn't ground.
///
/// The rounded bottom of the capsule can rest on an edge (the rim of a step or ledge): the
/// contact normal then points from the edge to the sphere centre and is too steep to count as
/// ground even though the surface under the edge is flat. A short ray just inside the edge
/// finds that surface.
fn ground_under(ctx: &Ctx, hit: &Hit) -> Option<(Vec3, f32)> {
    let n = hit.normal;
    if ctx.walkable(n) {
        return Some((n, hit.point.y));
    }
    if n.y <= 0.05 {
        return None;
    }
    let into = -horizontal(n).normalize_or_zero();
    let origin = hit.point + Vec3::Y * 0.06 + into * 0.03;
    match ctx.world.cast_ray(origin, Vec3::NEG_Y, 0.15, MASK_MOVEMENT) {
        Some(r) if r.distance > 1e-3 && ctx.walkable(r.normal) => Some((r.normal, r.point.y)),
        _ => None,
    }
}

/// True if any upward-facing surface is within `distance` below the feet.
pub fn ground_within(ctx: &Ctx, body: Body, pos: Vec3, distance: f32) -> bool {
    ctx.sweep(body, pos, Vec3::NEG_Y * distance)
        .is_some_and(|h| h.normal.y > 0.3)
}

/// Pushes the capsule out of anything it penetrates or exactly touches (spawned inside or on
/// geometry, pushed by an external impulse, a bad sweep) to the skin distance. Returns true if
/// it moved.
pub fn depenetrate(ctx: &Ctx, body: Body, pos: &mut Vec3) -> bool {
    let mut moved = false;
    for _ in 0..4 {
        let contacts = ctx.contacts(body, *pos, SKIN);
        let deepest = contacts
            .iter()
            .filter(|c| c.distance < TOUCH_DISTANCE)
            .min_by(|a, b| a.distance.total_cmp(&b.distance));
        let Some(c) = deepest else { break };
        *pos += c.normal * (SKIN - c.distance);
        moved = true;
    }
    moved
}

/// Surfaces next to an airborne player.
#[derive(Clone, Copy, Debug, Default)]
pub struct Nearby {
    /// Horizontal normal of the closest wall within the wall-detect distance.
    pub wall: Option<Vec3>,
    /// Normal of a too-steep-to-stand-on slope the player is touching.
    pub steep: Option<Vec3>,
}

/// Looks for walls within `wall_distance` and steep slopes being touched (one contact query).
pub fn probe_nearby(ctx: &Ctx, body: Body, pos: Vec3, wall_distance: f32) -> Nearby {
    let touch = SKIN * 3.0;
    let mut wall: Option<(f32, Vec3)> = None;
    let mut steep: Option<(f32, Vec3)> = None;
    for c in ctx.contacts(body, pos, wall_distance.max(touch) + SKIN) {
        if c.normal.y.abs() < WALL_NORMAL_Y {
            let n = horizontal(c.normal).normalize_or_zero();
            if n != Vec3::ZERO
                && c.distance <= wall_distance + SKIN
                && wall.is_none_or(|(d, _)| c.distance < d)
            {
                wall = Some((c.distance, n));
            }
        } else if c.normal.y > 0.0
            && !ctx.walkable(c.normal)
            && c.distance <= touch
            && steep.is_none_or(|(d, _)| c.distance < d)
        {
            steep = Some((c.distance, c.normal));
        }
    }
    Nearby {
        wall: wall.map(|(_, n)| n),
        steep: steep.map(|(_, n)| n),
    }
}

/// True if a capsule of `height` fits with its feet at `pos` (used before standing up).
pub fn fits(ctx: &Ctx, pos: Vec3, height: f32, radius: f32) -> bool {
    // Lift the test capsule's bottom slightly so resting contact with the floor doesn't count.
    let lift = SKIN * 2.0;
    let center = pos + Vec3::Y * ((height + lift) * 0.5);
    let half = ((height - lift) * 0.5 - radius).max(0.0);
    !ctx.world
        .intersects_capsule(center, half, radius, MASK_MOVEMENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn along_ground_keeps_horizontal_motion() {
        let n = Vec3::new(0.0, 1.0, 1.0).normalize(); // slope rising towards -Z
        let m = along_ground(Vec3::new(0.0, 0.0, -1.0), n);
        assert!((m.z + 1.0).abs() < 1e-6);
        assert!((m.y - 1.0).abs() < 1e-5, "{m}");
        assert!(m.dot(n).abs() < 1e-5);
    }

    #[test]
    fn crease_clipping() {
        let a = Vec3::X;
        let b = Vec3::Z;
        let v = Vec3::new(-1.0, 0.5, -1.0);
        let c = clip_planes(v, &[a, b]);
        assert!(c.x.abs() < 1e-6 && c.z.abs() < 1e-6);
        assert!((c.y - 0.5).abs() < 1e-6);
    }
}
