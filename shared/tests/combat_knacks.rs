//! Knacks in the world (docs/arsenal.md, "Knacks"): the PvP rule (the Thorn applies as
//! Surge), Surge, Tallow Skin, the Kindle Flask (drunk: the haste predicted to the bit, the
//! heal over time and the healing rules, PvP), the Snare (arm,
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

fn healed_by_flasks(events: &[SimEvent], who: PlayerId) -> Vec<f32> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Healed {
                player,
                amount,
                source: HealSource::KindleFlask,
                ..
            } if *player == who => Some(*amount),
            _ => None,
        })
        .collect()
}

/// Drinks the flask (one tap of the knack).
fn drink(a: &mut Arena, p: PlayerId) {
    let projectiles = a.world.projectiles.len();
    let ev = a.tap(p, Buttons::THROW, 0);
    assert_eq!(knack_uses(&ev), vec![Knack::KindleFlask]);
    assert_eq!(a.world.projectiles.len(), projectiles, "nothing thrown");
}

#[test]
fn the_kindle_flask_is_drunk_hastes_and_heals_over_time() {
    let (mut a, p) = with_knack(Arena::coop(), Knack::KindleFlask);
    let mate = a.player(1, Vec3::new(1.5, 0.0, 0.0), 0.0);
    a.tuning.combat.hard_damage_recover_rate = 0.0;
    for (id, damage) in [(p, 60.0), (mate, 50.0)] {
        a.world.players.get_mut(&id).unwrap().combat.damage = damage;
    }
    let f = a.tuning.combat.knacks.kindle_flask.clone();
    drink(&mut a, p);
    let m = a.world.players[&p].movement;
    assert_eq!((m.haste_time, m.haste_scale), (f.haste_time, f.haste_scale));
    let e = entity(&a, NetId::player(p)).unwrap();
    assert!(e.flags & player_flags::HASTE != 0, "replicated");
    // A step of 8 HP every second, the first a second after drinking.
    a.ticks(59);
    assert!(healed_by_flasks(&a.events, p).is_empty());
    a.ticks(1);
    assert_eq!(healed_by_flasks(&a.events, p), vec![8.0]);
    a.ticks(240);
    // 8 HP/s for 5 s = 40 HP; only the drinker.
    assert_eq!(healed_by_flasks(&a.events, p), vec![8.0; 5]);
    assert!((a.health(p) - 80.0).abs() < 1e-3, "{}", a.health(p));
    assert!(healed_by_flasks(&a.events, mate).is_empty(), "yours only");
    assert_eq!(a.world.players[&mate].movement.haste_time, 0.0);
    // The haste is over too.
    a.ticks(2);
    assert!(a.world.knacks.kindle.is_empty(), "done");
    assert_eq!(a.world.players[&p].movement.haste_time, 0.0);
    let e = entity(&a, NetId::player(p)).unwrap();
    assert_eq!(e.flags & player_flags::HASTE, 0);
    a.ticks(120);
    assert_eq!(healed_by_flasks(&a.events, p).len(), 5);
}

#[test]
fn kindle_heals_follow_the_healing_rules() {
    // Never above the effective max: 30 hard damage caps the 40 HP at 20 from 50 down.
    let (mut a, p) = with_knack(Arena::coop(), Knack::KindleFlask);
    a.tuning.combat.hard_damage_recover_rate = 0.0;
    {
        let c = &mut a.world.players.get_mut(&p).unwrap().combat;
        c.damage = 50.0;
        c.hard = 30.0;
    }
    drink(&mut a, p);
    a.ticks(400);
    assert!((a.health(p) - 70.0).abs() < 1e-3, "{}", a.health(p));
    let total: f32 = healed_by_flasks(&a.events, p).iter().sum();
    assert!((total - 20.0).abs() < 1e-3, "{total}");
    // At full health nothing is healed (no events).
    let (mut a, p) = with_knack(Arena::coop(), Knack::KindleFlask);
    drink(&mut a, p);
    a.ticks(400);
    assert!(healed_by_flasks(&a.events, p).is_empty());
    // Death ends it: no heal after a respawn.
    let (mut a, p) = with_knack(Arena::coop(), Knack::KindleFlask);
    a.world.players.get_mut(&p).unwrap().combat.damage = 50.0;
    drink(&mut a, p);
    a.ticks(70);
    let t = a.tuning.clone();
    a.world.kill_player(&t, p, Attack::Dev);
    a.world.respawn_player(&t, p);
    a.world.players.get_mut(&p).unwrap().combat.damage = 50.0;
    a.ticks(300);
    assert_eq!(healed_by_flasks(&a.events, p), vec![8.0]);
}

/// Horizontal speed after running forward (and, with `dash`, dashing) for `ticks`.
fn run(a: &mut Arena, p: PlayerId, ticks: u32, dash: bool) -> f32 {
    let input = a.held.get_mut(&p).unwrap();
    input.forward = 127;
    input.buttons = if dash { Buttons::DASH } else { Buttons::NONE };
    a.ticks(ticks);
    let input = a.held.get_mut(&p).unwrap();
    input.forward = 0;
    input.buttons = Buttons::NONE;
    let v = a.world.players[&p].movement.velocity;
    Vec3::new(v.x, 0.0, v.z).length()
}

#[test]
fn kindle_haste_speeds_up_running_and_the_dash() {
    let (mut a, p) = with_knack(Arena::coop(), Knack::KindleFlask);
    let mv = a.tuning.movement.clone();
    let haste = a.tuning.combat.knacks.kindle_flask.haste_scale;
    let plain = run(&mut a, p, 40, false);
    assert!((plain - mv.ground_speed).abs() < 0.5, "{plain}");
    a.ticks(30);
    drink(&mut a, p);
    let hasted = run(&mut a, p, 40, false);
    assert!(
        (hasted - mv.ground_speed * haste).abs() < 0.5,
        "{hasted} vs {}",
        mv.ground_speed * haste
    );
    // The dash too (a press while running).
    let dash = run(&mut a, p, 2, true);
    assert!(
        (dash - mv.dash_speed * haste).abs() < 0.5,
        "{dash} vs {}",
        mv.dash_speed * haste
    );
    // With a slow as well, both apply.
    a.ticks(30);
    {
        let m = &mut a.world.players.get_mut(&p).unwrap().movement;
        m.slow_time = 1.0;
        m.slow_scale = 0.5;
    }
    let both = run(&mut a, p, 40, false);
    assert!(
        (both - mv.ground_speed * haste * 0.5).abs() < 0.5,
        "{both} vs {}",
        mv.ground_speed * haste * 0.5
    );
}

#[test]
fn kindle_haste_is_predicted_bit_exactly() {
    // The owner's client takes the server's movement state after the drink (reconciliation)
    // and predicts every further input with the shared step: it must match the server's to
    // the bit, through running, jumping, sliding and dashing, until the haste wears off.
    let (mut a, p) = with_knack(Arena::coop(), Knack::KindleFlask);
    drink(&mut a, p);
    let mut predicted = a.world.players[&p].movement;
    assert!(predicted.haste_time > 0.0);
    let t = a.tuning.clone();
    let dt = shared::sim::tick_dt(60);
    for i in 0..400u32 {
        let input = a.held.get_mut(&p).unwrap();
        input.forward = if i % 90 < 70 { 127 } else { -40 };
        input.right = if (i / 50) % 2 == 0 { 60 } else { -60 };
        input.yaw = i as f32 * 0.013;
        let mut b = Buttons::NONE;
        b.set(Buttons::JUMP, i % 37 < 2);
        b.set(Buttons::CROUCH, i % 61 > 50);
        b.set(Buttons::DASH, i % 83 == 10);
        input.buttons = b;
        let sent: PlayerInput = *input;
        a.tick();
        let env = a.world.move_env();
        shared::sim::player_move_step(&mut predicted, &sent, &env, &t.movement, dt);
        assert_eq!(a.world.players[&p].movement, predicted, "input {i}");
    }
    assert_eq!(predicted.haste_time, 0.0, "worn off");
}

#[test]
fn the_kindle_flask_is_allowed_in_pvp_and_heals_the_drinker_only() {
    let (mut a, p) = with_knack(Arena::pvp(), Knack::KindleFlask);
    assert_eq!(
        a.world.players[&p].combat.weapons.knack(),
        Knack::KindleFlask
    );
    let other = a.player(1, Vec3::new(1.0, 0.0, 0.0), 0.0);
    for id in [p, other] {
        a.world.players.get_mut(&id).unwrap().combat.damage = 50.0;
    }
    drink(&mut a, p);
    assert!(a.world.players[&p].movement.haste_time > 0.0);
    a.ticks(310);
    let total: f32 = healed_by_flasks(&a.events, p).iter().sum();
    let f = &a.tuning.combat.knacks.kindle_flask;
    let steps = (f.haste_time / f.haste_heal_interval).round();
    assert!((total - f.haste_heal * steps).abs() < 1e-3, "{total}");
    assert!(healed_by_flasks(&a.events, other).is_empty());
    assert_eq!(a.world.players[&other].movement.haste_time, 0.0);
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
