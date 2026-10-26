//! Game modes in the simulation: campaign checkpoints, the solo death → checkpoint restart,
//! co-op downed / revive / timeout / team wipe, kill volumes, the exit (solo and co-op
//! countdown) with graded results, deathmatch frags, frag and time limits, farthest-spawn
//! respawns and spawn protection, the scoreboard, late joiners and restarts; PvE difficulty and
//! co-op scaling (waves, health, the AI's scale, downed time, revive health); grind waves, spawn
//! points, the alive cap, players back at the next wave, the overrun and its results.

mod combat_support;

use std::sync::Arc;

use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::level::{Checkpoint, LevelDef, LevelKind, SpawnPoint, Volume};
use shared::loadout::Loadout;
use shared::lobby::{Difficulty, GameMode, MatchSettings};
use shared::protocol::{NetFrame, NetId, player_flags};
use shared::sim::modes::{authored_enemies, grade_player};
use shared::sim::{
    Attack, Blocked, DamageInfo, EndReason, Grade, GrindEvent, GunGameEvent, HitTarget, LifeState,
    MatchPhase, MatchResults, ModeEvent, ModeRules, Owner, PlayerId, SimEvent, SimWorld,
};
use shared::tuning::{DifficultyTuning, GrindTuning, GunGameTuning, ModeTuning};

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
    let info = DamageInfo::new(amount, Owner::Player(by), Attack::Quill).at(at, Vec3::NEG_Z);
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
    let wisp = a.world.spawn_projectile(
        &t,
        shared::sim::ProjectileKind::Wisp,
        Owner::World,
        Vec3::new(0.0, 1.0, -70.0),
        Vec3::ZERO,
    );
    // Long-lived: it would still be around without the restart.
    a.world
        .projectiles
        .get_mut(&wisp.unwrap())
        .unwrap()
        .expire_tick += 6000;
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
    let wisp = a.world.spawn_projectile(
        &t,
        shared::sim::ProjectileKind::Wisp,
        Owner::World,
        Vec3::new(0.0, 1.0, -90.0),
        Vec3::ZERO,
    );
    a.world
        .projectiles
        .get_mut(&wisp.unwrap())
        .unwrap()
        .expire_tick += 6000;
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
    // The shared freeze (`SimWorld::frozen`): enemies idle where they stand.
    assert!(a.world.frozen);
    let e = &a.world.enemies[&h];
    let moved = (e.position - start) * Vec3::new(1.0, 0.0, 1.0);
    assert!(moved.length() < 0.2, "AI frozen: moved {moved}");
    assert!(e.target.is_none());
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
    let info = DamageInfo::new(500.0, Owner::Player(p0), Attack::Tumble)
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
    assert_eq!(
        a.world.mode.stats(p1).frags,
        0,
        "a credited knock-off costs the victim nothing"
    );
    assert_eq!(a.world.mode.stats(p0).frags, 1, "knock-off credit");
    // Too long after the hit: no credit.
    a.ticks(200);
    hit(&mut a, p1, p0, 10.0);
    a.ticks(60 * 4);
    place(&mut a, p1, Vec3::new(65.0, 0.0, 0.0));
    a.tick();
    assert_eq!(a.world.mode.stats(p0).frags, 1);
    assert_eq!(a.world.mode.stats(p1).frags, -1, "uncredited world death");
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

#[test]
fn a_solo_death_mid_fight_resets_the_encounter_and_its_kills() {
    let level = LevelDef::load_named("crucible_of_veins").expect("level");
    let cp = level.checkpoints[0].spawn.position;
    let mut a = Arena::new(level);
    let p = a.player(0, cp, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    a.ticks(3);
    assert_eq!(a.world.mode.checkpoint, Some(0));
    a.world.start_encounter(0);
    a.ticks(90);
    let ids: Vec<NetId> = a
        .world
        .enemies
        .values()
        .filter(|e| e.is_alive() && e.encounter.is_some())
        .map(|e| e.id)
        .collect();
    assert!(!ids.is_empty(), "the first wave is out");
    // The player kills one of them...
    let t = a.tuning.clone();
    let at = a.world.enemies[&ids[0]].position;
    let info = DamageInfo::new(500.0, Owner::Player(p), Attack::Quill).at(at, Vec3::NEG_Z);
    a.world.apply_damage(&t, HitTarget::Entity(ids[0]), info);
    a.tick();
    assert_eq!(a.world.mode.stats(p).kills, 1);
    // ... and dies: back at the checkpoint, the fight starts over and the kill is taken back.
    kill(&mut a, p);
    a.ticks(100);
    assert!(!a.world.players[&p].combat.dead);
    assert_eq!(
        a.world.encounters.states[0].phase,
        shared::sim::encounter::EncounterPhase::Idle
    );
    assert!(
        a.world
            .enemies
            .values()
            .all(|e| !e.is_alive() || e.encounter.is_none()),
        "its enemies are gone"
    );
    assert_eq!(a.world.mode.stats(p).kills, 0);
    assert_eq!(a.world.mode.team_kills, 0);
    assert_eq!(a.world.mode.enemies_spawned, 0);
}

// ---- difficulty and co-op scaling -------------------------------------------------------------

/// A campaign level whose single encounter is one wave of `gnawers` Gnawers in the open.
fn encounter_level(gnawers: usize) -> LevelDef {
    use shared::level::{Encounter, EnemySpawn, Wave};
    let mut level = campaign_level();
    let spawns = (0..gnawers)
        .map(|i| EnemySpawn {
            enemy: "gnawer".into(),
            position: Vec3::new(i as f32 * 3.0 - 6.0, 0.0, -60.0),
            yaw: 0.0,
        })
        .collect();
    level.encounters = vec![Encounter {
        trigger: vol([-300.0, -1.0, -300.0], [-299.0, 0.0, -299.0]),
        arena: None,
        rally: None,
        lock_doors: vec![],
        unlock_doors: vec![],
        waves: vec![Wave { delay: 0.0, spawns }],
    }];
    level
}

/// Starts the encounter with `players` players at `difficulty`; returns the health of every
/// Mould its wave spawned.
fn encounter_wave(difficulty: Difficulty, players: u8) -> Vec<f32> {
    let mut a = Arena::new(encounter_level(4));
    a.world.set_mode_rules(ModeRules {
        difficulty,
        ..ModeRules::campaign()
    });
    for i in 0..players {
        let p = a.player(i, Vec3::new(i as f32 * 2.0, 0.0, 30.0), 0.0);
        a.world.players.get_mut(&p).unwrap().notarget = true;
    }
    a.ticks(2);
    a.world.start_encounter(0);
    a.ticks(5);
    a.world
        .enemies
        .values()
        .filter(|e| e.encounter.is_some())
        .map(|e| e.max_health)
        .collect()
}

#[test]
fn difficulty_and_players_scale_encounter_waves() {
    let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
    // NORMAL solo: the level as authored.
    let wave = encounter_wave(Difficulty::Normal, 1);
    assert_eq!(wave.len(), 4);
    assert!(wave.iter().all(|h| close(*h, 45.0)), "{wave:?}");
    // NORMAL, four players: x2.8 the Moulds (the old rule gave x2.5) with x2.8 the health.
    let wave = encounter_wave(Difficulty::Normal, 4);
    assert_eq!(wave.len(), 11);
    assert!(wave.iter().all(|h| close(*h, 45.0 * 2.8)), "{wave:?}");
    // EASY solo thins the wave out and softens it; HARD with two players: x1.2 x1.6 the
    // Moulds, x1.3 x1.6 the health.
    let wave = encounter_wave(Difficulty::Easy, 1);
    assert_eq!(wave.len(), 3);
    assert!(wave.iter().all(|h| close(*h, 45.0 * 0.7)));
    let wave = encounter_wave(Difficulty::Hard, 2);
    assert_eq!(wave.len(), 8);
    assert!(wave.iter().all(|h| close(*h, 45.0 * 1.3 * 1.6)));
    // The results count what the encounters spawn for this many players.
    let mut a = Arena::new(encounter_level(4));
    for i in 0..4 {
        a.player(i, Vec3::new(i as f32 * 2.0, 0.0, 30.0), 0.0);
    }
    assert_eq!(a.world.expected_enemies(&a.tuning), 11);
}

#[test]
fn enemy_behaviour_scales_with_difficulty_and_follows_joins_and_leaves() {
    let mut a = campaign();
    a.world.apply_match_settings(MatchSettings {
        difficulty: Difficulty::Brutal,
        ..MatchSettings::new(GameMode::Campaign, "c")
    });
    a.player(0, Vec3::new(0.0, 0.0, 30.0), 0.0);
    a.tick();
    let d = a.tuning.difficulty.clone();
    let solo = a.world.enemy_scale;
    assert!((solo.damage - d.damage[3]).abs() < 1e-5);
    assert!((solo.aggression - d.aggression[3]).abs() < 1e-5);
    assert!((solo.projectile_speed - d.projectile_speed[3]).abs() < 1e-5);
    // A second player joins: the AI hits harder and more often at once.
    let p1 = a.player(1, Vec3::new(2.0, 0.0, 30.0), 0.0);
    a.tick();
    let duo = a.world.enemy_scale;
    assert!((duo.damage - d.damage[3] * (1.0 + d.player_damage)).abs() < 1e-5);
    assert!((duo.aggression - d.aggression[3] * (1.0 + d.player_aggression)).abs() < 1e-5);
    // ... and leaves again.
    a.world.remove_player(p1);
    a.held.remove(&p1);
    a.tick();
    assert_eq!(a.world.enemy_scale, solo);
    // PvP: no scaling at all.
    let mut dm = deathmatch(20, 0.0);
    dm.player(0, Vec3::ZERO, 0.0);
    dm.tick();
    assert_eq!(dm.world.enemy_scale, shared::sim::EnemyScale::default());
}

#[test]
fn downed_time_and_revive_health_follow_the_difficulty() {
    let mut a = campaign();
    a.world.set_mode_rules(ModeRules {
        difficulty: Difficulty::Hard,
        ..ModeRules::campaign()
    });
    let spot = Vec3::new(0.0, 0.0, -55.0);
    let pa = a.player(0, spot, 0.0);
    let pb = a.player(1, spot + Vec3::new(1.0, 0.0, 0.0), 0.0);
    a.ticks(2);
    kill(&mut a, pa);
    assert_eq!(life(&a, pa), LifeState::Downed);
    // 15 s x 0.8 on HARD.
    assert_eq!(a.world.players[&pa].combat.respawn_in, 720);
    // B revives at 40 x 0.85 = 34 health.
    a.set_buttons(pb, Buttons::USE);
    a.ticks(100);
    assert_eq!(life(&a, pa), LifeState::Alive);
    assert_eq!(a.health(pa), 34.0);
    // Four players on NORMAL: 15 x (1 - 0.1 x 2) = 12 s.
    let mut a = campaign();
    let ids: Vec<PlayerId> = (0..4)
        .map(|i| a.player(i, Vec3::new(i as f32 * 3.0, 0.0, 30.0), 0.0))
        .collect();
    a.ticks(2);
    kill(&mut a, ids[0]);
    assert_eq!(a.world.players[&ids[0]].combat.respawn_in, 720);
}

#[test]
fn tuning_file_has_every_difficulty_grind_and_gun_game_value() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/tuning.ron");
    let text = std::fs::read_to_string(&path).expect("assets/tuning.ron exists");
    let tuning: shared::tuning::Tuning = ron::from_str(&text).expect("assets/tuning.ron parses");
    assert_eq!(tuning.difficulty, DifficultyTuning::default());
    assert_eq!(tuning.grind, GrindTuning::default());
    assert_eq!(tuning.gun_game, GunGameTuning::default());
    for defaults in [
        ron::to_string(&DifficultyTuning::default()).unwrap(),
        ron::to_string(&GrindTuning::default()).unwrap(),
        ron::to_string(&GunGameTuning::default()).unwrap(),
    ] {
        let fields: Vec<&str> = defaults
            .split([',', '(', '['])
            .filter_map(|kv| kv.split_once(':').map(|(k, _)| k.trim()))
            .filter(|k| !k.is_empty())
            .collect();
        assert!(fields.len() > 1, "{fields:?}");
        for field in fields {
            assert!(
                text.contains(&format!("{field}:")),
                "assets/tuning.ron is missing `{field}`"
            );
        }
    }
}

// ---- grind ----------------------------------------------------------------------------------

/// A grind run on the flat arena (short breathers so the tests stay quick). Players are added
/// by the caller.
fn grind(difficulty: Difficulty) -> Arena {
    let mut a = Arena::new(arena_level());
    a.world.set_mode_rules(ModeRules::grind(difficulty));
    a.tuning.grind.first_delay = 0.5;
    a.tuning.grind.breather = 1.0;
    a
}

/// Adds a player the Moulds ignore (the tests kill them on purpose).
fn grind_player(a: &mut Arena, id: u8, pos: Vec3) -> PlayerId {
    let p = a.player(id, pos, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    p
}

fn grind_events(events: &[SimEvent]) -> Vec<GrindEvent> {
    mode_events(events)
        .into_iter()
        .filter_map(|e| match e {
            ModeEvent::Grind(g) => Some(g),
            _ => None,
        })
        .collect()
}

/// Ticks until wave `wave` starts; the tick's events.
fn until_wave(a: &mut Arena, wave: u32) -> Vec<SimEvent> {
    for _ in 0..900 {
        let ev = a.tick();
        if grind_events(&ev)
            .iter()
            .any(|e| matches!(e, GrindEvent::WaveStarted { wave: w, .. } if *w == wave))
        {
            return ev;
        }
    }
    panic!("wave {wave} never started");
}

/// Wave size, heavy, boss of the wave that started in `events`.
fn started(events: &[SimEvent]) -> (u32, bool, bool) {
    grind_events(events)
        .into_iter()
        .find_map(|e| match e {
            GrindEvent::WaveStarted {
                enemies,
                heavy,
                boss,
                ..
            } => Some((enemies, heavy, boss)),
            _ => None,
        })
        .expect("a wave started")
}

/// `by` kills every grind Mould (spawned, then the ones trickling in) until the wave is
/// cleared; all the events.
fn clear_wave(a: &mut Arena, by: PlayerId) -> Vec<SimEvent> {
    let t = a.tuning.clone();
    let mut all = Vec::new();
    for _ in 0..3000 {
        for id in a.world.mode.grind.alive.clone() {
            let at = a.world.enemies[&id].center();
            let info = DamageInfo::new(1e5, Owner::Player(by), Attack::Quill).at(at, Vec3::NEG_Z);
            a.world.apply_damage(&t, HitTarget::Entity(id), info);
        }
        let ev = a.tick();
        let done = grind_events(&ev)
            .iter()
            .any(|e| matches!(e, GrindEvent::WaveCleared { .. }));
        all.extend(ev);
        if done {
            return all;
        }
    }
    panic!("the wave was never cleared");
}

#[test]
fn grind_waves_grow_and_clear_into_breathers() {
    let mut a = grind(Difficulty::Normal);
    let p = grind_player(&mut a, 0, Vec3::new(-40.0, 0.0, -40.0));
    a.tick();
    // The first breather is on the scoreboard (late joiners and the HUD).
    let g = a.world.scoreboard().grind.expect("grind status");
    assert_eq!((g.wave, g.left), (0, 0));
    assert!(g.next_wave_tick.is_some());
    let (first, heavy, boss) = started(&until_wave(&mut a, 1));
    assert_eq!(
        (first, heavy, boss),
        (6, false, false),
        "wave 1: six Gnawers"
    );
    assert_eq!(a.world.mode.grind.alive.len(), 6);
    let ev = clear_wave(&mut a, p);
    let next_tick = a.world.mode.grind.next_wave.expect("breather");
    assert!(grind_events(&ev).contains(&GrindEvent::WaveCleared { wave: 1, next_tick }));
    assert_eq!(a.world.mode.stats(p).kills, 6);
    let g = a.world.scoreboard().grind.unwrap();
    assert_eq!((g.wave, g.left, g.next_wave_tick), (1, 0, Some(next_tick)));
    let mut sizes = vec![first];
    for wave in 2..=5 {
        let (n, heavy, _) = started(&until_wave(&mut a, wave));
        assert_eq!(heavy, wave == 5, "every 5th wave is heavy");
        sizes.push(n);
        clear_wave(&mut a, p);
    }
    assert!(sizes[3] > sizes[0], "{sizes:?}");
    assert!(
        a.world
            .enemies
            .values()
            .all(|e| !e.is_alive() || e.def.name != "Gaoler"),
        "the heavy wave's Gaoler is dead too"
    );
    assert_eq!(a.world.mode.grind.cleared, 5);
    // Nobody was hurt: the run goes on.
    assert!(a.world.mode.is_playing());
}

#[test]
fn grind_moulds_appear_away_from_the_players() {
    // Without grind_spawns: nav points at least min_spawn_distance from every player.
    let mut a = grind(Difficulty::Hard);
    grind_player(&mut a, 0, Vec3::ZERO);
    until_wave(&mut a, 1);
    let min = a.tuning.grind.min_spawn_distance;
    assert!(!a.world.mode.grind.alive.is_empty());
    for id in a.world.mode.grind.alive.clone() {
        let e = &a.world.enemies[&id];
        let d = Vec3::new(e.position.x, 0.0, e.position.z).length();
        assert!(d >= min, "{d} m");
        assert!(e.position.y.abs() < 0.1, "on the floor: {}", e.position);
        assert!(e.aware, "they hunt the players");
    }
    // With grind_spawns: the authored points far enough away, never the one by the player.
    let mut level = arena_level();
    level.grind_spawns = vec![
        Vec3::new(3.0, 0.0, 0.0),
        Vec3::new(40.0, 0.0, 40.0),
        Vec3::new(-40.0, 0.0, 40.0),
    ];
    let mut a = Arena::new(level);
    a.world.set_mode_rules(ModeRules::grind(Difficulty::Normal));
    a.tuning.grind.first_delay = 0.2;
    grind_player(&mut a, 0, Vec3::ZERO);
    until_wave(&mut a, 1);
    assert_eq!(a.world.mode.grind.alive.len(), 6);
    for id in a.world.mode.grind.alive.clone() {
        let e = &a.world.enemies[&id];
        let d = Vec3::new(e.position.x, 0.0, e.position.z).length();
        assert!(d > 45.0, "around a far grind spawn: {}", e.position);
    }
}

#[test]
fn grind_trickles_big_waves_in_under_the_alive_cap() {
    let mut a = grind(Difficulty::Inferno);
    a.tuning.grind.budget_base = 60.0;
    grind_player(&mut a, 0, Vec3::new(-40.0, 0.0, -40.0));
    let (enemies, _, _) = started(&until_wave(&mut a, 1));
    let cap = a.tuning.grind.max_alive as usize;
    assert!(enemies as usize > cap, "{enemies}");
    assert_eq!(
        a.world.mode.grind.alive.len(),
        a.tuning.grind.spawn_burst as usize
    );
    for _ in 0..600 {
        a.tick();
        assert!(a.world.mode.grind.alive.len() <= cap);
    }
    assert_eq!(a.world.mode.grind.alive.len(), cap, "filled up to the cap");
    a.ticks(30); // the scoreboard is throttled
    let board = a.world.scoreboard();
    assert_eq!(
        board.grind.unwrap().left,
        enemies,
        "waiting Moulds count as left"
    );
}

#[test]
fn grind_players_who_are_out_come_back_at_the_next_wave() {
    let mut a = grind(Difficulty::Normal);
    let pa = grind_player(&mut a, 0, Vec3::new(-40.0, 0.0, -40.0));
    let pb = grind_player(&mut a, 1, Vec3::new(40.0, 0.0, -40.0));
    until_wave(&mut a, 1);
    // A goes down; nobody revives; the downed timer runs out: A is out, not respawned.
    kill(&mut a, pa);
    assert_eq!(life(&a, pa), LifeState::Downed);
    let downed = a.world.players[&pa].combat.respawn_in as u32;
    let ev = a.ticks(downed + 5);
    assert!(respawned(&ev, pa).is_none(), "no respawn mid-wave");
    assert_eq!(life(&a, pa), LifeState::Dead);
    assert!(a.world.players[&pa].combat.dead);
    assert!(
        a.world.players[&pa].combat.respawn_in > 60 * 60,
        "no countdown while the wave is on"
    );
    a.ticks(120);
    assert!(a.world.players[&pa].combat.dead, "still out");
    // B clears the wave: A's death view counts down to the next wave ...
    let ev = clear_wave(&mut a, pb);
    let next = grind_events(&ev)
        .into_iter()
        .find_map(|e| match e {
            GrindEvent::WaveCleared { next_tick, .. } => Some(next_tick),
            _ => None,
        })
        .unwrap();
    let wait = a.world.players[&pa].combat.respawn_in as u32;
    assert_eq!(a.world.tick + wait, next + 1);
    // ... and A is back, alive and protected, when it starts.
    let ev = until_wave(&mut a, 2);
    assert!(respawned(&ev, pa).is_some(), "respawned with the wave");
    assert_eq!(life(&a, pa), LifeState::Alive);
    assert!(!a.world.players[&pa].combat.dead);
    assert!(a.world.players[&pa].combat.protection > 0);
    assert_eq!(a.health(pa), 100.0);
    assert!(
        respawned(&ev, pb).is_none(),
        "the living stay where they are"
    );
    assert!(a.world.mode.is_playing());
}

#[test]
fn a_grind_run_ends_when_everyone_is_down_at_once() {
    let mut a = grind(Difficulty::Hard);
    let pa = grind_player(&mut a, 0, Vec3::new(-40.0, 0.0, -40.0));
    let pb = grind_player(&mut a, 1, Vec3::new(40.0, 0.0, -40.0));
    until_wave(&mut a, 1);
    clear_wave(&mut a, pa);
    until_wave(&mut a, 2);
    // One down: the run goes on.
    kill(&mut a, pa);
    let ev = a.ticks(10);
    assert!(grind_events(&ev).is_empty());
    // Both down: OVERRUN, then the results.
    kill(&mut a, pb);
    let ev = a.tick();
    let end_tick = grind_events(&ev)
        .into_iter()
        .find_map(|e| match e {
            GrindEvent::Overrun { end_tick } => Some(end_tick),
            _ => None,
        })
        .expect("overrun");
    assert_eq!(end_tick - a.world.tick, 150, "2.5 s");
    let ev = a.ticks(149);
    assert!(results(&ev).is_none() && respawned(&ev, pa).is_none());
    let r = results(&a.tick()).expect("results");
    assert_eq!(r.reason, EndReason::Overrun);
    assert_eq!(r.rules.difficulty, Difficulty::Hard);
    let run = r.grind.expect("grind results");
    assert_eq!((run.wave, run.cleared), (2, 1));
    assert_eq!(run.grade, Grade::D, "wave 2 is short of a C (wave 5)");
    assert_eq!(r.players.len(), 2);
    let alpha = r.players.iter().find(|p| p.player == pa).unwrap();
    assert!(alpha.stats.kills >= 6, "wave 1 was all A's");
    assert_eq!(alpha.stats.deaths, 1);
    let g = alpha.grades.expect("grades");
    assert_eq!(g.time, None, "no par time in grind");
    assert!(g.kills.is_some());
    assert!(r.team_grade.is_some());
    assert_eq!(r.total_enemies, a.world.mode.enemies_spawned);
    assert!(a.world.frozen);
}

#[test]
fn a_solo_grind_death_is_the_end_of_the_run() {
    let mut a = grind(Difficulty::Easy);
    let p = grind_player(&mut a, 0, Vec3::new(-40.0, 0.0, -40.0));
    until_wave(&mut a, 1);
    kill(&mut a, p);
    assert_eq!(life(&a, p), LifeState::Dead, "solo: nobody to revive you");
    let r = (0..200)
        .find_map(|_| results(&a.tick()))
        .expect("the run ends");
    assert_eq!(r.reason, EndReason::Overrun);
    assert_eq!(r.grind.unwrap().wave, 1);
    assert_eq!(r.players[0].stats.deaths, 1);
}

#[test]
fn the_lobby_settings_start_a_grind_run() {
    let mut w = SimWorld::new("a", Arc::new(arena_level()), 60);
    w.apply_match_settings(MatchSettings {
        difficulty: Difficulty::Brutal,
        ..MatchSettings::new(GameMode::Grind, "a")
    });
    assert_eq!(w.mode.rules, ModeRules::grind(Difficulty::Brutal));
    assert!(!w.rules.friendly_fire, "co-op rules");
    assert!(w.rules.coop_enemy_scaling);
}

// ---- gun game --------------------------------------------------------------------------------

/// A gun game on the flat arena with `players` players spread out (no time limit).
fn gun_game(players: u8) -> (Arena, Vec<PlayerId>) {
    let mut a = Arena::new(arena_level());
    a.world.set_mode_rules(ModeRules::gun_game(0.0));
    let ids = (0..players)
        .map(|i| a.player(i, Vec3::new(i as f32 * 6.0 - 20.0, 0.0, 0.0), 0.0))
        .collect();
    a.ticks(2);
    (a, ids)
}

/// `by` kills `victim` with `attack` (through the damage pipeline); the tick's events.
fn kill_with(a: &mut Arena, victim: PlayerId, by: PlayerId, attack: Attack) -> Vec<SimEvent> {
    let t = a.tuning.clone();
    let at = a.world.players[&victim].movement.position + Vec3::Y;
    let info = DamageInfo::new(1e4, Owner::Player(by), attack).at(at, Vec3::NEG_Z);
    a.world.apply_damage(&t, HitTarget::Player(victim), info);
    a.tick()
}

/// Brings a dead player back at once, unprotected (so the next test kill lands).
fn back_up(a: &mut Arena, p: PlayerId) {
    let t = a.tuning.clone();
    if a.world.players[&p].combat.dead {
        a.world.respawn_player(&t, p);
    }
    a.world.players.get_mut(&p).unwrap().combat.protection = 0;
}

fn gun_events(events: &[SimEvent]) -> Vec<GunGameEvent> {
    mode_events(events)
        .into_iter()
        .filter_map(|e| match e {
            ModeEvent::GunGame(g) => Some(g),
            _ => None,
        })
        .collect()
}

fn tier(a: &Arena, p: PlayerId) -> u8 {
    a.world.gun_game_tier(p)
}

#[test]
fn gun_game_promotes_with_the_tier_weapon_only() {
    let (mut a, ids) = gun_game(2);
    let (p0, p1) = (ids[0], ids[1]);
    let ladder = a.tuning.gun_game.ladder.clone();
    // Everyone carries the first build and nothing else.
    for p in [p0, p1] {
        assert_eq!(a.world.loadout(p), Loadout::single(ladder[0]));
        assert_eq!(tier(&a, p), 0);
    }
    // A kill with another weapon (say a bomb of an old tier) doesn't count.
    let ev = kill_with(&mut a, p1, p0, Attack::primary(ladder[3].weapon));
    assert!(gun_events(&ev).is_empty());
    assert_eq!(tier(&a, p0), 0);
    assert_eq!(a.world.mode.stats(p0).kills, 1, "still a kill");
    // A kill with the tier's weapon: up a tier, the next build in hand at once.
    back_up(&mut a, p1);
    let ev = kill_with(&mut a, p1, p0, Attack::primary(ladder[0].weapon));
    assert_eq!(
        gun_events(&ev),
        vec![GunGameEvent::Promoted {
            player: p0,
            tier: 1,
            victim: p1,
        }]
    );
    assert_eq!(tier(&a, p0), 1);
    let w = &a.world.players[&p0].combat.weapons;
    assert_eq!(w.loadout, Loadout::single(ladder[1]));
    assert_eq!(w.current(), ladder[1].weapon);
    // It sticks through a death and a respawn.
    let ev = kill_with(&mut a, p0, p1, Attack::Slam);
    assert!(gun_events(&ev).is_empty(), "slam kills don't count");
    back_up(&mut a, p0);
    assert_eq!(a.world.loadout(p0), Loadout::single(ladder[1]));
    // The scoreboard carries the tiers.
    let board = a.world.scoreboard();
    let row = board.rows.iter().find(|r| r.player == p0).unwrap();
    assert_eq!(row.tier, 1);
    assert_eq!(board.rules.kind, GameMode::GunGame);
}

#[test]
fn punch_kills_knock_the_victim_down_a_tier() {
    let (mut a, ids) = gun_game(2);
    let (p0, p1) = (ids[0], ids[1]);
    a.world.mode.player(p1).stats.tier = 3;
    a.tick();
    let ladder = a.tuning.gun_game.ladder.clone();
    assert_eq!(a.world.loadout(p1), Loadout::single(ladder[3]));
    let ev = kill_with(&mut a, p1, p0, Attack::Fist);
    assert_eq!(
        gun_events(&ev),
        vec![GunGameEvent::Demoted {
            player: p1,
            tier: 2,
            by: p0,
        }]
    );
    assert_eq!(tier(&a, p1), 2);
    assert_eq!(
        tier(&a, p0),
        0,
        "the puncher doesn't climb (the Fist isn't their tier)"
    );
    back_up(&mut a, p1);
    assert_eq!(a.world.loadout(p1), Loadout::single(ladder[2]));
    // Nobody goes below the first tier.
    let ev = kill_with(&mut a, p0, p1, Attack::Fist);
    assert!(gun_events(&ev).is_empty());
    assert_eq!(tier(&a, p0), 0);
}

#[test]
fn a_punch_kill_on_the_fist_tier_wins_the_gun_game() {
    let (mut a, ids) = gun_game(3);
    let (p0, p1, p2) = (ids[0], ids[1], ids[2]);
    let ladder = a.tuning.gun_game.ladder.clone();
    let fist = ladder.len() as u8;
    // The last weapon's kill reaches the Fist.
    a.world.mode.player(p0).stats.tier = fist - 1;
    a.world.mode.player(p1).stats.tier = 4;
    a.tick();
    let ev = kill_with(
        &mut a,
        p2,
        p0,
        Attack::primary(ladder[fist as usize - 1].weapon),
    );
    assert!(gun_events(&ev).contains(&GunGameEvent::Promoted {
        player: p0,
        tier: fist,
        victim: p2,
    }));
    // On the Fist tier weapon kills count for nothing ...
    back_up(&mut a, p2);
    let held = a.world.players[&p0].combat.weapons.current();
    let ev = kill_with(&mut a, p2, p0, Attack::primary(held));
    assert!(gun_events(&ev).is_empty() && results(&ev).is_none());
    assert_eq!(tier(&a, p0), fist);
    // ... a punch kill wins.
    let ev = kill_with(&mut a, p1, p0, Attack::Fist);
    let r = results(&ev).expect("the ladder is climbed");
    assert_eq!(r.reason, EndReason::Ladder);
    assert_eq!(r.winner, Some(p0));
    assert_eq!(r.players[0].player, p0);
    assert_eq!(r.players[0].stats.tier, fist + 1, "past the Fist");
    assert_eq!(r.players[1].player, p1, "then by tier");
    assert_eq!(r.players[1].stats.tier, 3, "punched down from 4");
    let g = r.gun_game.expect("gun game results");
    assert_eq!(g.ladder.len(), ladder.len());
    assert!(a.world.frozen);
}

#[test]
fn gun_game_time_limit_ranks_by_tier() {
    let mut a = Arena::new(arena_level());
    a.world.set_mode_rules(ModeRules::gun_game(5.0));
    let p0 = a.player(0, Vec3::new(-10.0, 0.0, 0.0), 0.0);
    let p1 = a.player(1, Vec3::new(10.0, 0.0, 0.0), 0.0);
    a.tick();
    a.world.mode.player(p0).stats.tier = 2;
    a.world.mode.player(p0).stats.kills = 9;
    a.world.mode.player(p1).stats.tier = 5;
    let r = (0..400)
        .find_map(|_| results(&a.tick()))
        .expect("time limit");
    assert_eq!(r.reason, EndReason::TimeLimit);
    assert_eq!(r.winner, Some(p1), "most tiers wins, not most kills");
    assert_eq!(r.rules.frag_limit, 0, "gun game never ends on frags");
}

#[test]
fn gun_game_late_joiners_start_on_the_lowest_tier() {
    let (mut a, ids) = gun_game(2);
    a.world.mode.player(ids[0]).stats.tier = 6;
    a.world.mode.player(ids[1]).stats.tier = 2;
    a.tick();
    let late = a.player(2, Vec3::new(30.0, 0.0, 30.0), 0.0);
    a.ticks(2);
    assert_eq!(tier(&a, late), 2);
    let ladder = a.tuning.gun_game.ladder.clone();
    assert_eq!(a.world.loadout(late), Loadout::single(ladder[2]));
}

#[test]
fn the_lobby_settings_start_a_gun_game() {
    let mut w = SimWorld::new("a", Arc::new(arena_level()), 60);
    w.apply_match_settings(MatchSettings {
        time_limit_secs: 300,
        frag_limit: 20,
        ..MatchSettings::new(GameMode::GunGame, "a")
    });
    assert_eq!(w.mode.rules, ModeRules::gun_game(300.0));
    assert!(w.rules.friendly_fire, "PvP");
}

#[test]
fn grind_waves_never_stall_without_spawn_points() {
    // No grind_spawns and a nav grid with nothing usable: the Moulds come in at the level spawn
    // farthest from the player instead of waiting forever.
    let mut a = grind(Difficulty::Normal);
    a.world.ai.nav = Some(Arc::new(shared::sim::nav::NavGrid::default()));
    grind_player(&mut a, 0, Vec3::new(-40.0, 0.0, -40.0));
    until_wave(&mut a, 1);
    assert_eq!(a.world.mode.grind.alive.len(), 6);
    for id in a.world.mode.grind.alive.clone() {
        let p = a.world.enemies[&id].position;
        assert!(p.distance(Vec3::new(40.0, 0.0, 40.0)) < 0.5, "{p}");
    }
}
