//! Waxwork (Precision, defend; `docs/rework.md` §1): a wax double of its owner where they
//! stood, posed as they were and facing their look, for `waxwork.duration` with
//! `waxwork.health`. One at a time (a live ability: `WeaponState::live`, its cooldown runs
//! from its end).
//!
//! **A stand-in**: the double takes its owner's place as a target under a decoy id,
//! `HitTarget::Player(`[`decoy_id`]`(owner))` (player ids stay below 128): it has a player's
//! hit shapes ([`SimWorld::hitboxes`], so lag compensation records it), every attack that may
//! hurt its owner may hurt it (`SimWorld::can_hit` / `is_hostile` resolve the decoy to its
//! owner; the owner's own shots and co-op teammates' pass through), and the damage pipeline
//! hands its damage here ([`SimWorld::damage_waxwork`]: no brands' or tallow's rules, a head
//! hit ×`player_head_multiplier` like a player's). Moulds see it in their player views
//! ([`SimWorld::waxwork_views`]); one within `waxwork.lure_range` that can see it prefers it to
//! anyone (`select_target`, [`SimWorld::lure_target`]) and is replicated `LURED`. PvP opponents
//! see a still player (`kind::WAXWORK`: the owner's pose, look, weapon and name).
//!
//! **The burst**: destroyed, or at its end (after `SLUMPING` for its last
//! [`SLUMP_SECS`]), it bursts in hot wax: everything hostile to its owner within
//! `waxwork.burst_radius` in line of sight takes up to `burst_damage` (falling off like an
//! explosion; never its owner) and `burn_stacks` Burn (`Attack::Waxwork`).

use std::collections::BTreeMap;

use engine::physics::MASK_SHOTS;
use glam::Vec3;

use super::super::combat::{Attack, DamageEvent, DamageFlags, DamageInfo, Owner, secs_to_ticks};
use super::super::enemy::PlayerView;
use super::super::{HitPads, HitPart, HitTarget, Hitbox, PlayerId, SimEvent, SimWorld};
use super::{AbilityCast, AbilityPhase};
use crate::movement::MoveState;
use crate::protocol::{EntityState, NetFrame, NetId, kind, waxwork_flags};
use crate::tuning::Tuning;

/// The bit that turns a player id into its Waxwork's decoy id.
pub const DECOY_BIT: u8 = 0x80;
/// It slumps for this long before its natural end.
pub const SLUMP_SECS: f32 = 0.4;

/// The id the double of `owner`'s Waxwork stands in with.
pub fn decoy_id(owner: PlayerId) -> PlayerId {
    PlayerId(owner.0 | DECOY_BIT)
}

/// The owner a decoy id stands in for (`None`: a real player id).
pub fn decoy_owner(id: PlayerId) -> Option<PlayerId> {
    (id.0 & DECOY_BIT != 0).then_some(PlayerId(id.0 & !DECOY_BIT))
}

/// A standing wax double.
#[derive(Clone, Debug, PartialEq)]
pub struct Waxwork {
    pub id: NetId,
    pub owner: PlayerId,
    /// The owner's movement state when it was cast (the pose: feet, stance).
    pub body: MoveState,
    pub yaw: f32,
    pub pitch: f32,
    /// The owner's replicated pose flags and weapon / ability (`state`) at the cast.
    pub flags: u32,
    pub state: u16,
    pub health: f32,
    pub max_health: f32,
    /// It slumps from here and bursts at `until`.
    pub slump: u32,
    pub until: u32,
    /// Moulds this close that see it prefer it.
    pub lure_range: f32,
    /// The slot whose live ability it is (ended early when it is destroyed).
    pub slot: u8,
}

/// Standing Waxworks, one per owner at most.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Waxworks {
    pub doubles: BTreeMap<PlayerId, Waxwork>,
}

impl Waxworks {
    pub(crate) fn forget(&mut self, id: PlayerId) {
        self.doubles.remove(&id);
    }

    pub(crate) fn len(&self) -> usize {
        self.doubles.len()
    }

    pub(crate) fn holds(&self, id: NetId) -> bool {
        self.doubles.values().any(|w| w.id == id)
    }
}

impl SimWorld {
    /// A Waxwork cast: the double where its owner stands.
    pub(crate) fn cast_waxwork(&mut self, c: &AbilityCast) {
        if c.phase != AbilityPhase::Use {
            return;
        }
        let t = c.t;
        let x = &t.combat.waxwork;
        let Some(p) = self.players.get(&c.id) else {
            return;
        };
        let mut body = p.movement;
        body.velocity = Vec3::ZERO;
        let pose = p.entity_state(&t.movement);
        let (yaw, pitch, slot) = (c.input.yaw, c.input.pitch, p.combat.weapons.slot);
        let Some(id) = self.alloc_net_id() else {
            return;
        };
        let until = self.tick + secs_to_ticks(x.duration, self.tick_rate, 1) as u32;
        let slump = until.saturating_sub(secs_to_ticks(SLUMP_SECS, self.tick_rate, 0) as u32);
        self.abilities.waxwork.doubles.insert(
            c.id,
            Waxwork {
                id,
                owner: c.id,
                body,
                yaw,
                pitch,
                flags: pose.flags,
                state: pose.state,
                health: x.health,
                max_health: x.health,
                slump,
                until,
                lure_range: x.lure_range,
                slot,
            },
        );
    }

    /// Player `owner`'s standing double.
    pub fn waxwork(&self, owner: PlayerId) -> Option<&Waxwork> {
        self.abilities.waxwork.doubles.get(&owner)
    }

    /// Is `id` the decoy id of a standing double?
    pub fn is_decoy(&self, id: PlayerId) -> bool {
        decoy_owner(id).is_some_and(|owner| self.waxwork(owner).is_some())
    }

    /// Hit shapes of every standing double (a player's, where it stands).
    pub(crate) fn waxwork_hitboxes(&self, t: &Tuning, out: &mut Vec<Hitbox>) {
        for w in self.abilities.waxwork.doubles.values() {
            let target = HitTarget::Player(decoy_id(w.owner));
            out.push(super::super::player_hitbox(target, &w.body, &t.movement));
        }
    }

    /// The doubles as Moulds see players (`select_target` prefers one in range and in sight).
    pub(crate) fn waxwork_views(&self, t: &Tuning) -> Vec<PlayerView> {
        self.abilities
            .waxwork
            .doubles
            .values()
            .map(|w| {
                let (center, _, radius) = w.body.capsule(&t.movement);
                PlayerView {
                    id: decoy_id(w.owner),
                    position: w.body.position,
                    velocity: Vec3::ZERO,
                    chest: center + Vec3::Y * 0.3,
                    radius,
                    height: w.body.height(&t.movement),
                    targetable: self.tick < w.slump,
                }
            })
            .collect()
    }

    /// The double enemy `e`'s eye at `eye` is lured by: the nearest in its lure range that it
    /// can see.
    pub(crate) fn lure_target(&self, eye: Vec3, views: &[PlayerView]) -> Option<PlayerId> {
        views
            .iter()
            .filter(|v| v.targetable)
            .filter_map(|v| {
                let owner = decoy_owner(v.id)?;
                let w = self.waxwork(owner)?;
                let d = v.chest.distance(eye);
                (d <= w.lure_range
                    && !self.collision.world.line_blocked(eye, v.chest, MASK_SHOTS)
                    && !self.soot_blocks(eye, v.chest))
                .then_some((d, v.id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }

    /// Is enemy `id` lured by a double (its target is one)?
    pub fn is_lured(&self, id: NetId) -> bool {
        self.enemies
            .get(&id)
            .and_then(|e| e.target)
            .is_some_and(|target| self.is_decoy(target))
    }

    /// Damage aimed at `id` that is really aimed at a Waxwork's double (`apply_damage` asks
    /// first): `Some` = it was the double's, here is what happened (its `SimEvent::Damage` is
    /// emitted, for hit markers).
    pub(crate) fn damage_waxwork(
        &mut self,
        t: &Tuning,
        id: PlayerId,
        info: DamageInfo,
    ) -> Option<DamageEvent> {
        let owner = decoy_owner(id)?;
        let hostile = info.source != Owner::Player(owner)
            && self.is_hostile(info.source, HitTarget::Player(owner));
        let tick = self.tick;
        let w = self.abilities.waxwork.doubles.get_mut(&owner)?;
        let mut event = DamageEvent {
            target: HitTarget::Player(id),
            info,
            dealt: 0.0,
            hard: 0.0,
            health_after: w.health,
            killed: false,
            blocked: None,
            tick,
        };
        if !hostile || tick >= w.slump {
            event.blocked = Some(super::super::Blocked::FriendlyFire);
            return Some(event);
        }
        let mut amount = info.amount.max(0.0);
        if info.part == HitPart::Head && !info.flags.contains(DamageFlags::EXPLOSIVE) {
            amount *= t.combat.player_head_multiplier;
            event.info.flags.insert(DamageFlags::HEADSHOT);
        }
        event.dealt = amount.min(w.health);
        w.health -= amount;
        event.health_after = w.health.max(0.0);
        event.killed = w.health <= 0.0;
        self.events.push(SimEvent::Damage(event));
        if event.killed {
            self.burst_waxwork(t, owner, true);
        }
        Some(event)
    }

    /// Bursts player `owner`'s double in hot wax and removes it (`early`: destroyed before its
    /// end — its owner's cooldown starts now).
    fn burst_waxwork(&mut self, t: &Tuning, owner: PlayerId, early: bool) {
        let Some(w) = self.abilities.waxwork.doubles.remove(&owner) else {
            return;
        };
        if early && let Some(p) = self.players.get_mut(&owner) {
            p.combat.weapons.end_live(w.slot, &t.combat, self.tick_rate);
        }
        let x = &t.combat.waxwork;
        let (center, _, _) = w.body.capsule(&t.movement);
        self.events.push(SimEvent::Explosion {
            position: center,
            radius: x.burst_radius,
            attack: Attack::Waxwork,
            owner: Owner::Player(owner),
        });
        // Like an explosion, but never its owner (a decoy, not a bomb at their feet).
        let me = Owner::Player(owner);
        let c = &t.combat;
        let world = &self.collision.world;
        let mut hits = Vec::new();
        for h in self.current_hitboxes(t) {
            if !self.can_hit(me, h.target) || h.target == HitTarget::Player(decoy_id(owner)) {
                continue;
            }
            let pads: HitPads = self.hit_pads(t, me, h.target);
            let Some((d, _)) = h.distance_to_point_padded(center, pads) else {
                continue;
            };
            let aim = h.center().unwrap_or(center);
            if d > x.burst_radius || world.line_blocked(center, aim, MASK_SHOTS) {
                continue;
            }
            let frac = 1.0
                - (1.0 - c.explosion_edge_fraction)
                    * (d / x.burst_radius.max(1e-3)).clamp(0.0, 1.0);
            let dir = (aim - center).normalize_or(Vec3::Y);
            let kb = (dir + Vec3::Y * c.explosion_up_bias).normalize_or(Vec3::Y)
                * (x.burst_knockback * frac);
            let info = DamageInfo::new(x.burst_damage * frac, me, Attack::Waxwork)
                .at(aim, dir)
                .knockback(kb)
                .flags(DamageFlags::EXPLOSIVE);
            hits.push((h.target, info));
        }
        for (target, info) in hits {
            self.apply_damage(t, target, info);
            self.apply_burn(t, me, Attack::Waxwork, target, x.burn_stacks);
        }
    }

    /// Per tick: a double whose time is up bursts; one whose owner left is gone.
    pub(crate) fn step_waxwork(&mut self, t: &Tuning) {
        let now = self.tick;
        let due: Vec<PlayerId> = self
            .abilities
            .waxwork
            .doubles
            .values()
            .filter(|w| now >= w.until)
            .map(|w| w.owner)
            .collect();
        for owner in due {
            self.burst_waxwork(t, owner, false);
        }
    }

    /// The doubles as entities (`kind::WAXWORK`).
    pub(crate) fn replicate_waxworks(&self, _t: &Tuning, frame: &mut NetFrame) {
        for w in self.abilities.waxwork.doubles.values() {
            let mut flags = w.flags;
            if self.tick >= w.slump {
                flags |= waxwork_flags::SLUMPING;
            }
            frame.insert(&EntityState {
                id: w.id,
                kind: kind::WAXWORK,
                owner: w.owner.0 as u16,
                position: w.body.position,
                yaw: w.yaw,
                pitch: w.pitch,
                flags,
                state: w.state,
                health: w.health.ceil().clamp(0.0, 65535.0) as u16,
                aux: w.until,
                ..Default::default()
            });
        }
    }
}
