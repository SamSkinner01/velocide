//! Weapons: the per-player weapon state machine and what each fire mode does.
//!
//! Two halves:
//!
//! * [`step_weapons`] — a **pure, deterministic** function of `(WeaponState, PlayerInput,
//!   alive, tuning)` that decides *what* fires this input (cooldowns, Overcharge charging,
//!   sticky throw/detonate, punch, prism charges) and returns [`FireIntent`]s. All timers are
//!   integer ticks, so the owning client can run the same function on its predicted inputs
//!   (muzzle flash, sound and viewmodel recoil without waiting a round trip) and compare its
//!   prediction bit-exactly with the authoritative `OwnState::combat.weapons`.
//! * [`SimWorld::resolve_intent`] (server) — turns an intent into hitscan traces,
//!   projectiles, a punch/parry or a prism throw.
//!
//! Slots: `PlayerInput::weapon` 0 = Lancet, 1 = Thresher, 2 = Riveter, 3 = Cinderbox (other
//! values keep the current weapon). `PlayerInput::weapon_mode` 0 = alt A, 1 = alt B (others →
//! alt A). `FIRE` (held, automatic) = primary; `ALT_FIRE` = the selected alt mode; `PUNCH`
//! (press) = fist/parry; `THROW` (press) = prism.

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
    Lancet = 0,
    Thresher = 1,
    Riveter = 2,
    Cinderbox = 3,
}

/// Number of alt-fire modes per weapon.
pub const ALT_MODES: u8 = 2;

impl Weapon {
    pub const ALL: [Weapon; 4] = [
        Weapon::Lancet,
        Weapon::Thresher,
        Weapon::Riveter,
        Weapon::Cinderbox,
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
            Weapon::Lancet => "Lancet",
            Weapon::Thresher => "Thresher",
            Weapon::Riveter => "Riveter",
            Weapon::Cinderbox => "Cinderbox",
        }
    }

    pub fn primary(self) -> Attack {
        match self {
            Weapon::Lancet => Attack::Lancet,
            Weapon::Thresher => Attack::Pellet,
            Weapon::Riveter => Attack::Rivet,
            Weapon::Cinderbox => Attack::Rocket,
        }
    }

    /// Alt mode 0 (A) or 1 (B).
    pub fn alt(self, mode: u8) -> Attack {
        match (self, mode) {
            (Weapon::Lancet, 0) => Attack::Overcharge,
            (Weapon::Lancet, _) => Attack::Fan,
            (Weapon::Thresher, 0) => Attack::Slug,
            (Weapon::Thresher, _) => Attack::Flak,
            (Weapon::Riveter, 0) => Attack::Sawblade,
            (Weapon::Riveter, _) => Attack::Volley,
            (Weapon::Cinderbox, 0) => Attack::Sticky,
            (Weapon::Cinderbox, _) => Attack::Mortar,
        }
    }

    /// Seconds between primary shots.
    pub fn primary_interval(self, t: &CombatTuning) -> f32 {
        match self {
            Weapon::Lancet => t.lancet.interval,
            Weapon::Thresher => t.thresher.interval,
            Weapon::Riveter => 1.0 / t.riveter.rate.max(0.1),
            Weapon::Cinderbox => t.cinderbox.rocket_interval,
        }
    }

    /// Cooldown of an alt mode.
    pub fn alt_cooldown(self, mode: u8, t: &CombatTuning) -> f32 {
        match self.alt(mode) {
            Attack::Overcharge => t.lancet.overcharge_cooldown,
            Attack::Fan => t.lancet.fan_cooldown,
            Attack::Slug => t.thresher.slug_cooldown,
            Attack::Flak => t.thresher.flak_cooldown,
            Attack::Sawblade => t.riveter.saw_cooldown,
            Attack::Volley => t.riveter.volley_cooldown,
            Attack::Sticky => t.cinderbox.sticky_cooldown,
            _ => t.cinderbox.mortar_cooldown,
        }
    }
}

/// Weapons, fist and prism state of one player. Everything is in ticks/integers so client
/// prediction can reproduce it exactly. `Default` = fresh (all ready, full prisms).
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
    /// Lancet Overcharge: ticks the alt button has been held while charging.
    pub charge: u16,
    pub punch_cooldown: u16,
    /// Ticks left in the parry window opened by the last punch.
    pub parry: u16,
    /// Prisms spent and not yet recharged (charges = `prism.charges - prisms_used`).
    pub prisms_used: u8,
    /// Ticks accumulated towards the next prism charge.
    pub prism_recharge: u16,
    /// Live stickies (server-authoritative; predicted +1 per throw, 0 on detonate).
    pub stickies: u8,
    /// Ticks since the last sticky throw (saturating).
    pub sticky_since: u16,
}

/// Something a player's input fires this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FireIntent {
    Primary(Weapon),
    /// Alt fire; `charge` is the Overcharge fraction (0..=1), 0 for other modes.
    Alt {
        weapon: Weapon,
        mode: u8,
        charge: f32,
    },
    DetonateStickies,
    Punch,
    ThrowPrism,
}

impl FireIntent {
    pub fn attack(self) -> Option<Attack> {
        match self {
            FireIntent::Primary(w) => Some(w.primary()),
            FireIntent::Alt { weapon, mode, .. } => Some(weapon.alt(mode)),
            FireIntent::Punch => Some(Attack::Fist),
            _ => None,
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
        Weapon::from_slot(self.weapon).unwrap_or(Weapon::Lancet)
    }

    /// Prism charges available.
    pub fn prisms(&self, t: &CombatTuning) -> u32 {
        t.prism.charges.saturating_sub(self.prisms_used as u32)
    }

    /// Overcharge fraction (0..=1) for display.
    pub fn charge_fraction(&self, t: &CombatTuning, tick_rate: u32) -> f32 {
        let max = secs_to_ticks(t.lancet.overcharge_time, tick_rate, 1);
        (self.charge as f32 / max as f32).min(1.0)
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
    s.sticky_since = s.sticky_since.saturating_add(1);
    if s.prisms_used > 0 {
        s.prism_recharge += 1;
        if s.prism_recharge >= secs_to_ticks(t.prism.recharge_time, tick_rate, 1) {
            s.prisms_used -= 1;
            s.prism_recharge = 0;
        }
    } else {
        s.prism_recharge = 0;
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

    // Primary: automatic while held.
    if held.contains(Buttons::FIRE) && s.primary_cooldown[wi] == 0 {
        out.push(FireIntent::Primary(w));
        s.primary_cooldown[wi] = secs_to_ticks(w.primary_interval(t), tick_rate, 1);
    }

    // Alt fire.
    let alt_held = held.contains(Buttons::ALT_FIRE);
    let alt_pressed = held.pressed(prev, Buttons::ALT_FIRE);
    let alt_released = held.released(prev, Buttons::ALT_FIRE);
    let alt_cd = secs_to_ticks(w.alt_cooldown(s.mode, t), tick_rate, 1);
    match w.alt(s.mode) {
        Attack::Overcharge => {
            let max = secs_to_ticks(t.lancet.overcharge_time, tick_rate, 1);
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
        }
        Attack::Sticky => {
            if alt_pressed {
                let chain =
                    s.sticky_since <= secs_to_ticks(t.cinderbox.sticky_chain_window, tick_rate, 0);
                let room = (s.stickies as u32) < t.cinderbox.sticky_max;
                if s.stickies > 0 && !(chain && room) {
                    out.push(FireIntent::DetonateStickies);
                    s.stickies = 0;
                } else if room && s.alt_cooldown[wi] == 0 {
                    out.push(FireIntent::Alt {
                        weapon: w,
                        mode: s.mode,
                        charge: 0.0,
                    });
                    s.stickies += 1;
                    s.sticky_since = 0;
                    s.alt_cooldown[wi] = alt_cd;
                }
            }
        }
        _ => {
            if alt_held && s.alt_cooldown[wi] == 0 {
                out.push(FireIntent::Alt {
                    weapon: w,
                    mode: s.mode,
                    charge: 0.0,
                });
                s.alt_cooldown[wi] = alt_cd;
            }
        }
    }

    // Fist / parry.
    if held.pressed(prev, Buttons::PUNCH) && s.punch_cooldown == 0 {
        out.push(FireIntent::Punch);
        s.punch_cooldown = secs_to_ticks(t.punch_cooldown, tick_rate, 1);
        s.parry = secs_to_ticks(t.parry_window, tick_rate, 1);
    }

    // Prism.
    if held.pressed(prev, Buttons::THROW) && (s.prisms_used as u32) < t.prism.charges {
        out.push(FireIntent::ThrowPrism);
        if s.prisms_used == 0 {
            s.prism_recharge = 0;
        }
        s.prisms_used += 1;
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
        match intent {
            FireIntent::Primary(w) | FireIntent::Alt { weapon: w, .. } => {
                let attack = intent.attack().unwrap_or(w.primary());
                match attack {
                    Attack::Lancet => {
                        let shot =
                            ShotSpec::new(eye, aim, c.hitscan_range, c.lancet.damage, attack)
                                .ricochet(true);
                        self.fire_hitscan_group(t, history, id, input, attack, &[shot]);
                    }
                    Attack::Overcharge => {
                        let charge = match intent {
                            FireIntent::Alt { charge, .. } => charge,
                            _ => 1.0,
                        };
                        let dmg = c.lancet.overcharge_min_damage
                            + (c.lancet.overcharge_max_damage - c.lancet.overcharge_min_damage)
                                * charge.clamp(0.0, 1.0);
                        let shot = ShotSpec::new(eye, aim, c.hitscan_range, dmg, attack)
                            .ricochet(true)
                            .pierce(c.lancet.overcharge_pierce.max(1) - 1);
                        self.fire_hitscan_group(t, history, id, input, attack, &[shot]);
                    }
                    Attack::Fan => {
                        let n = c.lancet.fan_shots.max(1);
                        let shots: Vec<ShotSpec> = (0..n)
                            .map(|i| {
                                let f = if n == 1 {
                                    0.0
                                } else {
                                    i as f32 / (n - 1) as f32 * 2.0 - 1.0
                                };
                                let yaw = (c.lancet.fan_spread_degrees * f).to_radians();
                                let dir = Quat::from_rotation_y(yaw) * aim;
                                ShotSpec::new(
                                    eye,
                                    dir,
                                    c.hitscan_range,
                                    c.lancet.fan_damage,
                                    attack,
                                )
                                .ricochet(true)
                            })
                            .collect();
                        self.fire_hitscan_group(t, history, id, input, attack, &shots);
                    }
                    Attack::Pellet => {
                        let mut rng = SpreadRng::new(tick, id, input.seq);
                        let th = &c.thresher;
                        let shots: Vec<ShotSpec> = (0..th.pellets.max(1))
                            .map(|_| {
                                let dir = spread_dir(aim, th.cone_degrees, &mut rng);
                                ShotSpec::new(eye, dir, th.pellet_range, th.pellet_damage, attack)
                                    .knockback(th.pellet_knockback)
                            })
                            .collect();
                        self.fire_hitscan_group(t, history, id, input, attack, &shots);
                    }
                    Attack::Slug => {
                        let th = &c.thresher;
                        let shot = ShotSpec::new(eye, aim, c.hitscan_range, th.slug_damage, attack)
                            .knockback(th.slug_knockback)
                            .ricochet(true);
                        self.fire_hitscan_group(t, history, id, input, attack, &[shot]);
                        // Recoil (server-side impulse; the client reconciles).
                        self.apply_impulse(id, -aim * th.slug_recoil);
                    }
                    Attack::Flak => {
                        let th = &c.thresher;
                        self.spawn_projectile(
                            t,
                            ProjectileKind::Flak,
                            owner,
                            eye,
                            aim * th.flak_speed,
                        );
                        self.push_fired(id, attack, eye);
                    }
                    Attack::Rivet => {
                        let mut rng = SpreadRng::new(tick, id, input.seq);
                        let dir = spread_dir(aim, c.riveter.spread_degrees, &mut rng);
                        self.spawn_projectile(
                            t,
                            ProjectileKind::Rivet,
                            owner,
                            eye,
                            dir * c.riveter.rivet_speed,
                        );
                        self.push_fired(id, attack, eye);
                    }
                    Attack::Sawblade => {
                        self.spawn_projectile(
                            t,
                            ProjectileKind::Sawblade,
                            owner,
                            eye,
                            aim * c.riveter.saw_speed,
                        );
                        self.push_fired(id, attack, eye);
                    }
                    Attack::Volley => {
                        let r = &c.riveter;
                        let n = r.volley_count.max(1);
                        let (right, up) = aim_basis(aim);
                        let off = r.volley_ring_degrees.to_radians().tan();
                        for i in 0..n {
                            let a = i as f32 / n as f32 * std::f32::consts::TAU;
                            let dir =
                                (aim + (right * a.cos() + up * a.sin()) * off).normalize_or(aim);
                            let pid = self.spawn_projectile(
                                t,
                                ProjectileKind::Rivet,
                                owner,
                                eye,
                                dir * r.rivet_speed,
                            );
                            if let Some(proj) = pid.and_then(|pid| self.projectiles.get_mut(&pid)) {
                                proj.attack = Attack::Volley;
                                proj.damage = r.volley_damage;
                            }
                        }
                        self.push_fired(id, attack, eye);
                    }
                    Attack::Rocket => {
                        self.spawn_projectile(
                            t,
                            ProjectileKind::Rocket,
                            owner,
                            eye,
                            aim * c.cinderbox.rocket_speed,
                        );
                        self.push_fired(id, attack, eye);
                    }
                    Attack::Sticky => {
                        self.spawn_projectile(
                            t,
                            ProjectileKind::Sticky,
                            owner,
                            eye,
                            aim * c.cinderbox.sticky_speed,
                        );
                        self.push_fired(id, attack, eye);
                    }
                    Attack::Mortar => {
                        // Lob: rotate the aim upwards around its right axis.
                        let (right, _) = aim_basis(aim);
                        let lob = Quat::from_axis_angle(
                            right,
                            c.cinderbox.mortar_angle_degrees.to_radians(),
                        ) * aim;
                        // Never lob below horizontal when aiming down hard.
                        let dir = if lob.y < aim.y { aim } else { lob };
                        self.spawn_projectile(
                            t,
                            ProjectileKind::Mortar,
                            owner,
                            eye,
                            dir * c.cinderbox.mortar_speed,
                        );
                        self.push_fired(id, attack, eye);
                    }
                    _ => {}
                }
            }
            FireIntent::DetonateStickies => {
                self.detonate_stickies(t, id);
            }
            FireIntent::Punch => {
                self.punch(t, history, id, input);
            }
            FireIntent::ThrowPrism => {
                // Thrown forward and up, inheriting half the thrower's velocity.
                let vel = aim * c.prism.throw_speed + Vec3::Y * c.prism.throw_up_speed;
                self.spawn_prism(t, id, eye + aim * 0.5, vel + body_velocity * 0.5);
            }
        }
    }

    /// Projectile weapons: "shooter fired" event for remote muzzle flashes / sounds.
    fn push_fired(&mut self, id: PlayerId, attack: Attack, origin: Vec3) {
        self.events.push(SimEvent::Shot(super::ShotEvent {
            shooter: id,
            attack,
            origin,
            ends: Vec::new(),
        }));
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
        // 0.3 s interval: shots at ticks 0, 18, 36, 54.
        assert_eq!(shots, 4);
        // Switch to the Thresher: it fires immediately (own cooldown).
        let out = step_weapons(&mut s, &input(Buttons::FIRE, 1, 0), true, &t, 60);
        assert_eq!(out, vec![FireIntent::Primary(Weapon::Thresher)]);
    }

    #[test]
    fn overcharge_charges_and_fires_on_release() {
        let t = CombatTuning::default();
        let mut s = WeaponState::default();
        for _ in 0..24 {
            assert!(step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 0), true, &t, 60).is_empty());
        }
        let out = step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
        match out.as_slice() {
            [FireIntent::Alt { charge, .. }] => assert!((charge - 0.5).abs() < 1e-6, "{charge}"),
            other => panic!("{other:?}"),
        }
        // Switching weapons cancels a charge.
        for _ in 0..10 {
            step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 0), true, &t, 60);
        }
        step_weapons(&mut s, &input(Buttons::ALT_FIRE, 1, 0), true, &t, 60);
        assert_eq!(s.charge, 0);
    }

    #[test]
    fn stickies_chain_then_detonate() {
        let t = CombatTuning::default();
        let mut s = WeaponState::default();
        let tap = |s: &mut WeaponState, n: u32| {
            let mut all = Vec::new();
            all.extend(step_weapons(
                s,
                &input(Buttons::ALT_FIRE, 3, 0),
                true,
                &t,
                60,
            ));
            all.extend(step_weapons(s, &input(Buttons::NONE, 3, 0), true, &t, 60));
            for _ in 0..n {
                all.extend(step_weapons(s, &input(Buttons::NONE, 3, 0), true, &t, 60));
            }
            all
        };
        // Three quick taps: three throws (cooldown 12 ticks, chain window 24 ticks).
        for _ in 0..3 {
            let out = tap(&mut s, 12);
            assert!(
                matches!(out.as_slice(), [FireIntent::Alt { .. }]),
                "{out:?}"
            );
        }
        assert_eq!(s.stickies, 3);
        // At the cap, the next press detonates.
        let out = tap(&mut s, 0);
        assert_eq!(out, vec![FireIntent::DetonateStickies]);
        assert_eq!(s.stickies, 0);
        // One sticky, then a late press: detonate.
        tap(&mut s, 40);
        let out = tap(&mut s, 0);
        assert_eq!(out, vec![FireIntent::DetonateStickies]);
    }

    #[test]
    fn punch_opens_parry_window_and_prisms_recharge() {
        let t = CombatTuning::default();
        let mut s = WeaponState::default();
        let out = step_weapons(
            &mut s,
            &input(Buttons::PUNCH | Buttons::THROW, 0, 0),
            true,
            &t,
            60,
        );
        assert_eq!(out, vec![FireIntent::Punch, FireIntent::ThrowPrism]);
        assert_eq!(s.parry, 12);
        assert_eq!(s.prisms(&t), 3);
        for _ in 0..150 {
            step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
        }
        assert_eq!(s.prisms(&t), 4);
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
            let d = spread_dir(Vec3::NEG_Z, 7.0, &mut rng);
            let angle = d.angle_between(Vec3::NEG_Z).to_degrees();
            assert!(angle <= 3.5 + 1e-3, "{angle}");
        }
    }
}
