//! Own projectiles at the muzzle on the frame the shot is predicted (`docs/feel.md`).
//!
//! The server only tells us about a projectile we fired once a snapshot with it arrives: the
//! input's trip to the server, its input buffer and the snapshot's trip back (2–4 ticks in
//! single player, 150 ms and more at 100 ms ping). Until then a local **stand-in** flies the
//! path the server will give it — same kind, launch point, speed and gravity, mirroring
//! `SimWorld::resolve_intent` — and is drawn by exactly the code that draws the real thing
//! (model, glow, trail, light).
//!
//! When the server's projectile turns up among our own, the closest unclaimed stand-in of its
//! kind **hands over** to it: the real projectile is drawn where the stand-in was and eases onto
//! its own path (a time offset — the input delay — and a position offset — spread the client
//! can't know — both fade out over [`HANDOVER`]), and the stand-in is gone: no pop, no double.
//! Stand-ins stop at the level (bombs bounce, arrows and limpets stick, the rest vanish) and at
//! the bodies they reach (the server decides what happens there), and fade if the server never
//! reports their projectile (it hit something on its first tick).

use std::collections::{HashMap, HashSet};

use engine::physics::{CollisionWorld, MASK_SHOTS};
use glam::{Quat, Vec3};
use shared::loadout::{Ability, Knack, WeaponClass, WeaponKind};
use shared::protocol::{EntityState, NetId, kind, projectile_flags};
use shared::sim::FireIntent;
use shared::sim::projectiles::{ProjectileKind, replicated_position};
use shared::sim::weapons::aim_basis;
use shared::tuning::CombatTuning;

use super::Rng;

/// Seconds over which a handed-over projectile eases from the stand-in's place onto its own
/// path.
pub const HANDOVER: f32 = 0.3;
/// Seconds over which a stand-in blends from the drawn muzzle onto its true (eye-based) path
/// (the same blend own projectiles get, `entities.rs`).
const MUZZLE_BLEND: f32 = 0.15;
/// Most stand-ins alive at once (oldest dropped).
const MAX_STAND_INS: usize = 48;
/// Largest input delay a handover makes up for (ticks).
const MAX_TIME_OFFSET: f64 = 15.0;
/// Stand-in ids: far above anything the server allocates.
const FIRST_ID: u32 = 0xFFF0_0000;

/// One projectile a predicted shot fires: replicated kind, launch point, velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Launch {
    pub kind: u16,
    pub start: Vec3,
    pub velocity: Vec3,
}

/// `aim` rotated `degrees` upwards about its right axis, never below the aim (lobs; as
/// `shared::sim::weapons`).
fn lob_dir(aim: Vec3, degrees: f32) -> Vec3 {
    let (right, _) = aim_basis(aim);
    let lob = Quat::from_axis_angle(right, degrees.to_radians()) * aim;
    if lob.y < aim.y { aim } else { lob }
}

/// A direction inside a cone of full aperture `degrees` (the client can't know the server's
/// spread: the handover blends the difference away).
fn spread(aim: Vec3, degrees: f32, rng: &mut Rng) -> Vec3 {
    if degrees <= 0.0 {
        return aim;
    }
    let (right, up) = aim_basis(aim);
    let r = rng.unit().sqrt() * (degrees * 0.5).to_radians().tan();
    let a = rng.angle();
    (aim + right * (r * a.cos()) + up * (r * a.sin())).normalize_or(aim)
}

/// The projectiles a predicted intent launches (mirrors `SimWorld::fire_primary` /
/// `fire_ability`): `eye` is the predicted eye, `live_limpets` the Limpets already out (a
/// throw with the maximum out detonates them instead). Hitscan, area and melee intents launch
/// nothing.
pub fn launches(
    intent: &FireIntent,
    aim: Vec3,
    eye: Vec3,
    c: &CombatTuning,
    world: Option<&CollisionWorld>,
    live_limpets: u32,
    rng: &mut Rng,
) -> Vec<Launch> {
    let aim = aim.normalize_or(Vec3::NEG_Z);
    // `want` unless the level blocks the way from the eye (as the server's safe launch point).
    let safe = |want: Vec3| match world {
        Some(w) if w.line_blocked(eye, want, MASK_SHOTS) => eye,
        _ => want,
    };
    let one = |kind: u16, start: Vec3, velocity: Vec3| {
        vec![Launch {
            kind,
            start,
            velocity,
        }]
    };
    let arrow = |kind: u16, dir: Vec3, draw: f32| {
        let l = &c.longwick;
        let speed = l.min_speed + (l.max_speed - l.min_speed) * draw.clamp(0.0, 1.0);
        Launch {
            kind,
            start: safe(eye + dir * 0.4),
            velocity: dir * speed,
        }
    };
    match *intent {
        FireIntent::Primary { weapon, charge } => match weapon {
            WeaponKind::Tumbler => one(kind::BOMB, eye, aim * c.tumbler.bomb_speed),
            WeaponKind::Riveter => {
                let r = &c.riveter;
                let dir = spread(aim, r.spread_degrees, rng);
                one(kind::RIVET, safe(eye + aim * 0.4), dir * r.rivet_speed)
            }
            WeaponKind::Cinderbox => {
                let cb = &c.cinderbox;
                let dir = spread(aim, cb.spread_degrees, rng);
                one(
                    kind::CINDER_ROCKET,
                    safe(eye + aim * 0.5),
                    dir * cb.rocket_speed,
                )
            }
            WeaponKind::Longwick => vec![arrow(kind::ARROW, aim, charge)],
            _ => Vec::new(),
        },
        FireIntent::Ability {
            weapon,
            ability,
            charge,
        } => {
            if weapon.class() == WeaponClass::Draw {
                let kind = match ability {
                    Ability::PinningArrow => kind::PIN_ARROW,
                    Ability::EmberArrow => kind::EMBER_ARROW,
                    Ability::Brand => kind::BRAND_ARROW,
                    _ => kind::ARROW,
                };
                if ability != Ability::TripleNock {
                    return vec![arrow(kind, aim, charge)];
                }
                let tn = &c.triple_nock;
                let n = tn.arrows.max(1);
                let (right, _) = aim_basis(aim);
                let up = right.cross(aim).normalize_or(Vec3::Y);
                return (0..n)
                    .map(|i| {
                        let f = if n == 1 {
                            0.0
                        } else {
                            i as f32 / (n - 1) as f32 - 0.5
                        };
                        let dir =
                            Quat::from_axis_angle(up, (tn.fan_degrees * f).to_radians()) * aim;
                        arrow(kind, dir, charge)
                    })
                    .collect();
            }
            match ability {
                Ability::Flak => one(
                    kind::FLAK_SHELL,
                    safe(eye + aim * 0.5),
                    aim * c.flak.shell_speed,
                ),
                Ability::Wisp => one(
                    kind::WISP,
                    safe(eye + aim * 0.6),
                    aim * c.arclash.wisp_speed,
                ),
                Ability::Volley if weapon == WeaponKind::Riveter => {
                    let start = safe(eye + aim * 0.4);
                    (0..c.volley.shots.max(1))
                        .map(|_| Launch {
                            kind: kind::RIVET,
                            start,
                            velocity: spread(aim, c.volley.cone_degrees, rng)
                                * c.riveter.rivet_speed,
                        })
                        .collect()
                }
                Ability::Flare => one(
                    kind::FLARE,
                    safe(eye + aim * 0.5),
                    lob_dir(aim, c.flare.angle_degrees) * c.flare.speed,
                ),
                Ability::Gravewell => one(
                    kind::GRAVEWELL,
                    eye,
                    lob_dir(aim, c.tumbler.gravewell_angle_degrees) * c.tumbler.gravewell_speed,
                ),
                Ability::Limpet if charge < 1.0 && live_limpets < c.limpet.max_live.max(1) => {
                    one(kind::LIMPET, safe(eye + aim * 0.5), aim * c.limpet.speed)
                }
                Ability::Mortar => one(
                    kind::MORTAR_SHELL,
                    eye,
                    lob_dir(aim, c.mortar.angle_degrees) * c.mortar.speed,
                ),
                // The melee Hurl's blade / head (drawn by melee.rs).
                Ability::Hurl => one(kind::HURL, safe(eye + aim * 0.5), aim * c.hurl.speed),
                _ => Vec::new(),
            }
        }
        // The knacks' throws (drawn by knacks.rs).
        FireIntent::Knack(knack) => {
            let k = &c.knacks;
            let start = safe(eye + aim * 0.5);
            match knack {
                Knack::KindleFlask => one(
                    kind::KINDLE_FLASK,
                    start,
                    lob_dir(aim, k.kindle_flask.angle_degrees) * k.kindle_flask.speed,
                ),
                Knack::Snare => one(kind::SNARE, start, aim * k.snare.speed),
                Knack::SootPot => one(kind::SOOT_POT, start, aim * k.soot_pot.speed),
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

/// What a stand-in does when it reaches the level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnWorld {
    Bounce,
    Stick,
    Vanish,
}

fn on_world(k: u16) -> OnWorld {
    match ProjectileKind::from_net_kind(k) {
        Some(ProjectileKind::Bomb) => OnWorld::Bounce,
        Some(
            ProjectileKind::Gravewell
            | ProjectileKind::Arrow
            | ProjectileKind::PinArrow
            | ProjectileKind::BrandArrow
            | ProjectileKind::Limpet
            | ProjectileKind::Snare,
        ) => OnWorld::Stick,
        _ => OnWorld::Vanish,
    }
}

/// A local stand-in.
#[derive(Clone, Debug)]
struct Ghost {
    /// Drawn through the normal projectile path: `position` / `velocity` at `t0`, `aux` 0,
    /// `STUCK` once it stopped.
    entity: EntityState,
    /// Own-time (fractional server ticks, see `combat::own_projectiles`) of the shot ...
    born: f64,
    /// ... and of the current trajectory segment (a bounce restarts it).
    t0: f64,
    /// Seconds since the shot.
    age: f32,
    /// Drawn muzzle minus the true launch point (blends out over [`MUZZLE_BLEND`]).
    muzzle: Vec3,
    /// Where it was drawn last frame (contact tests sweep from there).
    last: Vec3,
    bounces_left: u32,
    /// Went into a body or vanished at the level: not drawn, still claimable.
    hidden: bool,
}

impl Ghost {
    /// True (undisplaced) position at own-time `now`.
    fn path(&self, now: f64, rate: u32, t: &CombatTuning) -> Vec3 {
        replicated_position(&self.entity, (now - self.t0).max(0.0), rate, t)
    }

    fn muzzle_weight(&self) -> f32 {
        (1.0 - self.age / MUZZLE_BLEND).clamp(0.0, 1.0)
    }
}

/// A handed-over projectile easing from the stand-in's place onto its own path.
#[derive(Clone, Copy, Debug)]
struct Handover {
    /// Ticks the stand-in was ahead (the input delay) ...
    time: f64,
    /// ... and what was left over (spread, launch point).
    offset: Vec3,
    since: f32,
}

impl Handover {
    fn weight(&self) -> f32 {
        let k = (self.since / HANDOVER).clamp(0.0, 1.0);
        1.0 - k * k * (3.0 - 2.0 * k)
    }
}

/// Counters for tests and tools.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StandInStats {
    pub spawned: u64,
    pub claimed: u64,
    pub expired: u64,
    /// Seconds from the shot to its handover, summed over the claims: how long the projectile
    /// would have been missing without its stand-in.
    pub waited: f64,
}

impl StandInStats {
    /// Mean seconds a stand-in flew before the server's projectile arrived.
    pub fn mean_wait(&self) -> f64 {
        self.waited / self.claimed.max(1) as f64
    }
}

/// Every stand-in and handover of the local player.
#[derive(Default)]
pub struct StandIns {
    ghosts: Vec<Ghost>,
    handovers: HashMap<NetId, Handover>,
    /// Own projectiles already seen (only a new one can claim a stand-in).
    seen: HashSet<NetId>,
    /// Stand-ins handed over this update: (stand-in id, real id), so per-projectile visuals
    /// (trails) carry over.
    claims: Vec<(NetId, NetId)>,
    next: u32,
    pub stats: StandInStats,
}

/// What a frame's update needs.
pub struct StandInFrame<'a> {
    /// The local player's projectiles from the newest snapshot.
    pub own: &'a [EntityState],
    /// Own-time now (fractional server ticks).
    pub now: f64,
    pub tick_rate: u32,
    pub tuning: &'a CombatTuning,
    pub world: Option<&'a CollisionWorld>,
    /// Displayed bodies a stand-in stops in: capsule (a, b, radius).
    pub bodies: &'a [(Vec3, Vec3, f32)],
    /// Seconds an unclaimed stand-in lives (a round trip and some).
    pub timeout: f32,
}

impl StandIns {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.ghosts.clear();
        self.handovers.clear();
        self.seen.clear();
        self.claims.clear();
    }

    pub fn len(&self) -> usize {
        self.ghosts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ghosts.is_empty()
    }

    /// Starts a stand-in for `launch`, drawn from `muzzle` (the drawn weapon's muzzle), owned
    /// by owner code `owner`, at own-time `now`.
    pub fn spawn(&mut self, launch: Launch, muzzle: Vec3, owner: u16, now: f64, t: &CombatTuning) {
        if !(launch.start.is_finite() && launch.velocity.is_finite() && now.is_finite()) {
            return;
        }
        if self.ghosts.len() >= MAX_STAND_INS {
            self.ghosts.remove(0);
        }
        let id = NetId(FIRST_ID.wrapping_add(self.next));
        self.next = (self.next + 1) % 0x000F_0000;
        let muzzle = if muzzle.is_finite() && muzzle.distance(launch.start) < 1.5 {
            muzzle - launch.start
        } else {
            Vec3::ZERO
        };
        self.ghosts.push(Ghost {
            entity: EntityState {
                id,
                kind: launch.kind,
                owner,
                position: launch.start,
                velocity: launch.velocity,
                ..Default::default()
            },
            born: now,
            t0: now,
            age: 0.0,
            muzzle,
            last: launch.start + muzzle,
            // A bomb bounces until its last contact (it explodes on that one).
            bounces_left: if launch.kind == kind::BOMB {
                t.tumbler.bomb_contacts.saturating_sub(1)
            } else {
                0
            },
            hidden: false,
        });
        self.stats.spawned += 1;
    }

    /// Advances the stand-ins (contacts, expiry) and hands them over to newly seen own
    /// projectiles. `dt`: real seconds.
    pub fn update(&mut self, f: &StandInFrame, dt: f32) {
        let dt = dt.max(0.0);
        let t = f.tuning;
        let rate = f.tick_rate;
        // Handovers fade; forget projectiles that are gone.
        let live: HashSet<NetId> = f.own.iter().map(|e| e.id).collect();
        self.seen.retain(|id| live.contains(id));
        self.handovers.retain(|id, _| live.contains(id));
        for h in self.handovers.values_mut() {
            h.since += dt;
        }
        self.handovers.retain(|_, h| h.since < HANDOVER);

        // Move the stand-ins: sweep from where they were drawn to where they are now.
        for g in &mut self.ghosts {
            g.age += dt;
            if g.hidden || g.entity.flags & projectile_flags::STUCK != 0 {
                continue;
            }
            let true_now = g.path(f.now, rate, t);
            let now = true_now + g.muzzle * g.muzzle_weight();
            let from = g.last;
            let d = now - from;
            let len = d.length();
            g.last = now;
            if len < 1e-4 || !len.is_finite() {
                continue;
            }
            let dir = d / len;
            let pk = ProjectileKind::from_net_kind(g.entity.kind);
            let passes_bodies = pk.is_some_and(|k| !k.hits_targets());
            let body = (!passes_bodies)
                .then(|| {
                    f.bodies
                        .iter()
                        .filter_map(|&(a, b, r)| {
                            shared::sim::HitShape::capsule(shared::sim::HitPart::Body, a, b, r)
                                .ray_hit(from, dir, len)
                        })
                        .min_by(|a, b| a.total_cmp(b))
                })
                .flatten();
            let wall = f.world.and_then(|w| w.cast_ray(from, dir, len, MASK_SHOTS));
            match (body, wall) {
                (Some(b), w) if w.as_ref().is_none_or(|w| b <= w.distance) => g.hidden = true,
                (_, Some(hit)) => match on_world(g.entity.kind) {
                    OnWorld::Bounce if g.bounces_left > 0 => {
                        // Restart the trajectory from the contact, reflected.
                        let vel = super::entities::replicated_velocity(
                            &g.entity,
                            (f.now - g.t0).max(0.0),
                            rate,
                            t,
                        );
                        let n = hit.normal;
                        let reflected = (vel - n * (2.0 * vel.dot(n))) * t.tumbler.bomb_restitution;
                        let at = from + dir * hit.distance + n * 0.02;
                        g.entity.position = at - g.muzzle * g.muzzle_weight();
                        g.entity.velocity = reflected;
                        g.t0 = f.now;
                        g.bounces_left -= 1;
                        g.last = at;
                    }
                    OnWorld::Stick => {
                        g.entity.position = from + dir * hit.distance;
                        g.entity.flags |= projectile_flags::STUCK;
                        g.muzzle = Vec3::ZERO;
                    }
                    _ => g.hidden = true,
                },
                _ => {}
            }
        }

        // Hand over: every own projectile seen for the first time claims the closest
        // stand-in of its kind.
        self.claims.clear();
        for e in f.own {
            if !self.seen.insert(e.id) || e.flags & projectile_flags::PARRIED != 0 {
                continue;
            }
            let best = self
                .ghosts
                .iter()
                .enumerate()
                .filter(|(_, g)| g.entity.kind == e.kind)
                .map(|(i, g)| {
                    let launch = g.path(g.born, rate, t);
                    let off = launch.distance(e.position);
                    let turn = g
                        .entity
                        .velocity
                        .normalize_or_zero()
                        .dot(e.velocity.normalize_or_zero());
                    (i, off + (1.0 - turn) * 20.0)
                })
                .filter(|(_, score)| *score < 3.0)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((i, _)) = best else {
                continue;
            };
            let g = self.ghosts.remove(i);
            let time = (e.aux as f64 - g.born).clamp(0.0, MAX_TIME_OFFSET);
            let drawn = g.path(f.now, rate, t) + g.muzzle * g.muzzle_weight();
            let real = replicated_position(e, f.now + time, rate, t);
            let offset = if g.hidden || !(drawn - real).is_finite() {
                Vec3::ZERO
            } else {
                drawn - real
            };
            self.handovers.insert(
                e.id,
                Handover {
                    time,
                    offset: offset.clamp_length_max(4.0),
                    since: 0.0,
                },
            );
            self.claims.push((g.entity.id, e.id));
            self.stats.claimed += 1;
            self.stats.waited += g.age as f64;
        }

        // Unclaimed for too long: the server never had it in flight (or it's long gone).
        let before = self.ghosts.len();
        self.ghosts.retain(|g| g.age < f.timeout.max(0.1));
        self.stats.expired += (before - self.ghosts.len()) as u64;
    }

    /// Stand-ins to draw this frame: (entity, the time to evaluate its trajectory at, visual
    /// offset), like `EntityFrame::projectiles`.
    pub fn drawn(&self, now: f64) -> impl Iterator<Item = (&EntityState, f64, Vec3)> {
        self.ghosts.iter().filter(|g| !g.hidden).map(move |g| {
            let stuck = g.entity.flags & projectile_flags::STUCK != 0;
            let offset = if stuck {
                Vec3::ZERO
            } else {
                g.muzzle * g.muzzle_weight()
            };
            (&g.entity, (now - g.t0).max(0.0), offset)
        })
    }

    /// For an own projectile being handed over: (extra ticks to draw it ahead, extra offset).
    pub fn handover(&self, id: NetId) -> Option<(f64, Vec3)> {
        self.handovers.get(&id).map(|h| {
            let w = h.weight();
            (h.time * w as f64, h.offset * w)
        })
    }

    /// Stand-ins handed over in the last update: (stand-in id, real id).
    pub fn claims(&self) -> &[(NetId, NetId)] {
        &self.claims
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning() -> CombatTuning {
        CombatTuning::default()
    }

    fn frame<'a>(
        own: &'a [EntityState],
        now: f64,
        t: &'a CombatTuning,
        bodies: &'a [(Vec3, Vec3, f32)],
    ) -> StandInFrame<'a> {
        StandInFrame {
            own,
            now,
            tick_rate: 60,
            tuning: t,
            world: None,
            bodies,
            timeout: 0.6,
        }
    }

    /// The server's copy of a projectile launched at `start` / `velocity` on tick `launch`.
    fn server(id: u32, kind: u16, start: Vec3, velocity: Vec3, launch: u32) -> EntityState {
        EntityState {
            id: NetId(id),
            kind,
            owner: 1,
            position: start,
            velocity,
            aux: launch,
            ..Default::default()
        }
    }

    #[test]
    fn every_projectile_intent_launches_like_the_server() {
        let t = tuning();
        let mut rng = Rng::default();
        let aim = Vec3::NEG_Z;
        let eye = Vec3::new(0.0, 1.6, 0.0);
        let cases = [
            (
                FireIntent::Primary {
                    weapon: WeaponKind::Tumbler,
                    charge: 1.0,
                },
                1,
            ),
            (
                FireIntent::Primary {
                    weapon: WeaponKind::Riveter,
                    charge: 1.0,
                },
                1,
            ),
            (
                FireIntent::Primary {
                    weapon: WeaponKind::Quill,
                    charge: 1.0,
                },
                0,
            ),
            (
                FireIntent::Ability {
                    weapon: WeaponKind::Longwick,
                    ability: Ability::TripleNock,
                    charge: 1.0,
                },
                t.triple_nock.arrows as usize,
            ),
            (
                FireIntent::Ability {
                    weapon: WeaponKind::Riveter,
                    ability: Ability::Volley,
                    charge: 0.0,
                },
                t.volley.shots as usize,
            ),
            (
                FireIntent::Ability {
                    weapon: WeaponKind::Cinderbox,
                    ability: Ability::Limpet,
                    charge: 1.0,
                },
                0,
            ),
            (FireIntent::Punch, 0),
        ];
        for (intent, n) in cases {
            let l = launches(&intent, aim, eye, &t, None, 0, &mut rng);
            assert_eq!(l.len(), n, "{intent:?}");
        }
        // A full-draw arrow: the bow's top speed from 0.4 m ahead of the eye.
        let full = FireIntent::Primary {
            weapon: WeaponKind::Longwick,
            charge: 1.0,
        };
        let a = launches(&full, aim, eye, &t, None, 0, &mut rng)[0];
        assert_eq!(a.kind, kind::ARROW);
        assert!((a.velocity.length() - t.longwick.max_speed).abs() < 1e-3);
        assert!(a.start.distance(eye + aim * 0.4) < 1e-4);
        // Lobs go up, never below the aim.
        let mortar = FireIntent::Ability {
            weapon: WeaponKind::Cinderbox,
            ability: Ability::Mortar,
            charge: 0.0,
        };
        let m = launches(&mortar, aim, eye, &t, None, 0, &mut rng)[0];
        assert!(m.velocity.y > 0.0);
        // The fourth limpet detonates instead of throwing.
        let limpet = FireIntent::Ability {
            weapon: WeaponKind::Cinderbox,
            ability: Ability::Limpet,
            charge: 0.0,
        };
        assert_eq!(launches(&limpet, aim, eye, &t, None, 0, &mut rng).len(), 1);
        let max = t.limpet.max_live;
        assert!(launches(&limpet, aim, eye, &t, None, max, &mut rng).is_empty());
    }

    #[test]
    fn a_stand_in_hands_over_without_a_pop_or_a_double() {
        let t = tuning();
        let rate = 60;
        let start = Vec3::new(0.0, 1.6, -0.4);
        let velocity = Vec3::new(0.0, 0.0, -90.0);
        let launch = Launch {
            kind: kind::RIVET,
            start,
            velocity,
        };
        let mut s = StandIns::new();
        // Fired at own-time 100.0; the server launches it 3 ticks later (input delay).
        s.spawn(launch, start + Vec3::new(0.12, -0.1, 0.0), 1, 100.0, &t);
        let mut now = 100.0;
        let dt = 1.0 / 144.0;
        let mut last: Option<Vec3> = None;
        let drawn_at = |s: &StandIns, now: f64, own: &[EntityState]| -> (Vec3, usize) {
            let mut n = 0;
            let mut p = Vec3::ZERO;
            for (e, time, off) in s.drawn(now) {
                p = replicated_position(e, time, rate, &t) + off;
                n += 1;
            }
            for e in own {
                let (dtk, off) = s.handover(e.id).unwrap_or((0.0, Vec3::ZERO));
                p = replicated_position(e, now + dtk, rate, &t) + off;
                n += 1;
            }
            (p, n)
        };
        let real = server(500, kind::RIVET, start, velocity, 103);
        let mut biggest_step = 0.0f32;
        for i in 0..60 {
            now += dt as f64 * rate as f64;
            // The snapshot with the real rivet arrives at frame 12 (~83 ms).
            let own: Vec<EntityState> = if i >= 12 { vec![real] } else { Vec::new() };
            s.update(&frame(&own, now, &t, &[]), dt);
            let (p, n) = drawn_at(&s, now, &own);
            assert_eq!(n, 1, "frame {i}: exactly one rivet drawn");
            if let Some(prev) = last {
                let step: f32 = (p - prev).length();
                biggest_step = biggest_step.max(step);
            }
            last = Some(p);
        }
        // 90 m/s at 144 frames/s is 0.625 m per frame: no frame jumps much more than that.
        assert!(biggest_step < 0.8, "biggest step {biggest_step}");
        assert_eq!(s.stats.claimed, 1);
        assert!(s.is_empty());
        // After the handover the real rivet is drawn on its own path.
        let (dtk, off) = s.handover(NetId(500)).unwrap_or((0.0, Vec3::ZERO));
        assert!(dtk.abs() < 1e-6 && off.length() < 1e-6);
    }

    #[test]
    fn stand_ins_stop_in_bodies_and_expire_unclaimed() {
        let t = tuning();
        let mut s = StandIns::new();
        let launch = Launch {
            kind: kind::ARROW,
            start: Vec3::new(0.0, 1.6, 0.0),
            velocity: Vec3::new(0.0, 0.0, -100.0),
        };
        s.spawn(launch, launch.start, 1, 50.0, &t);
        // A body 5 m ahead.
        let bodies = [(Vec3::new(0.0, 0.5, -5.0), Vec3::new(0.0, 1.8, -5.0), 0.5)];
        let mut now = 50.0;
        for _ in 0..10 {
            now += 1.0;
            s.update(&frame(&[], now, &t, &bodies), 1.0 / 60.0);
        }
        assert_eq!(s.drawn(now).count(), 0, "hidden in the body");
        assert_eq!(s.len(), 1, "still claimable");
        for _ in 0..60 {
            now += 1.0;
            s.update(&frame(&[], now, &t, &bodies), 1.0 / 60.0);
        }
        assert!(s.is_empty());
        assert_eq!(s.stats.expired, 1);
    }

    #[test]
    fn unmatched_projectiles_claim_nothing() {
        let t = tuning();
        let mut s = StandIns::new();
        let launch = Launch {
            kind: kind::BOMB,
            start: Vec3::new(0.0, 1.6, 0.0),
            velocity: Vec3::new(0.0, 0.0, -30.0),
        };
        s.spawn(launch, launch.start, 1, 10.0, &t);
        // A different kind, and the same kind flying elsewhere.
        let other = [
            server(1, kind::RIVET, launch.start, launch.velocity, 11),
            server(2, kind::BOMB, Vec3::new(40.0, 1.6, 0.0), Vec3::X * 30.0, 11),
        ];
        s.update(&frame(&other, 12.0, &t, &[]), 1.0 / 60.0);
        assert_eq!(s.stats.claimed, 0);
        assert_eq!(s.len(), 1);
        assert!(s.handover(NetId(1)).is_none());
    }
}
