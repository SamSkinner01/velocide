//! Knacks in the world (docs/arsenal.md, "Knacks"): the PvP rule (the Thorn applies as
//! Surge), Surge, Tallow Skin, the Kindle Flask's pool and the healing rules, the Snare (arm,
//! spring, pin, damage, two out), the Soot Pot's cloud hiding players from Moulds (a Heron's
//! lock breaks), replication, and the owner's prediction staying exact for every knack.

mod combat_support;
mod weapon_support;

use std::f32::consts::PI;

use combat_support::*;
use glam::Vec3;
use shared::input::{Buttons, PlayerInput};
use shared::loadout::{Knack, Loadout, WeaponKind};
use shared::lobby::{GameMode, MatchSettings};
use shared::protocol::{NetFrame, NetId, kind, player_flags, projectile_flags};
use shared::sim::{
    AiState, Attack, DamageFlags, DamageInfo, FireIntent, HealSource, HitTarget, Owner, PlayerId,
    ProjectileKind, SimEvent, step_weapons,
};
use weapon_support::*;

/// Player 0 at the origin facing -Z with the default weapons and `knack` (not targeted).
fn with_knack(mut a: Arena, knack: Knack) -> (Arena, PlayerId) {
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    a.world
        .request_loadout(p, Loadout::default().with_knack(knack));
    a.ticks(2);
    (a, p)
}

/// Looks at the floor `ahead` metres in front (feet level).
fn look_at_floor(a: &mut Arena, p: PlayerId, ahead: f32) {
    let feet = a.world.players[&p].movement.position;
    a.aim(p, feet + Vec3::new(0.0, 0.0, -ahead));
}

fn entity(a: &Arena, id: NetId) -> Option<shared::protocol::EntityState> {
    let mut frame = NetFrame::new();
    a.world.replicate(&a.tuning, &mut frame);
    frame.get(id)
}

fn knack_uses(events: &[SimEvent]) -> Vec<Knack> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::KnackUsed { knack, .. } => Some(*knack),
            _ => None,
        })
        .collect()
}

#[test]
fn pvp_worlds_turn_the_thorn_into_surge_and_keep_the_request() {
    // Co-op: the Thorn.
    let (a, p) = with_knack(Arena::coop(), Knack::Thorn);
    assert_eq!(a.world.players[&p].combat.weapons.knack(), Knack::Thorn);
    // PvP: Surge in hand, the Thorn kept in the request; a PvP knack applies as picked.
    let (mut a, p) = with_knack(Arena::pvp(), Knack::Thorn);
    assert!(a.world.pvp_world());
    assert_eq!(a.world.players[&p].combat.weapons.knack(), Knack::Surge);
    assert_eq!(a.world.loadout_requests[&p].knack, Knack::Thorn);
    a.world
        .request_loadout(p, Loadout::default().with_knack(Knack::Snare));
    a.world.kill_player(&a.tuning.clone(), p, Attack::Dev);
    a.world.respawn_player(&a.tuning.clone(), p);
    assert_eq!(a.world.players[&p].combat.weapons.knack(), Knack::Snare);

    // The warm-up follows the next match: PvP → Surge, back to co-op → the Thorn again.
    let (mut a, p) = with_knack(Arena::coop(), Knack::Thorn);
    a.world
        .start_warmup(&MatchSettings::new(GameMode::Deathmatch, "x"));
    assert_eq!(a.world.players[&p].combat.weapons.knack(), Knack::Surge);
    a.world
        .set_warmup_next(&MatchSettings::new(GameMode::Campaign, "x"));
    assert_eq!(a.world.players[&p].combat.weapons.knack(), Knack::Thorn);
    // Replicated charges (`aux` bits 8–11) follow the knack in hand.
    let e = entity(&a, NetId::player(p)).unwrap();
    assert_eq!((e.aux >> 8) & 0xF, a.tuning.combat.thorn.charges);
}

#[test]
fn surge_is_an_impulse_along_the_look() {
    let (mut a, p) = with_knack(Arena::coop(), Knack::Surge);
    let before = a.world.players[&p].movement.velocity;
    let ev = a.tap(p, Buttons::THROW, 0);
    assert_eq!(knack_uses(&ev), vec![Knack::Surge]);
    let dv = a.world.players[&p].movement.velocity - before;
    let s = &a.tuning.combat.knacks.surge;
    assert!((dv.z + s.speed).abs() < 0.5 && dv.x.abs() < 1e-3, "{dv}");
    // Two charges: the third press does nothing until one recharges.
    a.ticks(1);
    a.tap(p, Buttons::THROW, 1);
    let ev = a.tap(p, Buttons::THROW, 1);
    assert!(knack_uses(&ev).is_empty());
    assert_eq!(
        a.world.players[&p]
            .combat
            .weapons
            .knack_charges(&a.tuning.combat),
        0
    );
}

#[test]
fn tallow_skin_halves_damage_and_stops_knockback() {
    let (mut a, p) = with_knack(Arena::coop(), Knack::TallowSkin);
    let husk = a.husk(Vec3::new(0.0, 0.0, -30.0), PI);
    a.tick();
    let t = a.tuning.clone();
    let hit = |a: &mut Arena, source: Owner, flags: DamageFlags| {
        let at = a.world.players[&p].movement.position + Vec3::Y;
        let info = DamageInfo::new(20.0, source, Attack::Enemy { kind: 0, attack: 0 })
            .at(at, Vec3::Z)
            .knockback(Vec3::new(0.0, 0.0, 10.0))
            .flags(flags);
        let before = a.world.players[&p].movement.velocity;
        let d = a.world.apply_damage(&t, HitTarget::Player(p), info);
        let after = a.world.players[&p].movement.velocity;
        (d.dealt, (after - before).length())
    };
    let ev = a.tap(p, Buttons::THROW, 0);
    assert_eq!(knack_uses(&ev), vec![Knack::TallowSkin]);
    assert!(a.world.has_tallow(p));
    let flags = entity(&a, NetId::player(p)).unwrap().flags;
    assert!(flags & player_flags::TALLOW != 0, "replicated");
    let (dealt, pushed) = hit(&mut a, Owner::Enemy(husk), DamageFlags::NONE);
    assert_eq!(dealt, 10.0, "half");
    assert_eq!(pushed, 0.0, "no knockback");
    // Your own blasts still push (blast jumps).
    let (_, pushed) = hit(&mut a, Owner::Player(p), DamageFlags::EXPLOSIVE);
    assert!(pushed > 9.0);
    // Worn off after 3 s.
    let ticks = (t.combat.knacks.tallow_skin.duration * 60.0) as u32;
    a.ticks(ticks);
    assert!(!a.world.has_tallow(p));
    let flags = entity(&a, NetId::player(p)).unwrap().flags;
    assert_eq!(flags & player_flags::TALLOW, 0);
    let (dealt, pushed) = hit(&mut a, Owner::Enemy(husk), DamageFlags::NONE);
    assert_eq!((dealt, pushed), (20.0, 10.0));
}

/// Throws the flask at the floor at `p`'s feet and ticks until its pool is there.
fn pool_at_feet(a: &mut Arena, p: PlayerId) -> NetId {
    look_at_floor(a, p, 0.3);
    let ev = a.tap(p, Buttons::THROW, 0);
    assert_eq!(knack_uses(&ev), vec![Knack::KindleFlask]);
    for _ in 0..120 {
        if let Some(id) = a.world.knacks.pools.keys().next() {
            return *id;
        }
        a.tick();
    }
    panic!("no pool");
}

fn healed_by_pools(events: &[SimEvent], who: PlayerId) -> f32 {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Healed {
                player,
                amount,
                source: HealSource::KindleFlask(_),
                ..
            } if *player == who => Some(*amount),
            _ => None,
        })
        .sum()
}

#[test]
fn kindle_pools_heal_the_team_through_the_healing_rules() {
    let (mut a, p) = with_knack(Arena::coop(), Knack::KindleFlask);
    let mate = a.player(1, Vec3::new(1.5, 0.0, 0.0), 0.0);
    a.tuning.combat.hard_damage_recover_rate = 0.0;
    for (id, damage, hard) in [(p, 60.0, 30.0), (mate, 50.0, 0.0)] {
        let c = &mut a.world.players.get_mut(&id).unwrap().combat;
        c.damage = damage;
        c.hard = hard;
    }
    let pool = pool_at_feet(&mut a, p);
    let e = entity(&a, pool).expect("replicated");
    assert_eq!(e.kind, kind::KINDLE_POOL);
    assert_eq!(e.state, 35, "radius in dm");
    assert_eq!(e.owner, p.0 as u16);
    a.ticks(300);
    assert!(a.world.knacks.pools.is_empty(), "dried up after 4 s");
    // 10 HP/s for 4 s = 40 HP, in 0.25 s steps — never above the effective max (100 − 30
    // hard damage for the thrower).
    assert!((a.health(p) - 70.0).abs() < 1e-3, "{}", a.health(p));
    assert!((healed_by_pools(&a.events, p) - 30.0).abs() < 1e-3);
    assert!(
        (healed_by_pools(&a.events, mate) - 40.0).abs() < 1e-3,
        "the teammate"
    );
    assert!((a.health(mate) - 90.0).abs() < 1e-3);

    // PvP: the pool is only the thrower's.
    let (mut a, p) = with_knack(Arena::pvp(), Knack::KindleFlask);
    let other = a.player(1, Vec3::new(1.0, 0.0, 0.0), 0.0);
    for id in [p, other] {
        a.world.players.get_mut(&id).unwrap().combat.damage = 50.0;
    }
    pool_at_feet(&mut a, p);
    a.ticks(300);
    assert!(healed_by_pools(&a.events, p) > 30.0);
    assert_eq!(healed_by_pools(&a.events, other), 0.0);
}

/// Throws a snare onto the floor ~2 m ahead and ticks until it lies there; its id.
fn lay_snare(a: &mut Arena, p: PlayerId) -> NetId {
    let before: Vec<NetId> = a.world.projectiles.keys().copied().collect();
    look_at_floor(a, p, 2.0);
    let ev = a.tap(p, Buttons::THROW, 0);
    assert_eq!(knack_uses(&ev), vec![Knack::Snare]);
    for _ in 0..120 {
        if let Some(id) = a
            .world
            .projectiles
            .values()
            .find(|pr| {
                pr.kind == ProjectileKind::Snare && pr.stuck.is_some() && !before.contains(&pr.id)
            })
            .map(|pr| pr.id)
        {
            return id;
        }
        a.tick();
    }
    panic!("the snare never settled");
}

#[test]
fn snares_arm_then_pin_and_hurt_the_first_hostile() {
    let (mut a, p) = with_knack(Arena::coop(), Knack::Snare);
    let snare = lay_snare(&mut a, p);
    let at = a.world.projectiles[&snare].position;
    let e = entity(&a, snare).unwrap();
    assert_eq!(e.kind, kind::SNARE);
    assert!(e.flags & projectile_flags::STUCK != 0);
    assert_eq!(e.flags & projectile_flags::ARMED, 0, "not armed yet");
    // Walked onto at once: it holds until armed (0.6 s after landing), then springs.
    let d = dummy(&mut a, at + Vec3::new(0.8, 0.0, 0.0), 0.0, 1000.0);
    let arm = (a.tuning.combat.knacks.snare.arm_time * 60.0).round() as u32;
    let landed = a.world.projectiles[&snare].launch_tick;
    let mut sprung = None;
    for _ in 0..arm + 5 {
        let ev = a.tick();
        if ev
            .iter()
            .any(|e| matches!(e, SimEvent::SnareSprung { id, .. } if *id == snare))
        {
            sprung = Some((a.world.tick, ev));
            break;
        }
    }
    let (tick, ev) = sprung.expect("sprung");
    assert_eq!(tick, landed + arm, "the moment it armed");
    let hits: Vec<_> = ev
        .iter()
        .filter_map(|e| match e {
            SimEvent::Damage(dm) if dm.target == HitTarget::Entity(d) => Some(dm.info),
            _ => None,
        })
        .collect();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].amount, 15.0);
    assert_eq!(hits[0].attack, Attack::Snare);
    assert!(a.world.is_pinned(HitTarget::Entity(d)), "Pinned");
    assert!(!a.world.projectiles.contains_key(&snare), "gone");

    // Armed and waiting: replicated as such; a teammate (and the owner) walk over it.
    a.world.enemies.remove(&d);
    let snare = lay_snare(&mut a, p);
    let at = a.world.projectiles[&snare].position;
    a.ticks(arm + 2);
    let e = entity(&a, snare).unwrap();
    assert!(e.flags & projectile_flags::ARMED != 0);
    let mate = a.player(1, at, 0.0);
    a.ticks(5);
    assert!(
        a.world.projectiles.contains_key(&snare),
        "teammates don't spring it"
    );
    assert!(!a.world.is_pinned(HitTarget::Player(mate)));
}

#[test]
fn at_most_two_snares_lie_out() {
    let (mut a, p) = with_knack(Arena::coop(), Knack::Snare);
    a.tuning.combat.knacks.snare.charges = 3;
    let first = lay_snare(&mut a, p);
    a.ticks(2);
    let second = lay_snare(&mut a, p);
    a.ticks(2);
    let third = lay_snare(&mut a, p);
    let mine: Vec<NetId> = a
        .world
        .projectiles
        .values()
        .filter(|pr| pr.kind == ProjectileKind::Snare)
        .map(|pr| pr.id)
        .collect();
    assert_eq!(mine.len(), 2);
    assert!(!mine.contains(&first), "the oldest went");
    assert!(mine.contains(&second) && mine.contains(&third));
}

#[test]
fn a_snare_slows_a_hostile_player_in_pvp() {
    let (mut a, p) = with_knack(Arena::pvp(), Knack::Snare);
    let v = a.player(1, Vec3::new(20.0, 0.0, 20.0), 0.0);
    a.ticks(2);
    let snare = lay_snare(&mut a, p);
    let at = a.world.projectiles[&snare].position;
    a.ticks(60);
    // The other player walks onto it.
    let pl = a.world.players.get_mut(&v).unwrap();
    pl.combat.protection = 0;
    pl.movement.position = Vec3::new(at.x + 0.5, 0.0, at.z);
    let ev = a.ticks(2);
    assert_eq!(damage_to(&ev, HitTarget::Player(v)), 15.0);
    assert!(a.world.players[&v].movement.slow_time > 0.0, "slowed");
    assert!(a.world.is_pinned(HitTarget::Player(v)));
}

/// Throws the soot pot at the floor at `p`'s feet; ticks until its cloud is there.
fn soot_at_feet(a: &mut Arena, p: PlayerId) -> NetId {
    look_at_floor(a, p, 0.3);
    let ev = a.tap(p, Buttons::THROW, 0);
    assert_eq!(knack_uses(&ev), vec![Knack::SootPot]);
    for _ in 0..120 {
        if let Some(id) = a.world.knacks.clouds.keys().next() {
            return *id;
        }
        a.tick();
    }
    panic!("no cloud");
}

#[test]
fn soot_hides_players_from_moulds() {
    let (mut a, p) = with_knack(Arena::coop(), Knack::SootPot);
    a.world.players.get_mut(&p).unwrap().notarget = false;
    a.world.players.get_mut(&p).unwrap().combat.god = true;
    let g = a
        .world
        .spawn_enemy("gnawer", Vec3::new(0.0, 0.0, -25.0), 0.0)
        .expect("gnawer");
    a.ticks(30);
    assert_eq!(a.world.enemies[&g].target, Some(p), "it saw the player");
    let cloud = soot_at_feet(&mut a, p);
    let e = entity(&a, cloud).expect("replicated");
    assert_eq!((e.kind, e.state), (kind::SOOT_CLOUD, 50));
    a.tick();
    assert_eq!(a.world.enemies[&g].target, None, "lost in the soot");
    // It stays lost while the cloud lasts ...
    let mut seen = false;
    for _ in 0..300 {
        a.tick();
        seen |= a.world.enemies[&g].target.is_some();
    }
    assert!(!seen);
    // ... and finds the player again once it clears (7 s).
    a.ticks(200);
    assert!(a.world.knacks.clouds.is_empty());
    assert_eq!(a.world.enemies[&g].target, Some(p));
}

#[test]
fn soot_breaks_a_herons_lock() {
    let (mut a, p) = with_knack(Arena::coop(), Knack::SootPot);
    {
        let pl = a.world.players.get_mut(&p).unwrap();
        pl.notarget = false;
        pl.combat.god = true;
    }
    let h = a
        .world
        .spawn_enemy("heron", Vec3::new(0.0, 0.0, -32.0), 0.0)
        .expect("heron");
    let mut aiming = false;
    for _ in 0..6 * 60 {
        a.tick();
        if a.world.enemies[&h].aiming() {
            aiming = true;
            break;
        }
    }
    assert!(aiming, "the heron took aim");
    let needles_before = needles(&a);
    soot_at_feet(&mut a, p);
    a.tick();
    let e = &a.world.enemies[&h];
    assert!(!e.aiming() && e.state != AiState::Windup, "the lock broke");
    let mut fired = false;
    for _ in 0..240 {
        a.tick();
        fired |= needles(&a) > needles_before;
    }
    assert!(!fired, "no needle into the soot");
}

fn needles(a: &Arena) -> usize {
    a.world
        .projectiles
        .values()
        .filter(|p| p.kind == ProjectileKind::Needle)
        .count()
}

#[test]
fn thrown_knacks_fly_and_land_as_replicated_projectiles() {
    for (knack, pk, k) in [
        (
            Knack::KindleFlask,
            ProjectileKind::KindleFlask,
            kind::KINDLE_FLASK,
        ),
        (Knack::Snare, ProjectileKind::Snare, kind::SNARE),
        (Knack::SootPot, ProjectileKind::SootPot, kind::SOOT_POT),
    ] {
        let (mut a, p) = with_knack(Arena::coop(), knack);
        look_at_floor(&mut a, p, 8.0);
        a.tap(p, Buttons::THROW, 0);
        let id = a
            .world
            .projectiles
            .values()
            .find(|pr| pr.kind == pk)
            .map(|pr| pr.id)
            .expect("in flight");
        assert_eq!(entity(&a, id).unwrap().kind, k, "{knack:?}");
        a.ticks(120);
        match knack {
            Knack::KindleFlask => assert_eq!(a.world.knacks.pools.len(), 1),
            Knack::SootPot => assert_eq!(a.world.knacks.clouds.len(), 1),
            _ => assert!(a.world.projectiles[&id].stuck.is_some()),
        }
    }
}

#[test]
fn knack_prediction_stays_exact() {
    for knack in Knack::ALL {
        let (mut a, p) = with_knack(Arena::coop(), knack);
        a.world.players.get_mut(&p).unwrap().combat.god = true;
        let t = a.tuning.clone();
        let mut predicted = a.world.players[&p].combat.weapons;
        let mut used = 0;
        for i in 0..1200u32 {
            let input = a.held.get_mut(&p).unwrap();
            input.weapon = ((i / 100) % 4) as u8;
            input.pitch = -0.3;
            let mut b = Buttons::NONE;
            b.set(Buttons::FIRE, i % 29 < 10);
            b.set(Buttons::THROW, i % 23 < 2);
            input.buttons = b;
            let sent: PlayerInput = *input;
            a.tick();
            let intents = step_weapons(&mut predicted, &sent, true, &t.combat, 60);
            used += intents
                .iter()
                .filter(|i| matches!(i, FireIntent::Knack(k) if *k == knack))
                .count();
            assert_eq!(
                a.world.players[&p].combat.weapons, predicted,
                "{knack:?} input {i}"
            );
        }
        assert!(used >= knack.charges(&t.combat) as usize, "{knack:?}");
    }
    // The weapon kinds are untouched by a knack.
    assert_eq!(
        Loadout::default().slot(0).map(|b| b.weapon),
        Some(WeaponKind::Quill)
    );
}
