//! Loadouts and builds in the world (docs/arsenal.md): when a requested loadout takes effect
//! (dead, warm-up, a fresh body, else at the next respawn), forced loadouts (gun game), the
//! replicated weapon / ability / draw, every weapon and ability firing in a live world, kills
//! naming the weapon that fired a shared ability, and prediction staying exact with a
//! non-default loadout.

mod combat_support;
mod weapon_support;

use combat_support::*;
use glam::Vec3;
use shared::input::{Buttons, PlayerInput};
use shared::loadout::{Ability, Knack, Loadout, WeaponBuild, WeaponClass, WeaponKind};
use shared::protocol::{NetFrame, NetId, player_flags, player_weapon};
use shared::sim::{Attack, PlayerId, SimEvent, WeaponState, step_weapons};
use weapon_support::*;

/// Bow, salvo launcher, needle pistol, ember sprayer; Surge on Q (a knack allowed in PvP, so
/// the loadout applies unchanged everywhere).
fn bow_first() -> Loadout {
    Loadout {
        slots: [
            Some(WeaponKind::Longwick.default_build()),
            Some(WeaponKind::Cinderbox.default_build()),
            Some(WeaponKind::Lancet.default_build()),
            Some(WeaponKind::Kiln.default_build()),
        ],
        knack: Knack::Surge,
    }
}

fn kiln_only() -> Loadout {
    Loadout::single(WeaponKind::Kiln.default_build()).with_knack(Knack::Surge)
}

#[test]
fn requests_apply_when_dead_in_warmup_or_fresh_else_at_respawn() {
    let mut a = Arena::pvp();
    let p = a.player(0, Vec3::ZERO, 0.0);
    // A fresh body that hasn't acted yet (just joined) takes it at once.
    a.world.request_loadout(p, bow_first());
    assert_eq!(a.world.loadout(p), bow_first());
    a.ticks(3);

    // In play: stored for the next spawn, the weapons in hand stay.
    a.world.request_loadout(p, kiln_only());
    assert_eq!(a.world.loadout(p), bow_first());
    assert_eq!(a.world.spawn_loadout(p), kiln_only());
    a.world.kill_player(&a.tuning, p, Attack::Dev);
    assert_eq!(
        a.world.loadout(p),
        bow_first(),
        "dying alone changes nothing"
    );
    a.world.respawn_player(&a.tuning, p);
    assert_eq!(a.world.loadout(p), kiln_only());
    assert_eq!(
        a.world.players[&p].combat.weapons.current(),
        WeaponKind::Kiln
    );

    // While dead: at once.
    a.ticks(2);
    a.world.kill_player(&a.tuning, p, Attack::Dev);
    a.world.request_loadout(p, bow_first());
    assert_eq!(a.world.loadout(p), bow_first());
    a.world.respawn_player(&a.tuning, p);
    a.ticks(2);

    // Warm-up: at once, cooldowns afresh, the knack's charges kept (same knack).
    a.world.warmup = true;
    a.set_buttons(p, Buttons::FIRE | Buttons::THROW);
    a.ticks(2);
    let used = a.world.players[&p].combat.weapons.knack_used;
    assert!(used > 0);
    a.world.request_loadout(p, kiln_only());
    let w = a.world.players[&p].combat.weapons;
    assert_eq!(w.loadout, kiln_only());
    assert_eq!(w.primary_cooldown, [0; 4]);
    assert_eq!(w.knack_used, used);
}

#[test]
fn requests_are_sanitised() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let bad = Loadout {
        slots: [
            Some(WeaponBuild {
                weapon: WeaponKind::Quill,
                abilities: [Ability::Gust, Ability::Gust],
            }),
            Some(WeaponKind::Quill.default_build()),
            None,
            None,
        ],
        knack: Knack::Snare,
    };
    a.world.request_loadout(p, bad);
    let l = a.world.loadout(p);
    assert!(l.is_valid(), "{l:?}");
    assert_eq!(l, bad.sanitized());
    assert_eq!(l.slot(1), None, "no weapon twice");
    // Nothing at all: the default weapons (the knack is kept).
    a.world
        .request_loadout(p, Loadout::unarmed().with_knack(Knack::Snare));
    assert_eq!(
        a.world.loadout(p),
        Loadout::default().with_knack(Knack::Snare)
    );
}

#[test]
fn forced_loadouts_win_until_lifted() {
    let mut a = Arena::pvp();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.select(p, 2, 1);
    a.set_buttons(p, Buttons::FIRE);
    a.ticks(5);
    let bow = Loadout::single(WeaponKind::Longwick.default_build()).with_knack(Knack::Surge);
    a.world.force_loadout(p, Some(bow));
    let w = a.world.players[&p].combat.weapons;
    assert_eq!((w.loadout, w.slot), (bow, 0));
    assert_eq!(w.primary_cooldown, [0; 4], "nothing of the old weapons");
    // The input still asks for slot 2, which is empty now: the bow stays in hand.
    a.ticks(3);
    assert_eq!(
        a.world.players[&p].combat.weapons.current(),
        WeaponKind::Longwick
    );
    // A request doesn't override it, not even at a respawn.
    a.world.request_loadout(p, kiln_only());
    a.world.kill_player(&a.tuning, p, Attack::Dev);
    a.world.respawn_player(&a.tuning, p);
    assert_eq!(a.world.loadout(p), bow);
    // Lifted: the request applies at the next spawn.
    a.world.force_loadout(p, None);
    assert_eq!(a.world.loadout(p), bow);
    a.world.kill_player(&a.tuning, p, Attack::Dev);
    a.world.respawn_player(&a.tuning, p);
    assert_eq!(a.world.loadout(p), kiln_only());
}

#[test]
fn only_a_forced_loadout_can_leave_a_player_unarmed() {
    let mut a = Arena::pvp();
    let p = a.player(0, Vec3::ZERO, 0.0);
    // (PvP: the Thorn of these loadouts applies as Surge, the request keeps it.)
    let in_pvp = |l: Loadout| l.with_knack(Knack::Surge);
    // A request for nothing (any client) is sanitised to the default loadout ...
    a.world.request_loadout(p, Loadout::unarmed());
    assert_eq!(a.world.loadout(p), in_pvp(Loadout::default()));
    assert_eq!(a.world.loadout_requests[&p], Loadout::default());
    // ... a forced one is kept: fists only, and through respawns.
    a.select(p, 2, 0);
    a.world.force_loadout(p, Some(Loadout::unarmed()));
    let w = a.world.players[&p].combat.weapons;
    assert!(w.is_unarmed() && w.held().is_none());
    a.world.kill_player(&a.tuning.clone(), p, Attack::Dev);
    a.world.respawn_player(&a.tuning.clone(), p);
    assert_eq!(a.world.loadout(p), in_pvp(Loadout::unarmed()));
    // Replicated as unarmed (the weapon id sentinel); the charge level stays empty.
    let e = player_entity(&a, p);
    assert!(!shared::protocol::player_armed(e.state));
    assert_eq!(player_weapon(e.state).2, 0);
    assert_eq!(
        e.flags & (player_flags::CHARGING | player_flags::DRAWING),
        0
    );
    // Lifting it arms the player at the next spawn.
    a.world.force_loadout(p, None);
    a.world.kill_player(&a.tuning.clone(), p, Attack::Dev);
    a.world.respawn_player(&a.tuning.clone(), p);
    assert_eq!(a.world.loadout(p), in_pvp(Loadout::default()));
    assert!(shared::protocol::player_armed(player_entity(&a, p).state));
}

#[test]
fn unarmed_players_punch_and_their_prediction_stays_exact() {
    let mut a = Arena::pvp();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let v = a.player(1, Vec3::new(0.0, 0.0, -1.8), 0.0);
    a.world.players.get_mut(&p).unwrap().combat.god = true;
    // A branding arrow still in flight when the weapons go: its kill names the bow, never
    // the default build an unarmed state falls back to.
    let bow = WeaponBuild {
        weapon: WeaponKind::Longwick,
        abilities: [Ability::Brand, Ability::TripleNock],
    };
    a.world.force_loadout(p, Some(Loadout::single(bow)));
    a.world.force_loadout(p, Some(Loadout::unarmed()));
    assert_eq!(
        a.world
            .attack_weapon(shared::sim::Owner::Player(p), Attack::Brand),
        Attack::Brand.weapon()
    );
    let chest = a.world.players[&v].movement.position + Vec3::Y * 1.2;
    a.aim(p, chest);
    let t = a.tuning.clone();
    let mut predicted = a.world.players[&p].combat.weapons;
    let mut events = Vec::new();
    for i in 0..240u32 {
        let input = a.held.get_mut(&p).unwrap();
        input.weapon = (i / 20 % 5) as u8;
        input.weapon_mode = (i / 30 % 2) as u8;
        let mut b = Buttons::FIRE | Buttons::ALT_FIRE;
        b.set(Buttons::PUNCH, i % 25 == 0);
        input.buttons = b;
        let sent: PlayerInput = *input;
        events.extend(a.tick());
        step_weapons(&mut predicted, &sent, true, &t.combat, 60);
        assert_eq!(a.world.players[&p].combat.weapons, predicted, "input {i}");
    }
    assert!(!events.iter().any(|e| matches!(e, SimEvent::Shot(_))));
    let punches = damage_to(&events, shared::sim::HitTarget::Player(v));
    assert!(punches > 0.0, "the fist still lands");
}

#[test]
fn players_without_a_body_keep_their_request_and_leaving_forgets_it() {
    let mut a = Arena::coop();
    let id = PlayerId(5);
    a.world.request_loadout(id, bow_first());
    assert_eq!(a.world.loadout(id), bow_first());
    a.world.add_player(id, "late");
    assert_eq!(a.world.players[&id].combat.weapons.loadout, bow_first());
    a.world.remove_player(id);
    assert!(a.world.loadout_requests.is_empty());
    assert_eq!(a.world.loadout(id), Loadout::default());
    // A level restart hands everyone their loadout too.
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.ticks(2);
    a.world.request_loadout(p, kiln_only());
    a.world.restart_level();
    assert_eq!(a.world.loadout(p), kiln_only());
}

fn player_entity(a: &Arena, p: PlayerId) -> shared::protocol::EntityState {
    let mut frame = NetFrame::new();
    a.world.replicate(&a.tuning, &mut frame);
    frame.get(NetId::player(p)).expect("player replicated")
}

#[test]
fn the_player_entity_carries_weapon_ability_and_draw() {
    let mut a = Arena::pvp();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.request_loadout(p, bow_first());
    a.select(p, 0, 1);
    a.set_buttons(p, Buttons::FIRE);
    a.ticks(20);
    let e = player_entity(&a, p);
    let (weapon, ability, level) = player_weapon(e.state);
    assert_eq!(
        (weapon, ability),
        (WeaponKind::Longwick, Ability::PinningArrow)
    );
    let expected = 20.0 / 45.0 * 255.0;
    assert!((level as f32 - expected).abs() <= 3.0, "{level}");
    assert_ne!(e.flags & player_flags::DRAWING, 0);
    a.set_buttons(p, Buttons::NONE);
    a.tick();
    let e = player_entity(&a, p);
    assert_eq!(e.flags & player_flags::DRAWING, 0);
    assert_eq!(player_weapon(e.state).2, 0);
    // Switching shows the new weapon.
    a.select(p, 2, 0);
    a.tick();
    let (weapon, ability, _) = player_weapon(player_entity(&a, p).state);
    assert_eq!((weapon, ability), (WeaponKind::Lancet, Ability::Skewer));
}

/// The attack showed up as a shot, damage, an explosion or a nova.
fn seen(events: &[SimEvent], attack: Attack) -> bool {
    events.iter().any(|e| match e {
        SimEvent::Shot(s) => s.attack == attack,
        SimEvent::Swing { attack: a, .. } => *a == attack,
        SimEvent::Damage(d) => d.info.attack == attack,
        SimEvent::Explosion { attack: x, .. } => *x == attack,
        SimEvent::Discharge { .. } => attack == Attack::Discharge,
        _ => false,
    })
}

#[test]
fn every_weapon_and_ability_fires_in_a_live_world() {
    for w in WeaponKind::ALL {
        for &ability in w.abilities() {
            let other = *w.abilities().iter().find(|o| **o != ability).unwrap();
            let build = WeaponBuild {
                weapon: w,
                abilities: [ability, other],
            };
            let mut a = Arena::coop();
            let p = a.player(0, Vec3::ZERO, 0.0);
            a.world.players.get_mut(&p).unwrap().combat.god = true;
            a.world.request_loadout(p, Loadout::single(build));
            // Melee: in reach.
            let range = if w.class() == WeaponClass::Melee {
                2.5
            } else {
                10.0
            };
            let target = dummy(&mut a, Vec3::new(0.0, 0.0, -range), 0.0, 1.0e5);
            let body = dummy_body(&a, target);
            a.aim(p, body);
            let mut events = Vec::new();
            for i in 0..150 {
                let b = match i {
                    0..60 => Buttons::FIRE | Buttons::ALT_FIRE,
                    62..64 => Buttons::ALT_FIRE,
                    100..102 => Buttons::FIRE,
                    _ => Buttons::NONE,
                };
                a.set_buttons(p, b);
                events.extend(a.tick());
            }
            assert!(seen(&events, w.primary()), "{w:?} primary");
            assert!(seen(&events, ability.attack()), "{w:?} {ability:?}");
            assert!(
                damage_to(&events, shared::sim::HitTarget::Entity(target)) > 0.0,
                "{w:?} {ability:?} hit nothing"
            );
        }
    }
}

#[test]
fn kills_name_the_weapon_that_fired_a_shared_ability() {
    // Brand on the bow: an armed branding arrow.
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let build = WeaponBuild {
        weapon: WeaponKind::Longwick,
        abilities: [Ability::Brand, Ability::EmberArrow],
    };
    a.world.request_loadout(p, Loadout::single(build));
    let target = dummy(&mut a, Vec3::new(0.0, 0.0, -12.0), 0.0, 40.0);
    let body = dummy_body(&a, target);
    a.aim(p, body);
    let mut events = a.tap(p, Buttons::ALT_FIRE, 0);
    a.set_buttons(p, Buttons::FIRE);
    events.extend(a.ticks(50));
    a.set_buttons(p, Buttons::NONE);
    events.extend(a.ticks(30));
    let k = kills(&events);
    assert_eq!(k.len(), 1, "{events:?}");
    assert_eq!(k[0].attack, Attack::Brand);
    assert_eq!(k[0].weapon, Some(WeaponKind::Longwick));
    assert_eq!(Attack::Brand.weapon(), None, "Brand alone can't tell");

    // The Quill's own Brand.
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let target = dummy(&mut a, Vec3::new(0.0, 0.0, -12.0), 0.0, 5.0);
    let body = dummy_body(&a, target);
    a.aim(p, body);
    let k = kills(&a.tap(p, Buttons::ALT_FIRE, 2));
    assert_eq!(
        (k[0].attack, k[0].weapon),
        (Attack::Brand, Some(WeaponKind::Quill))
    );
}

#[test]
fn prediction_stays_exact_with_a_custom_loadout() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().combat.god = true;
    a.world.request_loadout(p, bow_first());
    dummy(&mut a, Vec3::new(0.0, 0.0, -12.0), 0.0, 1.0e6);
    dummy(&mut a, Vec3::new(4.0, 0.0, -9.0), 0.0, 1.0e6);
    let t = a.tuning.clone();
    let mut predicted = a.world.players[&p].combat.weapons;
    for i in 0..1200u32 {
        let phase = i / 75;
        let input = a.held.get_mut(&p).unwrap();
        input.weapon = (phase % 5) as u8; // 4 = out of range: keeps the weapon
        input.weapon_mode = ((phase / 5) % 2) as u8;
        input.yaw = (i as f32 * 0.013).sin() * 0.4;
        let mut b = Buttons::NONE;
        b.set(Buttons::FIRE, i % 31 < 20);
        b.set(Buttons::ALT_FIRE, i % 47 < 12 || i % 47 == 30);
        b.set(Buttons::PUNCH, i % 50 == 0);
        b.set(Buttons::THROW, i % 70 == 3);
        input.buttons = b;
        let sent: PlayerInput = *input;
        a.tick();
        step_weapons(&mut predicted, &sent, true, &t.combat, 60);
        assert_eq!(a.world.players[&p].combat.weapons, predicted, "input {i}");
    }
    assert_ne!(predicted, WeaponState::new(bow_first()));
}
