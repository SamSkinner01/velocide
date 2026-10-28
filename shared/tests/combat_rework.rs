//! The rework (`docs/rework.md`) in a live world, arsenal-a's set: the Lamplighter (focus,
//! damage by the focus, the full focus's pierce, the replicated focus), the Wickline (the zip,
//! the yank, a heavy target, a miss), the Tally (marks in the owner's state, a shot each,
//! nothing tallied), the Waxwork (the decoy Moulds prefer, its burst, PvP replication), the
//! Blink Arrow (the step and its search, the window), the Beacon Arrow (the reveal and its
//! ×1.15), the Hurl's return (both legs, the blade passing through, the head turning back, the
//! catch) and the Riposte (blocks, the stagger, the counter, nothing blocked).

mod combat_support;
mod weapon_support;

use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::level::LevelKind;
use shared::loadout::{Ability, Loadout, WeaponBuild, WeaponKind};
use shared::protocol::{NetFrame, NetId, enemy_flags, kind, player_flags, projectile_flags};
use shared::sim::combat::secs_to_ticks;
use shared::sim::{
    Attack, DamageEvent, DamageFlags, DamageInfo, HitPart, HitTarget, Owner, PlayerId,
    ProjectileKind, SimEvent,
};
use shared::tuning::Tuning;
use weapon_support::*;

const BIG: f32 = 1.0e5;

/// A co-op arena with one god-mode player at the origin looking down -Z, carrying `weapon`
/// with `abilities`.
fn armed(weapon: WeaponKind, abilities: [Ability; 2]) -> (Arena, PlayerId) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    arm(&mut a, p, weapon, abilities);
    (a, p)
}

fn arm(a: &mut Arena, p: PlayerId, weapon: WeaponKind, abilities: [Ability; 2]) {
    a.world.players.get_mut(&p).unwrap().combat.god = true;
    let build = WeaponBuild { weapon, abilities };
    assert!(build.is_valid(), "{build:?}");
    a.world.force_loadout(p, Some(Loadout::single(build)));
}

fn c() -> shared::tuning::CombatTuning {
    Tuning::default().combat
}

fn damage_events(events: &[SimEvent]) -> Vec<DamageEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Damage(d) => Some(*d),
            _ => None,
        })
        .collect()
}

fn hits_on(events: &[SimEvent], target: HitTarget) -> Vec<DamageEvent> {
    damage_events(events)
        .into_iter()
        .filter(|d| d.target == target && d.blocked.is_none())
        .collect()
}

/// Holds `buttons` for `ticks`, then releases them for `after` ticks.
fn hold(a: &mut Arena, p: PlayerId, buttons: Buttons, ticks: u32, after: u32) -> Vec<SimEvent> {
    a.set_buttons(p, buttons);
    let mut ev = a.ticks(ticks);
    a.set_buttons(p, Buttons::NONE);
    ev.extend(a.ticks(after));
    ev
}

fn replicated(a: &Arena, id: NetId) -> shared::protocol::EntityState {
    let mut frame = NetFrame::default();
    a.world.replicate(&a.tuning, &mut frame);
    frame.get(id).expect("replicated")
}

fn cooldown(a: &Arena, p: PlayerId) -> u16 {
    let w = &a.world.players[&p].combat.weapons;
    w.alt_cooldown[w.slot as usize]
}

// ---- the Lamplighter --------------------------------------------------------------------------

#[test]
fn the_lamplighter_deals_its_focus_and_a_full_focus_pierces() {
    let l = c().lamplighter;
    let focus = secs_to_ticks(l.focus_time, 60, 1) as u32;
    let shot = |ticks: u32, head: bool| {
        let (mut a, p) = armed(
            WeaponKind::Lamplighter,
            [Ability::Wickline, Ability::Waxwork],
        );
        let front = dummy(&mut a, Vec3::new(0.0, 0.0, -20.0), 0.0, BIG);
        let back = dummy(&mut a, Vec3::new(0.0, 0.0, -24.0), 0.0, BIG);
        a.ticks(2);
        let at = if head {
            dummy_head(&a, front)
        } else {
            dummy_body(&a, front)
        };
        a.aim(p, at);
        let ev = hold(&mut a, p, Buttons::FIRE, ticks, 2);
        (
            hits_on(&ev, HitTarget::Entity(front)),
            hits_on(&ev, HitTarget::Entity(back)),
        )
    };
    // A tap: the minimum, no pierce.
    let (front, back) = shot(1, false);
    assert_eq!(front.len(), 1);
    assert!(
        (front[0].dealt - l.min_damage).abs() < 2.0,
        "{}",
        front[0].dealt
    );
    assert!(back.is_empty());
    // Half a focus: in between, exact (no spread).
    let (front, _) = shot(focus / 2, false);
    let half = l.min_damage + (l.max_damage - l.min_damage) * 0.5;
    assert!((front[0].dealt - half).abs() < 3.0, "{}", front[0].dealt);
    // A full focus: the maximum, then the one behind at ×0.7, PIERCED.
    let (front, back) = shot(focus + 5, false);
    assert_eq!(front[0].dealt, l.max_damage);
    assert_eq!(front[0].info.attack, Attack::Lamplighter);
    assert_eq!(back.len(), 1);
    assert!((back[0].dealt - l.max_damage * l.pierce_falloff).abs() < 1e-3);
    assert!(back[0].info.flags.contains(DamageFlags::PIERCED));
    // A full focus to the head: ×2 as any ray.
    let (front, _) = shot(focus + 5, true);
    assert_eq!(front[0].dealt, l.max_damage * 2.0);
    assert_ne!(front[0].info.part, HitPart::Body);
}

#[test]
fn others_see_the_focus_like_the_bows_draw() {
    let (mut a, p) = armed(
        WeaponKind::Lamplighter,
        [Ability::Wickline, Ability::Waxwork],
    );
    let focus = secs_to_ticks(c().lamplighter.focus_time, 60, 1) as u32;
    a.set_buttons(p, Buttons::FIRE);
    a.ticks(focus / 2);
    let e = replicated(&a, NetId::player(p));
    assert_ne!(e.flags & player_flags::DRAWING, 0);
    let (weapon, _, level) = shared::protocol::player_weapon(e.state);
    assert_eq!(weapon, WeaponKind::Lamplighter);
    assert!((100..=156).contains(&level), "about half: {level}");
    a.ticks(focus);
    let (_, _, level) = shared::protocol::player_weapon(replicated(&a, NetId::player(p)).state);
    assert_eq!(level, 255);
    a.set_buttons(p, Buttons::NONE);
    a.tick();
    assert_eq!(
        replicated(&a, NetId::player(p)).flags & player_flags::DRAWING,
        0
    );
}

// ---- Wickline ---------------------------------------------------------------------------------

#[test]
fn the_wickline_zips_to_the_level() {
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![brush([-10.0, 0.0, -21.0], [10.0, 12.0, -20.0])],
    ));
    let p = a.player(0, Vec3::ZERO, 0.0);
    arm(
        &mut a,
        p,
        WeaponKind::Quill,
        [Ability::Brand, Ability::Wickline],
    );
    a.select(p, 0, 1);
    a.ticks(2);
    a.aim(p, Vec3::new(0.0, 6.0, -20.0));
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    let line = ev.iter().find_map(|e| match e {
        SimEvent::Wickline { to, anchored, .. } => Some((*to, *anchored)),
        _ => None,
    });
    let (to, anchored) = line.expect("the line");
    assert!(anchored && (to.z + 20.0).abs() < 0.2, "{to}");
    let v = a.world.players[&p].movement.velocity;
    let dir = (to - a.eye(p)).normalize();
    assert!(v.dot(dir) > c().wickline.zip_speed * 0.9, "zipped: {v}");
    assert!(v.y >= c().wickline.zip_lift);
    // A full cooldown.
    assert!(cooldown(&a, p) > secs_to_ticks(3.5, 60, 1));
}

#[test]
fn the_wickline_yanks_a_light_target_and_a_heavy_one_yanks_you() {
    let (mut a, p) = armed(WeaponKind::Quill, [Ability::Brand, Ability::Wickline]);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let light = dummy(&mut a, Vec3::new(0.0, 0.0, -15.0), 0.0, BIG.min(200.0));
    a.select(p, 0, 1);
    a.ticks(2);
    let body = dummy_body(&a, light);
    a.aim(p, body);
    let ev = a.tap(p, Buttons::ALT_FIRE, 1);
    let hits = hits_on(&ev, HitTarget::Entity(light));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].dealt, c().wickline.damage);
    assert_eq!(hits[0].info.attack, Attack::Wickline);
    let v = a.world.enemies[&light].velocity;
    assert!(v.z > 10.0, "pulled towards the caster: {v}");
    assert!(a.world.players[&p].movement.velocity.length() < 1.0);
    // A heavy one (base health 300+): the caster goes to it instead, no damage to it.
    let (mut a, p) = armed(WeaponKind::Quill, [Ability::Brand, Ability::Wickline]);
    let heavy = dummy(&mut a, Vec3::new(0.0, 0.0, -15.0), 0.0, 800.0);
    a.select(p, 0, 1);
    a.ticks(2);
    let body = dummy_body(&a, heavy);
    a.aim(p, body);
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert!(hits_on(&ev, HitTarget::Entity(heavy)).is_empty());
    assert!(a.world.players[&p].movement.velocity.z < -10.0);
}

#[test]
fn a_wickline_that_misses_serves_half_its_cooldown() {
    let (mut a, p) = armed(WeaponKind::Quill, [Ability::Brand, Ability::Wickline]);
    a.select(p, 0, 1);
    a.ticks(2);
    // Straight up at the open sky: nothing within 34 m.
    a.held.get_mut(&p).unwrap().pitch = 1.5;
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert!(ev.iter().any(|e| matches!(
        e,
        SimEvent::Wickline {
            anchored: false,
            ..
        }
    )));
    assert_eq!(a.world.players[&p].movement.velocity, Vec3::ZERO);
    let half = secs_to_ticks(c().wickline.cooldown * c().wickline.miss_cooldown, 60, 0);
    assert!(
        cooldown(&a, p) <= half && cooldown(&a, p) + 2 >= half,
        "{}",
        cooldown(&a, p)
    );
}

#[test]
fn a_pvp_wickline_yanks_a_player_at_half_strength() {
    let mut a = Arena::pvp();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let v = a.player(1, Vec3::new(0.0, 0.0, -12.0), 0.0);
    a.world.players.get_mut(&v).unwrap().combat.protection = 0;
    arm(
        &mut a,
        p,
        WeaponKind::Quill,
        [Ability::Brand, Ability::Wickline],
    );
    a.select(p, 0, 1);
    a.ticks(2);
    let chest = a.world.players[&v].movement.position + Vec3::Y * 1.0;
    a.aim(p, chest);
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert_eq!(damage_to(&ev, HitTarget::Player(v)), c().wickline.damage);
    let pull = a.world.players[&v].movement.velocity;
    let expected = c().wickline.yank_speed * c().wickline.player_yank_scale;
    assert!((pull.length() - expected).abs() < 1.0, "{pull}");
    assert!(pull.z > 0.0);
}

// ---- Tally ------------------------------------------------------------------------------------

#[test]
fn a_tally_marks_what_the_crosshair_passes_and_shoots_each_one() {
    let (mut a, p) = armed(WeaponKind::Lancet, [Ability::Tally, Ability::Waxwork]);
    let left = dummy(&mut a, Vec3::new(-4.0, 0.0, -15.0), 0.0, BIG);
    let right = dummy(&mut a, Vec3::new(4.0, 0.0, -15.0), 0.0, BIG);
    let aside = dummy(&mut a, Vec3::new(15.0, 0.0, -15.0), 0.0, BIG);
    a.ticks(2);
    // Sweep from the left one to the right one over 30 ticks, holding ALT.
    let (l, r) = (dummy_head(&a, left), dummy_body(&a, right));
    a.set_buttons(p, Buttons::ALT_FIRE);
    for i in 0..=30 {
        let at = l.lerp(r, i as f32 / 30.0);
        a.aim(p, at);
        a.tick();
    }
    let tallied: Vec<NetId> = a.world.players[&p]
        .combat
        .tallied
        .iter()
        .flatten()
        .copied()
        .collect();
    assert_eq!(
        tallied,
        vec![left, right],
        "the owner sees the marks in order"
    );
    assert!(!tallied.contains(&aside), "never passed over");
    assert_ne!(
        replicated(&a, NetId::player(p)).flags & player_flags::TALLYING,
        0
    );
    // The release: one shot at each, where the crosshair passed (the left one's head).
    a.set_buttons(p, Buttons::NONE);
    let ev = a.ticks(2);
    let l_hits = hits_on(&ev, HitTarget::Entity(left));
    let r_hits = hits_on(&ev, HitTarget::Entity(right));
    assert_eq!((l_hits.len(), r_hits.len()), (1, 1));
    assert_eq!(
        l_hits[0].dealt,
        c().tally.damage * 2.0,
        "the head it passed over"
    );
    assert_eq!(r_hits[0].dealt, c().tally.damage);
    assert_eq!(l_hits[0].info.attack, Attack::Tally);
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::TallyFired { ends, .. } if ends.len() == 2))
    );
    assert!(
        a.world.players[&p]
            .combat
            .tallied
            .iter()
            .all(Option::is_none)
    );
    assert!(cooldown(&a, p) + 3 >= secs_to_ticks(c().tally.cooldown, 60, 1));
}

#[test]
fn a_tally_with_nothing_tallied_serves_a_quarter_of_its_cooldown() {
    let (mut a, p) = armed(WeaponKind::Lancet, [Ability::Tally, Ability::Waxwork]);
    a.ticks(2);
    let ev = hold(&mut a, p, Buttons::ALT_FIRE, 20, 1);
    assert!(!ev.iter().any(|e| matches!(e, SimEvent::TallyFired { .. })));
    let quarter = secs_to_ticks(c().tally.cooldown * c().tally.empty_cooldown, 60, 0);
    assert!(cooldown(&a, p) <= quarter && cooldown(&a, p) + 3 >= quarter);
}

#[test]
fn a_pvp_tally_holds_three_marks() {
    let mut a = Arena::pvp();
    a.tuning = Tuning::default().for_mode(shared::lobby::GameMode::Deathmatch);
    let p = a.player(0, Vec3::ZERO, 0.0);
    arm(
        &mut a,
        p,
        WeaponKind::Lancet,
        [Ability::Tally, Ability::Waxwork],
    );
    let foes: Vec<PlayerId> = (1..=4)
        .map(|i| a.player(i, Vec3::new(i as f32 * 3.0 - 7.5, 0.0, -15.0), 0.0))
        .collect();
    a.ticks(2);
    let chest = |a: &Arena, id: PlayerId| a.world.players[&id].movement.position + Vec3::Y;
    a.set_buttons(p, Buttons::ALT_FIRE);
    for i in 0..=40 {
        let at = chest(&a, foes[0]).lerp(chest(&a, foes[3]), i as f32 / 40.0);
        a.aim(p, at);
        a.tick();
    }
    let n = a.world.players[&p].combat.tallied.iter().flatten().count();
    assert_eq!(n, 3);
}

// ---- Waxwork ----------------------------------------------------------------------------------

/// The shipped Gnawer (it chases and bites) at `at`, facing the origin.
fn gnawer(a: &mut Arena, at: Vec3) -> NetId {
    let yaw = (at.x).atan2(at.z);
    a.world.spawn_enemy("gnawer", at, yaw).expect("a gnawer")
}

#[test]
fn moulds_prefer_the_waxwork_and_it_bursts_when_destroyed() {
    let (mut a, p) = armed(WeaponKind::Lancet, [Ability::Tally, Ability::Waxwork]);
    a.world.players.get_mut(&p).unwrap().combat.god = true;
    let g = gnawer(&mut a, Vec3::new(0.0, 0.0, -12.0));
    a.ticks(60);
    assert_eq!(a.world.enemies[&g].target, Some(p), "it hunts the player");
    // The double where the player stood; the player walks off to the side.
    a.select(p, 0, 1);
    a.tap(p, Buttons::ALT_FIRE, 0);
    let double = a.world.waxwork(p).map(|w| w.id).expect("the double stands");
    let e = replicated(&a, double);
    assert_eq!(e.kind, kind::WAXWORK);
    assert_eq!(e.owner, p.0 as u16);
    a.world.players.get_mut(&p).unwrap().movement.position = Vec3::new(8.0, 0.0, 6.0);
    a.ticks(90);
    let decoy = shared::sim::abilities::waxwork::decoy_id(p);
    assert_eq!(a.world.enemies[&g].target, Some(decoy), "lured");
    assert_ne!(replicated(&a, g).flags & enemy_flags::LURED, 0);
    // It bites the double, not the player, until the double breaks: the burst.
    let ev = a.ticks(600);
    assert!(a.world.waxwork(p).is_none(), "destroyed");
    assert!(ev.iter().any(|e| matches!(
        e,
        SimEvent::Explosion {
            attack: Attack::Waxwork,
            ..
        }
    )));
    let burst = hits_on(&ev, HitTarget::Entity(g));
    assert!(burst.iter().any(|h| h.info.attack == Attack::Waxwork));
    assert!(
        burst
            .iter()
            .any(|h| h.info.attack == Attack::Waxwork && h.info.flags == DamageFlags::BURN),
        "the hot wax burns"
    );
    // Its cooldown started then.
    assert!(a.world.players[&p].combat.weapons.live[0] == 0);
}

#[test]
fn the_waxwork_bursts_at_its_end_and_never_hurts_its_owner() {
    let (mut a, p) = armed(WeaponKind::Lancet, [Ability::Tally, Ability::Waxwork]);
    a.world.players.get_mut(&p).unwrap().combat.god = false;
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let near = dummy(&mut a, Vec3::new(2.0, 0.0, 0.0), 0.0, BIG);
    a.select(p, 0, 1);
    a.tap(p, Buttons::ALT_FIRE, 0);
    let x = c().waxwork;
    let ev = a.ticks(secs_to_ticks(x.duration, 60, 1) as u32 + 2);
    assert!(a.world.waxwork(p).is_none());
    let burst = hits_on(&ev, HitTarget::Entity(near));
    assert!(!burst.is_empty() && burst[0].info.attack == Attack::Waxwork);
    assert!(burst[0].dealt > x.burst_damage * 0.3 && burst[0].dealt <= x.burst_damage);
    assert!(a.world.is_burning(HitTarget::Entity(near)));
    assert_eq!(
        a.health(p),
        c().max_health,
        "the owner standing in it is spared"
    );
    // The live ability ran out with it: the cooldown runs.
    assert!(cooldown(&a, p) > 0);
}

#[test]
fn pvp_opponents_shoot_the_double_and_its_owners_shots_pass_it() {
    let mut a = Arena::pvp();
    let owner = a.player(0, Vec3::ZERO, 0.0);
    let foe = a.player(1, Vec3::new(0.0, 0.0, -15.0), std::f32::consts::PI);
    arm(
        &mut a,
        owner,
        WeaponKind::Quill,
        [Ability::Waxwork, Ability::Brand],
    );
    a.world.players.get_mut(&owner).unwrap().combat.god = false;
    a.tap(owner, Buttons::ALT_FIRE, 0);
    assert!(a.world.waxwork(owner).is_some());
    // The owner steps away; the foe shoots the double's chest.
    a.world.players.get_mut(&owner).unwrap().movement.position = Vec3::new(6.0, 0.0, 4.0);
    a.ticks(5);
    let chest = Vec3::new(0.0, 1.0, 0.0);
    a.aim(foe, chest);
    let ev = a.tap(foe, Buttons::FIRE, 2);
    let decoy = shared::sim::abilities::waxwork::decoy_id(owner);
    assert_eq!(damage_to(&ev, HitTarget::Player(decoy)), c().quill.damage);
    assert_eq!(damage_to(&ev, HitTarget::Player(owner)), 0.0);
    let w = a.world.waxwork(owner).unwrap();
    assert_eq!(w.health, c().waxwork.health - c().quill.damage);
    // The owner's own shot passes through it.
    a.aim(owner, Vec3::new(0.0, 1.0, 0.0));
    let ev = a.tap(owner, Buttons::FIRE, 2);
    assert_eq!(damage_to(&ev, HitTarget::Player(decoy)), 0.0);
}

// ---- Blink Arrow ------------------------------------------------------------------------------

#[test]
fn the_blink_arrow_steps_you_in_front_of_where_it_stuck() {
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![brush([-10.0, 0.0, -31.0], [10.0, 12.0, -30.0])],
    ));
    let p = a.player(0, Vec3::ZERO, 0.0);
    arm(
        &mut a,
        p,
        WeaponKind::Longwick,
        [Ability::BlinkArrow, Ability::PinningArrow],
    );
    a.ticks(2);
    a.aim(p, Vec3::new(0.0, 3.0, -30.0));
    a.tap(p, Buttons::ALT_FIRE, 0);
    let ev = hold(&mut a, p, Buttons::FIRE, 40, 30);
    assert!(!ev.iter().any(|e| matches!(e, SimEvent::Blink { .. })));
    let arrow = a
        .world
        .projectiles
        .values()
        .find(|p| p.kind == ProjectileKind::BlinkArrow)
        .expect("stuck in the wall");
    assert!(arrow.stuck.is_some());
    assert_ne!(arrow.entity_state().flags & projectile_flags::STUCK, 0);
    // ALT: the step, in front of the wall, moving on along the flight.
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    let to = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Blink { to, .. } => Some(*to),
            _ => None,
        })
        .expect("blinked");
    let at = a.world.players[&p].movement.position;
    assert_eq!(at, to);
    assert!(
        at.z > -30.0 && at.z < -28.5,
        "just in front of the wall: {at}"
    );
    let v = a.world.players[&p].movement.velocity;
    assert!(
        (v.length() - c().blink_arrow.exit_speed).abs() < 0.5 && v.z < 0.0,
        "{v}"
    );
    assert!(
        !a.world
            .projectiles
            .values()
            .any(|p| p.kind == ProjectileKind::BlinkArrow)
    );
    // The cooldown runs from the blink.
    assert!(cooldown(&a, p) > secs_to_ticks(c().blink_arrow.cooldown - 0.2, 60, 1));
}

#[test]
fn a_blink_arrow_left_alone_runs_out_after_its_window() {
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![brush([-10.0, 0.0, -31.0], [10.0, 12.0, -30.0])],
    ));
    let p = a.player(0, Vec3::ZERO, 0.0);
    arm(
        &mut a,
        p,
        WeaponKind::Longwick,
        [Ability::BlinkArrow, Ability::PinningArrow],
    );
    a.ticks(2);
    a.aim(p, Vec3::new(0.0, 3.0, -30.0));
    a.tap(p, Buttons::ALT_FIRE, 0);
    hold(&mut a, p, Buttons::FIRE, 40, 30);
    a.ticks(secs_to_ticks(c().blink_arrow.window, 60, 1) as u32);
    assert!(
        !a.world
            .projectiles
            .values()
            .any(|p| p.kind == ProjectileKind::BlinkArrow)
    );
    let before = a.world.players[&p].movement.position;
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert!(!ev.iter().any(|e| matches!(e, SimEvent::Blink { .. })));
    assert!(a.world.players[&p].movement.position.distance(before) < 0.01);
    assert!(cooldown(&a, p) > 0);
}

#[test]
fn a_blink_into_a_tight_spot_lands_where_the_body_fits() {
    // A low gap under a slab at the wall: the arrow sticks 0.5 m above the floor where a
    // standing body can't be; the step searches back along its path.
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![
            brush([-10.0, 0.0, -31.0], [10.0, 12.0, -30.0]),
            brush([-10.0, 1.0, -30.0], [10.0, 1.4, -27.0]),
        ],
    ));
    let p = a.player(0, Vec3::ZERO, 0.0);
    arm(
        &mut a,
        p,
        WeaponKind::Longwick,
        [Ability::BlinkArrow, Ability::PinningArrow],
    );
    a.ticks(2);
    a.aim(p, Vec3::new(0.0, 0.5, -30.0));
    a.tap(p, Buttons::ALT_FIRE, 0);
    hold(&mut a, p, Buttons::FIRE, 40, 30);
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    let to = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Blink { to, .. } => Some(*to),
            _ => None,
        })
        .expect("blinked");
    // Out from under the slab (standing), or under it crouched: never inside it.
    let m = &a.tuning.movement;
    let s = &a.world.players[&p].movement;
    let (center, half, radius) = s.capsule(m);
    assert!(
        !a.world.collision.world.intersects_capsule(
            center,
            half,
            radius,
            engine::physics::MASK_MOVEMENT
        ),
        "the body fits at {to}"
    );
}

// ---- Beacon Arrow -----------------------------------------------------------------------------

#[test]
fn a_beacon_reveals_what_is_near_it_and_they_take_more_damage() {
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![brush([-10.0, 0.0, -31.0], [10.0, 12.0, -30.0])],
    ));
    let p = a.player(0, Vec3::ZERO, 0.0);
    arm(
        &mut a,
        p,
        WeaponKind::Longwick,
        [Ability::Brand, Ability::BeaconArrow],
    );
    let near = dummy(&mut a, Vec3::new(4.0, 0.0, -26.0), 0.0, BIG);
    let far = dummy(&mut a, Vec3::new(-20.0, 0.0, 5.0), 0.0, BIG);
    a.ticks(2);
    a.select(p, 0, 1);
    a.aim(p, Vec3::new(0.0, 2.0, -30.0));
    a.tap(p, Buttons::ALT_FIRE, 0);
    hold(&mut a, p, Buttons::FIRE, 40, 40);
    assert!(a.world.is_revealed(HitTarget::Entity(near)));
    assert!(
        !a.world.is_revealed(HitTarget::Entity(far)),
        "out of its radius"
    );
    assert!(
        !a.world.is_revealed(HitTarget::Player(p)),
        "never its owner"
    );
    assert_ne!(replicated(&a, near).flags & enemy_flags::REVEALED, 0);
    // ×1.15 from a player's hit (not from the world's).
    let t = a.tuning.clone();
    let hit = |a: &mut Arena, source| {
        let info = DamageInfo::new(20.0, source, Attack::Quill).flags(DamageFlags::HITSCAN);
        a.world.apply_damage(&t, HitTarget::Entity(near), info)
    };
    let ev = hit(&mut a, Owner::Player(p));
    assert!(ev.info.flags.contains(DamageFlags::REVEALED));
    assert!((ev.dealt - 20.0 * c().beacon_arrow.damage_multiplier).abs() < 1e-3);
    let ev = hit(&mut a, Owner::World);
    assert_eq!(ev.dealt, 20.0);
    // It goes out after its duration.
    a.ticks(secs_to_ticks(c().beacon_arrow.duration, 60, 1) as u32);
    assert!(!a.world.is_revealed(HitTarget::Entity(near)));
}

#[test]
fn a_beacon_in_a_target_rides_along() {
    let (mut a, p) = armed(WeaponKind::Longwick, [Ability::Brand, Ability::BeaconArrow]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -20.0), 0.0, BIG);
    let other = dummy(&mut a, Vec3::new(30.0, 0.0, -20.0), 0.0, BIG);
    a.ticks(2);
    a.select(p, 0, 1);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    a.tap(p, Buttons::ALT_FIRE, 0);
    let ev = hold(&mut a, p, Buttons::FIRE, 40, 30);
    assert!(!hits_on(&ev, HitTarget::Entity(d)).is_empty());
    assert!(a.world.is_revealed(HitTarget::Entity(d)));
    assert!(!a.world.is_revealed(HitTarget::Entity(other)));
    // The dummy moves next to the other one: the beacon goes with it.
    a.world.enemies.get_mut(&d).unwrap().position = Vec3::new(28.0, 0.0, -20.0);
    a.ticks(2);
    assert!(a.world.is_revealed(HitTarget::Entity(other)));
}

// ---- Hurl -------------------------------------------------------------------------------------

fn hurl_hits(ev: &[SimEvent], target: NetId) -> Vec<f32> {
    hits_on(ev, HitTarget::Entity(target))
        .iter()
        .filter(|h| h.info.attack == Attack::Hurl)
        .map(|h| h.dealt)
        .collect()
}

#[test]
fn the_trimmers_blade_comes_back_through_everything_it_passed() {
    let (mut a, p) = armed(WeaponKind::Trimmer, [Ability::Lunge, Ability::Hurl]);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let first = dummy(&mut a, Vec3::new(0.0, 0.0, -5.0), 0.0, BIG);
    let second = dummy(&mut a, Vec3::new(0.0, 0.0, -9.0), 0.0, BIG);
    a.select(p, 0, 1);
    a.ticks(2);
    let body = dummy_body(&a, first);
    a.aim(p, body);
    // Watch the cooldown at the catch.
    a.set_buttons(p, Buttons::ALT_FIRE);
    let mut ev = a.tick();
    a.set_buttons(p, Buttons::NONE);
    let mut refunded = None;
    for _ in 0..120 {
        let before = cooldown(&a, p);
        let t = a.tick();
        if t.iter()
            .any(|e| matches!(e, SimEvent::HurlCaught { player } if *player == p))
        {
            refunded = Some(before.saturating_sub(1) - cooldown(&a, p));
        }
        ev.extend(t);
    }
    let h = c().hurl;
    let out = c().trimmer.damage * h.damage_scale;
    let back = c().trimmer.damage * h.return_damage_scale;
    // Each once on the way out (passing through), once on the way back.
    for d in [first, second] {
        let hits = hurl_hits(&ev, d);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(
            (hits[0] - out).abs() < 1e-3 && (hits[1] - back).abs() < 1e-3,
            "{hits:?}"
        );
    }
    // Caught: 40 % of the cooldown back.
    let refund = secs_to_ticks(Ability::Hurl.cooldown(&c()) * h.catch_refund, 60, 0);
    assert_eq!(refunded, Some(refund), "caught, {refund} ticks back");
    assert!(
        !a.world
            .projectiles
            .values()
            .any(|x| x.kind == ProjectileKind::Hurl)
    );
}

#[test]
fn the_tampers_head_turns_back_at_the_first_target() {
    let (mut a, p) = armed(WeaponKind::Tamper, [Ability::Hurl, Ability::Whirl]);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let first = dummy(&mut a, Vec3::new(0.0, 0.0, -6.0), 0.0, BIG);
    let second = dummy(&mut a, Vec3::new(0.0, 0.0, -9.0), 0.0, BIG);
    a.ticks(2);
    let body = dummy_body(&a, first);
    a.aim(p, body);
    let ev = a.tap(p, Buttons::ALT_FIRE, 120);
    assert_eq!(hurl_hits(&ev, first).len(), 1, "struck once on the way out");
    assert!(hurl_hits(&ev, second).is_empty(), "it never got there");
    assert!(ev.iter().any(|e| matches!(e, SimEvent::HurlCaught { .. })));
}

#[test]
fn a_hurl_turns_back_at_its_range_and_replicates_its_return() {
    let (mut a, p) = armed(WeaponKind::Trimmer, [Ability::Hurl, Ability::Lunge]);
    a.ticks(2);
    a.held.get_mut(&p).unwrap().pitch = 0.05;
    a.tap(p, Buttons::ALT_FIRE, 0);
    let id = *a.world.projectiles.keys().next().unwrap();
    let mut farthest: f32 = 0.0;
    let mut returning = false;
    for _ in 0..90 {
        a.tick();
        let Some(x) = a.world.projectiles.get(&id) else {
            break;
        };
        farthest = farthest.max(x.position.z.abs());
        let flags = x.entity_state().flags;
        returning |=
            flags & projectile_flags::RETURNING != 0 && flags & projectile_flags::GUIDED != 0;
    }
    assert!(returning);
    let h = c().hurl;
    assert!(
        farthest >= h.max_distance - 1.0 && farthest <= h.max_distance + 1.5,
        "{farthest}"
    );
    assert!(!a.world.projectiles.contains_key(&id), "caught");
}

// ---- Riposte ----------------------------------------------------------------------------------

#[test]
fn the_riposte_blocks_from_the_front_staggers_and_counters() {
    let (mut a, p) = armed(WeaponKind::Tamper, [Ability::Riposte, Ability::Whirl]);
    a.world.players.get_mut(&p).unwrap().combat.god = false;
    a.world.players.get_mut(&p).unwrap().combat.protection = 0;
    // A Gnawer right in front, about to bite.
    let g = gnawer(&mut a, Vec3::new(0.0, 0.0, -1.8));
    a.ticks(2);
    let body = a.world.enemies[&g].center();
    a.aim(p, body);
    let r = c().riposte;
    let hold_ticks = secs_to_ticks(r.hold_time, 60, 1) as u32 - 2;
    a.set_buttons(p, Buttons::ALT_FIRE);
    let mut ev = Vec::new();
    let mut blocks = 0;
    // Guard; take its bites (ten Gnawers' worth of tries) until one is blocked.
    for _ in 0..hold_ticks {
        let t = a.tick();
        blocks += t
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    SimEvent::GuardBlock {
                        ability: Ability::Riposte,
                        ..
                    }
                )
            })
            .count();
        ev.extend(t);
        if blocks > 0 {
            break;
        }
    }
    assert!(blocks > 0, "a bite reached the guard");
    assert_eq!(a.health(p), c().max_health, "blocked: no damage");
    assert_eq!(
        a.world.enemies[&g].state,
        shared::sim::AiState::Stunned,
        "the biter is staggered"
    );
    // Let go: the counter-swing at 2x the Tamper's swing.
    a.set_buttons(p, Buttons::NONE);
    let ev = a.ticks(2);
    let counter: Vec<DamageEvent> = hits_on(&ev, HitTarget::Entity(g))
        .into_iter()
        .filter(|h| h.info.attack == Attack::Riposte)
        .collect();
    assert_eq!(counter.len(), 1);
    let expected = c().tamper.damage * (r.damage_scale + r.per_block * (blocks as f32 - 1.0));
    assert!((counter[0].info.amount - expected.min(c().tamper.damage * r.max_scale)).abs() < 1e-3);
    assert!(ev.iter().any(|e| matches!(
        e,
        SimEvent::Swing {
            attack: Attack::Riposte,
            ..
        }
    )));
}

#[test]
fn a_riposte_that_blocked_nothing_swings_nothing() {
    let (mut a, p) = armed(WeaponKind::Trimmer, [Ability::Riposte, Ability::Lunge]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -2.0), 0.0, BIG);
    a.ticks(2);
    let ev = hold(&mut a, p, Buttons::ALT_FIRE, 20, 2);
    assert!(hits_on(&ev, HitTarget::Entity(d)).is_empty());
    assert!(!ev.iter().any(|e| matches!(
        e,
        SimEvent::Swing {
            attack: Attack::Riposte,
            ..
        }
    )));
    // The full cooldown either way.
    assert!(cooldown(&a, p) > secs_to_ticks(c().riposte.cooldown - 0.1, 60, 1));
    // While it was up it slowed its owner (server-set, reconciled) and replicated GUARDING.
    let mut a2 = armed(WeaponKind::Trimmer, [Ability::Riposte, Ability::Lunge]);
    a2.0.set_buttons(a2.1, Buttons::ALT_FIRE);
    a2.0.ticks(3);
    let e = replicated(&a2.0, NetId::player(a2.1));
    assert_ne!(e.flags & player_flags::GUARDING, 0);
    assert!(a2.0.world.players[&a2.1].movement.guard_time > 0.0);
}

#[test]
fn the_riposte_turns_a_parryable_shot_from_the_front_back_but_not_one_from_behind() {
    let (mut a, p) = armed(WeaponKind::Tamper, [Ability::Riposte, Ability::Whirl]);
    a.world.players.get_mut(&p).unwrap().combat.god = false;
    a.world.players.get_mut(&p).unwrap().combat.protection = 0;
    let shooter = dummy(&mut a, Vec3::new(0.0, 0.0, -30.0), 0.0, BIG);
    a.ticks(2);
    a.set_buttons(p, Buttons::ALT_FIRE);
    a.tick();
    let t = a.tuning.clone();
    let eye = a.eye(p);
    let attack = Attack::Enemy {
        kind: 60,
        attack: 0,
    };
    let needle = a
        .world
        .spawn_enemy_projectile(
            &t,
            ProjectileKind::Needle,
            shooter,
            attack,
            eye + Vec3::new(0.0, 0.0, -5.0),
            Vec3::new(0.0, 0.0, 30.0),
            20.0,
        )
        .unwrap();
    let ev = a.ticks(12);
    assert_eq!(a.health(p), c().max_health, "blocked");
    assert!(ev.iter().any(|e| matches!(e, SimEvent::GuardBlock { .. })));
    let turned = a
        .world
        .projectiles
        .get(&needle)
        .expect("deflected, not gone");
    assert!(turned.parried && turned.owner == Owner::Player(p));
    // From behind it lands.
    let from_behind = a
        .world
        .spawn_enemy_projectile(
            &t,
            ProjectileKind::Needle,
            shooter,
            attack,
            eye + Vec3::new(0.0, 0.0, 5.0),
            Vec3::new(0.0, 0.0, -30.0),
            20.0,
        )
        .unwrap();
    a.ticks(20);
    assert!(!a.world.projectiles.contains_key(&from_behind));
    assert!(a.health(p) < c().max_health, "hit in the back");
}
