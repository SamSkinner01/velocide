//! The rework's Scatter, Rapid and Explosive abilities and the Thorn relays on screen
//! (docs/rework.md, area **client-b**): the bookkeeping that ties [`bolas`](super::bolas),
//! [`bulwark`](super::bulwark), [`wisp`](super::wisp), [`stoke`](super::stoke),
//! [`lantern`](super::lantern), [`kite`](super::kite) and [`relay`](super::relay) to the
//! frame — what lives across frames, the HUD (your slab's bar, the heat gauge), the heat on
//! your viewmodel.
//!
//! [`GearPreview`] / `InGame::preview_gear` stage any of it client-side (tools: `combat_shots`'
//! `gear_*.png`, `relay_*.png`): a bolas thrown, binding and linking; a slab raised, taking
//! blows, cracking and shattering (yours or another Shrike's); the wisp orbiting and flung;
//! your weapon stoked and venting; a lantern set down and firing; a kite steered and bursting;
//! a relay off a thorn of each kind.

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::{DrawItem, PointLight, UiDrawList};
use engine::physics::{CollisionWorld, MASK_SHOTS};
use engine::ui::{Rect, Theme};
use glam::{Mat4, Vec3, Vec4};
use shared::loadout::{Ability, WeaponKind};
use shared::protocol::NetId;
use shared::sim::{AbilityPhase, Attack, FireIntent};

use super::super::InGame;
use super::super::effects::Effects;
use super::Rng;
use super::fx::{Beams, FxCtx};
use super::stoke::Heat;
use super::{bolas, bulwark, kite, lantern, relay, stoke, wisp};

/// The replicated gear (`gear/net.rs`).
mod net;
use crate::app::Common;
use crate::models::{ModelLibrary, PlayerAnim, Tint};
use crate::net::Session;
pub use net::{NetGear, draws_entity};

/// Something staged client-side by `InGame::preview_gear` (tools: look at the visuals without
/// the simulation making them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GearPreview {
    /// A bolas thrown from `from` that binds a body standing at `to` (`height` tall) for
    /// `seconds`; `link`: the cord snaps on to a second one standing there.
    Bolas {
        from: Vec3,
        to: Vec3,
        height: f32,
        link: Option<Vec3>,
        seconds: f32,
    },
    /// A slab raised for `seconds` taking `hits` blows over it (it shatters on the last if
    /// `shatter`); `holder`: `None` = yours (in front of the camera), else a Shrike standing
    /// at those feet, facing that yaw.
    Bulwark {
        holder: Option<(Vec3, f32)>,
        seconds: f32,
        hits: u32,
        shatter: bool,
    },
    /// The wisp orbiting `around` (feet; `None` = you) for `seconds`, then flung along
    /// `fling` if given.
    Wisp {
        around: Option<Vec3>,
        seconds: f32,
        fling: Option<Vec3>,
    },
    /// Your weapon stoked for `stoke` seconds, then spent and venting for `vent`.
    Stoke { stoke: f32, vent: f32 },
    /// A lantern tossed from `from` that settles at `to` (on the floor) and fires `carrier`'s
    /// primary at `target` for `seconds`.
    Lantern {
        from: Vec3,
        to: Vec3,
        carrier: WeaponKind,
        target: Vec3,
        seconds: f32,
    },
    /// A kite launched from `from` along `velocity`, steered towards `steer` for `steer_for`
    /// seconds, bursting where it meets the level or after `seconds`.
    Kite {
        from: Vec3,
        velocity: Vec3,
        steer: Vec3,
        steer_for: f32,
        seconds: f32,
    },
    /// `attack` fired from `from` reaching a thorn at `thorn`, relayed at `to` (`turned`: a
    /// projectile turning there).
    Relay {
        attack: Attack,
        turned: bool,
        from: Vec3,
        thorn: Vec3,
        to: Vec3,
    },
}

#[derive(Clone, Copy, Debug)]
struct Flight {
    position: Vec3,
    velocity: Vec3,
    to: Vec3,
    height: f32,
    link: Option<Vec3>,
    seconds: f32,
    age: f32,
}

#[derive(Clone, Copy, Debug)]
struct Bound {
    feet: Vec3,
    height: f32,
    age: f32,
    seconds: f32,
}

#[derive(Clone, Copy, Debug)]
struct Link {
    a: Vec3,
    b: Vec3,
    age: f32,
    seconds: f32,
    /// Seconds to the next share pulse, and the pulse running (1 → 0).
    next: f32,
    pulse: f32,
}

#[derive(Clone, Copy, Debug)]
struct Slab {
    holder: Option<(Vec3, f32)>,
    age: f32,
    seconds: f32,
    hits: u32,
    taken: u32,
    shatter: bool,
    flash: f32,
    cycle: f32,
}

impl Slab {
    fn absorbed(&self) -> f32 {
        self.taken as f32 / self.hits.max(1) as f32
    }
}

#[derive(Clone, Copy, Debug)]
struct Orbit {
    around: Option<Vec3>,
    age: f32,
    seconds: f32,
    fling: Option<Vec3>,
}

#[derive(Clone, Copy, Debug)]
struct Lamp {
    position: Vec3,
    velocity: Vec3,
    to: Vec3,
    carrier: WeaponKind,
    target: Vec3,
    seconds: f32,
    /// Seconds since it landed (`None`: in flight).
    lit: Option<f32>,
    age: f32,
    next_shot: f32,
    kick: f32,
}

#[derive(Clone, Debug)]
struct Flyer {
    position: Vec3,
    velocity: Vec3,
    steer: Vec3,
    steer_for: f32,
    seconds: f32,
    age: f32,
    trail: kite::Trail,
}

#[derive(Clone, Copy, Debug)]
struct Incoming {
    attack: Attack,
    turned: bool,
    from: Vec3,
    thorn: Vec3,
    to: Vec3,
    age: f32,
}

/// What waits for an `FxCtx` (played after the update).
#[derive(Clone, Copy, Debug)]
enum Pending {
    Relay(Attack, bool, Vec3, Vec3),
    KiteBurst(Vec3),
    WispPop(Vec3),
}

/// The gear's visuals that live across frames.
#[derive(Default)]
pub struct GearView {
    staged: Vec<GearPreview>,
    flights: Vec<Flight>,
    bound: Vec<Bound>,
    links: Vec<Link>,
    slabs: Vec<Slab>,
    orbits: Vec<Orbit>,
    flung: Vec<(Vec3, Vec3, f32)>,
    lamps: Vec<Lamp>,
    kites: Vec<Flyer>,
    incoming: Vec<Incoming>,
    pending: Vec<Pending>,
    /// Your weapon's stoke: seconds gone, stoke and vent lengths (staged).
    stoke: Option<(f32, f32, f32)>,
    /// Your weapon's heat this frame (the viewmodel tint, the gauge).
    heat: Heat,
    /// Your slab this frame: what it can still take, its time left (the HUD bar).
    own_slab: Option<(f32, f32)>,
    /// The replicated gear's bookkeeping (`net.rs`).
    net: net::NetState,
    time: f64,
}

/// What [`GearView::update`] needs of the frame.
pub struct GearFrame<'a> {
    pub effects: &'a mut Effects,
    pub beams: &'a mut Beams,
    pub audio: &'a mut Audio,
    pub rng: &'a mut Rng,
    pub world: Option<&'a CollisionWorld>,
    /// The camera (your eye) and look.
    pub eye: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// Your feet.
    pub feet: Option<Vec3>,
    /// Your weapon's muzzle and its direction (world), from the last viewmodel draw.
    pub muzzle: Option<(Vec3, Vec3)>,
    pub dt: f32,
}

fn cast(world: Option<&CollisionWorld>, from: Vec3, step: Vec3) -> Option<Vec3> {
    let len = step.length();
    if len < 1e-5 {
        return None;
    }
    world
        .and_then(|w| w.cast_ray(from, step / len, len, MASK_SHOTS))
        .map(|h| h.point)
}

impl GearView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        *self = Self {
            time: self.time,
            ..Self::default()
        };
    }

    /// Your weapon's heat this frame.
    pub fn heat(&self) -> Heat {
        self.heat
    }

    /// A predicted ability intent of yours that isn't a shot (`plays_as_shot`): the Wisp flung
    /// off its orbit. (The Bulwark's slab and the Stoke follow your predicted weapons in
    /// `update_net`.)
    pub fn local_intent(&mut self, intent: &FireIntent, audio: &mut Audio) {
        if let FireIntent::Ability {
            ability: Ability::Wisp,
            phase: AbilityPhase::Recast,
            ..
        } = intent
        {
            audio.play_2d(Sfx::WispFling);
        }
    }

    fn preview(&mut self, p: GearPreview, f: &mut GearFrame) {
        match p {
            GearPreview::Bolas {
                from,
                to,
                height,
                link,
                seconds,
            } => {
                let aim = to + Vec3::Y * height * 0.3;
                let flight = (aim - from).length() / 30.0;
                // A shallow lob: up enough to land on it under the bolas' gravity (8).
                let velocity = (aim - from) / flight.max(0.05) + Vec3::Y * 4.0 * flight;
                self.flights.push(Flight {
                    position: from,
                    velocity,
                    to,
                    height,
                    link,
                    seconds,
                    age: 0.0,
                });
                f.audio.play(Sfx::BolasThrow, PlayParams::at(from));
            }
            GearPreview::Bulwark {
                holder,
                seconds,
                hits,
                shatter,
            } => {
                let root = match holder {
                    Some((feet, yaw)) => bulwark::slab_transform(feet + Vec3::Y * 1.7, yaw, 0.0),
                    None => bulwark::slab_transform(f.eye, f.yaw, f.pitch),
                };
                bulwark::raise(f.effects, f.audio, root, holder.is_none());
                self.slabs.push(Slab {
                    holder,
                    age: 0.0,
                    seconds,
                    hits,
                    taken: 0,
                    shatter,
                    flash: 0.0,
                    cycle: 0.0,
                });
            }
            GearPreview::Wisp {
                around,
                seconds,
                fling,
            } => {
                f.audio
                    .play(Sfx::WispLaunch, PlayParams::at(around.unwrap_or(f.eye)));
                self.orbits.push(Orbit {
                    around,
                    age: 0.0,
                    seconds,
                    fling,
                });
            }
            GearPreview::Stoke { stoke, vent } => {
                self.stoke = Some((0.0, stoke, vent));
                f.audio.play_2d(Sfx::StokeIgnite);
            }
            GearPreview::Lantern {
                from,
                to,
                carrier,
                target,
                seconds,
            } => {
                let t = ((to - from).length() / 10.0).max(0.2);
                let velocity = (to - from) / t + Vec3::Y * 0.5 * 16.0 * t;
                self.lamps.push(Lamp {
                    position: from,
                    velocity,
                    to,
                    carrier,
                    target,
                    seconds,
                    lit: None,
                    age: 0.0,
                    next_shot: 0.3,
                    kick: 0.0,
                });
                f.audio.play(Sfx::KnackThrow, PlayParams::at(from));
            }
            GearPreview::Kite {
                from,
                velocity,
                steer,
                steer_for,
                seconds,
            } => {
                kite::launch(f.effects, f.audio, from, true);
                self.kites.push(Flyer {
                    position: from,
                    velocity,
                    steer,
                    steer_for,
                    seconds,
                    age: 0.0,
                    trail: kite::Trail::default(),
                });
            }
            GearPreview::Relay {
                attack,
                turned,
                from,
                thorn,
                to,
            } => self.incoming.push(Incoming {
                attack,
                turned,
                from,
                thorn,
                to,
                age: 0.0,
            }),
        }
    }

    /// Advances everything staged and works out your weapon's heat and your slab's bar.
    pub fn update(&mut self, f: &mut GearFrame) {
        let dt = f.dt.max(0.0);
        self.time += dt as f64;
        for p in std::mem::take(&mut self.staged) {
            self.preview(p, f);
        }
        self.update_bolas(f, dt);
        self.update_slabs(f, dt);
        self.update_orbits(f, dt);
        self.update_lamps(f, dt);
        self.update_kites(f, dt);
        // Relays: the incoming attack reaches its thorn, then the relay plays.
        for r in &mut self.incoming {
            if r.age == 0.0 {
                match relay::relayed(r.attack, r.turned) {
                    relay::Relayed::Ray => {
                        let style = super::attack_style(r.attack);
                        f.beams.beam(
                            r.from,
                            r.thorn,
                            style.beam_width.max(0.04),
                            style.beam_color,
                            0.25,
                        );
                    }
                    relay::Relayed::Projectile => {
                        f.beams
                            .beam(r.from, r.thorn, 0.05, Vec4::new(1.4, 1.0, 0.6, 0.6), 0.2);
                    }
                    _ => {}
                }
            }
            r.age += dt;
            if r.age >= 0.05 {
                self.pending
                    .push(Pending::Relay(r.attack, r.turned, r.thorn, r.to));
                r.age = f32::INFINITY;
            }
        }
        self.incoming.retain(|r| r.age.is_finite());
        // Your stoke: the heat on your weapon, its embers or its steam.
        self.heat = Heat::default();
        if let Some((age, stoke_t, vent_t)) = &mut self.stoke {
            *age += dt;
            let was_stoked = *age - dt < *stoke_t;
            if *age < *stoke_t {
                self.heat.stoked = Some(*age / stoke_t.max(0.01));
            } else if *age < *stoke_t + *vent_t {
                if was_stoked {
                    f.audio.play_2d(Sfx::StokeVent);
                }
                self.heat.venting = Some(1.0 - (*age - *stoke_t) / vent_t.max(0.01));
            }
            if *age >= *stoke_t + *vent_t {
                self.stoke = None;
            }
        }
        if let Some((muzzle, dir)) = f.muzzle {
            if let Some(p) = self.heat.stoked {
                stoke::stoked_effects(f.effects, f.rng, muzzle, dir, p, dt);
            }
            if let Some(v) = self.heat.venting {
                let body = muzzle - dir * 0.25 + Vec3::Y * 0.03;
                stoke::vent_effects(f.effects, f.rng, muzzle, body, dir, v, true, dt);
            }
        }
    }

    fn update_bolas(&mut self, f: &mut GearFrame, dt: f32) {
        let mut landed = Vec::new();
        for b in &mut self.flights {
            b.age += dt;
            let aim = b.to + Vec3::Y * b.height * 0.3;
            let hit = cast(f.world, b.position, b.velocity * dt);
            if b.position.distance(aim) < 0.8 || hit.is_some() || b.age > 3.0 {
                landed.push(*b);
                b.age = f32::INFINITY;
                continue;
            }
            b.position += b.velocity * dt;
            b.velocity.y -= 8.0 * dt;
            bolas::flight_trail(f.effects, f.rng, b.position, b.age, dt);
        }
        self.flights.retain(|b| b.age.is_finite());
        for b in landed {
            bolas::bind(f.effects, f.beams, f.audio, b.to, b.height);
            self.bound.push(Bound {
                feet: b.to,
                height: b.height,
                age: 0.0,
                seconds: b.seconds,
            });
            if let Some(other) = b.link {
                let a = b.to + Vec3::Y * b.height * 0.5;
                let c = other + Vec3::Y * b.height * 0.5;
                bolas::link_snap(f.effects, f.beams, f.audio, a, c);
                bolas::bind(f.effects, f.beams, f.audio, other, b.height);
                self.bound.push(Bound {
                    feet: other,
                    height: b.height,
                    age: 0.0,
                    seconds: b.seconds,
                });
                self.links.push(Link {
                    a,
                    b: c,
                    age: 0.0,
                    seconds: 4.0,
                    next: 0.6,
                    pulse: 0.0,
                });
            }
        }
        for b in &mut self.bound {
            b.age += dt;
            let left = 1.0 - b.age / b.seconds.max(0.01);
            bolas::wrap(f.beams, b.feet, b.height, 0.5, left, self.time);
        }
        self.bound.retain(|b| b.age < b.seconds);
        for l in &mut self.links {
            l.age += dt;
            l.next -= dt;
            l.pulse = (l.pulse - dt * 3.0).max(0.0);
            if l.next <= 0.0 {
                l.next = 0.9;
                l.pulse = 1.0;
                bolas::share_pulse(f.effects, f.audio, l.b);
            }
            let left = 1.0 - l.age / l.seconds.max(0.01);
            bolas::link_cord(f.beams, l.a, l.b, left, l.pulse, self.time);
        }
        self.links.retain(|l| l.age < l.seconds);
    }

    fn slab_root(s: &Slab, f: &GearFrame) -> Mat4 {
        match s.holder {
            Some((feet, yaw)) => bulwark::slab_transform(feet + Vec3::Y * 1.7, yaw, 0.0),
            None => bulwark::slab_transform(f.eye, f.yaw, f.pitch),
        }
    }

    fn update_slabs(&mut self, f: &mut GearFrame, dt: f32) {
        self.own_slab = None;
        let mut gone = Vec::new();
        for s in &mut self.slabs {
            s.age += dt;
            s.flash = (s.flash - dt * 4.0).max(0.0);
            s.cycle = PlayerAnim::advance_cycle(s.cycle, Vec3::ZERO, dt);
            let root = Self::slab_root(s, f);
            // Blows spread over its time (the last one in its final fifth).
            let due = ((s.age / (s.seconds * 0.8).max(0.01)) * s.hits as f32) as u32;
            while s.taken < due.min(s.hits) {
                let before = bulwark::crack_stage(s.absorbed());
                s.taken += 1;
                s.flash = 1.0;
                let k = s.taken as f32;
                let at = bulwark::face_point(root, (k * 1.7).sin() * 0.6, (k * 2.3).cos() * 0.5);
                let normal = root.transform_vector3(Vec3::NEG_Z);
                let cracked = bulwark::crack_stage(s.absorbed()) > before;
                bulwark::absorb(f.effects, f.audio, f.rng, at, normal, cracked);
            }
            let broken = s.shatter && s.taken >= s.hits && s.hits > 0;
            if broken || s.age >= s.seconds {
                gone.push((root, broken));
                s.age = f32::INFINITY;
                continue;
            }
            if s.holder.is_none() {
                let hold = if s.shatter {
                    1.0 - s.absorbed()
                } else {
                    1.0 - 0.8 * s.absorbed()
                };
                self.own_slab = Some((hold, 1.0 - s.age / s.seconds.max(0.01)));
            }
        }
        self.slabs.retain(|s| s.age.is_finite());
        for (root, broken) in gone {
            if broken {
                bulwark::shatter(f.effects, f.beams, f.audio, f.rng, root);
            } else {
                bulwark::lower(f.effects, f.audio, root);
            }
        }
    }

    fn update_orbits(&mut self, f: &mut GearFrame, dt: f32) {
        let mut flung = Vec::new();
        for o in &mut self.orbits {
            o.age += dt;
            let own = o.around.is_none();
            let feet = o.around.or(f.feet).unwrap_or(f.eye - Vec3::Y * 1.6);
            let angle = wisp::orbit_angle(o.age, 0.8);
            wisp::orbit_effects(
                f.effects,
                f.beams,
                f.rng,
                feet,
                angle,
                wisp::Orbit::DEFAULT,
                own,
                dt,
            );
            if o.age >= o.seconds {
                let at = wisp::orbit_position(feet, angle, wisp::Orbit::DEFAULT);
                if let Some(dir) = o.fling {
                    wisp::fling(f.effects, f.beams, f.audio, f.rng, at, dir);
                    flung.push((at, dir.normalize_or(Vec3::NEG_Z) * 18.0, 0.0));
                }
                o.age = f32::INFINITY;
            }
        }
        self.orbits.retain(|o| o.age.is_finite());
        self.flung.extend(flung);
        let mut popped = Vec::new();
        for w in &mut self.flung {
            w.2 += dt;
            let hit = cast(f.world, w.0, w.1 * dt);
            if hit.is_some() || w.2 > 2.0 {
                popped.push(hit.unwrap_or(w.0));
                w.2 = f32::INFINITY;
                continue;
            }
            w.0 += w.1 * dt;
            wisp::flight_effects(f.effects, f.beams, f.rng, w.0, dt);
        }
        self.flung.retain(|w| w.2.is_finite());
        self.pending
            .extend(popped.into_iter().map(Pending::WispPop));
    }

    fn update_lamps(&mut self, f: &mut GearFrame, dt: f32) {
        for l in &mut self.lamps {
            l.age += dt;
            l.kick = (l.kick - dt * 6.0).max(0.0);
            match &mut l.lit {
                None => {
                    l.position += l.velocity * dt;
                    l.velocity.y -= 16.0 * dt;
                    if l.velocity.y < 0.0 && l.position.y <= l.to.y {
                        l.position = l.to;
                        l.lit = Some(0.0);
                        lantern::set_down(f.effects, f.audio, l.to);
                    }
                }
                Some(t) => {
                    *t += dt;
                    l.next_shot -= dt;
                    if *t >= l.seconds {
                        lantern::burn_out(f.effects, f.audio, f.rng, l.position, false);
                        l.age = f32::INFINITY;
                        continue;
                    }
                    if l.next_shot <= 0.0 && *t > 0.3 {
                        l.next_shot = match l.carrier {
                            WeaponKind::Riveter => 0.18,
                            WeaponKind::Kiln => 0.16,
                            _ => 0.2,
                        };
                        let (yaw, pitch) = lantern::aim(l.position, l.target);
                        let from = lantern::muzzle(l.position, yaw, pitch);
                        lantern::shot(
                            f.effects,
                            f.beams,
                            f.audio,
                            f.rng,
                            l.carrier,
                            from,
                            l.target,
                            (8.0, 20.0),
                        );
                        l.kick = 1.0;
                    }
                }
            }
        }
        self.lamps.retain(|l| l.age.is_finite());
    }

    fn update_kites(&mut self, f: &mut GearFrame, dt: f32) {
        let mut burst = Vec::new();
        for k in &mut self.kites {
            k.age += dt;
            if k.age < k.steer_for {
                // Turn towards the steer point at up to 120°/s (docs/rework.md).
                let want =
                    (k.steer - k.position).normalize_or(k.velocity.normalize_or(Vec3::NEG_Z));
                let dir = k.velocity.normalize_or(Vec3::NEG_Z);
                let angle = dir.angle_between(want);
                let max = 120f32.to_radians() * dt;
                if angle > 1e-4 {
                    let t = (max / angle).min(1.0);
                    let new = dir.lerp(want, t).normalize_or(dir);
                    k.velocity = new * k.velocity.length();
                }
            }
            let hit = cast(f.world, k.position, k.velocity * dt);
            if hit.is_some() || k.age >= k.seconds {
                burst.push(hit.unwrap_or(k.position));
                k.age = f32::INFINITY;
                continue;
            }
            k.position += k.velocity * dt;
            k.trail.follow(k.position, k.velocity, dt);
            let heart = kite::heart(k.position, k.velocity, k.trail.bank);
            kite::trail_effects(
                f.effects,
                f.beams,
                f.rng,
                &k.trail,
                heart,
                k.age < k.steer_for,
                dt,
            );
            if k.trail.flutter_due(dt) {
                f.audio.play(Sfx::KiteFlutter, PlayParams::at(k.position));
            }
            if k.age < k.steer_for {
                // Steered (the staged one: towards its point, as your crosshair would).
                kite::steer_marker(f.beams, f.world, f.eye, k.steer - f.eye, self.time);
            }
        }
        self.kites.retain(|k| k.age.is_finite());
        self.pending
            .extend(burst.into_iter().map(Pending::KiteBurst));
    }

    /// Whether anything waits for [`GearView::play_pending`].
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Plays what needed the Fx context: relays, the kite's and the wisp's bursts.
    pub fn play_pending(&mut self, ctx: &mut FxCtx) {
        for p in std::mem::take(&mut self.pending) {
            match p {
                Pending::Relay(attack, turned, thorn, to) => {
                    relay::play(ctx, attack, turned, &[thorn, to])
                }
                Pending::KiteBurst(at) => {
                    super::arsenal::blast(ctx, super::arsenal::Blast::Kite, at, 4.5)
                }
                Pending::WispPop(at) => super::fx::explosion(ctx, at, 4.0, Attack::Wisp, true),
            }
        }
    }

    /// Draws the staged gear.
    pub fn draw(
        &self,
        lib: Option<&ModelLibrary>,
        eye: Vec3,
        yaw: f32,
        pitch: f32,
        draws: &mut Vec<DrawItem>,
        lights: &mut Vec<PointLight>,
    ) {
        for l in &self.lamps {
            if let Some(t) = l.lit
                && lights.len() < 48
            {
                lights.push(lantern::light(
                    l.position,
                    1.0 - t / l.seconds.max(0.01),
                    self.time,
                ));
            }
        }
        let Some(lib) = lib else {
            return;
        };
        for b in &self.flights {
            bolas::draw_flight(lib, b.position, b.velocity, b.age, draws);
        }
        for b in &self.bound {
            let left = 1.0 - b.age / b.seconds.max(0.01);
            bolas::draw_weights(lib, b.feet, b.height, 0.5, left, self.time, draws);
        }
        for s in &self.slabs {
            let root = match s.holder {
                Some((feet, y)) => {
                    // The holder: a Shrike with its arms up behind the slab.
                    let anim = PlayerAnim {
                        yaw: y,
                        on_ground: true,
                        time: self.time,
                        weapon: WeaponKind::Bellows.id(),
                        cycle: s.cycle,
                        ..Default::default()
                    };
                    lib.draw_player_scaled(
                        &anim,
                        feet,
                        crate::models::shrike::REMOTE_SCALE,
                        &Tint::team(crate::palette::palette().team(2)),
                        draws,
                    );
                    bulwark::slab_transform(feet + Vec3::Y * 1.7, y, 0.0)
                }
                None => bulwark::slab_transform(eye, yaw, pitch),
            };
            bulwark::draw_slab(
                lib,
                root,
                s.absorbed(),
                s.flash,
                1.0 - s.age / s.seconds.max(0.01),
                s.holder.is_none(),
                self.time,
                draws,
            );
        }
        for o in &self.orbits {
            let feet = o.around.unwrap_or(eye - Vec3::Y * 1.6);
            let at =
                wisp::orbit_position(feet, wisp::orbit_angle(o.age, 0.8), wisp::Orbit::DEFAULT);
            wisp::draw_orbiting(lib, at, self.time, draws);
        }
        for w in &self.flung {
            wisp::draw_orbiting(lib, w.0, self.time, draws);
        }
        for l in &self.lamps {
            let left = l.lit.map_or(1.0, |t| 1.0 - t / l.seconds.max(0.01));
            let (yaw, pitch) = lantern::aim(l.position, l.target);
            let spin = if l.lit.is_none() { l.age * 9.0 } else { 0.0 };
            lantern::draw(
                lib, l.position, spin, yaw, pitch, l.kick, l.carrier, left, self.time, draws,
            );
        }
        for k in &self.kites {
            let pulse = 0.5 + 0.5 * ((self.time * 9.0).sin() as f32);
            kite::draw(lib, k.position, k.velocity, k.trail.bank, pulse, draws);
        }
    }

    /// Your slab's bar and the heat gauge.
    pub fn hud(&self, list: &mut UiDrawList, t: &Theme, c: Rect, now: f64) {
        if let Some((hold, left)) = self.own_slab {
            bulwark::hud(list, t, c, hold, left, now);
        }
        stoke::hud(list, t, c, &self.heat, now);
    }
}

impl InGame {
    /// Stages a gear visual client-side (tools and screenshots; see [`GearPreview`]).
    #[doc(hidden)]
    pub fn preview_gear(&mut self, preview: GearPreview) {
        self.combat.gear.staged.push(preview);
    }

    /// Per frame (visual time, after the Fx playback): the gear's bookkeeping, your weapon's
    /// heat on the viewmodel, what waits for the Fx context.
    pub(super) fn combat_gear(&mut self, common: &mut Common, vdt: f32) {
        let InGame {
            session,
            combat,
            effects,
            viewmodel,
            feedback,
            remote,
            controls,
            last_camera,
            ..
        } = self;
        let Some(net) = session.as_ref().map(Session::client) else {
            return;
        };
        let cam = *last_camera;
        let muzzle = viewmodel.last_draw().map(|d| {
            (
                cam.position + cam.rotation * d.muzzle,
                (cam.rotation * d.muzzle_dir).normalize_or(cam.rotation * Vec3::NEG_Z),
            )
        });
        let mut frame = GearFrame {
            effects,
            beams: &mut combat.beams,
            audio: &mut common.audio,
            rng: &mut combat.rng,
            world: net.world().map(|w| &w.collision.world),
            eye: cam.position,
            yaw: controls.yaw,
            pitch: controls.pitch,
            feet: net.local_state().map(|s| s.position),
            muzzle,
            dt: vdt,
        };
        combat.gear.update(&mut frame);
        // The replicated gear (net.rs).
        let (own, own_time, local_owner) = super::own_projectiles(net);
        let own_offset = viewmodel
            .last_draw()
            .map_or(Vec3::ZERO, |d| cam.rotation * d.muzzle);
        let entity_frame = super::entities::EntityFrame {
            entities: net.remote_entities(),
            render_tick: net.render_tick(),
            tick_rate: net.tick_rate(),
            tuning: &net.tuning().combat,
            own: &own,
            own_time,
            local_owner,
            own_offset,
            stand_ins: Some(&combat.stand_ins),
            camera: cam.position,
            view: cam.rotation * Vec3::NEG_Z,
            world: net.world().map(|w| &w.collision.world),
        };
        let local = net.local_player_id().map(NetId::player);
        let own_entity = local.and_then(|id| net.frames().latest().and_then(|(_, f)| f.get(id)));
        let gear = NetGear {
            frame: &entity_frame,
            local,
            own: own_entity,
            weapons: combat.predictor.state(),
            dead: net.own_combat().is_some_and(|c| c.combat.dead),
            remote,
            movement: &net.tuning().movement,
        };
        combat.gear.update_net(&gear, &mut frame);
        let (glow, body) = stoke::heat_tint(&combat.gear.heat, combat.gear.time);
        viewmodel.set_heat(glow, body);
        if combat.gear.has_pending() {
            let view = &combat.entities;
            let thorn_position = |id: NetId| view.thorn_position(id);
            let mut ctx = FxCtx {
                audio: &mut common.audio,
                effects,
                feedback,
                beams: &mut combat.beams,
                remote,
                world: net.world().map(|w| &w.collision.world),
                entities: net.remote_entities(),
                local: net.local_player_id(),
                listener: cam.position,
                local_chest: net.local_state().map(|s| s.position + Vec3::Y * 1.1),
                thorn_position: &thorn_position,
                rng: &mut combat.rng,
            };
            combat.gear.play_pending(&mut ctx);
        }
    }
}
