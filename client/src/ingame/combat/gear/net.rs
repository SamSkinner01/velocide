//! The gear as the server replicates it (docs/arsenal.md, "The rework on the wire"): the
//! `BOLAS`, `KITE` and `LANTERN_TOSS` projectiles, an `ORBITING` `WISP`, `LANTERN`s on the
//! floor, players `GUARDING` with a Bulwark, `BOUND` bodies (players and Moulds) and the cords
//! between them, `STOKED` / `OVERHEATED` weapons; `Fx::GuardBlock` (a Bulwark's), `BolasBound`
//! and `LanternShot` ([`GearView::note_fx`]). Your own slab and stoke come from your predicted
//! weapons (`WeaponState::charge` while the Bulwark is held, `stoke` / `overheat`), the slab's
//! strength from your entity's `aux`.

use std::collections::HashMap;

use engine::audio::{PlayParams, Sfx};
use engine::gfx::{DrawItem, PointLight};
use glam::{Mat4, Vec3};
use shared::loadout::{Ability, LOADOUT_SLOTS, WeaponKind};
use shared::protocol::{
    EntityState, Fx, FxEvent, NetId, enemy_flags, kind, player_flags, player_weapon,
    projectile_flags,
};
use shared::sim::WeaponState;
use shared::sim::combat::secs_to_ticks;
use shared::sim::projectiles::replicated_position;

use super::super::super::remote_players::RemotePlayers;
use super::super::entities::{EntityFrame, enemy_size, replicated_velocity};
use super::{GearFrame, GearView};
use super::{bolas, bulwark, kite, lantern, stoke, wisp};
use crate::models::ModelLibrary;
use crate::models::shrike::REMOTE_SCALE;

/// A replicated lantern seen last frame.
#[derive(Clone, Copy, Debug)]
struct SeenLamp {
    position: Vec3,
    /// The tick it goes out (`aux`).
    out: u32,
    /// Its aim's pitch (from its last shot) and how hard that shot kicked.
    pitch: f32,
    kick: f32,
    still: bool,
}

/// A slab up last frame.
#[derive(Clone, Copy, Debug)]
struct SeenSlab {
    root: Mat4,
    /// What it had absorbed (0..1), for the crack stages a block crosses.
    absorbed: f32,
    flash: f32,
    still: bool,
}

/// A bolas cord between two bound bodies (`Fx::BolasBound` with two positions).
#[derive(Clone, Copy, Debug)]
struct NetLink {
    a: NetId,
    b: NetId,
    age: f32,
    seconds: f32,
    pulse: f32,
    /// Their health last frame: a drop on either is damage carried along the cord.
    health: (u16, u16),
}

/// The replicated gear's bookkeeping, and what it draws this frame.
#[derive(Default)]
pub(super) struct NetState {
    /// Fx of this frame waiting to be played with the gear.
    noted: Vec<Fx>,
    trails: HashMap<NetId, (kite::Trail, bool)>,
    lamps: HashMap<NetId, SeenLamp>,
    slabs: HashMap<NetId, SeenSlab>,
    links: Vec<NetLink>,
    /// Your weapons' stoke per slot last frame (stoked, overheated), for the sounds.
    stoked: [(bool, bool); LOADOUT_SLOTS],
    /// Seconds to each orbiting wisp's next hum.
    hums: HashMap<NetId, f32>,
    /// Your slab shattered: not drawn again until you let go (your predicted hold lasts until
    /// the server's correction).
    own_broke: bool,
    // This frame's draws.
    bolas: Vec<(Vec3, Vec3, f32)>,
    kites: Vec<(Vec3, Vec3, f32)>,
    tosses: Vec<(Vec3, f32, WeaponKind)>,
    lanterns: Vec<(Vec3, f32, f32, f32, WeaponKind, f32)>,
    slabs_drawn: Vec<(Mat4, f32, f32, f32, bool)>,
    orbits: Vec<Vec3>,
    weights: Vec<(Vec3, f32, f32)>,
    lights: Vec<PointLight>,
}

/// What [`GearView::update_net`] needs of the frame.
pub struct NetGear<'a> {
    pub frame: &'a EntityFrame<'a>,
    /// You: your id, your entity in the latest snapshot (flags, the slab's strength), your
    /// predicted weapons, whether you are dead.
    pub local: Option<NetId>,
    pub own: Option<EntityState>,
    pub weapons: &'a WeaponState,
    pub dead: bool,
    /// Others' drawn muzzles (their stoked weapons' embers).
    pub remote: &'a RemotePlayers,
}

/// A body for the cords: feet, height, radius.
fn body(e: &EntityState) -> Option<(Vec3, f32, f32)> {
    if e.kind == kind::PLAYER {
        Some((e.position, 1.9 * REMOTE_SCALE, 0.45))
    } else if kind::is_enemy(e.kind) {
        let (r, h) = enemy_size(e.kind);
        Some((e.position, h, r))
    } else {
        None
    }
}

/// Bound now (its `BOUND` flag)?
fn bound(e: &EntityState) -> bool {
    if e.kind == kind::PLAYER {
        e.flags & player_flags::BOUND != 0
    } else {
        kind::is_enemy(e.kind) && e.flags & enemy_flags::BOUND != 0
    }
}

/// A player's eye (`LOW_PROFILE`: sliding, lower), drawn [`REMOTE_SCALE`] larger.
fn eye_of(e: &EntityState) -> Vec3 {
    let h = if e.flags & player_flags::LOW_PROFILE != 0 {
        0.85
    } else {
        1.6
    };
    e.position + Vec3::Y * h * REMOTE_SCALE
}

impl GearView {
    /// Keeps the gear's Fx of this frame (a Bulwark's block, a bolas bind, a lantern's shot)
    /// to play with the gear after the update (the frame's due Fx, played through `fx::play`
    /// first).
    pub fn note_fx(&mut self, due: &[FxEvent]) {
        for ev in due {
            match &ev.fx {
                Fx::GuardBlock {
                    ability: Ability::Bulwark,
                    ..
                }
                | Fx::BolasBound { .. }
                | Fx::LanternShot { .. } => self.net.noted.push(ev.fx.clone()),
                _ => {}
            }
        }
    }

    /// Your slab's root this frame (the tuning's slab in front of your eye).
    fn own_slab_root(n: &NetGear, f: &GearFrame) -> Mat4 {
        bulwark::slab_transform_for(f.eye, f.yaw, f.pitch, &n.frame.tuning.bulwark)
    }

    /// Where `player`'s slab stands this frame.
    fn slab_root_of(&self, n: &NetGear, f: &GearFrame, player: NetId) -> Option<Mat4> {
        if Some(player) == n.local {
            return Some(Self::own_slab_root(n, f));
        }
        let e = n.frame.entities.iter().find(|e| e.id == player)?;
        Some(bulwark::slab_transform_for(
            eye_of(e),
            e.yaw,
            e.pitch,
            &n.frame.tuning.bulwark,
        ))
    }

    /// The replicated gear this frame: its effects and sounds, and what [`GearView::draw_net`]
    /// draws.
    pub fn update_net(&mut self, n: &NetGear, f: &mut GearFrame) {
        let t = n.frame.tuning;
        let rate = n.frame.tick_rate.max(1);
        let dt = f.dt.max(0.0);
        let time = self.time;
        {
            let s = &mut self.net;
            s.bolas.clear();
            s.kites.clear();
            s.tosses.clear();
            s.lanterns.clear();
            s.slabs_drawn.clear();
            s.orbits.clear();
            s.weights.clear();
            s.lights.clear();
        }
        let own_feet = f.feet;
        let find = |id: NetId| -> Option<EntityState> {
            if Some(id) == n.local {
                return n.own.map(|mut e| {
                    if let Some(feet) = own_feet {
                        e.position = feet;
                    }
                    e
                });
            }
            n.frame.entities.iter().find(|e| e.id == id).copied()
        };

        // The noted Fx.
        for fx in std::mem::take(&mut self.net.noted) {
            match fx {
                Fx::GuardBlock {
                    player,
                    position,
                    broke,
                    ..
                } => {
                    let id = NetId::player(player);
                    let Some(root) = self.slab_root_of(n, f, id) else {
                        continue;
                    };
                    let at = bulwark::block_point(root, position.to_vec3());
                    let normal = root
                        .transform_vector3(Vec3::NEG_Z)
                        .normalize_or(Vec3::NEG_Z);
                    let strength = find(id).map_or(1.0, |e| ((e.aux >> 12) & 0xF) as f32 / 15.0);
                    let absorbed = 1.0 - strength;
                    let seen = self.net.slabs.get(&id).map_or(0.0, |s| s.absorbed);
                    if broke {
                        bulwark::shatter(f.effects, f.beams, f.audio, f.rng, root);
                        self.net.slabs.remove(&id);
                        if Some(id) == n.local {
                            self.net.own_broke = true;
                        }
                    } else {
                        let cracked = bulwark::crack_stage(absorbed) > bulwark::crack_stage(seen);
                        bulwark::absorb(f.effects, f.audio, f.rng, at, normal, cracked);
                        if let Some(s) = self.net.slabs.get_mut(&id) {
                            s.flash = 1.0;
                            s.absorbed = absorbed;
                        }
                    }
                }
                Fx::BolasBound { positions } => {
                    let near = |p: Vec3| {
                        n.frame
                            .entities
                            .iter()
                            .filter(|e| body(e).is_some())
                            .filter(|e| e.position.distance(p) < 3.0)
                            .min_by(|a, b| {
                                a.position.distance(p).total_cmp(&b.position.distance(p))
                            })
                            .copied()
                    };
                    let bodies: Vec<EntityState> =
                        positions.iter().filter_map(|p| near(p.to_vec3())).collect();
                    for e in &bodies {
                        if let Some((feet, h, _)) = body(e) {
                            bolas::bind(f.effects, f.beams, f.audio, feet, h);
                        }
                    }
                    if let [a, b] = bodies[..] {
                        let chest = |e: &EntityState| {
                            body(e).map_or(e.position, |(feet, h, _)| feet + Vec3::Y * h * 0.5)
                        };
                        bolas::link_snap(f.effects, f.beams, f.audio, chest(&a), chest(&b));
                        self.net.links.push(NetLink {
                            a: a.id,
                            b: b.id,
                            age: 0.0,
                            seconds: t.bolas.link_time.max(0.1),
                            pulse: 0.0,
                            health: (a.health, b.health),
                        });
                    }
                }
                Fx::LanternShot {
                    lantern: id,
                    origin,
                    ends,
                } => {
                    let carrier = n
                        .frame
                        .entities
                        .iter()
                        .find(|e| e.id == id)
                        .and_then(|e| WeaponKind::from_id(e.state as u8))
                        .unwrap_or(WeaponKind::Arclash);
                    let from = origin.to_vec3();
                    let cone = (t.lantern.kiln_range, t.kiln.cone_degrees);
                    for end in &ends {
                        lantern::shot(
                            f.effects,
                            f.beams,
                            f.audio,
                            f.rng,
                            carrier,
                            from,
                            end.to_vec3(),
                            cone,
                        );
                    }
                    if let (Some(l), Some(end)) = (self.net.lamps.get_mut(&id), ends.first()) {
                        l.kick = 1.0;
                        l.pitch = lantern::aim(l.position, end.to_vec3()).1;
                    }
                }
                _ => {}
            }
        }

        // Projectiles: bolas, kites, tossed lanterns, orbiting wisps.
        for v in self.net.trails.values_mut() {
            v.1 = false;
        }
        let orbit = wisp::Orbit::of(t);
        let mut hums_seen = Vec::new();
        for (e, at, offset) in n.frame.projectiles() {
            let pos = replicated_position(e, at, rate, t) + offset;
            let vel = replicated_velocity(e, at, rate, t);
            let age = ((at - e.aux as f64) / rate as f64) as f32;
            let own = Some(e.owner) == n.frame.local_owner;
            match e.kind {
                kind::BOLAS => {
                    bolas::flight_trail(f.effects, f.rng, pos, age, dt);
                    self.net.bolas.push((pos, vel, age));
                }
                kind::KITE => {
                    let (trail, seen) = self.net.trails.entry(e.id).or_default();
                    *seen = true;
                    trail.follow(pos, vel, dt);
                    let guided = e.flags & projectile_flags::GUIDED != 0;
                    let heart = kite::heart(pos, vel, trail.bank);
                    kite::trail_effects(f.effects, f.beams, f.rng, trail, heart, guided, dt);
                    if trail.flutter_due(dt) {
                        f.audio.play(Sfx::KiteFlutter, PlayParams::at(pos));
                    }
                    if guided && own {
                        let look = shared::input::look_dir(f.yaw, f.pitch);
                        kite::steer_marker(f.beams, f.world, f.eye, look, time);
                    }
                    self.net.kites.push((pos, vel, trail.bank));
                }
                kind::LANTERN_TOSS => {
                    let carrier = WeaponKind::from_id(e.state as u8).unwrap_or(WeaponKind::Arclash);
                    self.net.tosses.push((pos, age * 9.0, carrier));
                }
                kind::WISP if e.flags & projectile_flags::ORBITING != 0 => {
                    // On its circle round the owner: the angle the server has it at, round
                    // where the owner is drawn (you: your predicted feet).
                    let owner = NetId::player(shared::sim::PlayerId(e.owner as u8));
                    let (replicated, drawn) = if own {
                        (n.own.map(|o| o.position), f.feet)
                    } else {
                        let p = find(owner).map(|o| o.position);
                        (p, p)
                    };
                    let (Some(replicated), Some(feet)) = (replicated, drawn) else {
                        continue;
                    };
                    let d = e.position - replicated;
                    let angle = d.z.atan2(d.x);
                    wisp::orbit_effects(f.effects, f.beams, f.rng, feet, angle, orbit, own, dt);
                    let at = wisp::orbit_position(feet, angle, orbit);
                    self.net.orbits.push(at);
                    let hum = self.net.hums.entry(e.id).or_insert(0.0);
                    *hum -= dt;
                    if *hum <= 0.0 {
                        *hum = 0.45;
                        f.audio.play(Sfx::WispHum, PlayParams::at(at));
                    }
                    hums_seen.push(e.id);
                }
                _ => {}
            }
        }
        self.net.trails.retain(|_, v| v.1);
        self.net.hums.retain(|id, _| hums_seen.contains(id));

        // Lanterns on the floor.
        for l in self.net.lamps.values_mut() {
            l.still = false;
        }
        let full = (t.lantern.duration * rate as f32).max(1.0);
        for e in n.frame.entities.iter().filter(|e| e.kind == kind::LANTERN) {
            let carrier = WeaponKind::from_id(e.state as u8).unwrap_or(WeaponKind::Arclash);
            let left = ((e.aux as f64 - n.frame.render_tick) as f32 / full).clamp(0.0, 1.0);
            let lamp = self.net.lamps.entry(e.id).or_insert_with(|| {
                lantern::set_down(f.effects, f.audio, e.position);
                SeenLamp {
                    position: e.position,
                    out: e.aux,
                    pitch: 0.0,
                    kick: 0.0,
                    still: true,
                }
            });
            lamp.still = true;
            lamp.position = e.position;
            lamp.out = e.aux;
            lamp.kick = (lamp.kick - dt * 6.0).max(0.0);
            self.net
                .lanterns
                .push((e.position, e.yaw, lamp.pitch, lamp.kick, carrier, left));
            self.net.lights.push(lantern::light(e.position, left, time));
        }
        let render_tick = n.frame.render_tick;
        let gone: Vec<(NetId, SeenLamp)> = self
            .net
            .lamps
            .iter()
            .filter(|(_, l)| !l.still)
            .map(|(id, l)| (*id, *l))
            .collect();
        for (id, l) in gone {
            // Gone well before its time: a Mould broke it.
            let destroyed = render_tick + (rate as f64) * 0.25 < l.out as f64;
            lantern::burn_out(f.effects, f.audio, f.rng, l.position, destroyed);
            self.net.lamps.remove(&id);
        }

        // Slabs (others' and yours), bound bodies, stoked weapons.
        for s in self.net.slabs.values_mut() {
            s.still = false;
            s.flash = (s.flash - dt * 4.0).max(0.0);
        }
        let bulwark_t = &t.bulwark;
        let mut slabs_now: Vec<(NetId, Mat4, f32, f32, bool)> = Vec::new();
        for e in n.frame.entities {
            if e.kind == kind::PLAYER && Some(e.id) != n.local {
                let (_, ability, level) = player_weapon(e.state);
                if e.flags & player_flags::GUARDING != 0 && ability == Ability::Bulwark {
                    let root = bulwark::slab_transform_for(eye_of(e), e.yaw, e.pitch, bulwark_t);
                    let strength = ((e.aux >> 12) & 0xF) as f32 / 15.0;
                    slabs_now.push((
                        e.id,
                        root,
                        1.0 - strength,
                        1.0 - level as f32 / 255.0,
                        false,
                    ));
                }
                let stoked = e.flags & player_flags::STOKED != 0;
                let vented = e.flags & player_flags::OVERHEATED != 0;
                if (stoked || vented)
                    && let Some(v) = n.remote.visual(e.id)
                    && v.muzzle != Vec3::ZERO
                {
                    let dir = v.muzzle_dir.normalize_or(Vec3::NEG_Z);
                    if stoked {
                        stoke::stoked_effects(f.effects, f.rng, v.muzzle, dir, 0.6, dt);
                    } else {
                        let body = v.muzzle - dir * 0.4;
                        stoke::vent_effects(f.effects, f.rng, v.muzzle, body, dir, 0.6, false, dt);
                    }
                }
            }
            if bound(e)
                && let Some(b) = body(e)
            {
                self.net.weights.push(b);
            }
        }
        // Yours: up while the Bulwark is held (predicted), its strength the server's.
        let w = n.weapons;
        let holding = !n.dead && w.charge > 0 && w.ability() == Ability::Bulwark;
        if !holding {
            self.net.own_broke = false;
        }
        if holding && !self.net.own_broke {
            let strength = n
                .own
                .filter(|e| e.flags & player_flags::GUARDING != 0)
                .map_or(1.0, |e| ((e.aux >> 12) & 0xF) as f32 / 15.0);
            let left = 1.0 - w.charge_fraction(t, rate);
            let root = Self::own_slab_root(n, f);
            if let Some(id) = n.local {
                slabs_now.push((id, root, 1.0 - strength, left, true));
            }
            self.own_slab = Some((strength, left));
        }
        if let (Some(own), Some(feet)) = (n.own, f.feet)
            && own.flags & player_flags::BOUND != 0
        {
            self.net.weights.push((feet, 1.9, 0.4));
        }
        for (id, root, absorbed, left, own) in slabs_now {
            let s = self.net.slabs.entry(id).or_insert_with(|| {
                bulwark::raise(f.effects, f.audio, root, own);
                SeenSlab {
                    root,
                    absorbed,
                    flash: 0.0,
                    still: true,
                }
            });
            s.still = true;
            s.root = root;
            s.absorbed = s.absorbed.max(absorbed);
            self.net
                .slabs_drawn
                .push((root, s.absorbed, s.flash, left, own));
        }
        let lowered: Vec<Mat4> = self
            .net
            .slabs
            .values()
            .filter(|s| !s.still)
            .map(|s| s.root)
            .collect();
        for root in lowered {
            bulwark::lower(f.effects, f.audio, root);
        }
        self.net.slabs.retain(|_, s| s.still);
        for &(feet, h, r) in &self.net.weights {
            bolas::wrap(f.beams, feet, h, r, 1.0, time);
        }

        // Cords between linked bodies (while both stay bound): a pulse along them when damage
        // is carried across.
        let mut links = std::mem::take(&mut self.net.links);
        links.retain_mut(|l| {
            l.age += dt;
            l.pulse = (l.pulse - dt * 3.0).max(0.0);
            let (Some(a), Some(b)) = (find(l.a), find(l.b)) else {
                return false;
            };
            // (The flags may lag the bind's Fx by a snapshot.)
            if l.age >= l.seconds || (l.age > 0.3 && !(bound(&a) && bound(&b))) {
                return false;
            }
            let chest = |e: &EntityState| {
                body(e).map_or(e.position + Vec3::Y, |(feet, h, _)| {
                    feet + Vec3::Y * h * 0.5
                })
            };
            if a.health < l.health.0 || b.health < l.health.1 {
                l.pulse = 1.0;
                let to = if a.health < l.health.0 {
                    chest(&b)
                } else {
                    chest(&a)
                };
                bolas::share_pulse(f.effects, f.audio, to);
            }
            l.health = (a.health, b.health);
            bolas::link_cord(
                f.beams,
                chest(&a),
                chest(&b),
                1.0 - l.age / l.seconds,
                l.pulse,
                time,
            );
            true
        });
        self.net.links = links;

        // Your stoke (predicted per slot): the heat on the weapon in hand, the sounds as any
        // slot catches or is spent.
        if self.stoke.is_none() {
            let slot = (w.slot as usize).min(LOADOUT_SLOTS - 1);
            let stoke_t = secs_to_ticks(t.stoke.duration, rate, 1).max(1) as f32;
            let vent_t = secs_to_ticks(t.stoke.overheat, rate, 1).max(1) as f32;
            if w.stoke[slot] > 0 {
                self.heat.stoked = Some(1.0 - w.stoke[slot] as f32 / stoke_t);
            } else if w.overheat[slot] > 0 {
                self.heat.venting = Some(w.overheat[slot] as f32 / vent_t);
            }
        }
        for i in 0..LOADOUT_SLOTS {
            let now = (w.stoke[i] > 0, w.overheat[i] > 0);
            let was = self.net.stoked[i];
            if now.0 && !was.0 {
                f.audio.play_2d(Sfx::StokeIgnite);
            }
            if now.1 && !was.1 {
                f.audio.play_2d(Sfx::StokeVent);
            }
            self.net.stoked[i] = now;
        }
    }

    /// Draws the replicated gear worked out by [`GearView::update_net`].
    pub fn draw_net(
        &self,
        lib: Option<&ModelLibrary>,
        draws: &mut Vec<DrawItem>,
        lights: &mut Vec<PointLight>,
    ) {
        for l in &self.net.lights {
            if lights.len() < 48 {
                lights.push(*l);
            }
        }
        let Some(lib) = lib else {
            return;
        };
        let time = self.time;
        for &(pos, vel, age) in &self.net.bolas {
            bolas::draw_flight(lib, pos, vel, age, draws);
        }
        for &(pos, vel, bank) in &self.net.kites {
            let pulse = 0.5 + 0.5 * ((time * 9.0).sin() as f32);
            kite::draw(lib, pos, vel, bank, pulse, draws);
        }
        for &(pos, spin, carrier) in &self.net.tosses {
            lantern::draw(lib, pos, spin, 0.0, 0.0, 0.0, carrier, 1.0, time, draws);
        }
        for &(feet, yaw, pitch, kick, carrier, left) in &self.net.lanterns {
            lantern::draw(lib, feet, 0.0, yaw, pitch, kick, carrier, left, time, draws);
        }
        for &(root, absorbed, flash, left, own) in &self.net.slabs_drawn {
            bulwark::draw_slab(lib, root, absorbed, flash, left, own, time, draws);
        }
        for &at in &self.net.orbits {
            wisp::draw_orbiting(lib, at, time, draws);
        }
        for &(feet, h, r) in &self.net.weights {
            bolas::draw_weights(lib, feet, h, r, 1.0, time, draws);
        }
    }
}

/// Drawn by the gear (the entity drawer skips it): an orbiting wisp.
pub fn draws_entity(e: &EntityState) -> bool {
    e.kind == kind::WISP && e.flags & projectile_flags::ORBITING != 0
}
