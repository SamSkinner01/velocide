//! The scatterguns' blasts (the Bellows and the Thresher, `docs/balance.md`).
//!
//! **Pattern**: the pellets fly in a fixed pattern (a centre pellet, an inner ring at half the
//! cone and the outer ring on its edge), each nudged by a little jitter, instead of uniformly at
//! random over the cone: a good aim always lands the same pellets. The jitter is seeded by the
//! input alone ([`pellet_dirs`]), so a client draws its pellets exactly where the server sent
//! them.
//!
//! **Point blank**: a pellet that hits within `point_blank_range` deals ×
//! `point_blank_multiplier` (easing back to × 1 at `point_blank_end`; [`PointBlank`]).
//!
//! **Stagger**: a blast that lands at least `stagger_fraction` of its pellets on a light Mould
//! (base health below `pinning_arrow.heavy_health`) staggers it for `stagger_time`: it stops
//! steering and the pellets' push carries it. A pellet alone never pushes hard enough to
//! stagger (`ai::STAGGER_KNOCKBACK` is per hit), so a blast that connected used to leave it
//! running at you.

use std::f32::consts::{FRAC_PI_2, TAU};

use glam::{Vec2, Vec3};

use super::combat::secs_to_ticks;
use super::hitscan::{PointBlank, ShotSpec, TraceResult};
use super::weapons::{SpreadRng, aim_basis};
use super::{HitTarget, HitboxHistory, PlayerId, SimWorld};
use crate::input::PlayerInput;
use crate::loadout::WeaponKind;
use crate::protocol::NetId;
use crate::tuning::{CombatTuning, Tuning};

/// Salt of the pattern jitter (distinct from every other spread of the same input).
const PELLET_SALT: u32 = 0x9E11_E700;

/// A scattergun's blast numbers (the Bellows' and the Thresher's sections share their names).
struct Blast {
    pellets: u32,
    damage: f32,
    cone_degrees: f32,
    range: f32,
    knockback: f32,
    jitter_degrees: f32,
    point_blank: PointBlank,
    stagger_fraction: f32,
    stagger_time: f32,
}

impl Blast {
    fn of(c: &CombatTuning, weapon: WeaponKind) -> Self {
        macro_rules! blast {
            ($s:expr) => {
                Blast {
                    pellets: $s.pellets,
                    damage: $s.pellet_damage,
                    cone_degrees: $s.cone_degrees,
                    range: $s.pellet_range,
                    knockback: $s.pellet_knockback,
                    jitter_degrees: $s.pattern_jitter_degrees,
                    point_blank: PointBlank {
                        range: $s.point_blank_range,
                        end: $s.point_blank_end,
                        multiplier: $s.point_blank_multiplier,
                    },
                    stagger_fraction: $s.stagger_fraction,
                    stagger_time: $s.stagger_time,
                }
            };
        }
        if weapon == WeaponKind::Thresher {
            blast!(c.thresher)
        } else {
            blast!(c.bellows)
        }
    }
}

/// The fixed pattern of `count` pellets on the unit disc: the centre, an inner ring at half
/// the radius holding half of the rest (rounded down, starting straight up), the outer ring on
/// the edge half a step round from it. Twelve (the Bellows): 1 + 5 + 6; eight (the Thresher):
/// 1 + 3 + 4.
pub fn pellet_pattern(count: u32) -> Vec<Vec2> {
    let rest = count.max(1) as usize - 1;
    let inner = rest / 2;
    let mut out = vec![Vec2::ZERO];
    for (n, radius) in [(inner, 0.5), (rest - inner, 1.0)] {
        let step = TAU / n.max(1) as f32;
        let start = FRAC_PI_2 + if radius < 1.0 { 0.0 } else { step * 0.5 };
        out.extend((0..n).map(|i| Vec2::from_angle(start + step * i as f32) * radius));
    }
    out
}

/// The directions of a blast of `count` pellets along `aim`: [`pellet_pattern`] spread over a
/// cone of full aperture `cone_degrees`, each pellet moved up to `jitter_degrees` (uniform over
/// a disc) by a generator seeded by the input's `seq` alone (the client draws the same).
pub fn pellet_dirs(
    aim: Vec3,
    count: u32,
    cone_degrees: f32,
    jitter_degrees: f32,
    seq: u32,
) -> Vec<Vec3> {
    let half = (cone_degrees * 0.5).to_radians().tan();
    let jitter = jitter_degrees.to_radians().tan();
    let mut rng = SpreadRng::new(0, PlayerId(0), seq ^ PELLET_SALT);
    let (right, up) = aim_basis(aim);
    pellet_pattern(count)
        .into_iter()
        .map(|p| {
            let r = rng.next_f32().sqrt() * jitter;
            let a = rng.next_f32() * TAU;
            let o = p * half + Vec2::from_angle(a) * r;
            (aim + right * o.x + up * o.y).normalize_or(aim)
        })
        .collect()
}

/// The directions `weapon`'s blast fires from `input` (`None` for anything but a scattergun).
pub fn blast_dirs(c: &CombatTuning, weapon: WeaponKind, input: &PlayerInput) -> Option<Vec<Vec3>> {
    matches!(weapon, WeaponKind::Bellows | WeaponKind::Thresher).then(|| {
        let b = Blast::of(c, weapon);
        pellet_dirs(
            input.look_dir(),
            b.pellets,
            b.cone_degrees,
            b.jitter_degrees,
            input.seq,
        )
    })
}

impl SimWorld {
    /// A scattergun's primary: the blast's pellets against the view the shooter saw, then the
    /// stagger of every light Mould it connected with.
    pub(crate) fn fire_blast(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        weapon: WeaponKind,
    ) {
        let Some(p) = self.players.get(&id) else {
            return;
        };
        let eye = p.movement.eye_position(&t.movement);
        let b = Blast::of(&t.combat, weapon);
        let attack = weapon.primary();
        let dirs = pellet_dirs(
            input.look_dir(),
            b.pellets,
            b.cone_degrees,
            b.jitter_degrees,
            input.seq,
        );
        let shots: Vec<ShotSpec> = dirs
            .into_iter()
            .map(|dir| {
                ShotSpec::new(eye, dir, b.range, b.damage, attack)
                    .knockback(b.knockback)
                    .point_blank(b.point_blank)
            })
            .collect();
        let (_, results) = self.fire_hitscan_group(t, history, id, input, attack, &shots);
        self.blast_stagger(t, &b, &results);
    }

    /// Staggers every light Mould the blast landed at least `stagger_fraction` of its pellets
    /// on.
    fn blast_stagger(&mut self, t: &Tuning, b: &Blast, results: &[TraceResult]) {
        let needed = ((b.stagger_fraction * b.pellets as f32).ceil() as u32).max(1);
        let mut landed: Vec<(NetId, u32)> = Vec::new();
        for r in results {
            if let Some(HitTarget::Entity(id)) = r.impact.and_then(|i| i.target).map(|h| h.0) {
                match landed.iter_mut().find(|l| l.0 == id) {
                    Some(l) => l.1 += 1,
                    None => landed.push((id, 1)),
                }
            }
        }
        let ticks = secs_to_ticks(b.stagger_time, self.tick_rate, 0);
        let heavy = t.combat.pinning_arrow.heavy_health;
        for (id, n) in landed {
            if n < needed || ticks == 0 {
                continue;
            }
            if let Some(e) = self.enemies.get_mut(&id)
                && e.is_alive()
                && e.def.health < heavy
            {
                e.stagger = e.stagger.max(ticks);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pattern_is_a_centre_and_two_rings() {
        let p = pellet_pattern(12);
        assert_eq!(p.len(), 12);
        assert_eq!(p[0], Vec2::ZERO);
        let ring = |r: f32| p.iter().filter(|v| (v.length() - r).abs() < 1e-4).count();
        assert_eq!((ring(0.5), ring(1.0)), (5, 6));
        let p = pellet_pattern(7);
        assert_eq!(p.len(), 7);
        assert_eq!(
            (
                p.iter().filter(|v| (v.length() - 0.5).abs() < 1e-4).count(),
                p.iter().filter(|v| (v.length() - 1.0).abs() < 1e-4).count()
            ),
            (3, 3)
        );
        assert_eq!(pellet_pattern(1), vec![Vec2::ZERO]);
        assert_eq!(pellet_pattern(0).len(), 1);
    }

    #[test]
    fn pellets_keep_their_place_give_or_take_the_jitter() {
        let aim = Vec3::new(0.3, -0.2, -1.0).normalize();
        let cone = 8.0f32;
        let jitter = 0.5f32;
        let pattern = pellet_pattern(12);
        let half = (cone * 0.5).to_radians().tan();
        let (right, up) = aim_basis(aim);
        for seq in 0..200 {
            let dirs = pellet_dirs(aim, 12, cone, jitter, seq);
            assert_eq!(
                dirs,
                pellet_dirs(aim, 12, cone, jitter, seq),
                "seeded by seq"
            );
            for (d, p) in dirs.iter().zip(&pattern) {
                let place = (aim + right * (p.x * half) + up * (p.y * half)).normalize();
                assert!(d.angle_between(place).to_degrees() <= jitter + 1e-3);
                assert!(d.angle_between(aim).to_degrees() <= cone * 0.5 + jitter + 1e-3);
            }
        }
        // No jitter: the pattern exactly, the centre pellet on the aim.
        let exact = pellet_dirs(aim, 12, cone, 0.0, 7);
        assert!(exact[0].angle_between(aim) < 1e-4);
        assert!((exact[11].angle_between(aim).to_degrees() - cone * 0.5).abs() < 0.01);
    }
}
