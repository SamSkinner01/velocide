//! The balance harness (`docs/balance.md`): measures what every weapon and ability really
//! deals by running the simulation — a flat test level, one god-mode shooter carrying a build,
//! Moulds (from the shipped definitions) or a PvP player standing still at a range — and
//! reading the damage events. Spread, pellets, cones, bloom, hit assist, weak points, armour,
//! projectile travel and gravity, explosion falloff, burns, brands and cooldowns are the real
//! code paths; only the aim is idealised: a skilled player who puts the crosshair on the aim
//! point every tick (the low ballistic arc for anything with gravity or a lob).
//!
//! Targets never move (their position, velocity and facing are restored every tick: no
//! knockback drift, no AI) and take damage without dying unless a scenario uses real health,
//! so one run gives a damage-over-time curve and every time to kill is read off it.
//!
//! Shared by `examples/balance_report.rs` (the tables) and `tests/combat_balance.rs` (the
//! bands), through `#[path]`.

#![allow(dead_code)]

use std::sync::Arc;

use glam::Vec3;
use shared::input::{Buttons, PlayerInput};
use shared::level::{Brush, LevelDef, LevelKind, SpawnPoint};
use shared::loadout::{Ability, Loadout, WeaponBuild, WeaponClass, WeaponKind};
use shared::lobby::GameMode;
use shared::protocol::NetId;
use shared::sim::weapons::{AbilityStyle, FireIntent, PrimaryStyle, WeaponState, step_weapons};
use shared::sim::{HitPart, HitTarget, PlayerId, SimEvent, SimWorld, SpawnOptions, TickInputs};
use shared::tuning::{CombatTuning, Tuning};

pub const TICK_RATE: u32 = 60;
pub const DT: f32 = 1.0 / TICK_RATE as f32;

/// Health of the targets that only record damage (they never die).
const ENDLESS: f32 = 1.0e7;

/// The Moulds in the tables, in order (the Effigy is a training dummy, not a fight).
pub const MOULDS: [&str; 8] = [
    "gnawer",
    "censer",
    "harrier",
    "snuffer",
    "heron",
    "bellwether",
    "smelter",
    "gaoler",
];

/// The ranges every table is measured at (m, feet to feet).
pub const RANGES: [f32; 3] = [5.0, 15.0, 30.0];

/// The weapons the tables and the bands cover (all of them).
pub fn weapons() -> Vec<WeaponKind> {
    WeaponKind::ALL.to_vec()
}

/// The classes of [`weapons`].
pub fn classes() -> Vec<WeaponClass> {
    WeaponClass::ALL.to_vec()
}

/// Which balance profile (and level kind) a measurement runs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Pve,
    Pvp,
}

impl Side {
    pub fn mode(self) -> GameMode {
        match self {
            Side::Pve => GameMode::Campaign,
            Side::Pvp => GameMode::Deathmatch,
        }
    }
}

/// What the shooter presses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// The primary, as fast as it fires (the bow: full draws).
    Primary,
    /// The build's ability `mode` (0 = A, 1 = B) as often as it is ready; the primary idle (on
    /// the bow every full draw that can carry it is armed).
    Ability(u8),
    /// The primary held and ability `mode` used whenever it is ready (a build in a fight).
    Both(u8),
}

impl Trigger {
    fn fires_primary(self) -> bool {
        !matches!(self, Trigger::Ability(_))
    }

    fn ability_mode(self) -> Option<u8> {
        match self {
            Trigger::Primary => None,
            Trigger::Ability(m) | Trigger::Both(m) => Some(m),
        }
    }
}

/// Where the shooter aims on the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aim {
    /// The body shape's centre.
    Body,
    /// A Mould's weak point `i` (as listed in its definition) / a player's head (any `i`).
    Weak(usize),
}

/// Assumptions layered over the shipped tuning, for numbers ahead of other areas' merges. None
/// at the moment: everything the refit's balance pass assumed (**pve**'s Mould hit shapes and
/// hit assist, **feel**'s projectile hit pad) is on main, so every run measures it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Assume {}

/// One damage event on a tracked target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Seconds since the first input.
    pub time: f32,
    pub target: HitTarget,
    pub dealt: f32,
    pub weak: bool,
}

/// What a run recorded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trace {
    pub hits: Vec<Hit>,
    /// Seconds simulated.
    pub secs: f32,
    /// Times the ability was used (a Limpet use = a full set of charges, detonated).
    pub uses: u32,
    /// Seconds at which each tracked target with real health died.
    pub deaths: Vec<f32>,
}

impl Trace {
    pub fn total(&self) -> f32 {
        self.hits.iter().map(|h| h.dealt).sum()
    }

    /// Damage per second over the whole run.
    pub fn dps(&self) -> f32 {
        self.total() / self.secs.max(DT)
    }

    /// Seconds until `health` worth of damage landed on one target (all targets when
    /// `None`), or `None` if it never did.
    pub fn time_to(&self, health: f32, target: Option<HitTarget>) -> Option<f32> {
        let mut sum = 0.0;
        for h in &self.hits {
            if target.is_some_and(|t| t != h.target) {
                continue;
            }
            sum += h.dealt;
            if sum >= health - 1e-3 {
                return Some(h.time);
            }
        }
        None
    }

    /// Damage dealt within the first `secs`.
    pub fn within(&self, secs: f32) -> f32 {
        self.hits
            .iter()
            .filter(|h| h.time <= secs + 1e-4)
            .map(|h| h.dealt)
            .sum()
    }
}

/// When a run ends (whichever comes first).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    /// Seconds of pressing.
    pub secs: f32,
    /// Total damage on the tracked targets.
    pub damage: Option<f32>,
    /// Ability uses (then the run stops pressing and waits `settle` seconds for what it fired
    /// to land).
    pub uses: Option<u32>,
    pub settle: f32,
}

impl Stop {
    pub fn secs(secs: f32) -> Self {
        Self {
            secs,
            damage: None,
            uses: None,
            settle: 0.0,
        }
    }

    /// Until `damage` landed, at most `secs`.
    pub fn damage(damage: f32, secs: f32) -> Self {
        Self {
            damage: Some(damage),
            ..Self::secs(secs)
        }
    }

    /// One use of the ability, then `settle` seconds for it to land.
    pub fn one_use(settle: f32) -> Self {
        Self {
            uses: Some(1),
            settle,
            ..Self::secs(12.0)
        }
    }
}

struct Tracked {
    target: HitTarget,
    position: Vec3,
    yaw: f32,
    alive: bool,
}

/// A world with one shooter and its targets.
pub struct Bench {
    pub world: SimWorld,
    pub tuning: Tuning,
    pub side: Side,
    pub shooter: PlayerId,
    pub build: WeaponBuild,
    shooter_at: Vec3,
    tracked: Vec<Tracked>,
    seq: u32,
    prev: Buttons,
    /// Limpet: detonating the charges out.
    detonating: bool,
    detonations: u32,
    /// The bow looses at this fraction of a full draw (1.0: full draws).
    pub draw: f32,
    /// Players take real damage and die (a duel): no top-up, the real `max_health`.
    real_health: bool,
    /// PvP opponents use their knack (looking at their feet) on this run tick.
    pub opponent_knack_at: Option<u32>,
    /// Hold the shooter at this many HP missing (no hard damage) and record what heals it.
    pub wounded: Option<f32>,
    /// HP the shooter was healed by (with `wounded`).
    pub healed: f32,
    run_tick: u32,
}

/// 100 m square floor at y = 0 (small: every world builds a nav grid for its Moulds).
fn flat_level(kind: LevelKind) -> LevelDef {
    LevelDef {
        name: "balance".into(),
        kind,
        brushes: vec![Brush {
            min: Vec3::new(-50.0, -1.0, -50.0),
            max: Vec3::new(50.0, 0.0, 50.0),
            rotation: Vec3::ZERO,
            material: "concrete".into(),
            tint: Vec3::ONE,
            clip_only: false,
            no_collide: false,
        }],
        spawns: vec![SpawnPoint {
            position: Vec3::new(0.0, 0.0, 40.0),
            yaw: 0.0,
        }],
        ..Default::default()
    }
}

/// The tuning a `side` match runs with (`Tuning::for_mode`), plus the assumptions.
pub fn effective(base: &Tuning, side: Side, _assume: Assume) -> Tuning {
    base.for_mode(side.mode())
}

/// A valid build of `weapon` with `ability` in slot A (the other slot: the first other
/// ability of its class).
pub fn build_with(weapon: WeaponKind, ability: Ability) -> WeaponBuild {
    let other = weapon
        .abilities()
        .iter()
        .copied()
        .find(|a| *a != ability)
        .unwrap_or(ability);
    WeaponBuild {
        weapon,
        abilities: [ability, other],
    }
}

/// Every weapon that can carry `ability`, its home weapon first.
pub fn carriers(ability: Ability) -> Vec<WeaponKind> {
    let home = ability.home();
    let mut out = vec![home];
    out.extend(
        WeaponKind::ALL
            .into_iter()
            .filter(|w| *w != home && ability.fits(*w)),
    );
    out
}

/// Launch speed, gravity and lob angle (degrees above the aim) of what `intent` fires, if it
/// flies a curve worth leading.
fn ballistics(c: &CombatTuning, intent: FireIntent) -> Option<(f32, f32, f32)> {
    let arrow = |draw: f32| {
        let l = &c.longwick;
        let k = draw.clamp(0.0, 1.0);
        (
            l.min_speed + (l.max_speed - l.min_speed) * k,
            l.arrow_gravity,
            0.0,
        )
    };
    match intent {
        FireIntent::Primary { weapon, charge } => match weapon {
            WeaponKind::Tumbler => Some((c.tumbler.bomb_speed, c.tumbler.bomb_gravity, 0.0)),
            WeaponKind::Riveter => Some((c.riveter.rivet_speed, c.riveter.rivet_gravity, 0.0)),
            WeaponKind::Longwick => Some(arrow(charge)),
            _ => None,
        },
        FireIntent::Ability {
            weapon,
            ability,
            charge,
        } => {
            if weapon.class() == WeaponClass::Draw {
                return Some(arrow(charge));
            }
            match ability {
                Ability::Volley if weapon == WeaponKind::Riveter => {
                    Some((c.riveter.rivet_speed, c.riveter.rivet_gravity, 0.0))
                }
                Ability::Flare => Some((c.flare.speed, c.flare.gravity, c.flare.angle_degrees)),
                Ability::Gravewell => Some((
                    c.tumbler.gravewell_speed,
                    c.tumbler.gravewell_gravity,
                    c.tumbler.gravewell_angle_degrees,
                )),
                Ability::Limpet => Some((c.limpet.speed, c.limpet.gravity, 0.0)),
                Ability::Mortar => Some((c.mortar.speed, c.mortar.gravity, c.mortar.angle_degrees)),
                Ability::Hurl => Some((c.hurl.speed, c.hurl.gravity, 0.0)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Lands on the floor rather than on the target (aim at the feet).
fn aims_at_feet(intent: FireIntent) -> bool {
    matches!(
        intent,
        FireIntent::Ability {
            ability: Ability::Gravewell | Ability::Flare,
            ..
        }
    )
}

/// Yaw and pitch from `from` towards `to`; with `(speed, gravity, lob)` the low ballistic arc
/// (45° when out of reach), minus the lob the weapon adds.
fn aim_angles(from: Vec3, to: Vec3, flight: Option<(f32, f32, f32)>) -> (f32, f32) {
    let d = to - from;
    let yaw = (-d.x).atan2(-d.z);
    let dx = Vec3::new(d.x, 0.0, d.z).length().max(1e-3);
    let dy = d.y;
    let pitch = match flight {
        Some((v, g, lob)) if g > 1e-3 => {
            let v2 = v * v;
            let disc = v2 * v2 - g * (g * dx * dx + 2.0 * dy * v2);
            let theta = if disc < 0.0 {
                std::f32::consts::FRAC_PI_4
            } else {
                ((v2 - disc.sqrt()) / (g * dx)).atan()
            };
            theta - lob.to_radians()
        }
        Some((_, _, lob)) => dy.atan2(dx) - lob.to_radians(),
        None => dy.atan2(dx),
    };
    (yaw, pitch.clamp(-1.5, 1.5))
}

impl Bench {
    /// A world for `side` with the shooter at the origin looking down -Z, carrying `build`.
    pub fn new(base: &Tuning, side: Side, assume: Assume, build: WeaponBuild) -> Self {
        let kind = match side {
            Side::Pve => LevelKind::Campaign,
            Side::Pvp => LevelKind::Arena,
        };
        let mut world = SimWorld::new("balance", Arc::new(flat_level(kind)), TICK_RATE);
        let mut tuning = effective(base, side, assume);
        // Players never die: damage is read from the events (times to kill are read off the
        // curve against the real 100 HP).
        tuning.combat.max_health = ENDLESS;
        let shooter = PlayerId(0);
        let p = world.add_player(shooter, "shooter");
        p.movement.position = Vec3::ZERO;
        p.movement.on_ground = true;
        p.movement.ground_normal = Vec3::Y;
        p.movement.mode = shared::movement::MoveMode::Walk;
        p.combat.god = true;
        p.notarget = true;
        world.force_loadout(shooter, Some(Loadout::single(build)));
        Self {
            world,
            tuning,
            side,
            shooter,
            build,
            shooter_at: Vec3::ZERO,
            tracked: Vec::new(),
            seq: 0,
            prev: Buttons::NONE,
            detonating: false,
            detonations: 0,
            draw: 1.0,
            real_health: false,
            opponent_knack_at: None,
            wounded: None,
            healed: 0.0,
            run_tick: 0,
        }
    }

    /// A PvP duel: players take real damage and die (the opponent's death ends a run).
    pub fn duel(base: &Tuning, assume: Assume, build: WeaponBuild) -> Self {
        let mut b = Self::new(base, Side::Pvp, assume, build);
        b.real_health = true;
        b.tuning.combat.max_health = effective(base, Side::Pvp, assume).combat.max_health;
        b
    }

    /// A Mould `name` with its feet at `at`, facing the shooter (or away: `behind`), with
    /// `health` (`None`: it never dies).
    pub fn mould(&mut self, name: &str, at: Vec3, behind: bool, health: Option<f32>) -> NetId {
        let def = self.world.enemy_defs.get(name).expect("a shipped Mould");
        let to_shooter = self.shooter_at - at;
        let facing = (-to_shooter.x).atan2(-to_shooter.z);
        let yaw = if behind {
            facing + std::f32::consts::PI
        } else {
            facing
        };
        let scale = health.unwrap_or(ENDLESS) / def.health.max(1e-3);
        let id = self
            .world
            .spawn_enemy_with(
                name,
                at,
                yaw,
                SpawnOptions {
                    health_scale: scale,
                    ..Default::default()
                },
            )
            .expect("the Mould spawns");
        self.tracked.push(Tracked {
            target: HitTarget::Entity(id),
            position: at,
            yaw,
            alive: true,
        });
        id
    }

    /// A Mould at `range` straight ahead (flyers at their hover height).
    pub fn mould_at(&mut self, name: &str, range: f32, behind: bool, health: Option<f32>) -> NetId {
        let def = self.world.enemy_defs.get(name).expect("a shipped Mould");
        let y = match def.locomotion {
            shared::sim::enemy::Locomotion::Flying { min_height, .. } => min_height + 1.0,
            _ => 0.0,
        };
        self.mould(name, Vec3::new(0.0, y, -range), behind, health)
    }

    /// A PvP opponent standing at `range` straight ahead, facing the shooter.
    pub fn opponent(&mut self, range: f32) -> PlayerId {
        let id = PlayerId(1 + self.tracked.len() as u8);
        let at = Vec3::new(0.0, 0.0, -range);
        let p = self.world.add_player(id, "opponent");
        p.movement.position = at;
        p.movement.on_ground = true;
        p.movement.ground_normal = Vec3::Y;
        p.movement.mode = shared::movement::MoveMode::Walk;
        p.yaw = std::f32::consts::PI;
        p.combat.protection = 0;
        p.combat.damage = 0.0;
        self.tracked.push(Tracked {
            target: HitTarget::Player(id),
            position: at,
            yaw: std::f32::consts::PI,
            alive: true,
        });
        id
    }

    /// `count` Moulds `name` with real `health` in a tight pack (rows of three, `spacing`
    /// apart) whose front row stands at `range`.
    pub fn pack(&mut self, name: &str, count: usize, range: f32, spacing: f32, health: f32) {
        for i in 0..count {
            let col = (i % 3) as f32 - 1.0;
            let row = (i / 3) as f32;
            let at = Vec3::new(col * spacing, 0.0, -range - row * spacing);
            self.mould(name, at, false, Some(health));
        }
    }

    fn restore(&mut self) {
        let lunging = self.world.melee.lunges.contains_key(&self.shooter);
        if let Some(p) = self.world.players.get_mut(&self.shooter) {
            if !lunging {
                p.movement.position = self.shooter_at;
                p.movement.velocity = Vec3::ZERO;
                p.movement.on_ground = true;
            }
            if let Some(missing) = self.wounded {
                p.combat.damage = missing;
                p.combat.hard = 0.0;
            }
        }
        for t in &mut self.tracked {
            match t.target {
                HitTarget::Entity(id) => match self.world.enemies.get_mut(&id) {
                    Some(e) if e.is_alive() => {
                        e.position = t.position;
                        e.velocity = Vec3::ZERO;
                        e.yaw = t.yaw;
                    }
                    _ => t.alive = false,
                },
                HitTarget::Player(id) => {
                    if let Some(p) = self.world.players.get_mut(&id) {
                        p.movement.position = t.position;
                        p.movement.velocity = Vec3::ZERO;
                        p.movement.on_ground = true;
                        p.movement.slow_time = 0.0;
                        p.combat.protection = 0;
                        if !self.real_health {
                            // Kept topped up: only the events count.
                            p.combat.damage = 0.0;
                            p.combat.hard = 0.0;
                        }
                    }
                }
            }
        }
    }

    /// The point to aim at on the nearest living target, and the feet under it.
    fn aim_point(&self, aim: Aim) -> Option<(Vec3, Vec3)> {
        let boxes = self.world.current_hitboxes(&self.tuning);
        let target = self.tracked.iter().filter(|t| t.alive).min_by(|a, b| {
            let da = a.position.distance(self.shooter_at);
            let db = b.position.distance(self.shooter_at);
            da.total_cmp(&db)
        })?;
        let hb = boxes.iter().find(|h| h.target == target.target)?;
        let body = hb.center()?;
        let point = match (aim, target.target) {
            (Aim::Body, _) => body,
            (Aim::Weak(_), HitTarget::Player(_)) => hb.part_center(HitPart::Head).unwrap_or(body),
            (Aim::Weak(i), HitTarget::Entity(_)) => hb
                .part_center(HitPart::Weak(i.min(u8::MAX as usize) as u8))
                .unwrap_or(body),
        };
        Some((point, target.position))
    }

    /// The buttons the policy holds this tick for `trigger`.
    fn buttons(&mut self, trigger: Trigger) -> Buttons {
        let c = &self.tuning.combat;
        let s = self.world.players[&self.shooter].combat.weapons;
        let w = self.build.weapon;
        let mut b = Buttons::NONE;
        let bow = w.class() == WeaponClass::Draw;
        if trigger.fires_primary() || bow {
            match w.primary_style() {
                PrimaryStyle::Draw => {
                    let full = WeaponState::draw_ticks(c, TICK_RATE) as f32;
                    let release = (full * self.draw.clamp(0.0, 1.0)).round().max(1.0);
                    if (s.draw as f32) < release {
                        b = b | Buttons::FIRE;
                    }
                }
                _ => b = b | Buttons::FIRE,
            }
        }
        let Some(mode) = trigger.ability_mode() else {
            return b;
        };
        let ability = self.build.ability(mode);
        let ready = s.alt_cooldown[s.slot as usize] == 0;
        let was = self.prev.contains(Buttons::ALT_FIRE);
        if bow {
            if ready && s.armed == 0 && !was {
                b = b | Buttons::ALT_FIRE;
            }
            return b;
        }
        match ability.style() {
            AbilityStyle::Instant | AbilityStyle::Burst => b = b | Buttons::ALT_FIRE,
            AbilityStyle::Charge => {
                if s.charge < s.charge_ticks(c, TICK_RATE) {
                    b = b | Buttons::ALT_FIRE;
                }
            }
            AbilityStyle::Hold => {
                let charges: Vec<_> = self
                    .world
                    .limpets_of(self.shooter)
                    .into_iter()
                    .filter_map(|id| self.world.projectiles.get(&id))
                    .map(|p| p.stuck.is_some())
                    .collect();
                let max = c.limpet.max_live.max(1) as usize;
                if self.detonating {
                    if charges.is_empty() {
                        self.detonating = false;
                        self.detonations += 1;
                    } else {
                        b = b | Buttons::ALT_FIRE;
                    }
                } else if charges.len() >= max {
                    // Every charge stuck: hold to set them all off.
                    if charges.iter().all(|stuck| *stuck) {
                        self.detonating = true;
                    }
                } else if ready && !was {
                    b = b | Buttons::ALT_FIRE;
                }
            }
        }
        b
    }

    /// Runs `trigger` aiming at `aim` until `stop` (or until every target with real health is
    /// dead), then keeps aiming without pressing for `stop.settle` seconds, and records the
    /// damage on the tracked targets.
    pub fn run(&mut self, trigger: Trigger, aim: Aim, stop: Stop) -> Trace {
        let mut trace = Trace::default();
        let ticks = (stop.secs * TICK_RATE as f32).round() as u32;
        let settle = (stop.settle * TICK_RATE as f32).round() as u32;
        let mode = trigger.ability_mode().unwrap_or(0);
        let limpet = trigger.ability_mode().map(|m| self.build.ability(m)) == Some(Ability::Limpet);
        let start = self.world.tick;
        let mut uses = 0;
        self.detonations = 0;
        self.run_tick = 0;
        let mut pressed = 0;
        let mut idle = 0;
        loop {
            let pressing = pressed < ticks;
            if !pressing {
                if idle >= settle {
                    break;
                }
                idle += 1;
            }
            pressed += 1;
            let buttons = if pressing {
                self.buttons(trigger)
            } else {
                Buttons::NONE
            };
            let Some(used) = self.step_aimed(buttons, mode, aim, start, &mut trace) else {
                break;
            };
            uses += used as u32;
            let real = self
                .tracked
                .iter()
                .filter(|t| matches!(t.target, HitTarget::Entity(_)))
                .count();
            if real > 0 && trace.deaths.len() >= real && self.mortal() {
                break;
            }
            if self.real_health && !trace.deaths.is_empty() {
                break;
            }
            let used = if limpet { self.detonations } else { uses };
            if pressing
                && (stop.damage.is_some_and(|d| trace.total() >= d)
                    || stop.uses.is_some_and(|n| used >= n))
            {
                // Stop pressing; what was fired still lands during the settle.
                pressed = ticks;
            }
        }
        trace.uses = if limpet { self.detonations } else { uses };
        trace
    }

    /// Steps `secs` aiming at `aim` without pressing anything (projectiles land, bursts finish,
    /// burns tick).
    pub fn run_idle(&mut self, secs: f32, aim: Aim) -> Trace {
        self.run(
            Trigger::Primary,
            aim,
            Stop {
                settle: secs,
                ..Stop::secs(0.0)
            },
        )
    }

    /// One tick: `buttons` aimed at `aim` (leading what is about to fire); records the damage
    /// in `trace`. Returns whether the ability was used this tick, `None` when there is nothing
    /// left to aim at.
    fn step_aimed(
        &mut self,
        buttons: Buttons,
        mode: u8,
        aim: Aim,
        start: u32,
        trace: &mut Trace,
    ) -> Option<bool> {
        self.restore();
        let (point, feet) = self.aim_point(aim)?;
        let eye = self.world.players[&self.shooter]
            .movement
            .eye_position(&self.tuning.movement);
        self.seq += 1;
        let mut input = PlayerInput {
            seq: self.seq,
            buttons,
            weapon: 0,
            weapon_mode: mode,
            view_tick: self.world.tick + 1,
            ..Default::default()
        };
        (input.yaw, input.pitch) = aim_angles(eye, point, None);
        // Lead what is about to fire (the weapons are deterministic: step a copy).
        let mut probe = self.world.players[&self.shooter].combat.weapons;
        let before = probe.alt_cooldown[probe.slot as usize];
        let intents = step_weapons(&mut probe, &input, true, &self.tuning.combat, TICK_RATE);
        let used = probe.alt_cooldown[probe.slot as usize] > before;
        let lead = intents
            .iter()
            .rev()
            .find_map(|i| ballistics(&self.tuning.combat, *i).map(|b| (*i, b)));
        if let Some((intent, flight)) = lead {
            let at = if aims_at_feet(intent) { feet } else { point };
            (input.yaw, input.pitch) = aim_angles(eye, at, Some(flight));
        }
        self.prev = buttons;
        let mut inputs = TickInputs::new();
        inputs.insert(self.shooter, vec![input]);
        if let Some(at) = self.opponent_knack_at {
            for t in &self.tracked {
                if let HitTarget::Player(id) = t.target {
                    let knack = self.run_tick == at;
                    inputs.insert(
                        id,
                        vec![PlayerInput {
                            seq: self.seq,
                            yaw: std::f32::consts::PI,
                            pitch: -1.45,
                            buttons: if knack { Buttons::THROW } else { Buttons::NONE },
                            view_tick: self.world.tick + 1,
                            ..Default::default()
                        }],
                    );
                }
            }
        }
        self.run_tick += 1;
        let events = self.world.step(&inputs, &self.tuning, None);
        let time = (self.world.tick - start - 1) as f32 * DT;
        for e in &events {
            if let SimEvent::Healed { player, amount, .. } = e
                && *player == self.shooter
            {
                self.healed += amount;
            }
            if let SimEvent::Damage(d) = e
                && self.tracked.iter().any(|t| t.target == d.target)
            {
                if d.dealt > 0.0 {
                    trace.hits.push(Hit {
                        time,
                        target: d.target,
                        dealt: d.dealt,
                        weak: d.info.part != HitPart::Body,
                    });
                }
                if d.killed {
                    trace.deaths.push(time);
                }
            }
        }
        trace.secs = time + DT;
        Some(used)
    }

    /// Every tracked Mould has real health (a pack).
    fn mortal(&self) -> bool {
        self.tracked.iter().all(|t| match t.target {
            HitTarget::Entity(id) => self
                .world
                .enemies
                .get(&id)
                .is_none_or(|e| e.max_health < ENDLESS * 0.5),
            HitTarget::Player(_) => false,
        })
    }
}

// ---- the measurements the report and the tests use ----------------------------------------------

/// Health of Mould `name` at `difficulty` (index into the `difficulty.health` table: 1 =
/// NORMAL, 2 = HARD), one player.
pub fn mould_health(base: &Tuning, name: &str, difficulty: usize) -> f32 {
    let defs = shared::sim::EnemyRegistry::builtin();
    let def = defs.get(name).expect("a shipped Mould");
    def.health * base.difficulty.health[difficulty.min(4)]
}

/// The aim on Mould `name` for its weak point: the first weak point (the Gaoler from behind:
/// its lantern).
pub fn weak_of(name: &str, behind: bool) -> Aim {
    if name == "gaoler" && behind {
        Aim::Weak(1)
    } else {
        Aim::Weak(0)
    }
}

/// `trigger` with `build` on one endless Mould at `range` for `secs`.
#[allow(clippy::too_many_arguments)]
pub fn on_mould(
    base: &Tuning,
    assume: Assume,
    build: WeaponBuild,
    trigger: Trigger,
    name: &str,
    behind: bool,
    aim: Aim,
    range: f32,
    stop: Stop,
) -> Trace {
    let mut b = Bench::new(base, Side::Pve, assume, build);
    b.mould_at(name, range, behind, None);
    b.run(trigger, aim, stop)
}

/// `trigger` with `build` on a PvP opponent at `range` for `secs` (`head`: aiming at the head).
pub fn on_player(
    base: &Tuning,
    assume: Assume,
    build: WeaponBuild,
    trigger: Trigger,
    head: bool,
    range: f32,
    stop: Stop,
) -> Trace {
    let mut b = Bench::new(base, Side::Pvp, assume, build);
    b.opponent(range);
    let aim = if head { Aim::Weak(0) } else { Aim::Body };
    b.run(trigger, aim, stop)
}

/// `trigger` with `build` on a pack of six Gnawers (NORMAL health) at `range`: the trace's
/// `deaths` say when each fell.
pub fn on_pack(
    base: &Tuning,
    assume: Assume,
    build: WeaponBuild,
    trigger: Trigger,
    range: f32,
    secs: f32,
) -> Trace {
    let mut b = Bench::new(base, Side::Pve, assume, build);
    let health = mould_health(base, "gnawer", 1);
    b.pack("gnawer", 6, range, 1.6, health);
    b.run(trigger, Aim::Body, Stop::secs(secs))
}

/// `trigger` with `build` on six endless Gnawers in a pack at `range`: total damage.
pub fn on_endless_pack(
    base: &Tuning,
    assume: Assume,
    build: WeaponBuild,
    trigger: Trigger,
    range: f32,
    stop: Stop,
) -> Trace {
    let mut b = Bench::new(base, Side::Pve, assume, build);
    for i in 0..6 {
        let col = (i % 3) as f32 - 1.0;
        let row = (i / 3) as f32;
        let at = Vec3::new(col * 1.6, 0.0, -range - row * 1.6);
        b.mould("gnawer", at, false, None);
    }
    b.run(trigger, Aim::Body, stop)
}

/// The range an ability is measured at (m, feet to feet; on Gnawers in PvE): point-blank ones
/// (Gust, Discharge, Breach) at 4 m, the melee swings (Whirl, Upswing) at contact (2 m), the
/// Lunge across a 4.5 m gap, the rest at 10 m.
pub fn ability_range(ability: Ability) -> f32 {
    match ability {
        Ability::Gust | Ability::Discharge | Ability::Breach => 4.0,
        Ability::Whirl | Ability::Upswing => MELEE_RANGE,
        Ability::Lunge => 4.5,
        _ => 10.0,
    }
}

/// Melee distance, feet to feet, to a Gnawer or a player (m): well inside every swing's reach.
pub const MELEE_RANGE: f32 = 2.0;

/// Melee distance to Mould `name`: its hit shape's radius plus 1.2 m.
pub fn contact_range(name: &str) -> f32 {
    let defs = shared::sim::EnemyRegistry::builtin();
    let def = defs.get(name).expect("a shipped Mould");
    def.hit_radius.unwrap_or(def.radius) + 1.2
}

/// The range `weapon` is measured at against Mould `name` in a table at `range`: melee weapons
/// always at contact.
pub fn range_on(weapon: WeaponKind, name: &str, range: f32) -> f32 {
    if weapon.class() == WeaponClass::Melee {
        contact_range(name)
    } else {
        range
    }
}

/// The target of a single-pull measurement: a PvP opponent, or in PvE the Gaoler from behind
/// (a big unarmoured body: every pellet and splash lands).
fn single_target(b: &mut Bench, range: f32) {
    match b.side {
        Side::Pve => {
            b.mould_at("gaoler", range, true, None);
        }
        Side::Pvp => {
            b.opponent(range);
        }
    }
}

/// Damage of one trigger pull of `weapon`'s primary at `range` aimed at `aim` (a salvo, a
/// draw of `draw` (0..=1), a blast; burns included), PvE or PvP.
pub fn per_trigger(
    base: &Tuning,
    side: Side,
    weapon: WeaponKind,
    range: f32,
    aim: Aim,
    draw: f32,
) -> f32 {
    let build = weapon.default_build();
    let t = effective(base, side, Assume::default());
    // Long enough for one pull (a draw, a salvo), short of a second one.
    let one = match weapon.primary_style() {
        PrimaryStyle::Draw => t.combat.longwick.draw_time + t.combat.longwick.interval * 0.5,
        _ => weapon.primary_interval(&t.combat) - 1.5 * DT,
    };
    let mut b = Bench::new(base, side, Assume::default(), build);
    b.draw = draw;
    single_target(&mut b, range);
    // Then let what was fired land (no new trigger pulls), burns included.
    let stop = Stop {
        settle: 3.5,
        ..Stop::secs(one.max(DT))
    };
    b.run(Trigger::Primary, aim, stop).total()
}

/// Damage of one use of `ability` carried by `weapon` at `range` aimed at `aim` (on the bow:
/// the armed arrow at a `draw` (0..=1) draw; a Limpet use: a full set of charges detonated),
/// PvE (on one Gnawer) or PvP.
pub fn per_use(
    base: &Tuning,
    side: Side,
    weapon: WeaponKind,
    ability: Ability,
    range: f32,
    aim: Aim,
    draw: f32,
) -> f32 {
    let mut b = Bench::new(base, side, Assume::default(), build_with(weapon, ability));
    b.draw = draw;
    match side {
        Side::Pve => {
            b.mould_at("gnawer", range, false, None);
        }
        Side::Pvp => {
            b.opponent(range);
        }
    }
    b.run(Trigger::Ability(0), aim, Stop::one_use(4.0)).total()
}

/// What using ability A of `build` whenever it is ready adds to holding the primary: DPS over
/// 12 s at the ability's range on one Gnawer body (`pack`: six), both minus the primary alone.
pub fn build_gain(base: &Tuning, assume: Assume, build: WeaponBuild, pack: bool) -> f32 {
    let r = ability_range(build.ability(0));
    let run = |trigger| {
        let stop = Stop::secs(12.0);
        if pack {
            on_endless_pack(base, assume, build, trigger, r, stop)
        } else {
            on_mould(
                base,
                assume,
                build,
                trigger,
                "gnawer",
                false,
                Aim::Body,
                r,
                stop,
            )
        }
    };
    run(Trigger::Both(0)).dps() - run(Trigger::Primary).dps()
}

/// The range a primary is measured at: 5 m for the close-range ones (the scatterguns, the
/// Kiln's 13 m cone), melee at [`MELEE_RANGE`], 15 m for the rest.
pub fn weapon_range(weapon: WeaponKind) -> f32 {
    match weapon {
        WeaponKind::Bellows | WeaponKind::Thresher | WeaponKind::Kiln => 5.0,
        WeaponKind::Trimmer | WeaponKind::Tamper => MELEE_RANGE,
        _ => 15.0,
    }
}

/// A PvP duel: `build`'s primary held at the opponent's body from `range` until they die; the
/// opponent uses `knack` (if any) on the first tick. Seconds to the kill.
pub fn duel(
    base: &Tuning,
    build: WeaponBuild,
    knack: Option<shared::loadout::Knack>,
    range: f32,
) -> Option<f32> {
    let mut b = Bench::duel(base, Assume::default(), build);
    let id = b.opponent(range);
    if let Some(k) = knack {
        let quill = WeaponKind::Quill.default_build();
        b.world
            .force_loadout(id, Some(Loadout::single(quill).with_knack(k)));
        // A tick before the first shot (it would land first on the same tick).
        b.opponent_knack_at = Some(0);
        b.run_idle(DT, Aim::Body);
        b.opponent_knack_at = None;
    }
    let t = b.run(Trigger::Primary, Aim::Body, Stop::secs(10.0));
    t.deaths.first().copied()
}

/// HP per second the shooter is healed by (embers drawn in), held 40 HP down, holding
/// `build`'s primary for 10 s on a Gnawer (`pack`: six) at `range` (the swings at contact).
pub fn heal_rate(base: &Tuning, build: WeaponBuild, pack: bool, range: f32) -> f32 {
    let mut b = Bench::new(base, Side::Pve, Assume::default(), build);
    b.wounded = Some(40.0);
    if pack {
        for i in 0..6 {
            let col = (i % 3) as f32 - 1.0;
            let row = (i / 3) as f32;
            b.mould(
                "gnawer",
                Vec3::new(col * 1.6, 0.0, -range - row * 1.6),
                false,
                None,
            );
        }
    } else {
        b.mould_at("gnawer", range, false, None);
    }
    let t = b.run(Trigger::Primary, Aim::Body, Stop::secs(10.0));
    b.healed / t.secs.max(DT)
}
