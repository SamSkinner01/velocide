//! Weapons: the per-player weapon state machine and what each fire mode does.
//!
//! Two halves:
//!
//! * [`step_weapons`] — a **pure, deterministic** function of `(WeaponState, PlayerInput,
//!   alive, tuning)` that decides *what* fires this input (cooldowns, the Discharge charge,
//!   punch, thorn charges) and returns [`FireIntent`]s. All timers are integer ticks, so the
//!   owning client can run the same function on its predicted inputs (muzzle flash, sound and
//!   viewmodel recoil without waiting a round trip) and compare its prediction bit-exactly with
//!   the authoritative `OwnState::combat.weapons`.
//! * [`SimWorld::resolve_intent`] (server) — turns an intent into hitscan traces (`hitscan.rs`),
//!   area attacks (`area.rs`), projectiles (`projectiles.rs`), a punch/parry or a thorn throw
//!   (`thorn.rs`).
//!
//! Slots: `PlayerInput::weapon` 0 = Quill, 1 = Bellows, 2 = Arclash, 3 = Tumbler (other values
//! keep the current weapon). `PlayerInput::weapon_mode` 0 = alt A, 1 = alt B (others → alt A).
//! `FIRE` (held, automatic) = primary; `ALT_FIRE` = the selected alt mode (held; Discharge:
//! hold to charge, fires on release); `PUNCH` (press) = fist/parry; `THROW` (press) = thorn.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use super::combat::{Attack, DamageFlags, Owner, secs_to_ticks};
use super::hitscan::ShotSpec;
use super::projectiles::ProjectileKind;
use super::{HitboxHistory, PlayerId, SimEvent, SimWorld};
use crate::input::{Buttons, PlayerInput};
use crate::tuning::{CombatTuning, Tuning};

/// The four guns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Weapon {
    /// Precision spike-caster (hitscan).
    Quill = 0,
    /// Twin-barrel blast cannon (close range).
    Bellows = 1,
    /// Arc caster (sustained, chaining).
    Arclash = 2,
    /// Bouncing-bomb launcher (explosive).
    Tumbler = 3,
}

/// Number of alt-fire modes per weapon.
pub const ALT_MODES: u8 = 2;

impl Weapon {
    pub const ALL: [Weapon; 4] = [
        Weapon::Quill,
        Weapon::Bellows,
        Weapon::Arclash,
        Weapon::Tumbler,
    ];
    pub const COUNT: usize = 4;

    pub fn from_slot(slot: u8) -> Option<Weapon> {
        Self::ALL.get(slot as usize).copied()
    }

    pub fn slot(self) -> u8 {
        self as u8
    }

    pub fn name(self) -> &'static str {
        match self {
            Weapon::Quill => "Quill",
            Weapon::Bellows => "Bellows",
            Weapon::Arclash => "Arclash",
            Weapon::Tumbler => "Tumbler",
        }
    }

    pub fn primary(self) -> Attack {
        match self {
            Weapon::Quill => Attack::Quill,
            Weapon::Bellows => Attack::Blast,
            Weapon::Arclash => Attack::Arc,
            Weapon::Tumbler => Attack::Tumble,
        }
    }

    /// Alt mode 0 (A) or 1 (B).
    pub fn alt(self, mode: u8) -> Attack {
        match (self, mode) {
            (Weapon::Quill, 0) => Attack::Brand,
            (Weapon::Quill, _) => Attack::Splinter,
            (Weapon::Bellows, 0) => Attack::Gust,
            (Weapon::Bellows, _) => Attack::BothBarrels,
            (Weapon::Arclash, 0) => Attack::Wisp,
            (Weapon::Arclash, _) => Attack::Discharge,
            (Weapon::Tumbler, 0) => Attack::Breach,
            (Weapon::Tumbler, _) => Attack::Gravewell,
        }
    }

    /// Seconds between primary shots.
    pub fn primary_interval(self, t: &CombatTuning) -> f32 {
        match self {
            Weapon::Quill => t.quill.interval,
            Weapon::Bellows => t.bellows.interval,
            Weapon::Arclash => t.arclash.zap_interval,
            Weapon::Tumbler => t.tumbler.interval,
        }
    }

    /// Cooldown of an alt mode.
    pub fn alt_cooldown(self, mode: u8, t: &CombatTuning) -> f32 {
        match self.alt(mode) {
            Attack::Brand => t.quill.brand_cooldown,
            Attack::Splinter => t.quill.splinter_cooldown,
            Attack::Gust => t.bellows.gust_cooldown,
            Attack::BothBarrels => t.bellows.both_cooldown,
            Attack::Wisp => t.arclash.wisp_cooldown,
            Attack::Discharge => t.arclash.discharge_cooldown,
            Attack::Breach => t.tumbler.breach_cooldown,
            _ => t.tumbler.gravewell_cooldown,
        }
    }

    /// Does this alt mode charge while held and fire on release?
    pub fn alt_charges(self, mode: u8) -> bool {
        self.alt(mode) == Attack::Discharge
    }
}

/// Weapons, fist and thorn state of one player. Everything is in ticks/integers so client
/// prediction can reproduce it exactly. `Default` = fresh (all ready, full thorns).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeaponState {
    /// Current weapon slot (see [`Weapon`]).
    pub weapon: u8,
    /// Current alt mode (0 = A, 1 = B).
    pub mode: u8,
    /// Buttons of the previous input (edge detection, independent of movement's).
    pub prev_buttons: Buttons,
    /// Ticks until each weapon's primary can fire again.
    pub primary_cooldown: [u16; 4],
    /// Ticks until each weapon's alt fire can fire again.
    pub alt_cooldown: [u16; 4],
    /// Arclash Discharge: ticks the alt button has been held while charging.
    pub charge: u16,
    pub punch_cooldown: u16,
    /// Ticks left in the parry window opened by the last punch.
    pub parry: u16,
    /// Thorns spent and not yet recharged (charges = `thorn.charges - thorns_used`).
    pub thorns_used: u8,
    /// Ticks accumulated towards the next thorn charge.
    pub thorn_recharge: u16,
}

/// Something a player's input fires this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FireIntent {
    Primary(Weapon),
    /// Alt fire; `charge` is the Discharge fraction (0..=1), 0 for other modes.
    Alt {
        weapon: Weapon,
        mode: u8,
        charge: f32,
    },
    Punch,
    ThrowThorn,
}

impl FireIntent {
    pub fn attack(self) -> Option<Attack> {
        match self {
            FireIntent::Primary(w) => Some(w.primary()),
            FireIntent::Alt { weapon, mode, .. } => Some(weapon.alt(mode)),
            FireIntent::Punch => Some(Attack::Fist),
            FireIntent::ThrowThorn => None,
        }
    }
}

impl WeaponState {
    /// Fresh state keeping the selected weapon/mode (respawn).
    pub fn fresh(weapon: u8, mode: u8) -> Self {
        Self {
            weapon,
            mode,
            ..Default::default()
        }
    }

    pub fn current(&self) -> Weapon {
        Weapon::from_slot(self.weapon).unwrap_or(Weapon::Quill)
    }

    /// Thorn charges available.
    pub fn thorns(&self, t: &CombatTuning) -> u32 {
        t.thorn.charges.saturating_sub(self.thorns_used as u32)
    }

    /// Ticks to a full Discharge charge.
    pub fn charge_ticks(t: &CombatTuning, tick_rate: u32) -> u16 {
        secs_to_ticks(t.arclash.discharge_charge_time, tick_rate, 1)
    }

    /// Discharge charge fraction (0..=1) for display.
    pub fn charge_fraction(&self, t: &CombatTuning, tick_rate: u32) -> f32 {
        (self.charge as f32 / Self::charge_ticks(t, tick_rate) as f32).min(1.0)
    }

    pub fn on_death(&mut self) {
        self.charge = 0;
        self.parry = 0;
    }
}

/// Advances one player's weapons by one input and returns what fires. Deterministic: the
/// server and the owning client's prediction run exactly this. `alive == false` only ticks
/// the timers.
pub fn step_weapons(
    s: &mut WeaponState,
    input: &PlayerInput,
    alive: bool,
    t: &CombatTuning,
    tick_rate: u32,
) -> Vec<FireIntent> {
    let mut out = Vec::new();
    let held = input.buttons;
    let prev = std::mem::replace(&mut s.prev_buttons, held);

    for cd in s
        .primary_cooldown
        .iter_mut()
        .chain(s.alt_cooldown.iter_mut())
    {
        *cd = cd.saturating_sub(1);
    }
    s.punch_cooldown = s.punch_cooldown.saturating_sub(1);
    s.parry = s.parry.saturating_sub(1);
    if s.thorns_used > 0 {
        s.thorn_recharge += 1;
        if s.thorn_recharge >= secs_to_ticks(t.thorn.recharge_time, tick_rate, 1) {
            s.thorns_used -= 1;
            s.thorn_recharge = 0;
        }
    } else {
        s.thorn_recharge = 0;
    }
    if !alive {
        s.charge = 0;
        return out;
    }

    // Selection (instant; switching or changing mode cancels a charge).
    let mode = if input.weapon_mode < ALT_MODES {
        input.weapon_mode
    } else {
        0
    };
    if let Some(w) = Weapon::from_slot(input.weapon)
        && (w.slot() != s.weapon || mode != s.mode)
    {
        s.charge = 0;
        s.weapon = w.slot();
    }
    if mode != s.mode {
        s.charge = 0;
        s.mode = mode;
    }
    let w = s.current();
    let wi = w as usize;
    let interval = secs_to_ticks(w.primary_interval(t), tick_rate, 1);

    // Primary: automatic while held.
    if held.contains(Buttons::FIRE) && s.primary_cooldown[wi] == 0 {
        out.push(FireIntent::Primary(w));
        s.primary_cooldown[wi] = interval;
    }

    // Alt fire.
    let alt_held = held.contains(Buttons::ALT_FIRE);
    let alt_released = held.released(prev, Buttons::ALT_FIRE);
    let alt_cd = secs_to_ticks(w.alt_cooldown(s.mode, t), tick_rate, 1);
    if w.alt_charges(s.mode) {
        // Discharge: hold to charge (once off cooldown), fire on release.
        let max = WeaponState::charge_ticks(t, tick_rate);
        if alt_held {
            if s.alt_cooldown[wi] == 0 {
                s.charge = (s.charge + 1).min(max);
            }
        } else {
            if alt_released && s.charge > 0 {
                out.push(FireIntent::Alt {
                    weapon: w,
                    mode: s.mode,
                    charge: (s.charge as f32 / max as f32).min(1.0),
                });
                s.alt_cooldown[wi] = alt_cd;
            }
            s.charge = 0;
        }
    } else if alt_held && s.alt_cooldown[wi] == 0 {
        out.push(FireIntent::Alt {
            weapon: w,
            mode: s.mode,
            charge: 0.0,
        });
        s.alt_cooldown[wi] = alt_cd;
        if w.alt(s.mode) == Attack::BothBarrels {
            // Both barrels at once: the primary has to wait its interval too.
            s.primary_cooldown[wi] = s.primary_cooldown[wi].max(interval);
        }
    }

    // Fist / parry.
    if held.pressed(prev, Buttons::PUNCH) && s.punch_cooldown == 0 {
        out.push(FireIntent::Punch);
        s.punch_cooldown = secs_to_ticks(t.punch_cooldown, tick_rate, 1);
        s.parry = secs_to_ticks(t.parry_window, tick_rate, 1);
    }

    // Thorn.
    if held.pressed(prev, Buttons::THROW) && (s.thorns_used as u32) < t.thorn.charges {
        out.push(FireIntent::ThrowThorn);
        if s.thorns_used == 0 {
            s.thorn_recharge = 0;
        }
        s.thorns_used += 1;
    }
    out
}

/// Deterministic pseudo-random numbers for spread (splitmix64 of a seed).
pub(crate) struct SpreadRng(u64);

impl SpreadRng {
    pub fn new(tick: u32, player: PlayerId, salt: u32) -> Self {
        Self(
            ((tick as u64) << 32) ^ ((player.0 as u64) << 24) ^ salt as u64 ^ 0x9E37_79B9_7F4A_7C15,
        )
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Orthonormal right/up vectors for an aim direction.
pub fn aim_basis(dir: Vec3) -> (Vec3, Vec3) {
    let right = dir.cross(Vec3::Y).normalize_or(Vec3::X);
    let up = right.cross(dir).normalize_or(Vec3::Y);
    (right, up)
}

/// A random direction within a cone of full aperture `cone_degrees` around `dir` (uniform
/// over the disc).
pub(crate) fn spread_dir(dir: Vec3, cone_degrees: f32, rng: &mut SpreadRng) -> Vec3 {
    let half = (cone_degrees * 0.5).to_radians();
    let r = rng.next_f32().sqrt() * half.tan();
    let a = rng.next_f32() * std::f32::consts::TAU;
    let (right, up) = aim_basis(dir);
    (dir + right * (r * a.cos()) + up * (r * a.sin())).normalize_or(dir)
}

/// Salt of the splinter spread (distinct from the Bellows pellets of the same input).
pub(crate) const SPLINTER_SALT: u32 = 0x5911_7E00;

impl SimWorld {
    /// Executes one fire intent of player `id` (server). `input` is the input that fired it
    /// (view angles, lag-compensation view time).
    pub fn resolve_intent(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        intent: FireIntent,
    ) {
        let Some(p) = self.players.get(&id) else {
            return;
        };
        let eye = p.movement.eye_position(&t.movement);
        let body_velocity = p.movement.velocity;
        let aim = input.look_dir();
        let owner = Owner::Player(id);
        let c = &t.combat;
        let tick = self.tick;
        let attack = match intent {
            FireIntent::Punch => {
                self.punch(t, history, id, input);
                return;
            }
            FireIntent::ThrowThorn => {
                // Thrown along the aim, inheriting part of the thrower's velocity.
                let vel = aim * c.thorn.throw_speed + body_velocity * c.thorn.inherit_velocity;
                self.spawn_thorn(t, id, eye + aim * 0.5, vel);
                return;
            }
            FireIntent::Primary(w) => w.primary(),
            FireIntent::Alt { weapon, mode, .. } => weapon.alt(mode),
        };
        match attack {
            Attack::Quill => {
                let shot = ShotSpec::new(eye, aim, c.hitscan_range, c.quill.damage, attack)
                    .ricochet(true);
                self.fire_hitscan_group(t, history, id, input, attack, &[shot]);
            }
            Attack::Brand => {
                let shot = ShotSpec::new(eye, aim, c.hitscan_range, c.quill.brand_damage, attack)
                    .ricochet(true);
                let (_, results) = self.fire_hitscan_group(t, history, id, input, attack, &[shot]);
                // Whatever the (last leg of the) shot hit is branded, if it survived.
                for impact in results.iter().filter_map(|r| r.impact) {
                    if let Some((target, _)) = impact.target {
                        self.brand_target(t, id, target, impact.point);
                    }
                }
            }
            Attack::Splinter => {
                let shot =
                    ShotSpec::new(eye, aim, c.hitscan_range, c.quill.splinter_damage, attack)
                        .ricochet(true);
                let (view, results) =
                    self.fire_hitscan_group(t, history, id, input, attack, &[shot]);
                let mut rng = SpreadRng::new(tick, id, input.seq ^ SPLINTER_SALT);
                for impact in results.iter().filter_map(|r| r.impact) {
                    self.splinter_burst(t, id, &view.hitboxes, impact, &mut rng);
                }
            }
            Attack::Blast | Attack::BothBarrels => {
                let b = &c.bellows;
                let (count, cone) = if attack == Attack::Blast {
                    (b.pellets, b.cone_degrees)
                } else {
                    (b.both_pellets, b.both_cone_degrees)
                };
                let mut rng = SpreadRng::new(tick, id, input.seq);
                let shots: Vec<ShotSpec> = (0..count.max(1))
                    .map(|_| {
                        let dir = spread_dir(aim, cone, &mut rng);
                        ShotSpec::new(eye, dir, b.pellet_range, b.pellet_damage, attack)
                            .knockback(b.pellet_knockback)
                    })
                    .collect();
                self.fire_hitscan_group(t, history, id, input, attack, &shots);
                if attack == Attack::BothBarrels {
                    // Recoil (server-side impulse; the client reconciles): blast-jump.
                    self.apply_impulse(id, -aim * b.both_recoil);
                }
            }
            Attack::Gust => self.gust(t, history, id, input),
            Attack::Arc => {
                let a = &c.arclash;
                let shot = ShotSpec::new(eye, aim, a.zap_range, a.zap_damage, attack);
                let (view, results) =
                    self.fire_hitscan_group(t, history, id, input, attack, &[shot]);
                for impact in results.iter().filter_map(|r| r.impact) {
                    if let Some((target, _)) = impact.target {
                        self.arc_chain(t, owner, &view.hitboxes, target, impact.point, a.zap_damage);
                    }
                }
            }
            Attack::Wisp => {
                let start = self.safe_launch_point(eye, eye + aim * 0.6);
                self.spawn_projectile(
                    t,
                    ProjectileKind::Wisp,
                    owner,
                    start,
                    aim * c.arclash.wisp_speed,
                );
                self.push_fired(id, attack, eye);
            }
            Attack::Discharge => {
                let charge = match intent {
                    FireIntent::Alt { charge, .. } => charge,
                    _ => 1.0,
                };
                self.discharge(t, history, id, input, charge);
            }
            Attack::Tumble => {
                self.spawn_projectile(
                    t,
                    ProjectileKind::Bomb,
                    owner,
                    eye,
                    aim * c.tumbler.bomb_speed,
                );
                self.push_fired(id, attack, eye);
            }
            Attack::Breach => self.breach(t, history, id, input),
            Attack::Gravewell => {
                // Lob: rotate the aim upwards around its right axis.
                let tb = &c.tumbler;
                let (right, _) = aim_basis(aim);
                let lob =
                    Quat::from_axis_angle(right, tb.gravewell_angle_degrees.to_radians()) * aim;
                // Never lob below the aim when aiming straight down.
                let dir = if lob.y < aim.y { aim } else { lob };
                self.spawn_projectile(
                    t,
                    ProjectileKind::Gravewell,
                    owner,
                    eye,
                    dir * tb.gravewell_speed,
                );
                self.push_fired(id, attack, eye);
            }
            _ => {}
        }
    }

    /// Projectile weapons: "shooter fired" event for remote muzzle flashes / sounds.
    pub(crate) fn push_fired(&mut self, id: PlayerId, attack: Attack, origin: Vec3) {
        self.events.push(SimEvent::Shot(super::ShotEvent {
            shooter: id,
            attack,
            origin,
            ends: Vec::new(),
        }));
    }

    /// `want` unless the level blocks the way from `eye` (then `eye`): don't start inside a wall
    /// right in front of the shooter.
    pub(crate) fn safe_launch_point(&self, eye: Vec3, want: Vec3) -> Vec3 {
        if self
            .collision
            .world
            .line_blocked(eye, want, engine::physics::MASK_SHOTS)
        {
            eye
        } else {
            want
        }
    }

    /// Flags every hitscan hit carries.
    pub(crate) fn hitscan_flags(_attack: Attack) -> DamageFlags {
        DamageFlags::HITSCAN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(buttons: Buttons, weapon: u8, mode: u8) -> PlayerInput {
        PlayerInput {
            buttons,
            weapon,
            weapon_mode: mode,
            ..Default::default()
        }
    }

    #[test]
    fn primary_respects_interval_and_switching_is_instant() {
        let t = CombatTuning::default();
        let mut s = WeaponState::default();
        let mut shots = 0;
        for _ in 0..60 {
            shots += step_weapons(&mut s, &input(Buttons::FIRE, 0, 0), true, &t, 60).len();
        }
        // 0.28 s interval (17 ticks): shots at ticks 0, 17, 34, 51.
        assert_eq!(shots, 4);
        // Switch to the Bellows: it fires immediately (own cooldown).
        let out = step_weapons(&mut s, &input(Buttons::FIRE, 1, 0), true, &t, 60);
        assert_eq!(out, vec![FireIntent::Primary(Weapon::Bellows)]);
        // The Arclash zaps every 0.1 s while held.
        let zaps: usize = (0..60)
            .map(|_| step_weapons(&mut s, &input(Buttons::FIRE, 2, 0), true, &t, 60).len())
            .sum();
        assert_eq!(zaps, 10);
    }

    #[test]
    fn discharge_charges_and_fires_on_release() {
        let t = CombatTuning::default();
        let mut s = WeaponState::default();
        for _ in 0..30 {
            assert!(step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 1), true, &t, 60).is_empty());
        }
        assert_eq!(s.charge, 30);
        let out = step_weapons(&mut s, &input(Buttons::NONE, 2, 1), true, &t, 60);
        match out.as_slice() {
            [FireIntent::Alt { charge, .. }] => assert!((charge - 0.5).abs() < 1e-6, "{charge}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(s.charge, 0);
        // On cooldown (2 s): holding doesn't charge.
        for _ in 0..10 {
            step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 1), true, &t, 60);
        }
        assert_eq!(s.charge, 0);
        // The charge caps at 1.0 s.
        let mut s = WeaponState::default();
        for _ in 0..200 {
            step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 1), true, &t, 60);
        }
        assert_eq!(s.charge, 60);
        assert_eq!(s.charge_fraction(&t, 60), 1.0);
        // Switching mode or weapon cancels it without firing.
        let out = step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 0), true, &t, 60);
        assert_eq!(s.charge, 0);
        assert!(out.iter().all(|i| !matches!(i, FireIntent::Alt { charge, .. } if *charge > 0.0)));
        for _ in 0..10 {
            step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 1), true, &t, 60);
        }
        step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 1), true, &t, 60);
        assert_eq!(s.charge, 0);
    }

    #[test]
    fn both_barrels_also_holds_the_primary() {
        let t = CombatTuning::default();
        let mut s = WeaponState::default();
        let out = step_weapons(&mut s, &input(Buttons::ALT_FIRE, 1, 1), true, &t, 60);
        assert_eq!(
            out,
            vec![FireIntent::Alt {
                weapon: Weapon::Bellows,
                mode: 1,
                charge: 0.0
            }]
        );
        assert_eq!(s.primary_cooldown[1], secs_to_ticks(t.bellows.interval, 60, 1));
        assert!(step_weapons(&mut s, &input(Buttons::FIRE, 1, 1), true, &t, 60).is_empty());
        // Gust doesn't.
        let mut s = WeaponState::default();
        step_weapons(&mut s, &input(Buttons::ALT_FIRE, 1, 0), true, &t, 60);
        assert_eq!(s.primary_cooldown[1], 0);
    }

    #[test]
    fn punch_opens_parry_window_and_thorns_recharge() {
        let t = CombatTuning::default();
        let mut s = WeaponState::default();
        let out = step_weapons(
            &mut s,
            &input(Buttons::PUNCH | Buttons::THROW, 0, 0),
            true,
            &t,
            60,
        );
        assert_eq!(out, vec![FireIntent::Punch, FireIntent::ThrowThorn]);
        assert_eq!(s.parry, 12);
        assert_eq!(s.thorns(&t), 2);
        // Three charges: the fourth press throws nothing.
        for _ in 0..2 {
            step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
            step_weapons(&mut s, &input(Buttons::THROW, 0, 0), true, &t, 60);
        }
        assert_eq!(s.thorns(&t), 0);
        step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
        assert!(step_weapons(&mut s, &input(Buttons::THROW, 0, 0), true, &t, 60).is_empty());
        // One per 3 s.
        for _ in 0..180 {
            step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
        }
        assert_eq!(s.thorns(&t), 1);
        assert_eq!(s.parry, 0);
        // Dead players fire nothing.
        assert!(
            step_weapons(
                &mut s,
                &input(Buttons::FIRE | Buttons::PUNCH, 0, 0),
                false,
                &t,
                60
            )
            .is_empty()
        );
    }

    #[test]
    fn spread_stays_in_cone() {
        let mut rng = SpreadRng::new(5, PlayerId(1), 3);
        for _ in 0..200 {
            let d = spread_dir(Vec3::NEG_Z, 8.0, &mut rng);
            let angle = d.angle_between(Vec3::NEG_Z).to_degrees();
            assert!(angle <= 4.0 + 1e-3, "{angle}");
        }
    }
}
