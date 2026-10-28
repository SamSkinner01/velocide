//! Bow hit registration at latency (docs/feel.md "Hit registration").
//!
//! A real server and a real client over UDP loopback with the network simulator, on a virtual
//! clock. The client looses Longwick arrows at one target — an Effigy strafing on a script, a
//! Heron or a Gnawer on their own AI, a strafing player — aiming with lead and drop where *its
//! screen* shows the target, and every arrow gets three verdicts:
//!
//! * **view**: the arrow flown on the client's display clock from the moment its shot was
//!   aimed (the input's view time) against the hit shapes of the target as the client displays
//!   it at each moment, with the hit assist and projectile pad the server adds: what the player
//!   can expect from what they saw;
//! * **drawn**: what the client drew of it (its stand-in, then the server's projectile it
//!   handed over to): did it visibly reach the target and stop there?
//! * **server**: what the server's projectile did.
//!
//! `measure_bow_hit_registration` (ignored: `cargo test -p client --test bow_hitreg --
//! --ignored --nocapture`) prints the whole matrix; the other tests lock the numbers in.

use std::path::PathBuf;
use std::time::Duration;

use client::ingame::combat::Rng;
use client::ingame::combat::stand_in::{self, StandInFrame, StandIns};
use client::ingame::combat::weapon_feel::WeaponPredictor;
use client::net::{ClientConfig, NetClient, NetSimConfig};
use engine::physics::MASK_SHOTS;
use engine::time::FixedTimestep;
use glam::Vec3;
use server::{Server, ServerConfig};
use shared::input::{Buttons, PlayerInput, look_dir};
use shared::loadout::{Loadout, WeaponKind};
use shared::movement::MoveState;
use shared::protocol::{EntityState, NetId};
use shared::sim::projectiles::{ProjectileKind, replicated_position};
use shared::sim::{
    FireIntent, HitPads, HitPart, HitTarget, Hitbox, Owner, PlayerId, SimEvent, replicated_hitbox,
};
use shared::tuning::Tuning;

const STEP: Duration = Duration::from_millis(2);
const RATE: u32 = 60;

fn config() -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        level: shared::sim::BUILTIN_LEVEL.to_string(),
        // Defaults only: tests must not depend on the repo's assets/tuning.ron.
        tuning_path: Some(PathBuf::from("/nonexistent/velocide-test/tuning.ron")),
        enemies_dir: Some(PathBuf::from("/nonexistent/velocide-test/enemies")),
        watch_tuning: false,
        ..Default::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// Strafes left and right on a script (4–8 m/s, 0.5–1.5 s legs), never dies.
    Effigy,
    /// A ranged Mould on its own AI (strafes in its band).
    Heron,
    /// A melee Mould on its own AI (charges; put back at range when it gets close).
    Gnawer,
    /// Another client strafing (A/D, jumps).
    Player,
}

#[derive(Clone, Copy, Debug)]
struct Scenario {
    one_way_ms: f32,
    jitter_ms: f32,
    target: Target,
    /// The shooter strafes at full speed while it shoots.
    moving: bool,
    /// Only full draws (else full, full, half, tap in turn).
    full_only: bool,
    arrows: usize,
    seed: u64,
    /// `net.projectile_lag_comp_max_ms` on the server (`None`: the default).
    projectile_lag_comp_ms: Option<f32>,
}

impl Scenario {
    fn new(one_way_ms: f32, target: Target) -> Self {
        Self {
            one_way_ms,
            jitter_ms: (one_way_ms * 0.15).max(2.0),
            target,
            moving: false,
            full_only: false,
            arrows: 60,
            seed: 7,
            projectile_lag_comp_ms: None,
        }
    }
}

/// What happened to one arrow.
#[derive(Clone, Debug, Default)]
struct Arrow {
    /// The shot's view time (ticks) ...
    view: f64,
    /// ... launch point and velocity.
    start: Vec3,
    velocity: Vec3,
    /// The client's stand-in and the server projectile it handed over to.
    ghost: Option<NetId>,
    claimed: Option<NetId>,
    /// The target was teleported while it flew (Gnawer reset): not counted.
    void: bool,
    // ---- view ----
    view_last: Option<Vec3>,
    view_done: bool,
    view_hit: Option<HitPart>,
    // ---- drawn ----
    drawn_last: Option<Vec3>,
    /// Where the drawn arrow first touched the target ...
    drawn_touch: Option<Vec3>,
    /// ... and how far it was drawn past that point afterwards.
    drawn_beyond: f32,
    /// The stand-in, or the projectile it handed over to, stopped in a target on screen ...
    hid: bool,
    /// ... and was drawn flying again afterwards (the server had it fly on).
    reappeared: bool,
    /// Drawn on the last frame.
    drawn_live: bool,
    /// Diagnostics: the handover's offsets (ticks, m) and the render tick it was last drawn at.
    handover: Option<(f64, f32)>,
    drawn_at: f64,
    // ---- server ----
    spawn_tick: Option<u32>,
    server_id: Option<NetId>,
    server_done: bool,
    server_hit: Option<HitPart>,
    server_tick: u32,
}

impl Arrow {
    /// The client drew a hit: the arrow went into the target and stayed gone, or the drawn
    /// arrow touched the target and stopped there.
    fn drawn_hit(&self) -> bool {
        (self.hid && !self.reappeared) || (self.drawn_touch.is_some() && self.drawn_beyond < 1.0)
    }
}

/// Counts over one scenario.
#[derive(Clone, Copy, Debug, Default)]
struct Tally {
    arrows: u32,
    view_hits: u32,
    server_hits: u32,
    /// The view said hit, the server missed ...
    lost: u32,
    /// ... the view said miss, the server hit.
    gifted: u32,
    /// Both hit, but a different part (weak point vs body).
    wrong_part: u32,
    /// Weak-point hits by the view.
    view_weak: u32,
    /// The client drew a hit the server missed ...
    drawn_false_hit: u32,
    /// ... or drew a miss (or the arrow flying on through) where the server hit.
    drawn_false_miss: u32,
    /// Mean shooter RTT (ms) and server rewind (ms).
    rtt_ms: f32,
    rewind_ms: f32,
    /// Mean distance (m) of the launch point off the crosshair line from the drawn eye.
    parallax_m: f32,
}

impl Tally {
    fn agreement(&self) -> f32 {
        1.0 - (self.lost + self.gifted) as f32 / self.arrows.max(1) as f32
    }

    fn drawn_agreement(&self) -> f32 {
        1.0 - (self.drawn_false_hit + self.drawn_false_miss) as f32 / self.arrows.max(1) as f32
    }
}

struct Client {
    net: NetClient,
    ts: FixedTimestep,
    ticks: u32,
    predictor: WeaponPredictor,
    stand_ins: StandIns,
    rng: Rng,
}

impl Client {
    fn new(server: &Server, name: &str, netsim: Option<NetSimConfig>) -> Self {
        let net = NetClient::connect(
            server.local_addr(),
            ClientConfig::new(name).with_netsim(netsim),
        )
        .expect("client starts");
        Self {
            net,
            ts: FixedTimestep::new(RATE as f64),
            ticks: 0,
            predictor: WeaponPredictor::new(),
            stand_ins: StandIns::new(),
            rng: Rng::default(),
        }
    }

    fn id(&self) -> PlayerId {
        self.net.local_player_id().expect("connected")
    }

    fn eye(&self) -> Vec3 {
        let t = self.net.tuning();
        self.net
            .local_state()
            .map_or(Vec3::ZERO, |s| s.eye_position(&t.movement))
    }
}

/// A tiny deterministic generator for the scripts.
struct Lcg(u64);

impl Lcg {
    fn unit(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Yaw / pitch looking along `d`.
fn angles(d: Vec3) -> (f32, f32) {
    let d = d.normalize_or(Vec3::NEG_Z);
    ((-d.x).atan2(-d.z), d.y.clamp(-1.0, 1.0).asin())
}

/// The target as `net` displays it (interpolated at its render tick) and its hit shapes.
fn displayed(net: &NetClient, id: NetId) -> Option<(EntityState, Hitbox)> {
    let e = *net.remote_entity(id)?;
    let defs = &net.world()?.enemy_defs;
    let h = replicated_hitbox(&e, defs, &net.tuning().movement)?;
    Some((e, h))
}

/// The pads the server gives a player's arrow against `target`, and the arrow's radius against
/// targets (`hit_pads`, `projectile_target_pad`).
fn arrow_pads(t: &Tuning, target: HitTarget) -> (HitPads, f32) {
    let a = &t.combat.hit_assist;
    let pads = match target {
        HitTarget::Entity(_) => HitPads {
            body: a.enemy_body_pad,
            weak: a.enemy_weak_pad,
        },
        HitTarget::Player(_) => HitPads {
            body: a.player_body_pad,
            weak: a.player_head_pad,
        },
    };
    let radius = ProjectileKind::Arrow.radius(&t.combat) + t.combat.projectile_hit_pad;
    (pads, radius)
}

/// First contact of an arrow moving `from` → `to` with `target` (before the level).
fn contact(net: &NetClient, target: &Hitbox, from: Vec3, to: Vec3) -> Option<(f32, HitPart)> {
    let t = net.tuning();
    let motion = to - from;
    let (pads, radius) = arrow_pads(t, target.target);
    let (f, part) = target.sweep_sphere_padded(from, motion, radius, pads)?;
    let world = net.world().map(|w| &w.collision.world)?;
    let r = ProjectileKind::Arrow.radius(&t.combat);
    let wall = world
        .cast_sphere(from, r, motion, MASK_SHOTS)
        .map_or(1.0, |h| h.fraction);
    (f <= wall).then_some((f, part))
}

/// Does the level stop an arrow moving `from` → `to`?
fn walled(net: &NetClient, from: Vec3, to: Vec3) -> bool {
    let t = net.tuning();
    let r = ProjectileKind::Arrow.radius(&t.combat);
    net.world()
        .and_then(|w| {
            w.collision
                .world
                .cast_sphere(from, r, to - from, MASK_SHOTS)
        })
        .is_some()
}

/// The arrow on the client's display clock at render tick `render` (`None` before the shot).
fn view_position(a: &Arrow, render: f64, t: &Tuning) -> Option<Vec3> {
    let ticks = render - a.view;
    if ticks < 0.0 {
        return None;
    }
    let dt = (ticks / RATE as f64) as f32;
    let g = ProjectileKind::Arrow.gravity(&t.combat);
    Some(a.start + a.velocity * dt - Vec3::Y * (0.5 * g * dt * dt))
}

/// Where the client draws the arrow this frame (as `EntityFrame::projectiles` does): its
/// stand-in, or the server's projectile it handed over to, at the render tick (own-time for
/// projectiles not flown from the shot's view time).
fn drawn_position(c: &Client, a: &Arrow, frame: &Frame) -> Option<Vec3> {
    let t = &c.net.tuning().combat;
    if let Some(ghost) = a.ghost {
        for (e, time, offset) in c.stand_ins.drawn(frame.own_now, frame.render) {
            if e.id == ghost {
                return Some(replicated_position(e, time, RATE, t) + offset);
            }
        }
    }
    let id = a.claimed?;
    let (ahead, offset) = c.stand_ins.handover(id).unwrap_or((0.0, Vec3::ZERO));
    if let Some(e) = frame.rendered.iter().find(|e| e.id == id) {
        if c.stand_ins.hidden(id) {
            return None;
        }
        return Some(replicated_position(e, frame.render + ahead, RATE, t) + offset);
    }
    let e = frame
        .own
        .iter()
        .find(|e| e.id == id && !stand_in::on_render_clock(e))?;
    Some(replicated_position(e, frame.own_now + ahead, RATE, t) + offset)
}

/// One client frame's view of its own projectiles.
struct Frame {
    own: Vec<EntityState>,
    own_now: f64,
    rendered: Vec<EntityState>,
    render: f64,
}

// ---- the run ----

/// The shooter's draw / release cycle.
struct Cycle {
    held: u32,
    want: u32,
    rest: u32,
    fired: usize,
    head: bool,
}

impl Cycle {
    fn plan(&mut self, full_only: bool) {
        // Full draws (0.62 s), then a half and a tap.
        self.want = match (full_only, self.fired % 4) {
            (true, _) | (false, 0 | 1) => 42,
            (false, 2) => 20,
            _ => 3,
        };
        self.head = self.fired % 3 == 2;
    }
}

fn run(s: Scenario) -> (Tally, Vec<Arrow>) {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut server = Server::new(config()).expect("server starts");
    let netsim = Some(NetSimConfig::new(s.one_way_ms, s.jitter_ms, 0.0).with_seed(s.seed));
    let mut shooter = Client::new(&server, "Shooter", netsim);
    let mut other = (s.target == Target::Player).then(|| {
        let n = Some(NetSimConfig::new(25.0, 4.0, 0.0).with_seed(s.seed + 1));
        Client::new(&server, "Target", n)
    });
    // In game.
    for _ in 0..5000 {
        server.update(STEP);
        for c in std::iter::once(&mut shooter).chain(other.as_mut()) {
            c.net.update(STEP);
            c.net.drain_events();
            for _ in 0..c.ts.advance(STEP.as_secs_f64()) {
                if c.net.is_in_game() {
                    c.net.tick(PlayerInput::default());
                }
            }
        }
        if shooter.net.is_in_game() && other.as_ref().is_none_or(|o| o.net.is_in_game()) {
            break;
        }
    }
    assert!(shooter.net.is_in_game(), "shooter in game");
    let sid = shooter.id();
    if let Some(ms) = s.projectile_lag_comp_ms {
        let mut tuning = server.base_tuning().clone();
        tuning.net.projectile_lag_comp_max_ms = ms;
        server.set_tuning(tuning);
    }
    let shooter_home = Vec3::new(0.0, 0.0, 22.0);
    let target_home = Vec3::new(0.0, 0.0, -6.0);
    let target_id: NetId;
    {
        let w = server.world_mut();
        w.force_loadout(
            sid,
            Some(Loadout::single(WeaponKind::Longwick.default_build())),
        );
        let p = w.players.get_mut(&sid).unwrap();
        p.movement = MoveState::spawn(shooter_home);
        p.combat.god = true;
    }
    match s.target {
        Target::Player => {
            let tid = other.as_ref().unwrap().id();
            let w = server.world_mut();
            let p = w.players.get_mut(&tid).unwrap();
            p.movement = MoveState::spawn(target_home);
            p.combat.god = true;
            target_id = NetId::player(tid);
        }
        _ => {
            let name = match s.target {
                Target::Effigy => "effigy",
                Target::Heron => "heron",
                _ => "gnawer",
            };
            let at = if s.target == Target::Gnawer {
                Vec3::new(0.0, 0.0, -18.0)
            } else {
                target_home
            };
            target_id = server
                .world_mut()
                .spawn_enemy(name, at, std::f32::consts::PI)
                .expect("spawned");
        }
    }

    let mut arrows: Vec<Arrow> = Vec::new();
    let mut cycle = Cycle {
        held: 0,
        want: 42,
        rest: 60,
        fired: 0,
        head: false,
    };
    cycle.plan(s.full_only);
    let mut rng = Lcg(s.seed.wrapping_mul(977) + 13);
    let mut strafe = (6.0f32, 40u32);
    let mut other_strafe = (127i8, 40u32);
    let mut last_tick = server.tick();
    let mut rewinds = Vec::new();
    let mut rtts = Vec::new();
    let mut parallax = Vec::new();
    let mut last_camera: Option<Vec3> = None;
    let mut settle = 0u32;
    let mut seen: Vec<(f64, Vec3)> = Vec::new();
    // Ticks the target was put back (a teleport).
    let mut teleports: Vec<u32> = Vec::new();
    let dt = shared::sim::tick_dt(RATE);

    while cycle.fired < s.arrows || settle < 1200 {
        if cycle.fired >= s.arrows {
            settle += 1;
        }
        // ---- the target's motion (server side) ----
        let tick = server.tick();
        let new_tick = tick != last_tick;
        last_tick = tick;
        if let Some(e) = server.world_mut().enemies.get_mut(&target_id) {
            match s.target {
                Target::Effigy => {
                    // A dummy stands still on its own: moved here between ticks (the next tick
                    // records and replicates it there).
                    if new_tick {
                        strafe.1 = strafe.1.saturating_sub(1);
                        if strafe.1 == 0 {
                            let speed = 4.0 + 4.0 * rng.unit();
                            let sign = if e.position.x > 8.0 {
                                -1.0
                            } else if e.position.x < -8.0 {
                                1.0
                            } else if rng.unit() < 0.7 {
                                -strafe.0.signum()
                            } else {
                                strafe.0.signum()
                            };
                            strafe = (sign * speed, 30 + (rng.unit() * 60.0) as u32);
                        }
                        e.position.x += strafe.0 * dt;
                    }
                }
                Target::Gnawer | Target::Heron => {
                    // Never dies; kept in front of the shooter (put back when it charges
                    // close or wanders off: arrows aimed or flying around then don't count).
                    e.max_health = 1.0e5;
                    e.health = 1.0e5;
                    let p = e.position;
                    let close = p.distance(shooter_home) < 10.0;
                    if close || p.x.abs() > 16.0 || p.z < -24.0 || p.z > 10.0 {
                        let x = (rng.unit() - 0.5) * 16.0;
                        e.position = Vec3::new(x, 0.0, -14.0);
                        e.velocity = Vec3::ZERO;
                        teleports.push(tick);
                    }
                }
                Target::Player => {}
            }
        }
        if let Some(p) = server.world_mut().players.get_mut(&sid)
            && p.movement.position.distance(shooter_home) > 25.0
        {
            // A strafing shooter never wanders off.
            p.movement = MoveState::spawn(shooter_home);
        }

        // ---- server ----
        server.update(STEP);
        let now = server.tick();
        for ev in server.drain_sim_events() {
            match ev {
                SimEvent::Shot(shot) if shot.shooter == sid && shot.ends.is_empty() => {
                    if let Some(a) = arrows.iter_mut().find(|a| a.spawn_tick.is_none()) {
                        a.spawn_tick = Some(now);
                    }
                    let lag = server.world().players[&sid].view_lag_ticks;
                    rewinds.push(lag * 1000.0 / RATE as f32);
                }
                SimEvent::Damage(d) if d.info.source == Owner::Player(sid) => {
                    let Some(pid) = d.info.projectile else {
                        continue;
                    };
                    if !arrows.iter().any(|a| a.server_id == Some(pid))
                        && let Some(a) = arrows.iter_mut().find(|a| {
                            a.spawn_tick.is_some() && a.server_id.is_none() && !a.server_done
                        })
                    {
                        a.server_id = Some(pid);
                    }
                    let on_target = d.target == HitTarget::Entity(target_id)
                        || target_id.as_player().map(HitTarget::Player) == Some(d.target);
                    if on_target
                        && let Some(a) = arrows
                            .iter_mut()
                            .find(|a| a.server_id == Some(pid) && !a.server_done)
                    {
                        a.server_hit = Some(d.info.part);
                        a.server_tick = d.tick;
                        a.server_done = true;
                    }
                }
                _ => {}
            }
        }
        {
            let w = server.world();
            for (pid, p) in &w.projectiles {
                if p.owner != Owner::Player(sid) || !p.kind.is_arrow() {
                    continue;
                }
                if !arrows.iter().any(|a| a.server_id == Some(*pid))
                    && let Some(a) = arrows
                        .iter_mut()
                        .find(|a| a.spawn_tick.is_some() && a.server_id.is_none() && !a.server_done)
                {
                    a.server_id = Some(*pid);
                }
            }
            for a in arrows.iter_mut().filter(|a| !a.server_done) {
                if let Some(id) = a.server_id {
                    // Gone, or stuck in the level: done.
                    match w.projectiles.get(&id) {
                        Some(p) if p.stuck.is_none() => {}
                        _ => a.server_done = true,
                    }
                } else if a.spawn_tick.is_some_and(|t| now > t + 1) {
                    // Gone on its first tick without touching the target: it hit the level.
                    a.server_done = true;
                }
            }
        }

        // ---- the target player ----
        if let Some(o) = other.as_mut() {
            o.net.update(STEP);
            o.net.drain_events();
            for _ in 0..o.ts.advance(STEP.as_secs_f64()) {
                if !o.net.is_in_game() {
                    continue;
                }
                other_strafe.1 = other_strafe.1.saturating_sub(1);
                let x = o.net.local_state().map_or(0.0, |m| m.position.x);
                if other_strafe.1 == 0 || x.abs() > 12.0 {
                    // Facing +Z, `right` runs along -X.
                    let flip = if x > 12.0 {
                        127
                    } else if x < -12.0 {
                        -127
                    } else {
                        -other_strafe.0
                    };
                    other_strafe = (flip, 18 + (rng.unit() * 50.0) as u32);
                }
                let mut inp = PlayerInput {
                    // Facing the shooter (+Z): `right` strafes along -X.
                    yaw: std::f32::consts::PI,
                    right: other_strafe.0,
                    ..Default::default()
                };
                inp.buttons.set(Buttons::JUMP, o.ticks % 97 == 0);
                o.net.tick(inp);
                o.ticks += 1;
            }
        }

        // ---- the shooter ----
        let c = &mut shooter;
        c.net.update(STEP);
        c.net.drain_events();
        let mut shots = Vec::new();
        for _ in 0..c.ts.advance(STEP.as_secs_f64()) {
            if !c.net.is_in_game() {
                continue;
            }
            let t = c.net.tuning().clone();
            let draw = c.predictor.state().draw_fraction(&t.combat, RATE);
            let l = &t.combat.longwick;
            let speed = l.min_speed + (l.max_speed - l.min_speed) * draw;
            let mut inp = PlayerInput {
                weapon: 0,
                ..Default::default()
            };
            if s.moving {
                inp.right = if (c.ticks / 50).is_multiple_of(2) {
                    127
                } else {
                    -127
                };
            }
            // Aim where the screen shows the target, leading it by the flight time at the
            // speed it is seen moving (over the last ~100 ms of the display).
            if let Some((e, h)) = displayed(&c.net, target_id) {
                let render = c.net.render_tick();
                seen.push((render, e.position));
                if seen.len() > 6 {
                    seen.remove(0);
                }
                let (t0, p0) = seen[0];
                let target_vel = if render - t0 > 0.5 {
                    (e.position - p0) / ((render - t0) as f32 * dt)
                } else {
                    Vec3::ZERO
                };
                // A Mould's first weak point is its head.
                let part = match (cycle.head, h.target) {
                    (false, _) => HitPart::Body,
                    (true, HitTarget::Player(_)) => HitPart::Head,
                    (true, HitTarget::Entity(_)) => HitPart::Weak(0),
                };
                let at = h
                    .part_center(part)
                    .or_else(|| h.center())
                    .unwrap_or(e.position);
                let vel = c.net.local_state().map_or(Vec3::ZERO, |m| m.velocity);
                let eye = c.eye() + vel * dt;
                let mut flight = (at - eye).length() / speed.max(1.0);
                for _ in 0..3 {
                    let p = at + target_vel * flight;
                    flight = (p - eye).length() / speed.max(1.0);
                }
                let g = ProjectileKind::Arrow.gravity(&t.combat);
                let aim = at + target_vel * flight + Vec3::Y * (0.5 * g * flight * flight);
                (inp.yaw, inp.pitch) = angles(aim - eye);
            }
            // Draw, release, rest.
            if cycle.fired < s.arrows {
                if cycle.rest > 0 {
                    cycle.rest -= 1;
                } else if cycle.held < cycle.want {
                    cycle.held += 1;
                    inp.buttons.set(Buttons::FIRE, true);
                } else {
                    cycle.held = 0;
                    cycle.rest = 22;
                    cycle.fired += 1;
                    cycle.plan(s.full_only);
                }
            }
            c.net.tick(inp);
            c.ticks += 1;
            if let Some(shot) = c.predictor.after_tick(&c.net) {
                // How far off the line of the crosshair (from the eye as last drawn) the
                // arrow starts: the shooter's own movement since.
                if !shot.intents.is_empty()
                    && let (Some(cam), Some(eye)) = (last_camera, shot.eye)
                {
                    let aim = look_dir(shot.input.yaw, shot.input.pitch);
                    let off = eye - cam;
                    parallax.push((off - aim * off.dot(aim)).length());
                }
                shots.push(shot);
            }
        }
        rtts.push(c.net.stats().rtt_ms);
        let eye_height = c.net.tuning().movement.eye_height;
        last_camera = c
            .net
            .local_render_position(c.ts.alpha())
            .map(|p| p + Vec3::Y * eye_height);

        // ---- the frame: stand-ins, what is drawn, what the view says ----
        let t = c.net.tuning().clone();
        let (own, own_now, _) = stand_in::own_projectiles(&c.net);
        let frame = Frame {
            own,
            own_now,
            rendered: stand_in::rendered_own(&c.net),
            render: c.net.render_tick(),
        };
        let me = c.id().0 as u16;
        let Client {
            net,
            stand_ins,
            rng: crng,
            ..
        } = &mut *c;
        let world = net.world().map(|w| &w.collision.world);
        for shot in &shots {
            for intent in &shot.intents {
                if !matches!(
                    intent,
                    FireIntent::Primary {
                        weapon: WeaponKind::Longwick,
                        ..
                    }
                ) {
                    continue;
                }
                let aim = look_dir(shot.input.yaw, shot.input.pitch);
                let eye = shot.eye.unwrap_or(Vec3::ZERO);
                let view = stand_in::shot_view(net, &shot.input);
                for l in stand_in::launches(intent, aim, eye, &t.combat, world, 0, crng) {
                    let ghost = stand_ins.spawn(l, l.start, me, frame.own_now, view, &t.combat);
                    arrows.push(Arrow {
                        view: shot.input.view_time_ticks(),
                        start: l.start,
                        velocity: l.velocity,
                        ghost,
                        ..Default::default()
                    });
                }
            }
        }
        let targets = stand_in::displayed_targets(net, s.target == Target::Player);
        stand_ins.update(
            &StandInFrame {
                own: &frame.own,
                now: frame.own_now,
                rendered: &frame.rendered,
                render: frame.render,
                tick_rate: RATE,
                tuning: &t.combat,
                world,
                targets: &targets,
                timeout: stand_in::timeout(net),
            },
            STEP.as_secs_f32(),
        );
        for (from, to) in c.stand_ins.claims().to_vec() {
            if let Some(a) = arrows.iter_mut().find(|a| a.ghost == Some(from)) {
                a.claimed = Some(to);
                a.handover = c.stand_ins.handover(to).map(|(dt, off)| (dt, off.length()));
            }
        }
        let render = frame.render;
        let shown = displayed(&c.net, target_id);
        for a in &mut arrows {
            let drawn = drawn_position(c, a, &frame);
            // No longer drawn (gone into something, or the server removed it), right at the
            // target: drawn as a hit. Drawn flying again later: it wasn't.
            let at_target = a.drawn_last.zip(shown.as_ref()).is_some_and(|(p, (_, h))| {
                let (pads, radius) = arrow_pads(&t, h.target);
                h.distance_to_point_padded(p, pads)
                    .is_some_and(|(d, _)| d - radius < 0.6)
            });
            if a.drawn_live && drawn.is_none() && at_target && a.drawn_touch.is_none() {
                a.hid = true;
            }
            if a.hid && !a.drawn_live && drawn.is_some() {
                a.reappeared = true;
            }
            a.drawn_live = drawn.is_some();
            // View.
            if !a.view_done
                && let Some(p) = view_position(a, render, &t)
            {
                let from = a.view_last.unwrap_or(a.start);
                if let Some((_, h)) = &shown
                    && let Some((_, part)) = contact(&c.net, h, from, p)
                {
                    a.view_hit = Some(part);
                    a.view_done = true;
                } else if walled(&c.net, from, p) || (render - a.view) / RATE as f64 > 3.0 {
                    a.view_done = true;
                }
                a.view_last = Some(p);
            }
            // Drawn.
            if let Some(p) = drawn {
                if let Some(from) = a.drawn_last
                    && from.distance(p) < 8.0
                {
                    match a.drawn_touch {
                        None => {
                            if let Some((_, h)) = &shown
                                && let Some((f, _)) = contact(&c.net, h, from, p)
                            {
                                a.drawn_touch = Some(from.lerp(p, f));
                            }
                        }
                        Some(touch) => {
                            let along = (p - touch).dot(a.velocity.normalize_or_zero());
                            a.drawn_beyond = a.drawn_beyond.max(along);
                        }
                    }
                }
                a.drawn_last = Some(p);
                a.drawn_at = render;
            }
        }
    }

    // Arrows aimed at or flying past a target being put back: the test's doing, not counted.
    for a in &mut arrows {
        a.void |= teleports
            .iter()
            .any(|&tp| (a.view - 2.0..a.view + 60.0).contains(&(tp as f64)));
    }
    let mut tally = Tally::default();
    for a in arrows.iter().filter(|a| !a.void) {
        tally.arrows += 1;
        let view = a.view_hit.is_some();
        let server = a.server_hit.is_some();
        tally.view_hits += view as u32;
        tally.server_hits += server as u32;
        tally.lost += (view && !server) as u32;
        tally.gifted += (!view && server) as u32;
        tally.view_weak += a.view_hit.is_some_and(|p| p != HitPart::Body) as u32;
        if let (Some(v), Some(sv)) = (a.view_hit, a.server_hit)
            && v != sv
        {
            tally.wrong_part += 1;
        }
        let drawn = a.drawn_hit();
        tally.drawn_false_hit += (drawn && !server) as u32;
        tally.drawn_false_miss += (!drawn && server) as u32;
    }
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    tally.rtt_ms = mean(&rtts[rtts.len() / 2..]);
    tally.rewind_ms = mean(&rewinds);
    tally.parallax_m = mean(&parallax);
    (tally, arrows)
}

fn print_row(name: &str, t: &Tally) {
    println!(
        "{name:<34} rtt {:>4.0} ms rewind {:>4.0} ms | {:>3} arrows: view {:>3} hit ({:>2} weak) server {:>3} | view-hit/server-miss {:>3}, view-miss/server-hit {:>3}, part {:>2} | agree {:>5.1} % | drawn hit/server miss {:>3}, drawn miss/server hit {:>3}, agree {:>5.1} % | parallax {:.2} m",
        t.rtt_ms,
        t.rewind_ms,
        t.arrows,
        t.view_hits,
        t.view_weak,
        t.server_hits,
        t.lost,
        t.gifted,
        t.wrong_part,
        t.agreement() * 100.0,
        t.drawn_false_hit,
        t.drawn_false_miss,
        t.drawn_agreement() * 100.0,
        t.parallax_m,
    );
}

/// At 100–150 ms RTT the server agrees with what the shooter's screen showed on (nearly)
/// every arrow — strafing Moulds, a Mould on its own AI, a strafing player, a shooter on the
/// run; weak points included — and the client draws the outcome the server reports.
#[test]
fn arrows_hit_what_the_shooter_saw() {
    let cases = [
        Scenario::new(50.0, Target::Effigy),
        Scenario::new(75.0, Target::Heron),
        Scenario {
            full_only: true,
            ..Scenario::new(50.0, Target::Player)
        },
        Scenario {
            moving: true,
            ..Scenario::new(75.0, Target::Effigy)
        },
    ];
    for s in cases {
        let s = Scenario { arrows: 36, ..s };
        let (t, _) = run(s);
        let name = format!("{:?} {} ms one-way", s.target, s.one_way_ms);
        print_row(&name, &t);
        assert!(
            t.arrows >= 30 && t.view_hits >= 8,
            "{name}: a real test ({t:?})"
        );
        assert!(t.lost + t.gifted <= 1, "{name}: view vs server {t:?}");
        assert!(t.wrong_part <= 1, "{name}: parts {t:?}");
        assert!(
            t.drawn_false_hit + t.drawn_false_miss <= 1,
            "{name}: drawn vs server {t:?}"
        );
        if s.target == Target::Effigy {
            assert!(t.view_weak >= 4, "{name}: weak points shot too ({t:?})");
        }
        if s.moving {
            assert!(
                t.parallax_m < 0.08,
                "{name}: launched off the crosshair ({t:?})"
            );
        }
    }
}

/// The same arrows without projectile lag compensation: the server tests them against where
/// the strafing target is by the time the shot arrives, a round trip and an interpolation
/// delay after the shooter saw it, and loses a large share of the hits the shooter saw.
#[test]
fn without_projectile_lag_compensation_hits_seen_are_lost() {
    let s = Scenario {
        arrows: 36,
        projectile_lag_comp_ms: Some(0.0),
        ..Scenario::new(50.0, Target::Effigy)
    };
    let (t, _) = run(s);
    print_row("Effigy 50 ms one-way, no compensation", &t);
    assert!(t.view_hits >= 15, "{t:?}");
    assert!(t.lost * 4 >= t.view_hits, "a quarter or more lost: {t:?}");
}

/// A shot fired on a frame between ticks is aimed at what that frame showed: the next input
/// carries that frame's render tick as its view time (`NetClient::latch_view`), not the one
/// the screen has moved on to by the tick.
#[test]
fn the_press_frames_view_time_goes_with_the_shot() {
    let server_cfg = config();
    let mut server = Server::new(server_cfg).expect("server starts");
    let netsim = Some(NetSimConfig::new(40.0, 0.0, 0.0).with_seed(3));
    let mut c = Client::new(&server, "Shooter", netsim);
    let step = |server: &mut Server, c: &mut Client| {
        server.update(STEP);
        c.net.update(STEP);
        c.net.drain_events();
        for _ in 0..c.ts.advance(STEP.as_secs_f64()) {
            if c.net.is_in_game() {
                c.net.tick(PlayerInput::default());
            }
        }
    };
    for _ in 0..1500 {
        step(&mut server, &mut c);
    }
    assert!(c.net.is_in_game());
    let last_view = |c: &Client| {
        let p = c.net.prediction().expect("predicting");
        p.unacked().last().expect("an input").view_time_ticks()
    };
    // The press frame, then 3 frames (6 ms) of display before the tick.
    c.net.latch_view();
    let pressed = c.net.render_tick();
    for _ in 0..3 {
        server.update(STEP);
        c.net.update(STEP);
        c.net.latch_view();
    }
    assert!(c.net.render_tick() - pressed > 0.3, "the display moved on");
    c.net.tick(PlayerInput::default());
    assert!((last_view(&c) - pressed).abs() < 1.0 / 256.0 + 1e-9);
    // The next one is back on the display's clock.
    for _ in 0..3 {
        server.update(STEP);
        c.net.update(STEP);
    }
    c.net.tick(PlayerInput::default());
    assert!((last_view(&c) - c.net.render_tick()).abs() < 1.0 / 256.0 + 1e-9);
}

#[test]
#[ignore = "measurement: run with --ignored --nocapture"]
fn measure_bow_hit_registration() {
    for one_way in [30.0, 50.0, 75.0] {
        for target in [
            Target::Effigy,
            Target::Heron,
            Target::Gnawer,
            Target::Player,
        ] {
            let s = Scenario::new(one_way, target);
            let (t, _) = run(s);
            print_row(&format!("{:?} {}ms one-way", target, one_way), &t);
        }
        let s = Scenario {
            moving: true,
            ..Scenario::new(one_way, Target::Effigy)
        };
        let (t, _) = run(s);
        print_row(&format!("Effigy, shooter moving {one_way}ms"), &t);
    }
}

#[test]
#[ignore = "debug"]
fn debug_one() {
    let target = match std::env::var("HITREG_TARGET").as_deref() {
        Ok("heron") => Target::Heron,
        Ok("gnawer") => Target::Gnawer,
        Ok("player") => Target::Player,
        _ => Target::Effigy,
    };
    let one_way = std::env::var("HITREG_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50.0);
    let s = Scenario {
        moving: std::env::var("HITREG_MOVING").is_ok(),
        ..Scenario::new(one_way, target)
    };
    let (t, arrows) = run(s);
    print_row("Effigy 50", &t);
    for (i, a) in arrows.iter().enumerate() {
        if a.drawn_hit() == a.server_hit.is_some() && a.view_hit == a.server_hit {
            continue;
        }
        println!(
            "{i:>3} view {:?} server {:?} at {} drawn hit {} (hid {} again {}, touch {:?}, last {:?} at {:.2}, beyond {:.2}) handover {:?} view {:.2} spawn {:?} v {:.1}",
            a.view_hit,
            a.server_hit,
            a.server_tick,
            a.drawn_hit(),
            a.hid,
            a.reappeared,
            a.drawn_touch,
            a.drawn_last,
            a.drawn_at,
            a.drawn_beyond,
            a.handover,
            a.view,
            a.spawn_tick,
            a.velocity.length()
        );
    }
}
