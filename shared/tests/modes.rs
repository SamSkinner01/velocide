//! Game modes in the simulation: campaign checkpoints, the solo death → checkpoint restart,
//! co-op downed / revive / timeout / team wipe, kill volumes, the exit (solo and co-op
//! countdown) with graded results, deathmatch frags, frag and time limits, farthest-spawn
//! respawns and spawn protection, the scoreboard, late joiners and restarts.

mod combat_support;

use std::sync::Arc;

use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::level::{Checkpoint, LevelDef, LevelKind, SpawnPoint, Volume};
use shared::lobby::{GameMode, MatchSettings};
use shared::protocol::{NetFrame, NetId, player_flags};
use shared::sim::modes::{authored_enemies, grade_player};
use shared::sim::{
    Attack, Blocked, DamageInfo, EndReason, Grade, HitTarget, LifeState, MatchPhase, MatchResults,
    ModeEvent, ModeRules, Owner, PlayerId, SimEvent, SimWorld,
};
use shared::tuning::ModeTuning;

fn vol(min: [f32; 3], max: [f32; 3]) -> Volume {
    Volume {
        min: Vec3::from(min),
        max: Vec3::from(max),
    }
}

fn spawn(x: f32, z: f32) -> SpawnPoint {
    SpawnPoint {
        position: Vec3::new(x, 0.0, z),
        yaw: 0.0,
    }
}

const CP0: Vec3 = Vec3::new(0.0, 0.0, 0.0);
const CP1: Vec3 = Vec3::new(0.0, 0.0, -40.0);
const EXIT: Vec3 = Vec3::new(0.0, 0.0, -80.0);
const LAVA: Vec3 = Vec3::new(45.0, 0.0, 0.0);

/// Flat campaign level: 4 spawns at z 30, checkpoints at z 0 and z -40, the exit at z -80, a
/// lava kill volume at x 45, par 60 s.
fn campaign_level() -> LevelDef {
    let mut level = flat_level(LevelKind::Campaign, Vec::new());
    level.spawns = (0..4).map(|i| spawn(i as f32 * 2.0, 30.0)).collect();
    level.checkpoints = vec![
        Checkpoint {
            trigger: vol([-5.0, -1.0, -5.0], [5.0, 3.0, 5.0]),
            spawn: spawn(CP0.x, CP0.z),
        },
        Checkpoint {
            trigger: vol([-5.0, -1.0, -45.0], [5.0, 3.0, -35.0]),
            spawn: spawn(CP1.x, CP1.z),
        },
    ];
    level.exit = Some(vol([-5.0, -1.0, -85.0], [5.0, 4.0, -75.0]));
    level.kill_volumes = vec![vol([40.0, -2.0, -5.0], [50.0, 0.5, 5.0])];
    level.par_time = 60.0;
    level
}

/// Flat arena: spawns in the four corners and the centre.
fn arena_level() -> LevelDef {
    let mut level = flat_level(LevelKind::Arena, Vec::new());
    level.spawns = vec![
        spawn(-40.0, -40.0),
        spawn(40.0, -40.0),
        spawn(-40.0, 40.0),
        spawn(40.0, 40.0),
        spawn(0.0, 0.0),
    ];
    level.kill_volumes = vec![vol([60.0, -2.0, -5.0], [70.0, 0.5, 5.0])];
    level
}

fn campaign() -> Arena {
    Arena::new(campaign_level())
}

fn mode_events(events: &[SimEvent]) -> Vec<ModeEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Mode(m) => Some(m.clone()),
            _ => None,
        })
        .collect()
}

fn respawned(events: &[SimEvent], who: PlayerId) -> Option<Vec3> {
    events.iter().find_map(|e| match e {
        SimEvent::Respawned {
            player, position, ..
        } if *player == who => Some(*position),
        _ => None,
    })
}

fn results(events: &[SimEvent]) -> Option<MatchResults> {
    mode_events(events).into_iter().find_map(|e| match e {
        ModeEvent::Ended(r) => Some(*r),
        _ => None,
    })
}

fn place(a: &mut Arena, p: PlayerId, pos: Vec3) {
    let pl = a.world.players.get_mut(&p).unwrap();
    pl.movement.position = pos;
    pl.movement.velocity = Vec3::ZERO;
}

fn kill(a: &mut Arena, p: PlayerId) {
    let t = a.tuning.clone();
    a.world.kill_player(&t, p, Attack::Dev);
}

fn hit(a: &mut Arena, victim: PlayerId, by: PlayerId, amount: f32) -> shared::sim::DamageEvent {
    let t = a.tuning.clone();
    let at = a.world.players[&victim].movement.position + Vec3::Y;
    let info = DamageInfo::new(amount, Owner::Player(by), Attack::Slug).at(at, Vec3::NEG_Z);
    a.world.apply_damage(&t, HitTarget::Player(victim), info)
}

fn life(a: &Arena, p: PlayerId) -> LifeState {
    a.world.mode.life(p)
}

fn player_entity_flags(world: &SimWorld, a: &Arena, p: PlayerId) -> u32 {
    let mut frame = NetFrame::new();
    world.replicate(&a.tuning, &mut frame);
    frame.get(NetId::player(p)).expect("replicated").flags
}

// ---- campaign: checkpoints and solo deaths --------------------------------------------------

#[test]
fn checkpoints_set_the_respawn_point_and_only_move_forward() {
    let mut a = campaign();
    let p = a.player(0, Vec3::new(0.0, 0.0, 30.0), 0.0);
    a.ticks(2);
    assert_eq!(a.world.mode.checkpoint, None);
    place(&mut a, p, CP1);
    let ev = a.tick();
    assert!(mode_events(&ev).contains(&ModeEvent::Checkpoint {
        index: 1,
        player: p
    }));
    assert_eq!(a.world.mode.checkpoint, Some(1));
    // Walking back through the first one changes nothing.
    place(&mut a, p, CP0);
    let ev = a.tick();
    assert!(
        !mode_events(&ev)
            .iter()
            .any(|e| matches!(e, ModeEvent::Checkpoint { .. }))
    );
    assert_eq!(a.world.mode.checkpoint, Some(1));
    // The scoreboard carries it for everyone.
    let board = a.world.scoreboard();
    assert_eq!(board.checkpoint, Some(1));
}

#[test]
fn solo_death_restarts_at_the_last_checkpoint() {
    let mut a = campaign();
    let p = a.player(0, Vec3::new(0.0, 0.0, 30.0), 0.0);
    a.ticks(2);
    place(&mut a, p, CP1);
    a.tick();
    place(&mut a, p, Vec3::new(10.0, 0.0, -50.0));
    a.tick();
    kill(&mut a, p);
    assert!(a.world.players[&p].combat.dead);
    assert_eq!(life(&a, p), LifeState::Dead, "solo: dead, not downed");
    // Something nasty in flight when we died is gone after the restart.
    let t = a.tuning.clone();
    a.husk(Vec3::new(20.0, 0.0, -60.0), 0.0);
    a.world.spawn_projectile(
        &t,
        shared::sim::ProjectileKind::Rocket,
        Owner::World,
        Vec3::new(0.0, 1.0, -70.0),
        Vec3::ZERO,
    );
    // Death cam: 1.5 s.
    let ev = a.ticks(85);
    assert!(respawned(&ev, p).is_none());
    assert!(a.world.players[&p].combat.dead);
    let ev = a.ticks(10);
    assert_eq!(respawned(&ev, p), Some(CP1));
    let pl = &a.world.players[&p];
    assert!(!pl.combat.dead);
    assert_eq!(pl.combat.health(&a.tuning.combat), 100.0);
    assert!(pl.combat.protection > 0);
    assert!(
        a.world.projectiles.is_empty(),
        "level back to the checkpoint state"
    );
    assert_eq!(a.world.mode.stats(p).deaths, 1);
}

#[test]
fn before_any_checkpoint_players_respawn_at_their_start() {
    let mut a = campaign();
    let p = a.player(0, Vec3::new(20.0, 0.0, 20.0), 0.0);
    a.ticks(2);
    kill(&mut a, p);
    let ev = a.ticks(100);
    assert_eq!(respawned(&ev, p), Some(Vec3::new(0.0, 0.0, 30.0)));
}

// ---- campaign: co-op downed / revive --------------------------------------------------------

/// Two players; B has reached checkpoint 1; A is killed at `where_a` (and downed).
fn coop_downed(where_a: Vec3) -> (Arena, PlayerId, PlayerId) {
    let mut a = campaign();
    let pa = a.player(0, Vec3::new(0.0, 0.0, 30.0), 0.0);
    let pb = a.player(1, Vec3::new(2.0, 0.0, 30.0), 0.0);
    a.ticks(2);
    place(&mut a, pb, CP1);
    a.tick();
    place(&mut a, pa, where_a);
    place(&mut a, pb, where_a + Vec3::new(8.0, 0.0, 0.0));
    a.tick();
    kill(&mut a, pa);
    a.events.clear();
    (a, pa, pb)
}

#[test]
fn coop_death_downs_the_player_and_a_teammate_revives_them() {
    let spot = Vec3::new(0.0, 0.0, -55.0);
    let (mut a, pa, pb) = coop_downed(spot);
    assert_eq!(life(&a, pa), LifeState::Downed);
    assert!(a.world.players[&pa].combat.dead, "can't move or shoot");
    let ev = a.tick();
    // Teammates see the downed body: DOWNED, not DEAD.
    let flags = player_entity_flags(&a.world, &a, pa);
    assert!(flags & player_flags::DOWNED != 0);
    assert!(flags & player_flags::DEAD == 0);
    let _ = ev;
    assert!(a.events.iter().any(|e| matches!(
        e,
        SimEvent::Mode(ModeEvent::Downed { player, .. }) if *player == pa
    )));

    // B walks up (2 m away) and holds USE for a bit, then lets go: the revive is cancelled.
    place(&mut a, pb, spot + Vec3::new(2.0, 0.0, 0.0));
    a.set_buttons(pb, Buttons::USE);
    let ev = a.ticks(30);
    assert!(mode_events(&ev).iter().any(|e| matches!(
        e,
        ModeEvent::Reviving { player, reviver: Some(r), .. } if *player == pa && *r == pb
    )));
    a.set_buttons(pb, Buttons::NONE);
    let ev = a.tick();
    assert!(mode_events(&ev).iter().any(|e| matches!(
        e,
        ModeEvent::Reviving { player, reviver: None, .. } if *player == pa
    )));
    assert_eq!(life(&a, pa), LifeState::Downed);

    // Holding it for the full 1.5 s revives at 40 HP, right there.
    a.set_buttons(pb, Buttons::USE);
    let ev = a.ticks(85);
    assert_eq!(life(&a, pa), LifeState::Downed, "not yet: {} ticks", 85);
    assert!(results(&ev).is_none());
    let ev = a.ticks(10);
    assert!(mode_events(&ev).contains(&ModeEvent::Revived { player: pa, by: pb }));
    assert_eq!(life(&a, pa), LifeState::Alive);
    let pl = &a.world.players[&pa];
    assert!(!pl.combat.dead);
    assert_eq!(pl.combat.health(&a.tuning.combat), 40.0);
    assert!(
        pl.movement.position.distance(spot) < 0.5,
        "revived where they lay"
    );
    assert_eq!(a.world.mode.stats(pb).revives, 1);
    // The revived player's client leaves its death view through the usual Respawned.
    assert!(respawned(&ev, pa).is_some());
}

#[test]
fn reviving_needs_to_be_close() {
    let spot = Vec3::new(0.0, 0.0, -55.0);
    let (mut a, pa, pb) = coop_downed(spot);
    place(&mut a, pb, spot + Vec3::new(3.5, 0.0, 0.0));
    a.set_buttons(pb, Buttons::USE);
    let ev = a.ticks(120);
    assert!(
        mode_events(&ev).is_empty()
            || !mode_events(&ev)
                .iter()
                .any(|e| matches!(e, ModeEvent::Reviving { .. }))
    );
    assert_eq!(life(&a, pa), LifeState::Downed);
}

#[test]
fn unrevived_players_respawn_at_the_checkpoint_after_15_s() {
    let spot = Vec3::new(10.0, 0.0, -60.0);
    let (mut a, pa, _pb) = coop_downed(spot);
    let ev = a.ticks(60 * 15 - 5);
    assert!(respawned(&ev, pa).is_none());
    assert_eq!(life(&a, pa), LifeState::Downed);
    let ev = a.ticks(10);
    assert_eq!(respawned(&ev, pa), Some(CP1));
    assert_eq!(life(&a, pa), LifeState::Alive);
}

#[test]
fn everyone_down_at_once_wipes_the_team_back_to_the_checkpoint() {
    let spot = Vec3::new(10.0, 0.0, -60.0);
    let (mut a, pa, pb) = coop_downed(spot);
    a.ticks(30);
    let t = a.tuning.clone();
    a.world.spawn_projectile(
        &t,
        shared::sim::ProjectileKind::Rocket,
        Owner::World,
        Vec3::new(0.0, 1.0, -90.0),
        Vec3::ZERO,
    );
    kill(&mut a, pb);
    let ev = a.tick();
    let wiped = mode_events(&ev)
        .into_iter()
        .find_map(|e| match e {
            ModeEvent::TeamWiped { respawn_tick } => Some(respawn_tick),
            _ => None,
        })
        .expect("team wiped");
    assert_eq!(wiped, a.world.tick + 180);
    assert_eq!(life(&a, pa), LifeState::Dead, "no more revives");
    // Holding USE on a corpse does nothing now (nobody is alive anyway).
    let ev = a.ticks(175);
    assert!(respawned(&ev, pa).is_none());
    let ev = a.ticks(10);
    assert_eq!(respawned(&ev, pa), Some(CP1));
    assert_eq!(respawned(&ev, pb), Some(CP1));
    assert!(a.world.projectiles.is_empty(), "the fight resets");
    assert!(a.world.players.values().all(|p| !p.combat.dead));
}

#[test]
fn coop_kill_volume_deaths_are_not_revivable() {
    let mut a = campaign();
    let pa = a.player(0, Vec3::new(0.0, 0.0, 30.0), 0.0);
    let pb = a.player(1, Vec3::new(2.0, 0.0, 30.0), 0.0);
    a.ticks(2);
    place(&mut a, pa, LAVA);
    place(&mut a, pb, LAVA + Vec3::new(-6.0, 0.0, 0.0));
    let ev = a.tick();
    assert!(a.world.players[&pa].combat.dead);
    assert_eq!(life(&a, pa), LifeState::Dead);
    assert!(
        !mode_events(&ev)
            .iter()
            .any(|e| matches!(e, ModeEvent::Downed { .. }))
    );
    // 4 s later: back at the start (no checkpoint yet).
    let ev = a.ticks(60 * 4 + 2);
    assert_eq!(respawned(&ev, pa), Some(Vec3::new(0.0, 0.0, 30.0)));
}

// ---- campaign: exit and results ---------------------------------------------------------------

#[test]
fn solo_exit_ends_the_level_with_graded_results() {
    let mut a = campaign();
    let p = a.player(0, Vec3::new(0.0, 0.0, 30.0), 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    a.ticks(2);
    // Kill the level's only enemy with a headshot.
    let h = a.husk(Vec3::new(0.0, 0.0, 15.0), 0.0);
    a.ticks(10);
    let head = a.husk_head(h);
    a.aim(p, head);
    a.tap(p, Buttons::FIRE, 1);
    assert!(!a.enemy_alive(h));
    // Take a little damage.
    let t = a.tuning.clone();
    let info = DamageInfo::new(10.0, Owner::World, Attack::Dev).at(Vec3::ZERO, Vec3::NEG_Y);
    a.world.apply_damage(&t, HitTarget::Player(p), info);
    place(&mut a, p, EXIT);
    let ev = a.tick();
    let r = results(&ev).expect("level complete");
    assert_eq!(r.reason, EndReason::Exit);
    assert_eq!(r.rules.kind, GameMode::Campaign);
    assert_eq!(a.world.mode.phase, MatchPhase::Ended);
    assert_eq!(r.total_enemies, 1);
    assert_eq!(r.team_kills, 1);
    assert_eq!(r.par_secs, 60.0);
    assert!(r.duration_secs < 1.0);
    let me = &r.players[0];
    assert!(me.extracted);
    assert_eq!(me.stats.kills, 1);
    assert_eq!(me.stats.damage_taken, 10.0);
    assert_eq!(me.style_points, 45);
    let g = me.grades.expect("campaign grades");
    assert_eq!(g.time, Some(Grade::S), "well under par");
    assert_eq!(g.kills, Some(Grade::S), "100 %");
    // 45 style points for 1 enemy: below C (35)? No: 45 >= 35 -> C.
    assert_eq!(g.style, Grade::C);
    // 10 damage per enemy: C (<= 10).
    assert_eq!(g.damage, Grade::C);
    // (4 + 4 + 1 + 1) / 4 = 2.5 -> B (halves up... 2.5 rounds to 3 = A).
    assert_eq!(g.overall, Grade::A);
    assert_eq!(r.team_grade, Some(Grade::A));
    // Once it's over nothing hurts and nothing scores.
    let info = DamageInfo::new(50.0, Owner::World, Attack::Dev).at(Vec3::ZERO, Vec3::NEG_Y);
    a.tick();
    let e = a.world.apply_damage(&t, HitTarget::Player(p), info);
    assert_eq!(e.blocked, Some(Blocked::SpawnProtection));
}

#[test]
fn coop_exit_counts_down_and_leaves_stragglers_behind() {
    let mut a = campaign();
    let pa = a.player(0, Vec3::new(0.0, 0.0, 30.0), 0.0);
    let pb = a.player(1, Vec3::new(2.0, 0.0, 30.0), 0.0);
    a.ticks(2);
    place(&mut a, pa, EXIT);
    let ev = a.tick();
    let end_tick = mode_events(&ev)
        .into_iter()
        .find_map(|e| match e {
            ModeEvent::ExitCountdown { player, end_tick } if player == pa => Some(end_tick),
            _ => None,
        })
        .expect("countdown");
    assert_eq!(end_tick, a.world.tick + 600, "10 s");
    let ev = a.ticks(595);
    assert!(results(&ev).is_none());
    let ev = a.ticks(10);
    let r = results(&ev).expect("ended when the countdown ran out");
    let row = |id| r.players.iter().find(|p| p.player == id).unwrap();
    assert!(row(pa).extracted);
    assert!(!row(pb).extracted, "left behind");
    assert!(row(pa).grades.is_some() && row(pb).grades.is_some());
    assert!(r.team_grade.is_some());
}

#[test]
fn coop_exit_ends_at_once_when_everyone_alive_is_in() {
    let mut a = campaign();
    let pa = a.player(0, Vec3::new(0.0, 0.0, 30.0), 0.0);
    let pb = a.player(1, Vec3::new(2.0, 0.0, 30.0), 0.0);
    let pc = a.player(2, Vec3::new(4.0, 0.0, 30.0), 0.0);
    a.ticks(2);
    // C is downed somewhere: doesn't hold the team up.
    kill(&mut a, pc);
    place(&mut a, pa, EXIT);
    a.tick();
    place(&mut a, pb, EXIT + Vec3::new(2.0, 0.0, 0.0));
    let ev = a.tick();
    let r = results(&ev).expect("everyone alive is in");
    assert_eq!(r.players.len(), 3);
    assert!(!r.players.iter().find(|p| p.player == pc).unwrap().extracted);
}

#[test]
fn grades_follow_the_documented_thresholds() {
    let t = ModeTuning::default();
    // Par 300 s, 40 enemies, solo.
    let g = grade_player(&t, 290.0, 300.0, 40, 40.0 * 130.0, 30.0, 40, 1);
    assert_eq!(
        (g.time, g.kills, g.style, g.damage, g.overall),
        (Some(Grade::S), Some(Grade::S), Grade::S, Grade::S, Grade::S)
    );
    let g = grade_player(&t, 300.0 * 1.6, 300.0, 31, 40.0 * 61.0, 40.0 * 5.0, 40, 1);
    assert_eq!(g.time, Some(Grade::B));
    assert_eq!(g.kills, Some(Grade::B), "77.5 %");
    assert_eq!(g.style, Grade::B);
    assert_eq!(g.damage, Grade::B);
    assert_eq!(g.overall, Grade::B);
    let g = grade_player(&t, 3000.0, 300.0, 0, 0.0, 9999.0, 40, 1);
    assert_eq!(g.overall, Grade::D);
    // Co-op: 40 enemies, 2 players -> a share of 20 each.
    let g = grade_player(&t, 290.0, 300.0, 20, 20.0 * 90.0, 20.0 * 2.0, 40, 2);
    assert_eq!(g.kills, Some(Grade::S));
    assert_eq!(g.style, Grade::A);
    assert_eq!(g.damage, Grade::A);
    // No par time and no enemies: those grades are left out.
    let g = grade_player(&t, 100.0, 0.0, 0, 50.0, 0.0, 0, 1);
    assert_eq!((g.time, g.kills), (None, None));
}

#[test]
fn authored_enemy_count_comes_from_the_encounters() {
    let level = shared::level::LevelDef::load_named("crucible_of_veins").expect("level");
    assert_eq!(authored_enemies(&level), 86);
}

// ---- deathmatch -----------------------------------------------------------------------------

fn deathmatch(frag_limit: u32, time_limit: f32) -> Arena {
    let mut a = Arena::new(arena_level());
    a.world
        .set_mode_rules(ModeRules::deathmatch(frag_limit, time_limit));
    a
}

#[test]
fn arenas_default_to_deathmatch_rules() {
    let w = SimWorld::new("a", Arc::new(arena_level()), 60);
    assert_eq!(w.mode.rules, ModeRules::deathmatch(20, 600.0));
    assert!(w.rules.friendly_fire);
    let w = SimWorld::new("c", Arc::new(campaign_level()), 60);
    assert_eq!(w.mode.rules.kind, GameMode::Campaign);
    assert!(!w.rules.friendly_fire);
}

#[test]
fn the_lobbys_match_settings_drive_the_rules() {
    let mut w = SimWorld::new("a", Arc::new(arena_level()), 60);
    let settings = MatchSettings {
        mode: GameMode::Deathmatch,
        frag_limit: 5,
        time_limit_secs: 120,
        ..MatchSettings::new(GameMode::Deathmatch, "a")
    };
    w.apply_match_settings(settings);
    assert_eq!(w.mode.rules, ModeRules::deathmatch(5, 120.0));
    assert!(w.rules.friendly_fire);
    // A frag limit from the lobby ends the match.
    let mut a = Arena::new(arena_level());
    a.world.apply_match_settings(MatchSettings {
        frag_limit: 1,
        time_limit_secs: 0,
        ..MatchSettings::new(GameMode::Deathmatch, "a")
    });
    let p0 = a.player(0, Vec3::new(-10.0, 0.0, 0.0), 0.0);
    let p1 = a.player(1, Vec3::new(10.0, 0.0, 0.0), 0.0);
    a.ticks(2);
    hit(&mut a, p1, p0, 500.0);
    let r = results(&a.tick()).expect("1 frag ends it");
    assert_eq!(r.reason, EndReason::FragLimit);
    // Co-op settings keep the lobby's friendly-fire toggle.
    let mut c = SimWorld::new("c", Arc::new(campaign_level()), 60);
    c.apply_match_settings(MatchSettings {
        friendly_fire: true,
        ..MatchSettings::new(GameMode::Campaign, "c")
    });
    assert_eq!(c.mode.rules.kind, GameMode::Campaign);
    assert!(c.rules.friendly_fire);
}

#[test]
fn the_world_freezes_once_the_match_is_over() {
    let mut a = campaign();
    let p = a.player(0, Vec3::new(0.0, 0.0, 30.0), 0.0);
    a.ticks(2);
    let h = a.husk(Vec3::new(0.0, 0.0, 26.0), 0.0);
    let start = a.world.enemies[&h].position;
    let t = a.tuning.clone();
    a.world.end_match(&t, EndReason::Host);
    a.ticks(60);
    assert!(
        a.world.enemies[&h].position.distance(start) < 0.01,
        "AI frozen"
    );
    assert_eq!(a.health(p), 100.0);
    // Kill volumes put players back instead of killing them.
    place(&mut a, p, LAVA);
    a.tick();
    assert!(!a.world.players[&p].combat.dead);
    assert_eq!(a.world.mode.stats(p).deaths, 0);
}

#[test]
fn frags_count_kills_and_cost_suicides() {
    let mut a = deathmatch(0, 0.0);
    let p0 = a.player(0, Vec3::new(-10.0, 0.0, 0.0), 0.0);
    let p1 = a.player(1, Vec3::new(10.0, 0.0, 0.0), 0.0);
    a.ticks(2);
    hit(&mut a, p1, p0, 500.0);
    let ev = a.tick();
    assert_eq!(a.world.mode.stats(p0).frags, 1);
    assert_eq!(a.world.mode.stats(p0).kills, 1);
    assert_eq!(a.world.mode.stats(p1).deaths, 1);
    // FRAG style for the killer.
    assert!(ev.iter().any(
        |e| matches!(e, SimEvent::Style(shared::sim::StyleEvent::Bonus {
        player, bonus: shared::sim::StyleBonus::Frag, .. }) if *player == p0)
    ));
    // Own rocket: -1.
    a.ticks(200);
    let t = a.tuning.clone();
    let at = a.world.players[&p0].movement.position;
    let info = DamageInfo::new(500.0, Owner::Player(p0), Attack::Rocket)
        .at(at, Vec3::NEG_Y)
        .flags(shared::sim::DamageFlags::EXPLOSIVE);
    a.world.apply_damage(&t, HitTarget::Player(p0), info);
    a.tick();
    assert_eq!(a.world.mode.stats(p0).frags, 0);
    // The kill volume: -1 too.
    a.ticks(200);
    place(&mut a, p1, Vec3::new(65.0, 0.0, 0.0));
    a.tick();
    assert_eq!(a.world.mode.stats(p1).frags, -1);
}

#[test]
fn knocking_someone_into_a_kill_volume_frags_them() {
    let mut a = deathmatch(0, 0.0);
    let p0 = a.player(0, Vec3::new(-10.0, 0.0, 0.0), 0.0);
    let p1 = a.player(1, Vec3::new(10.0, 0.0, 0.0), 0.0);
    a.ticks(120);
    hit(&mut a, p1, p0, 10.0);
    a.ticks(30);
    place(&mut a, p1, Vec3::new(65.0, 0.0, 0.0));
    a.tick();
    assert_eq!(a.world.mode.stats(p1).frags, -1);
    assert_eq!(a.world.mode.stats(p0).frags, 1, "knock-off credit");
    // Too long after the hit: no credit.
    a.ticks(200);
    hit(&mut a, p1, p0, 10.0);
    a.ticks(60 * 4);
    place(&mut a, p1, Vec3::new(65.0, 0.0, 0.0));
    a.tick();
    assert_eq!(a.world.mode.stats(p0).frags, 1);
    assert_eq!(a.world.mode.stats(p1).frags, -2);
}

#[test]
fn respawn_after_2_s_at_the_farthest_spawn_with_protection() {
    let mut a = deathmatch(0, 0.0);
    let p0 = a.player(0, Vec3::new(-10.0, 0.0, 0.0), 0.0);
    let p1 = a.player(1, Vec3::new(38.0, 0.0, 39.0), 0.0);
    let p2 = a.player(2, Vec3::new(36.0, 0.0, -37.0), 0.0);
    a.ticks(2);
    hit(&mut a, p0, p1, 500.0);
    let ev = a.ticks(115);
    assert!(respawned(&ev, p0).is_none(), "2 s");
    let ev = a.ticks(10);
    // p1 near (40, 40), p2 near (40, -40): (-40, 40) and (-40, -40) are both ~76 m from the
    // nearest; (-40, 40) edges it out.
    let at = respawned(&ev, p0).expect("respawned");
    assert!(at.x < -30.0, "far side: {at}");
    // Spawn protection: 1.5 s of immunity, then hits count again.
    let e = hit(&mut a, p0, p2, 30.0);
    assert_eq!(e.blocked, Some(Blocked::SpawnProtection));
    a.ticks(95);
    let e = hit(&mut a, p0, p2, 30.0);
    assert_eq!(e.blocked, None);
    assert_eq!(e.dealt, 30.0);
}

#[test]
fn frag_limit_ends_the_match_with_standings() {
    let mut a = deathmatch(2, 0.0);
    let p0 = a.player(0, Vec3::new(-10.0, 0.0, 0.0), 0.0);
    let p1 = a.player(1, Vec3::new(10.0, 0.0, 0.0), 0.0);
    a.ticks(2);
    hit(&mut a, p1, p0, 500.0);
    let ev = a.tick();
    assert!(results(&ev).is_none());
    a.ticks(60 * 4);
    hit(&mut a, p1, p0, 500.0);
    let ev = a.tick();
    let r = results(&ev).expect("frag limit");
    assert_eq!(r.reason, EndReason::FragLimit);
    assert_eq!(r.winner, Some(p0));
    assert_eq!(r.players[0].player, p0, "standings: best first");
    assert_eq!(r.players[0].stats.frags, 2);
    assert_eq!(r.players[1].stats.deaths, 2);
    assert!(r.players[0].grades.is_none());
    assert!(r.players[0].best_rank >= 1, "two frags = 200 style points");
    // Over: no more frags.
    a.ticks(60 * 4);
    hit(&mut a, p1, p0, 500.0);
    a.tick();
    assert_eq!(a.world.mode.stats(p0).frags, 2);
}

#[test]
fn time_limit_ends_the_match() {
    let mut a = deathmatch(20, 3.0);
    let p0 = a.player(0, Vec3::new(-10.0, 0.0, 0.0), 0.0);
    let p1 = a.player(1, Vec3::new(10.0, 0.0, 0.0), 0.0);
    a.ticks(2);
    hit(&mut a, p0, p1, 500.0);
    let ev = a.ticks(170);
    assert!(results(&ev).is_none());
    let ev = a.ticks(15);
    let r = results(&ev).expect("time limit");
    assert_eq!(r.reason, EndReason::TimeLimit);
    assert_eq!(r.winner, Some(p1));
    assert!((r.duration_secs - 3.0).abs() < 0.05, "{}", r.duration_secs);
}

// ---- scoreboard, late joiners, restart ----------------------------------------------------------

#[test]
fn the_scoreboard_goes_out_when_it_changes() {
    let mut a = deathmatch(0, 0.0);
    let p0 = a.player(0, Vec3::new(-10.0, 0.0, 0.0), 0.0);
    let p1 = a.player(1, Vec3::new(10.0, 0.0, 0.0), 0.0);
    let ev = a.ticks(30);
    let boards: Vec<_> = mode_events(&ev)
        .into_iter()
        .filter_map(|e| match e {
            ModeEvent::Scoreboard(b) => Some(b),
            _ => None,
        })
        .collect();
    assert_eq!(boards.len(), 1, "once when the match starts, then quiet");
    assert_eq!(boards[0].rows.len(), 2);
    hit(&mut a, p1, p0, 500.0);
    let ev = a.ticks(20);
    let board = mode_events(&ev)
        .into_iter()
        .find_map(|e| match e {
            ModeEvent::Scoreboard(b) => Some(b),
            _ => None,
        })
        .expect("scoreboard after a frag");
    let row = |id| *board.rows.iter().find(|r| r.player == id).unwrap();
    assert_eq!(row(p0).frags, 1);
    assert_eq!(row(p0).rank, 1, "a frag is 100 style points: CRUDE");
    assert_eq!(row(p1).deaths, 1);
    assert_eq!(row(p1).life, LifeState::Dead);
}

#[test]
fn late_joiners_start_at_the_checkpoint() {
    let mut a = campaign();
    let pa = a.player(0, Vec3::new(0.0, 0.0, 30.0), 0.0);
    a.ticks(2);
    place(&mut a, pa, CP1);
    a.ticks(2);
    let pb = a.player(1, Vec3::new(2.0, 0.0, 30.0), 0.0);
    let ev = a.tick();
    assert_eq!(respawned(&ev, pb), Some(CP1));
}

#[test]
fn restart_resets_the_match() {
    let mut a = campaign();
    let p = a.player(0, Vec3::new(0.0, 0.0, 30.0), 0.0);
    a.ticks(2);
    place(&mut a, p, CP1);
    a.tick();
    kill(&mut a, p);
    a.ticks(120);
    place(&mut a, p, EXIT);
    a.tick();
    assert_eq!(a.world.mode.phase, MatchPhase::Ended);
    a.world.restart_level();
    assert_eq!(a.world.mode.phase, MatchPhase::Playing);
    assert_eq!(a.world.mode.checkpoint, None);
    assert_eq!(a.world.mode.stats(p).deaths, 0);
    assert_eq!(a.world.mode.start_tick, a.world.tick);
    a.ticks(5);
    assert_eq!(a.world.mode.phase, MatchPhase::Playing);
}
