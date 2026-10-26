//! The third roster's mechanics on top of the state machine (`ai.rs`, docs/enemies.md):
//!
//! * **Toll** (the Bellwether): at the strike every other Mould within `toll.radius` that can
//!   be frenzied ([`Enemy::frenzies`]) is **frenzied** for `toll.duration` (its [`Frenzy`]
//!   multipliers stack on the difficulty scale and the elite bonus, [`SimWorld::fury`]).
//!   Damage to the bell (`interrupt`) during the long wind-up cancels the toll and staggers
//!   the ringer ([`interrupt`]).
//! * **Latch** (the Snuffer): a pounce whose first victim it reaches (not dashing, not
//!   protected, not already carrying a Snuffer) gets the impact and is **latched**:
//!   `AiState::Latched`, the Snuffer clings to their chest (moved with them every tick, no
//!   collision), smothers the wick (`latch.damage` per second, all of it hard damage) and slows
//!   them to `latch.slow` of their speeds through the movement state's predicted slow
//!   (`MoveState::slow_time` / `slow_scale`, the arsenal's Pinned slow: the owning client
//!   predicts it). A punch by the victim throws it off (counter damage, stun); a dash shakes it
//!   off (stun); a Pinning Arrow pins it off; it lets go when the victim dies / becomes
//!   untargetable, after `max_time`, or when the world freezes.
//! * **Pour** (the Smelter): the slag line is laid when the wind-up starts ([`SimWorld::
//!   lay_slag_line`], `hazard.rs`) and ignites on the strike tick; a cancelled pour removes it.
//! * **Training dummies** (the Effigy): no AI at all; they flinch when hit and heal back to
//!   full `regen_delay` seconds after the last hit.

use glam::Vec3;

use super::{AiState, AttackDef, AttackKind, Enemy, Interrupt, Latch, Pour};
use crate::input::look_dir;
use crate::sim::combat::{Attack, DamageFlags, DamageInfo, Owner, secs_to_ticks};
use crate::sim::{HitTarget, PlayerId, SimEvent, SimWorld};

/// The latch refreshes its victim's slow for this long every tick (it eases off this quickly
/// once it lets go).
const LATCH_SLOW_HOLD: f32 = 0.25;
use crate::tuning::Tuning;

/// A training dummy's flinch (the `Recover` state it shows after a hit), seconds.
pub(crate) const DUMMY_FLINCH: f32 = 0.35;

/// The interrupt weak point took enough damage: the attack is cancelled, the enemy staggers
/// (stunned) and the attack goes on cooldown as if it had been used.
pub(crate) fn interrupt(e: &mut Enemy, int: &Interrupt, rate: u32) {
    let cooldown = e.current_attack().map_or(0.0, |a| a.cooldown + a.recover);
    if let Some(cd) = e.cooldowns.get_mut(e.attack) {
        *cd = secs_to_ticks(cooldown, rate, 0) as u32;
    }
    e.interrupt_damage = 0.0;
    e.stun(secs_to_ticks(int.stun, rate, 1));
}

impl SimWorld {
    /// Allies a toll from `e` would frenzy now (other living Moulds within `radius`).
    pub(crate) fn toll_allies(&self, e: &Enemy, radius: f32) -> usize {
        let center = e.center();
        self.enemies
            .values()
            .filter(|o| o.id != e.id && o.frenzies() && o.center().distance(center) <= radius)
            .count()
    }

    /// The toll rings: every ally in reach is frenzied (refreshing a running frenzy).
    pub(crate) fn toll(&mut self, e: &Enemy, a: &AttackDef) {
        let Some(toll) = &a.toll else {
            return;
        };
        let until = self.tick + secs_to_ticks(toll.duration, self.tick_rate, 1) as u32 + 1;
        let center = e.center();
        for o in self.enemies.values_mut() {
            if o.id != e.id && o.frenzies() && o.center().distance(center) <= toll.radius {
                o.frenzy_until = o.frenzy_until.max(until);
                o.frenzy = toll.frenzy;
            }
        }
    }

    /// Lays the pour's slag line at the start of the wind-up, aimed at where the target is
    /// heading. Returns false (nothing laid, don't pour) when there isn't room for a line.
    pub(crate) fn begin_pour(
        &mut self,
        e: &mut Enemy,
        attack: usize,
        pour: &Pour,
        target: Vec3,
        target_velocity: Vec3,
    ) -> bool {
        let Some(a) = e.def.attacks.get(attack).cloned() else {
            return false;
        };
        let feet = e.feet();
        let goal = target + Vec3::new(target_velocity.x, 0.0, target_velocity.z) * pour.lead;
        let to = Vec3::new(goal.x - feet.x, 0.0, goal.z - feet.z);
        let dir = to.normalize_or(e.forward());
        let from = feet + dir * (e.radius() + pour.start);
        let len = (to.length() - e.radius() - pour.start + pour.overshoot).clamp(0.0, pour.length);
        let windup = secs_to_ticks(a.windup, self.tick_rate, 1) as u32;
        let fury = self.fury(e);
        let attack_id = Attack::Enemy {
            kind: e.def.net_kind,
            attack: attack.min(255) as u8,
        };
        let line = self.lay_slag_line(
            e.id,
            attack_id,
            from,
            dir,
            len,
            pour.width,
            self.tick + windup,
            pour.lifetime,
            a.damage * pour.interval * fury.damage,
            pour.interval,
        );
        match line {
            Some(id) => {
                e.pour_line = Some(id);
                // It pours along the line: face it now (the wind-up doesn't track).
                e.yaw = (-dir.x).atan2(-dir.z);
                true
            }
            None => false,
        }
    }

    /// A pour that didn't reach its strike (stunned, killed...) takes its unlit line with it.
    pub(crate) fn settle_pour(&mut self, e: &mut Enemy) {
        let pouring = e.state == AiState::Windup
            && e.current_attack()
                .is_some_and(|a| a.kind == AttackKind::Pour);
        if !pouring && let Some(line) = e.pour_line.take() {
            self.drop_unlit_line(line);
        }
    }

    /// Is some (other) enemy latched onto `player`?
    pub(crate) fn carries_latch(&self, player: PlayerId) -> bool {
        self.enemies.values().any(|o| o.latched == Some(player))
    }

    /// The first player a latching lunge reached that it may cling to (not dashing / on
    /// i-frames, not spawn-protected, not already carrying a latch).
    pub(crate) fn latch_victim(&self, reached: &[PlayerId]) -> Option<PlayerId> {
        reached.iter().copied().find(|id| {
            self.players.get(id).is_some_and(|p| {
                !p.movement.is_invulnerable() && p.combat.protection == 0 && !p.combat.dead
            }) && !self.carries_latch(*id)
        })
    }

    /// Clings to `victim`: the impact hit, then `AiState::Latched` for up to `max_time`.
    pub(crate) fn latch_onto(
        &mut self,
        t: &Tuning,
        e: &mut Enemy,
        a: &AttackDef,
        latch: &Latch,
        victim: PlayerId,
    ) {
        let fury = self.fury(e);
        let attack = Attack::Enemy {
            kind: e.def.net_kind,
            attack: e.attack.min(255) as u8,
        };
        if let Some(p) = self.players.get(&victim) {
            let chest = p.movement.position + Vec3::Y * 1.1;
            let dir = (chest - e.center()).normalize_or(e.forward());
            let info = DamageInfo::new(a.damage * fury.damage, Owner::Enemy(e.id), attack)
                .at(chest, dir)
                .flags(DamageFlags::MELEE);
            self.apply_damage(t, HitTarget::Player(victim), info);
        }
        e.latched = Some(victim);
        e.target = Some(victim);
        e.latch_tick_in = secs_to_ticks(latch.interval, self.tick_rate, 1);
        e.velocity = Vec3::ZERO;
        e.nav.reset();
        let len = secs_to_ticks(latch.max_time, self.tick_rate, 1) as u32;
        e.set_state(AiState::Latched, len);
        self.place_on_victim(t, e, victim);
    }

    /// Moves a latched enemy onto its victim's chest, facing them.
    fn place_on_victim(&self, t: &Tuning, e: &mut Enemy, victim: PlayerId) {
        let Some(p) = self.players.get(&victim) else {
            return;
        };
        let fwd = look_dir(p.yaw, 0.0);
        let (_, _, radius) = p.movement.capsule(&t.movement);
        let eye = p.movement.eye_position(&t.movement);
        let center = eye - Vec3::Y * 0.5 + fwd * (radius + e.radius() * 0.45);
        e.position = center - Vec3::Y * (e.height() * 0.5);
        e.yaw = p.yaw + std::f32::consts::PI;
        e.velocity = p.movement.velocity;
        e.on_ground = false;
    }

    /// One tick of a latch: punched / shaken off, let go, or smother the wick and hold the
    /// victim's speed. Returns true while it still clings.
    pub(crate) fn step_latched(&mut self, t: &Tuning, e: &mut Enemy) -> bool {
        let Some(a) = e.current_attack().cloned() else {
            self.release(e, 0.5, Vec3::ZERO);
            return false;
        };
        let (Some(latch), Some(victim)) = (a.latch.clone(), e.latched) else {
            self.release(e, a.recover, Vec3::ZERO);
            return false;
        };
        let rate = self.tick_rate;
        let alive = self.players.get(&victim).is_some_and(super::is_targetable);
        if !alive || e.state_ticks >= e.state_len {
            self.release(e, a.recover, Vec3::ZERO);
            return false;
        }
        let Some(p) = self.players.get(&victim) else {
            return false;
        };
        let aim = p.last_input.look_dir();
        let back = -look_dir(p.yaw, 0.0);
        let dashing = p.movement.is_dashing();
        let punched = self
            .events
            .iter()
            .any(|ev| matches!(ev, SimEvent::Punched { player, .. } if *player == victim));
        if punched {
            // Thrown off along the punch, cap knocked open, stunned: free shots.
            let fwd = Vec3::new(aim.x, 0.0, aim.z).normalize_or(-back);
            self.release(e, 0.0, fwd * 11.0 + Vec3::Y * 6.0);
            e.stun(secs_to_ticks(latch.punch_stun, rate, 1));
            let info = DamageInfo::new(latch.punch_damage, Owner::Player(victim), Attack::Fist)
                .at(e.center(), fwd)
                .flags(DamageFlags::MELEE);
            // The counter goes through the pipeline once the enemy is back in the world
            // (`step_enemies` re-inserts it after its step).
            self.ai.throws.push((e.id, info));
            return false;
        }
        if dashing {
            // Shaken off: it tumbles off behind.
            self.release(e, 0.0, back * 6.0 + Vec3::Y * 4.0);
            e.stun(secs_to_ticks(latch.shake_stun, rate, 1));
            return false;
        }
        if self.is_pinned(HitTarget::Entity(e.id)) {
            // A Pinning Arrow pins it off its victim (it stays rooted where it was hit).
            self.release(e, latch.shake_stun, Vec3::ZERO);
            return false;
        }
        self.place_on_victim(t, e, victim);
        // Smother the wick: every hit of it is hard damage.
        e.latch_tick_in = e.latch_tick_in.saturating_sub(1);
        if e.latch_tick_in == 0 {
            e.latch_tick_in = secs_to_ticks(latch.interval, rate, 1);
            let fury = self.fury(e);
            let attack = Attack::Enemy {
                kind: e.def.net_kind,
                attack: e.attack.min(255) as u8,
            };
            let chest = e.center();
            let info = DamageInfo::new(
                latch.damage * latch.interval * fury.damage,
                Owner::Enemy(e.id),
                attack,
            )
            .at(chest, -back)
            .flags(DamageFlags::MELEE);
            let ev = self.apply_damage(t, HitTarget::Player(victim), info);
            if ev.dealt > 0.0
                && let Some(p) = self.players.get_mut(&victim)
            {
                let c = &t.combat;
                let extra = ev.dealt * (1.0 - c.hard_damage_fraction).max(0.0);
                p.combat.hard = (p.combat.hard + extra)
                    .min(p.combat.damage)
                    .min(c.max_health);
            }
        }
        // Slow the victim (a slow, not a root) through the predicted movement slow: refreshed
        // every tick while it clings. A longer slow already running (a Pinning Arrow) keeps
        // whichever scale is stronger.
        if let Some(p) = self.players.get_mut(&victim) {
            let m = &mut p.movement;
            let scale = latch.slow.clamp(0.05, 1.0);
            m.slow_scale = if m.slow_time > LATCH_SLOW_HOLD {
                m.slow_scale.min(scale)
            } else {
                scale
            };
            m.slow_time = m.slow_time.max(LATCH_SLOW_HOLD);
        }
        true
    }

    /// Lets go of the victim: falls with `velocity`, recovers for `recover` seconds (callers
    /// that stun it override the state), the latch goes on cooldown.
    pub(crate) fn release(&self, e: &mut Enemy, recover: f32, velocity: Vec3) {
        e.latched = None;
        e.velocity = velocity;
        e.on_ground = false;
        e.nav.reset();
        let rate = self.tick_rate;
        let fury = self.fury(e);
        if let Some(a) = e.current_attack().cloned()
            && let Some(cd) = e.cooldowns.get_mut(e.attack)
        {
            *cd = secs_to_ticks((a.cooldown + a.recover) / fury.aggression, rate, 0) as u32;
        }
        e.set_state(
            AiState::Recover,
            secs_to_ticks(recover / fury.aggression, rate, 1) as u32,
        );
    }

    /// Counter damage of the punches that threw latched enemies off this tick.
    pub(crate) fn apply_pending_throws(&mut self, t: &Tuning) {
        for (id, info) in std::mem::take(&mut self.ai.throws) {
            self.apply_damage(t, HitTarget::Entity(id), info);
        }
    }

    /// A training dummy's tick: flinch back to idle, heal to full after a pause, stand still.
    pub(crate) fn step_dummy(&self, e: &mut Enemy) {
        let Some(dummy) = e.def.dummy.clone() else {
            return;
        };
        let rate = self.tick_rate;
        e.target = None;
        e.latched = None;
        e.aggro.clear();
        e.alerted_by = None;
        if e.state != AiState::Idle && e.state_ticks >= e.state_len {
            e.set_state(AiState::Idle, 0);
        }
        let quiet = self.tick.saturating_sub(e.last_hurt_tick);
        if quiet >= secs_to_ticks(dummy.regen_delay, rate, 1) as u32 && e.health < e.max_health {
            e.health = (e.health + dummy.regen_rate * self.dt()).min(e.max_health);
        }
        e.velocity = Vec3::new(0.0, e.velocity.y, 0.0);
    }
}
