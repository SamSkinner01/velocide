//! Own projectiles at the muzzle on the frame the shot is predicted (`docs/feel.md`).
//!
//! The server only tells us about a projectile we fired once a snapshot with it arrives: the
//! input's trip to the server, its input buffer and the snapshot's trip back (2–4 ticks in
//! single player, 150 ms and more at 100 ms ping). Until then a local **stand-in** flies the
//! path the server will give it — same kind, launch point, speed and gravity, mirroring
//! `SimWorld::resolve_intent` — and is drawn by exactly the code that draws the real thing
//! (model, glow, trail, light).
//!
//! **Clocks** (`docs/feel.md`, "Hit registration"). The server flies a projectile we fire from
//! the view time our shot was aimed at (`shared::sim::projectiles`, lag compensation): at
//! server tick `τ` it is where it is `τ − view` ticks into its flight, among the targets as
//! they were at `τ`. So it is drawn at the **render tick**, like every remote entity, and its
//! stand-in runs on the same clock from the shot's view time: what touches a target on screen
//! is what touches it on the server. Projectiles that aren't flown that way (deflected ones;
//! the orbiting Wisp, the steered Kite and the returning Hurl, driven from the present) are
//! drawn at own-time (the present, `own_projectiles`), their stand-ins too.
//!
//! When the server's projectile turns up among our own (render-clock ones: at the render tick,
//! so where the stand-in already is), the closest unclaimed stand-in of its kind **hands over**
//! to it: the real projectile is drawn where the stand-in was and eases onto its own path (a
//! time offset along its flight and a position offset — spread the client can't know, a capped
//! lag compensation — both fade out over [`HANDOVER`]), and the stand-in is gone: no pop, no
//! double. Stand-ins stop at the level (bombs bounce, arrows and limpets stick, the rest
//! vanish) and in the targets they reach — the hit shapes the server tests, padded as it pads
//! them, as displayed — and fade if the server never reports their projectile (it hit something
//! on its first tick). A handed-over projectile that reaches a target on screen is no longer
//! drawn there either (the server removes it a snapshot later), unless the server still has it
//! flying a little after: then it missed, and it shows again.

use std::collections::{HashMap, HashSet};

use engine::physics::{CollisionWorld, MASK_SHOTS};
use glam::{Quat, Vec3};
use shared::input::PlayerInput;
use shared::loadout::{Ability, Knack, WeaponClass, WeaponKind};
use shared::protocol::{EntityState, NetId, kind, projectile_flags};
use shared::sim::hitscan::assist_pads;
use shared::sim::projectiles::{ProjectileKind, projectile_target_pad, replicated_position};
use shared::sim::weapons::aim_basis;
use shared::sim::{AbilityPhase, FireIntent, Hitbox, Owner, PlayerId, replicated_hitbox};
use shared::tuning::CombatTuning;

use super::Rng;
use crate::net::NetClient;

/// Seconds over which a handed-over projectile eases from the stand-in's place onto its own
/// path.
pub const HANDOVER: f32 = 0.3;
/// Seconds over which a stand-in blends from the drawn muzzle onto its true (eye-based) path
/// (the same blend own projectiles get, `entities.rs`).
const MUZZLE_BLEND: f32 = 0.15;
/// Most stand-ins alive at once (oldest dropped).
const MAX_STAND_INS: usize = 48;
/// Largest time offset a handover makes up for (ticks).
const MAX_TIME_OFFSET: f64 = 15.0;
/// Ticks after a handed-over projectile went into a target on screen that the server may still
/// show it flying (its removal is in the next snapshot or two); later, it missed.
const SHOW_AGAIN_TICKS: f64 = 3.0;
/// Stand-in ids: far above anything the server allocates.
const FIRST_ID: u32 = 0xFFF0_0000;

/// Is `kind` flown from the shot's view time by the server (lag-compensated), so drawn at the
/// render tick and its stand-in on that clock?
pub fn render_clock_kind(kind: u16) -> bool {
    ProjectileKind::from_net_kind(kind).is_some_and(ProjectileKind::lag_compensated)
}

/// Is the local player's projectile `e` drawn at the render tick (see the module docs)? A
/// deflected one isn't (its flight restarted in the present), nor the kinds driven from it.
pub fn on_render_clock(e: &EntityState) -> bool {
    e.flags & projectile_flags::PARRIED == 0 && render_clock_kind(e.kind)
}

/// The local player's projectiles from the newest snapshot, the own-time to draw the
/// own-time ones at (the present, but never far past what the server has confirmed: impacts
/// we haven't heard of yet) and the local owner code.
pub fn own_projectiles(net: &NetClient) -> (Vec<EntityState>, f64, Option<u16>) {
    let Some(me) = net.local_player_id().map(|p| p.0 as u16) else {
        return (Vec::new(), 0.0, None);
    };
    let Some((latest, frame)) = net.frames().latest() else {
        return (Vec::new(), 0.0, Some(me));
    };
    let own = frame
        .states()
        .filter(|e| kind::is_projectile(e.kind) && e.owner == me)
        .collect();
    let now = net
        .server_tick_estimate()
        .unwrap_or(latest as f64)
        .min(latest as f64 + 6.0)
        .max(net.render_tick());
    (own, now, Some(me))
}

/// The local player's render-clock projectiles as displayed at the render tick.
pub fn rendered_own(net: &NetClient) -> Vec<EntityState> {
    let Some(me) = net.local_player_id().map(|p| p.0 as u16) else {
        return Vec::new();
    };
    net.remote_entities()
        .iter()
        .filter(|e| kind::is_projectile(e.kind) && e.owner == me && on_render_clock(e))
        .copied()
        .collect()
}

/// The hit shapes of everything the local player's projectiles can hit, as displayed at the
/// render tick: living Moulds, and other living players when they are hostile (`pvp`).
pub fn displayed_targets(net: &NetClient, pvp: bool) -> Vec<Hitbox> {
    let Some(world) = net.world() else {
        return Vec::new();
    };
    let me = net.local_player_id().map(NetId::player);
    net.remote_entities()
        .iter()
        .filter(|e| Some(e.id) != me && (pvp || e.kind != kind::PLAYER))
        .filter_map(|e| replicated_hitbox(e, &world.enemy_defs, &net.tuning().movement))
        .collect()
}

/// The time a shot's stand-in starts at on the render clock: the view time its input was
/// aimed at (sent with it), or for a shot shown before its tick (no input sent yet) the
/// render tick now (the input will carry the press frame's, `NetClient::latch_view`).
pub fn shot_view(net: &NetClient, input: &PlayerInput) -> f64 {
    if input.seq != 0 {
        input.view_time_ticks()
    } else {
        net.render_tick()
    }
}

/// Seconds an unclaimed stand-in waits for its projectile: until it shows at the render tick
/// (the round trip, the server's input buffer and the interpolation delay), and some.
pub fn timeout(net: &NetClient) -> f32 {
    let s = net.stats();
    ((s.rtt_ms + net.interp_delay_ms()) / 1000.0 * 1.25 + 0.25).clamp(0.3, 1.5)
}

/// First contact of a `kind` projectile owned by the local player moving `from` → `to`: with
/// one of `targets` (padded as the server pads it: hit assist plus `projectile_hit_pad`) before
/// the level, or with the level (plain radius): (fraction, surface normal if the level).
fn contact(
    kind: u16,
    from: Vec3,
    to: Vec3,
    targets: &[Hitbox],
    world: Option<&CollisionWorld>,
    t: &CombatTuning,
) -> Option<(f32, Option<Vec3>)> {
    let motion = to - from;
    if !motion.is_finite() || motion.length_squared() < 1e-10 {
        return None;
    }
    let pk = ProjectileKind::from_net_kind(kind);
    let radius = pk.map_or(0.1, |k| k.radius(t));
    let me = Owner::Player(PlayerId(0));
    let wall = world.and_then(|w| w.cast_sphere(from, radius, motion, MASK_SHOTS));
    let wall_f = wall.map_or(1.0, |h| h.fraction);
    let body = pk
        .filter(|k| k.hits_targets())
        .and_then(|_| {
            let r = radius + projectile_target_pad(t, me);
            targets
                .iter()
                .filter_map(|h| {
                    let pads = assist_pads(t, me, h.target);
                    h.sweep_sphere_padded(from, motion, r, pads).map(|(f, _)| f)
                })
                .min_by(|a, b| a.total_cmp(b))
        })
        .filter(|f| *f <= wall_f);
    match (body, wall) {
        (Some(f), _) => Some((f, None)),
        (None, Some(h)) => Some((h.fraction, Some(h.normal))),
        (None, None) => None,
    }
}

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
        // Only a use launches anything (a held ability's release, a recast: nothing new here).
        FireIntent::Ability { phase, .. } if phase != AbilityPhase::Use => Vec::new(),
        FireIntent::Ability {
            weapon,
            ability,
            charge,
            ..
        } => {
            if weapon.class() == WeaponClass::Draw {
                let kind = match ability {
                    Ability::PinningArrow => kind::PIN_ARROW,
                    Ability::Brand => kind::BRAND_ARROW,
                    Ability::BlinkArrow => kind::BLINK_ARROW,
                    Ability::BeaconArrow => kind::BEACON_ARROW,
                    _ => kind::ARROW,
                };
                return vec![arrow(kind, aim, charge)];
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
                Ability::Bolas => one(kind::BOLAS, safe(eye + aim * 0.5), aim * c.bolas.speed),
                Ability::Kite => one(kind::KITE, safe(eye + aim * 0.6), aim * c.kite.speed),
                Ability::Gravewell => one(
                    kind::GRAVEWELL,
                    eye,
                    lob_dir(aim, c.tumbler.gravewell_angle_degrees) * c.tumbler.gravewell_speed,
                ),
                Ability::Limpet if charge < 1.0 && live_limpets < c.limpet.max_live.max(1) => {
                    one(kind::LIMPET, safe(eye + aim * 0.5), aim * c.limpet.speed)
                }
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
            | ProjectileKind::BlinkArrow
            | ProjectileKind::BeaconArrow
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
    /// Runs on the render clock (else own-time; see the module docs).
    render_clock: bool,
    /// Its clock's time (fractional server ticks) at the shot: the render clock's is the
    /// shot's view time ...
    born: f64,
    /// ... and of the current trajectory segment (a bounce restarts it).
    t0: f64,
    /// Seconds since the shot.
    age: f32,
    /// Drawn muzzle minus the true launch point (blends out over [`MUZZLE_BLEND`]).
    muzzle: Vec3,
    /// Its true (undisplaced) position last frame: contact tests sweep from there, as the
    /// server's projectile flies the true path.
    last: Vec3,
    bounces_left: u32,
    /// Went into a target or vanished at the level: not drawn, still claimable.
    hidden: bool,
}

impl Ghost {
    /// True (undisplaced) position at time `now` of its clock.
    fn path(&self, now: f64, rate: u32, t: &CombatTuning) -> Vec3 {
        replicated_position(&self.entity, (now - self.t0).max(0.0), rate, t)
    }

    fn muzzle_weight(&self) -> f32 {
        (1.0 - self.age / MUZZLE_BLEND).clamp(0.0, 1.0)
    }

    /// Its clock's time now.
    fn now(&self, f: &StandInFrame) -> f64 {
        if self.render_clock { f.render } else { f.now }
    }
}

/// A handed-over projectile easing from the stand-in's place onto its own path.
#[derive(Clone, Copy, Debug)]
struct Handover {
    /// Ticks the stand-in was ahead along the flight (the input delay on own-time; on the
    /// render clock what a capped compensation didn't make up) ...
    time: f64,
    /// ... and what was left over (spread, launch point).
    offset: Vec3,
    since: f32,
}

/// A handed-over render-clock projectile in flight, watched against the targets on screen.
#[derive(Clone, Copy, Debug)]
struct Watch {
    /// Its true position last frame.
    last: Vec3,
    /// The render tick it went into a target: not drawn from then on ...
    into: Option<f64>,
    /// ... unless the server still had it flying [`SHOW_AGAIN_TICKS`] later: it missed (no
    /// more tests).
    missed: bool,
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
    /// Own projectiles already seen in the newest snapshot / at the render tick (only a new
    /// one can claim a stand-in of that clock).
    seen: HashSet<NetId>,
    seen_rendered: HashSet<NetId>,
    /// Handed-over render-clock projectiles in flight.
    watch: HashMap<NetId, Watch>,
    /// Stand-ins handed over this update: (stand-in id, real id), so per-projectile visuals
    /// (trails) carry over.
    claims: Vec<(NetId, NetId)>,
    next: u32,
    pub stats: StandInStats,
}

/// What a frame's update needs.
pub struct StandInFrame<'a> {
    /// The local player's projectiles from the newest snapshot (own-time ones hand over from
    /// here) ...
    pub own: &'a [EntityState],
    /// ... and own-time now (fractional server ticks, `combat::own_projectiles`).
    pub now: f64,
    /// The local player's render-clock projectiles as displayed ([`on_render_clock`], from the
    /// interpolated entities) ...
    pub rendered: &'a [EntityState],
    /// ... and the render tick they are displayed at.
    pub render: f64,
    pub tick_rate: u32,
    pub tuning: &'a CombatTuning,
    pub world: Option<&'a CollisionWorld>,
    /// The hit shapes of everything the local player's projectiles can hit, as displayed
    /// (`shared::sim::replicated_hitbox` of the hostile entities at the render tick): a
    /// stand-in stops where the server's projectile does.
    pub targets: &'a [Hitbox],
    /// Seconds an unclaimed stand-in lives (until its projectile shows at the render tick,
    /// and some).
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
        self.seen_rendered.clear();
        self.watch.clear();
        self.claims.clear();
    }

    pub fn len(&self) -> usize {
        self.ghosts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ghosts.is_empty()
    }

    /// Starts a stand-in for `launch`, drawn from `muzzle` (the drawn weapon's muzzle), owned
    /// by owner code `owner`: at own-time `now`, or (a render-clock kind) at `view`, the view
    /// time the shot was aimed at. Returns its (local) id.
    pub fn spawn(
        &mut self,
        launch: Launch,
        muzzle: Vec3,
        owner: u16,
        now: f64,
        view: f64,
        t: &CombatTuning,
    ) -> Option<NetId> {
        let render_clock = render_clock_kind(launch.kind);
        let now = if render_clock { view } else { now };
        if !(launch.start.is_finite() && launch.velocity.is_finite() && now.is_finite()) {
            return None;
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
            render_clock,
            born: now,
            t0: now,
            age: 0.0,
            muzzle,
            last: launch.start,
            // A bomb bounces until its last contact (it explodes on that one).
            bounces_left: if launch.kind == kind::BOMB {
                t.tumbler.bomb_contacts.saturating_sub(1)
            } else {
                0
            },
            hidden: false,
        });
        self.stats.spawned += 1;
        Some(id)
    }

    /// Every stand-in for tests and tools: (id, gone into a body or vanished at the level,
    /// stuck in the level).
    pub fn ghosts(&self) -> impl Iterator<Item = (NetId, bool, bool)> + '_ {
        self.ghosts.iter().map(|g| {
            let stuck = g.entity.flags & projectile_flags::STUCK != 0;
            (g.entity.id, g.hidden, stuck)
        })
    }

    /// Advances the stand-ins (contacts, expiry), hands them over to newly seen own
    /// projectiles and watches the handed-over ones. `dt`: real seconds.
    pub fn update(&mut self, f: &StandInFrame, dt: f32) {
        let dt = dt.max(0.0);
        let t = f.tuning;
        let rate = f.tick_rate;
        // Handovers fade; forget projectiles that are gone.
        let own: HashSet<NetId> = f.own.iter().map(|e| e.id).collect();
        let shown: HashSet<NetId> = f.rendered.iter().map(|e| e.id).collect();
        self.seen.retain(|id| own.contains(id));
        self.seen_rendered.retain(|id| shown.contains(id));
        self.watch.retain(|id, _| shown.contains(id));
        self.handovers
            .retain(|id, _| own.contains(id) || shown.contains(id));
        for h in self.handovers.values_mut() {
            h.since += dt;
        }
        self.handovers.retain(|_, h| h.since < HANDOVER);

        // Move the stand-ins along their true paths, stopping where the server's would.
        for g in &mut self.ghosts {
            g.age += dt;
            if g.hidden || g.entity.flags & projectile_flags::STUCK != 0 {
                continue;
            }
            let now = g.now(f);
            let to = g.path(now, rate, t);
            let from = g.last;
            g.last = to;
            let Some((frac, wall)) = contact(g.entity.kind, from, to, f.targets, f.world, t) else {
                continue;
            };
            let at = from.lerp(to, frac);
            let Some(n) = wall else {
                // Into a target: the server decides what happens there.
                g.hidden = true;
                continue;
            };
            match on_world(g.entity.kind) {
                OnWorld::Bounce if g.bounces_left > 0 => {
                    // Restart the trajectory from the contact, reflected.
                    let vel = super::entities::replicated_velocity(
                        &g.entity,
                        (now - g.t0).max(0.0),
                        rate,
                        t,
                    );
                    let reflected = (vel - n * (2.0 * vel.dot(n))) * t.tumbler.bomb_restitution;
                    g.entity.position = at + n * 0.02;
                    g.entity.velocity = reflected;
                    g.t0 = now;
                    g.bounces_left -= 1;
                    g.last = g.entity.position;
                }
                OnWorld::Stick => {
                    g.entity.position = at;
                    g.entity.flags |= projectile_flags::STUCK;
                    g.muzzle = Vec3::ZERO;
                }
                _ => g.hidden = true,
            }
        }

        // Hand over: every own projectile seen for the first time claims the closest
        // stand-in of its kind and clock (render-clock ones once displayed, where the stand-in
        // already is: its path at the projectile's launch tick; own-time ones by launch point).
        self.claims.clear();
        for e in f.own.iter().filter(|e| !on_render_clock(e)) {
            if !self.seen.insert(e.id) || e.flags & projectile_flags::PARRIED != 0 {
                continue;
            }
            if let Some(i) = self.closest(e, false, f, |g| g.born) {
                self.hand_over(i, e, f.now, f);
            }
        }
        for e in f.rendered {
            if !self.seen_rendered.insert(e.id) {
                continue;
            }
            if let Some(i) = self.closest(e, true, f, |_| e.aux as f64) {
                self.hand_over(i, e, f.render, f);
            }
        }

        // Handed-over projectiles in flight stop on screen where they reach a target (or the
        // level), as their stand-ins did.
        for e in f.rendered {
            let at = replicated_position(e, f.render, rate, t);
            let w = self.watch.entry(e.id).or_insert(Watch {
                last: at,
                into: None,
                missed: false,
            });
            let flying = e.flags & (projectile_flags::STUCK | projectile_flags::PENDING) == 0;
            match w.into {
                // Stuck in it (or the server has it otherwise now): shown as the server has it.
                _ if !flying => w.into = None,
                Some(when) if f.render > when + SHOW_AGAIN_TICKS => {
                    w.into = None;
                    w.missed = true;
                }
                None if !w.missed => {
                    // Bombs bounce off the level (the server's new flight shows it).
                    let world = f.world.filter(|_| on_world(e.kind) != OnWorld::Bounce);
                    if contact(e.kind, w.last, at, f.targets, world, t).is_some() {
                        w.into = Some(f.render);
                    }
                }
                _ => {}
            }
            w.last = at;
        }

        // Unclaimed for too long: the server never had it in flight (or it's long gone).
        let before = self.ghosts.len();
        self.ghosts.retain(|g| g.age < f.timeout.max(0.1));
        self.stats.expired += (before - self.ghosts.len()) as u64;
    }

    /// The unclaimed stand-in of `e`'s kind on its clock whose state at time `when` (of that
    /// clock: the moment `e`'s replicated state describes) is closest to `e`'s, if close enough:
    /// a few metres off its line (more far out: spread the client can't know), anywhere along it
    /// within [`MAX_TIME_OFFSET`] of flight (a compensation the server capped).
    fn closest(
        &self,
        e: &EntityState,
        render_clock: bool,
        f: &StandInFrame,
        when: impl Fn(&Ghost) -> f64,
    ) -> Option<usize> {
        let (rate, t) = (f.tick_rate, f.tuning);
        let dir = e.velocity.normalize_or_zero();
        self.ghosts
            .iter()
            .enumerate()
            .filter(|(_, g)| g.entity.kind == e.kind && g.render_clock == render_clock)
            .filter_map(|(i, g)| {
                let time = when(g);
                let p = g.path(time, rate, t);
                let vel = super::entities::replicated_velocity(
                    &g.entity,
                    (time - g.t0).max(0.0),
                    rate,
                    t,
                );
                let off = p - e.position;
                let along = off.dot(dir);
                let across = (off - dir * along).length();
                let turn = vel.normalize_or_zero().dot(dir);
                let reach = 3.0 + 0.06 * p.distance(g.path(g.born, rate, t));
                let behind = vel.length() * (MAX_TIME_OFFSET as f32 / rate.max(1) as f32) + 3.0;
                let score = across + 0.1 * along.abs() + (1.0 - turn) * 20.0;
                (across + (1.0 - turn) * 20.0 < reach && along.abs() < behind).then_some((i, score))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Stand-in `i` hands over to `e` (drawn at time `now` of its clock): the time offset is
    /// how far along its flight the stand-in is ahead, the position offset what is left.
    fn hand_over(&mut self, i: usize, e: &EntityState, now: f64, f: &StandInFrame) {
        let (rate, t) = (f.tick_rate, f.tuning);
        let g = self.ghosts.remove(i);
        let drawn = g.path(g.now(f), rate, t) + g.muzzle * g.muzzle_weight();
        let real = replicated_position(e, now, rate, t);
        let vel = super::entities::replicated_velocity(e, now, rate, t);
        let speed2 = vel.length_squared();
        let time = if g.hidden || speed2 < 1e-6 {
            0.0
        } else {
            ((drawn - real).dot(vel) / speed2 * rate as f32) as f64
        };
        let time = if time.is_finite() {
            time.clamp(-MAX_TIME_OFFSET, MAX_TIME_OFFSET)
        } else {
            0.0
        };
        let left = drawn - replicated_position(e, now + time, rate, t);
        let offset = if g.hidden || !left.is_finite() {
            Vec3::ZERO
        } else {
            left
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

    /// Stand-ins to draw this frame: (entity, the time to evaluate its trajectory at, visual
    /// offset), like `EntityFrame::projectiles`; `now` is own-time, `render` the render tick.
    pub fn drawn(&self, now: f64, render: f64) -> impl Iterator<Item = (&EntityState, f64, Vec3)> {
        self.ghosts.iter().filter(|g| !g.hidden).map(move |g| {
            let stuck = g.entity.flags & projectile_flags::STUCK != 0;
            let offset = if stuck {
                Vec3::ZERO
            } else {
                g.muzzle * g.muzzle_weight()
            };
            let clock = if g.render_clock { render } else { now };
            (&g.entity, (clock - g.t0).max(0.0), offset)
        })
    }

    /// A handed-over own projectile that reached a target (or the level) on screen: not drawn
    /// (the server's removal or stop is on its way).
    pub fn hidden(&self, id: NetId) -> bool {
        self.watch.get(&id).is_some_and(|w| w.into.is_some())
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

    /// A frame at render tick `render` showing `rendered` (the newest snapshot has them too;
    /// own-time is ahead of the render tick).
    fn frame<'a>(
        rendered: &'a [EntityState],
        render: f64,
        t: &'a CombatTuning,
        targets: &'a [Hitbox],
    ) -> StandInFrame<'a> {
        StandInFrame {
            own: rendered,
            now: render + 8.0,
            rendered,
            render,
            tick_rate: 60,
            tuning: t,
            world: None,
            targets,
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

    /// The same flight as `e`, replicated from tick `tick` on (as the server has it after
    /// making up its flight, or after a later step).
    fn from_tick(e: &EntityState, tick: u32, t: &CombatTuning) -> EntityState {
        EntityState {
            position: replicated_position(e, tick as f64, 60, t),
            velocity: super::super::entities::replicated_velocity(e, tick as f64, 60, t),
            aux: tick,
            ..*e
        }
    }

    /// A standing player at `feet` (body capsule and head, as the server shapes them).
    fn player_at(feet: Vec3) -> Hitbox {
        let t = shared::tuning::MovementTuning::default();
        let m = shared::movement::MoveState::spawn(feet);
        shared::sim::player_hitbox(PlayerId(3), &m, &t)
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
                    ability: Ability::BeaconArrow,
                    charge: 1.0,
                    phase: AbilityPhase::Use,
                },
                1,
            ),
            (
                FireIntent::Ability {
                    weapon: WeaponKind::Longwick,
                    ability: Ability::BlinkArrow,
                    charge: 0.0,
                    phase: AbilityPhase::Recast,
                },
                0,
            ),
            (
                FireIntent::Ability {
                    weapon: WeaponKind::Cinderbox,
                    ability: Ability::Limpet,
                    charge: 1.0,
                    phase: AbilityPhase::Use,
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
        let gravewell = FireIntent::Ability {
            weapon: WeaponKind::Tumbler,
            ability: Ability::Gravewell,
            charge: 0.0,
            phase: AbilityPhase::Use,
        };
        let m = launches(&gravewell, aim, eye, &t, None, 0, &mut rng)[0];
        assert!(m.velocity.y > 0.0);
        // The fourth limpet detonates instead of throwing.
        let limpet = FireIntent::Ability {
            weapon: WeaponKind::Cinderbox,
            ability: Ability::Limpet,
            charge: 0.0,
            phase: AbilityPhase::Use,
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
        // Drawn: the stand-in and the server's rivet once displayed, at render tick `render`.
        let drawn_at = |s: &StandIns, render: f64, shown: &[EntityState]| -> (Vec3, usize) {
            let mut n = 0;
            let mut p = Vec3::ZERO;
            for (e, time, off) in s.drawn(render + 8.0, render) {
                p = replicated_position(e, time, rate, &t) + off;
                n += 1;
            }
            for e in shown.iter().filter(|e| !s.hidden(e.id)) {
                let (dtk, off) = s.handover(e.id).unwrap_or((0.0, Vec3::ZERO));
                p = replicated_position(e, render + dtk, rate, &t) + off;
                n += 1;
            }
            (p, n)
        };
        // Aimed at view time 100.0; the server launched it on tick 109 having made up its
        // flight since then (`lag` ticks short of that when its compensation was capped).
        for lag in [0u32, 4] {
            let mut s = StandIns::new();
            s.spawn(
                launch,
                start + Vec3::new(0.12, -0.1, 0.0),
                1,
                140.0,
                100.0,
                &t,
            );
            let flight = server(500, kind::RIVET, start, velocity, 100 + lag);
            let real = from_tick(&flight, 109, &t);
            let mut render = 100.0;
            let dt = 1.0 / 144.0;
            let mut last: Option<Vec3> = None;
            let mut biggest_step = 0.0f32;
            for i in 0..90 {
                render += dt as f64 * rate as f64;
                // Displayed from its first snapshot (tick 110) on.
                let shown: Vec<EntityState> = if render >= 110.0 {
                    vec![real]
                } else {
                    Vec::new()
                };
                s.update(&frame(&shown, render, &t, &[]), dt);
                let (p, n) = drawn_at(&s, render, &shown);
                assert_eq!(n, 1, "lag {lag}, frame {i}: exactly one rivet drawn");
                if let Some(prev) = last {
                    biggest_step = biggest_step.max((p - prev).length());
                }
                last = Some(p);
            }
            // 90 m/s at 144 frames/s is 0.625 m per frame: no frame jumps much more than that
            // (a capped compensation eases back over the handover).
            assert!(biggest_step < 0.8, "lag {lag}: biggest step {biggest_step}");
            assert_eq!(s.stats.claimed, 1);
            assert!(s.is_empty());
            // After the handover the real rivet is drawn on its own path.
            let (dtk, off) = s.handover(NetId(500)).unwrap_or((0.0, Vec3::ZERO));
            assert!(dtk.abs() < 1e-6 && off.length() < 1e-6);
        }
        // Fully compensated it is where the stand-in was: nothing to make up.
        let mut s = StandIns::new();
        s.spawn(launch, start, 1, 140.0, 100.0, &t);
        let real = from_tick(&server(500, kind::RIVET, start, velocity, 100), 109, &t);
        s.update(&frame(&[real], 110.0, &t, &[]), 1.0 / 60.0);
        let (dtk, off) = s.handover(NetId(500)).expect("handed over");
        assert!(dtk.abs() < 1e-3 && off.length() < 1e-3, "{dtk} {off}");
    }

    #[test]
    fn stand_ins_stop_in_targets_as_the_server_pads_them() {
        let t = tuning();
        let arrow = |x: f32| Launch {
            kind: kind::ARROW,
            start: Vec3::new(x, 0.8, 0.0),
            velocity: Vec3::new(0.0, 0.0, -100.0),
        };
        // A player 5 m ahead: its body capsule is 0.4 m (`movement.radius`) wide each way.
        let targets = [player_at(Vec3::new(0.0, 0.0, -5.0))];
        let plain = t.longwick.arrow_radius + 0.4;
        let padded = plain + t.hit_assist.player_body_pad + t.projectile_hit_pad;
        for (x, hits) in [
            (0.0, true),
            // Beside the plain shapes but inside the server's pads: a hit.
            (plain + 0.05, true),
            (padded - 0.02, true),
            (padded + 0.05, false),
        ] {
            let mut s = StandIns::new();
            s.spawn(arrow(x), arrow(x).start, 1, 58.0, 50.0, &t);
            let mut render = 50.0;
            for _ in 0..10 {
                render += 1.0;
                s.update(&frame(&[], render, &t, &targets), 1.0 / 60.0);
            }
            let drawn = s.drawn(render + 8.0, render).count();
            assert_eq!(drawn == 0, hits, "{x} m off the axis");
            assert_eq!(s.len(), 1, "still claimable");
        }
        // Unclaimed, it fades.
        let mut s = StandIns::new();
        s.spawn(arrow(0.0), arrow(0.0).start, 1, 58.0, 50.0, &t);
        let mut render = 50.0;
        for _ in 0..70 {
            render += 1.0;
            s.update(&frame(&[], render, &t, &targets), 1.0 / 60.0);
        }
        assert!(s.is_empty());
        assert_eq!(s.stats.expired, 1);
    }

    #[test]
    fn a_handed_over_arrow_stops_on_screen_where_it_reaches_a_target() {
        let t = tuning();
        let start = Vec3::new(0.0, 1.2, 0.0);
        let velocity = Vec3::new(0.0, 0.0, -100.0);
        let launch = Launch {
            kind: kind::ARROW,
            start,
            velocity,
        };
        // A player 20 m ahead: reached 12 ticks into the flight.
        let targets = [player_at(Vec3::new(0.0, 0.0, -20.0))];
        let flight = server(700, kind::ARROW, start, velocity, 100);
        let real = from_tick(&flight, 103, &t);
        for server_hits in [true, false] {
            let mut s = StandIns::new();
            s.spawn(launch, start, 1, 108.0, 100.0, &t);
            let mut drawn_after_contact = Vec::new();
            let mut render = 100.0;
            while render < 125.0 {
                render += 0.25;
                // The server removes it once it hit (snapshot of tick 114); else it flies on.
                let shown: Vec<EntityState> = if render >= 104.0 && (!server_hits || render < 114.0)
                {
                    vec![real]
                } else {
                    Vec::new()
                };
                s.update(&frame(&shown, render, &t, &targets), 0.25 / 60.0);
                let n = s.drawn(render + 8.0, render).count()
                    + shown.iter().filter(|e| !s.hidden(e.id)).count();
                if render > 112.0 {
                    drawn_after_contact.push(n);
                }
            }
            assert_eq!(s.stats.claimed, 1);
            if server_hits {
                assert!(
                    drawn_after_contact.iter().all(|n| *n == 0),
                    "gone into the target: {drawn_after_contact:?}"
                );
            } else {
                // Hidden at the contact, shown again once the server still has it flying.
                assert_eq!(drawn_after_contact.last(), Some(&1));
                assert!(drawn_after_contact.contains(&0));
            }
        }
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
        s.spawn(launch, launch.start, 1, 18.0, 10.0, &t);
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
