//! Turning the simulation's combat [`SimEvent`]s into network traffic:
//!
//! * **reliable** `GameEvent`s for gameplay-critical things — `Damaged` (victim only),
//!   `HitConfirm` (attacker only, merged per target per tick), `Killed`, `Parried`,
//!   `Respawned` (everyone);
//! * **cosmetic** [`Fx`] events (remote shots, explosions, ricochets, heals, enemy telegraphs...)
//!   kept in an [`FxLog`] and batched into the next snapshot of each client (unreliable;
//!   important ones repeated once).
//!
//! The full "who gets what" table is in `docs/combat.md`.

use std::collections::{BTreeMap, VecDeque};

use shared::protocol::{DamageReport, Fx, FxEvent, GameEvent, HitReport, QPos, kind};
use shared::sim::{
    Blocked, DamageFlags, HealSource, HitTarget, Owner, PlayerId, SimEvent, SimWorld,
};

/// Most FX events put into one snapshot (the newest win; important ones first).
pub const MAX_FX_PER_SNAPSHOT: usize = 256;

#[derive(Clone, Debug)]
struct FxEntry {
    event: FxEvent,
    /// Not sent to this player (they predict it locally).
    exclude: Option<PlayerId>,
}

/// Recent cosmetic events, by tick.
#[derive(Debug, Default)]
pub struct FxLog {
    next_seq: u32,
    entries: VecDeque<FxEntry>,
}

impl FxLog {
    pub fn push(&mut self, tick: u32, fx: Fx, exclude: Option<PlayerId>) {
        self.next_seq = self.next_seq.wrapping_add(1);
        self.entries.push_back(FxEntry {
            event: FxEvent {
                seq: self.next_seq,
                tick,
                fx,
            },
            exclude,
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Events for a snapshot to `player`: everything after `since` (the previous snapshot
    /// tick) plus important events after `repeat_since` (the one before: redundancy).
    pub fn collect(&self, player: Option<PlayerId>, since: u32, repeat_since: u32) -> Vec<FxEvent> {
        let mut out: Vec<&FxEntry> = self
            .entries
            .iter()
            .filter(|e| player.is_none() || e.exclude != player)
            .filter(|e| {
                e.event.tick > since || (e.event.tick > repeat_since && e.event.fx.important())
            })
            .collect();
        if out.len() > MAX_FX_PER_SNAPSHOT {
            // Keep important ones, then the newest.
            out.sort_by_key(|e| (!e.event.fx.important(), std::cmp::Reverse(e.event.seq)));
            out.truncate(MAX_FX_PER_SNAPSHOT);
            out.sort_by_key(|e| e.event.seq);
        }
        out.into_iter().map(|e| e.event.clone()).collect()
    }

    /// Forgets events at or before `tick`.
    pub fn prune(&mut self, tick: u32) {
        while self.entries.front().is_some_and(|e| e.event.tick <= tick) {
            self.entries.pop_front();
        }
    }
}

/// Reliable messages produced by one tick.
#[derive(Debug, Default)]
pub struct Routed {
    pub broadcast: Vec<GameEvent>,
    pub to_player: Vec<(PlayerId, GameEvent)>,
}

fn radius_dm(r: f32) -> u16 {
    (r * 10.0).round().clamp(0.0, u16::MAX as f32) as u16
}

/// Splits one tick's events into reliable messages and FX.
pub fn route_events(events: &[SimEvent], world: &SimWorld, fx: &mut FxLog) -> Routed {
    let tick = world.tick;
    let mut routed = Routed::default();
    // (attacker, target) -> merged hit report.
    let mut confirms: BTreeMap<(PlayerId, HitTarget), HitReport> = BTreeMap::new();
    for event in events {
        match event {
            SimEvent::Respawned {
                player,
                position,
                yaw,
            } => routed.broadcast.push(GameEvent::Respawned {
                player: *player,
                position: *position,
                yaw: *yaw,
            }),
            SimEvent::Damage(d) => {
                if let HitTarget::Player(victim) = d.target {
                    let report = matches!(
                        d.blocked,
                        None | Some(Blocked::Invulnerable)
                            | Some(Blocked::SpawnProtection)
                            | Some(Blocked::God)
                            | Some(Blocked::FriendlyFire)
                    ) && (d.dealt > 0.0
                        || d.blocked.is_some()
                        || d.info.knockback.length_squared() > 0.0);
                    if report {
                        routed.to_player.push((
                            victim,
                            GameEvent::Damaged(DamageReport {
                                amount: d.dealt,
                                hard: d.hard,
                                health: d.health_after,
                                source: d.info.source,
                                attack: d.info.attack,
                                point: d.info.point,
                                direction: d.info.dir,
                                flags: d.info.flags,
                                blocked: d.blocked,
                                killed: d.killed,
                            }),
                        ));
                    }
                }
                if let Owner::Player(attacker) = d.info.source
                    && d.blocked.is_none()
                    && Some(attacker) != d.target.player()
                {
                    let target_kind = match d.target {
                        HitTarget::Player(_) => kind::PLAYER,
                        HitTarget::Entity(id) => world.enemies.get(&id).map_or(0, |e| e.net_kind()),
                    };
                    let entry = confirms.entry((attacker, d.target)).or_insert(HitReport {
                        target: d.target,
                        target_kind,
                        attack: d.info.attack,
                        damage: 0.0,
                        hits: 0,
                        flags: DamageFlags::NONE,
                        point: d.info.point,
                        killed: false,
                    });
                    entry.damage += d.dealt;
                    entry.hits = entry.hits.saturating_add(1);
                    entry.flags |= d.info.flags;
                    entry.point = d.info.point;
                    entry.attack = d.info.attack;
                    entry.killed |= d.killed;
                }
            }
            SimEvent::Killed(k) => routed.broadcast.push(GameEvent::Killed(*k)),
            SimEvent::Parried(p) => routed.broadcast.push(GameEvent::Parried(*p)),
            SimEvent::Shot(s) => fx.push(
                tick,
                Fx::Shot {
                    shooter: s.shooter,
                    attack: s.attack,
                    origin: s.origin.into(),
                    ends: s.ends.iter().map(|e| QPos::from(*e)).collect(),
                },
                Some(s.shooter),
            ),
            SimEvent::Ricochet {
                shooter,
                attack,
                points,
                projectile,
            } => fx.push(
                tick,
                Fx::Ricochet {
                    shooter: shared::protocol::owner_code(*shooter),
                    attack: *attack,
                    points: points.iter().map(|p| QPos::from(*p)).collect(),
                    projectile: *projectile,
                },
                None,
            ),
            SimEvent::Explosion {
                position,
                radius,
                attack,
                owner,
            } => fx.push(
                tick,
                Fx::Explosion {
                    position: (*position).into(),
                    radius_dm: radius_dm(*radius),
                    attack: *attack,
                    owner: shared::protocol::owner_code(*owner),
                },
                None,
            ),
            SimEvent::Punched { player, hit } => fx.push(
                tick,
                Fx::Punch {
                    player: *player,
                    hit: *hit,
                },
                Some(*player),
            ),
            SimEvent::Healed {
                player,
                amount,
                source: HealSource::Embers(_) | HealSource::KindleFlask,
                position,
            } => fx.push(
                tick,
                Fx::Heal {
                    player: *player,
                    amount_tenths: (amount * 10.0).round().clamp(0.0, u16::MAX as f32) as u16,
                    from: (*position).into(),
                },
                None,
            ),
            SimEvent::SlamShockwave {
                player,
                position,
                power,
                radius,
            } => fx.push(
                tick,
                Fx::SlamShockwave {
                    player: *player,
                    position: (*position).into(),
                    power: (power.clamp(0.0, 1.0) * 255.0).round() as u8,
                    radius_dm: radius_dm(*radius),
                },
                None,
            ),
            SimEvent::ThornThrown { player, .. } => {
                fx.push(tick, Fx::ThornThrown { player: *player }, Some(*player))
            }
            SimEvent::ThornPlanted { position, .. } => fx.push(
                tick,
                Fx::ThornPlanted {
                    position: (*position).into(),
                },
                None,
            ),
            SimEvent::ThornExpired { id } => fx.push(tick, Fx::ThornExpired { id: *id }, None),
            SimEvent::Burst {
                shooter,
                attack,
                origin,
                ends,
            } => fx.push(
                tick,
                Fx::Burst {
                    shooter: shared::protocol::owner_code(*shooter),
                    attack: *attack,
                    origin: (*origin).into(),
                    ends: ends.iter().map(|e| QPos::from(*e)).collect(),
                },
                None,
            ),
            SimEvent::ArcChain { shooter, points } => fx.push(
                tick,
                Fx::ArcChain {
                    shooter: shared::protocol::owner_code(*shooter),
                    points: points.iter().map(|p| QPos::from(*p)).collect(),
                },
                None,
            ),
            SimEvent::WispZap { from, to, .. } => fx.push(
                tick,
                Fx::WispZap {
                    from: (*from).into(),
                    to: (*to).into(),
                },
                None,
            ),
            SimEvent::Branded { position, .. } => fx.push(
                tick,
                Fx::Branded {
                    position: (*position).into(),
                },
                None,
            ),
            SimEvent::Deflected {
                player, position, ..
            } => fx.push(
                tick,
                Fx::Deflected {
                    player: *player,
                    position: (*position).into(),
                },
                None,
            ),
            SimEvent::Discharge {
                player,
                position,
                radius,
                ..
            } => fx.push(
                tick,
                Fx::Discharge {
                    player: *player,
                    position: (*position).into(),
                    radius_dm: radius_dm(*radius),
                },
                Some(*player),
            ),
            SimEvent::ProjectileBounced { kind, position, .. } => fx.push(
                tick,
                Fx::ProjectileBounce {
                    kind: kind.net_kind(),
                    position: (*position).into(),
                },
                None,
            ),
            SimEvent::ProjectileImpact {
                kind,
                position,
                normal,
                ..
            } => fx.push(
                tick,
                Fx::ProjectileImpact {
                    kind: kind.net_kind(),
                    position: (*position).into(),
                    normal: (*normal).into(),
                },
                None,
            ),
            SimEvent::EnemyAttack {
                id,
                attack,
                phase,
                parryable,
            } => fx.push(
                tick,
                Fx::EnemyAttack {
                    enemy: *id,
                    attack: *attack,
                    strike: *phase == shared::sim::EnemyAttackPhase::Strike,
                    parryable: *parryable,
                },
                None,
            ),
            SimEvent::EnemyFired {
                id,
                attack,
                position,
            } => fx.push(
                tick,
                Fx::EnemyFired {
                    enemy: *id,
                    attack: *attack,
                    muzzle: (*position).into(),
                },
                None,
            ),
            SimEvent::EnemyShockwave {
                id,
                position,
                radius,
            } => fx.push(
                tick,
                Fx::EnemyShockwave {
                    enemy: *id,
                    position: (*position).into(),
                    radius_dm: radius_dm(*radius),
                },
                None,
            ),
            SimEvent::EnemyAlerted { id, .. } => fx.push(tick, Fx::EnemyAlert { enemy: *id }, None),
            SimEvent::EnemySpawned { id, kind, position } => fx.push(
                tick,
                Fx::EnemySpawned {
                    enemy: *id,
                    kind: *kind,
                    position: (*position).into(),
                },
                None,
            ),
            // Arena encounters (docs/enemies.md): reliable, everyone.
            SimEvent::EncounterStarted { index, waves } => {
                routed.broadcast.push(GameEvent::EncounterStarted {
                    index: *index,
                    waves: *waves,
                })
            }
            SimEvent::WaveStarted {
                index,
                wave,
                waves,
                enemies,
            } => routed.broadcast.push(GameEvent::WaveStarted {
                index: *index,
                wave: *wave,
                waves: *waves,
                enemies: *enemies,
            }),
            SimEvent::EncounterCleared { index } => routed
                .broadcast
                .push(GameEvent::EncounterCleared { index: *index }),
            // Melee and knacks (docs/arsenal.md, "Replication and wire ids").
            SimEvent::Swing {
                player,
                attack,
                hits,
            } => fx.push(
                tick,
                Fx::Swing {
                    player: *player,
                    attack: *attack,
                    hits: *hits,
                },
                Some(*player),
            ),
            SimEvent::LungeStruck {
                player, position, ..
            } => fx.push(
                tick,
                Fx::LungeStruck {
                    player: *player,
                    position: (*position).into(),
                },
                None,
            ),
            SimEvent::KnackUsed { player, knack } => fx.push(
                tick,
                Fx::Knack {
                    player: *player,
                    knack: *knack,
                },
                Some(*player),
            ),
            SimEvent::SnareSprung { position, .. } => fx.push(
                tick,
                Fx::SnareSprung {
                    position: (*position).into(),
                },
                None,
            ),
            // ---- rework (docs/rework.md): everyone, the caster too (it can't predict them) ----
            SimEvent::Wickline {
                player,
                from,
                to,
                anchored,
            } => fx.push(
                tick,
                Fx::Wickline {
                    player: *player,
                    from: (*from).into(),
                    to: (*to).into(),
                    anchored: *anchored,
                },
                None,
            ),
            SimEvent::TallyFired {
                shooter,
                origin,
                ends,
            } => fx.push(
                tick,
                Fx::TallyFired {
                    shooter: *shooter,
                    origin: (*origin).into(),
                    ends: ends.iter().map(|e| QPos::from(*e)).collect(),
                },
                None,
            ),
            SimEvent::Blink { player, from, to } => fx.push(
                tick,
                Fx::Blink {
                    player: *player,
                    from: (*from).into(),
                    to: (*to).into(),
                },
                None,
            ),
            SimEvent::GuardBlock {
                player,
                ability,
                position,
                broke,
            } => fx.push(
                tick,
                Fx::GuardBlock {
                    player: *player,
                    ability: *ability,
                    position: (*position).into(),
                    broke: *broke,
                },
                None,
            ),
            SimEvent::BolasBound { positions, .. } => fx.push(
                tick,
                Fx::BolasBound {
                    positions: positions.iter().map(|p| QPos::from(*p)).collect(),
                },
                None,
            ),
            SimEvent::LanternShot {
                lantern,
                origin,
                ends,
            } => fx.push(
                tick,
                Fx::LanternShot {
                    lantern: *lantern,
                    origin: (*origin).into(),
                    ends: ends.iter().map(|e| QPos::from(*e)).collect(),
                },
                None,
            ),
            SimEvent::HurlCaught { player } => {
                fx.push(tick, Fx::HurlCaught { player: *player }, None)
            }
            // Visible through entities / other events, or server-internal.
            SimEvent::Jumped { .. }
            | SimEvent::Landed { .. }
            | SimEvent::Healed { .. }
            | SimEvent::ThornShattered { .. }
            | SimEvent::GravewellStuck { .. }
            | SimEvent::EnemyRemoved { .. } => {}
            // Style and game-mode events are routed by `server::modes`.
            SimEvent::Style(_) | SimEvent::Mode(_) => {}
        }
    }
    for ((attacker, _), report) in confirms {
        routed
            .to_player
            .push((attacker, GameEvent::HitConfirm(report)));
    }
    routed
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::sim::Attack;

    fn explosion() -> Fx {
        Fx::Explosion {
            position: QPos::default(),
            radius_dm: 45,
            attack: Attack::Tumble,
            owner: 0,
        }
    }

    #[test]
    fn fx_batching_excludes_and_repeats_important() {
        let mut log = FxLog::default();
        log.push(
            10,
            Fx::Punch {
                player: PlayerId(1),
                hit: false,
            },
            Some(PlayerId(1)),
        );
        log.push(11, explosion(), None);
        // Snapshot at 12 (previous at 10, before that 8).
        let to_p1 = log.collect(Some(PlayerId(1)), 10, 8);
        assert_eq!(to_p1.len(), 1, "own punch excluded");
        let to_p2 = log.collect(Some(PlayerId(2)), 10, 8);
        assert_eq!(
            to_p2.len(),
            1,
            "punch at tick 10 belongs to the previous snapshot"
        );
        // Next snapshot at 14: the explosion is repeated once, then dropped.
        let again = log.collect(Some(PlayerId(2)), 12, 10);
        assert_eq!(again.len(), 1);
        assert!(matches!(again[0].fx, Fx::Explosion { .. }));
        assert!(log.collect(Some(PlayerId(2)), 14, 12).is_empty());
        log.prune(12);
        assert!(log.is_empty());
    }
}
