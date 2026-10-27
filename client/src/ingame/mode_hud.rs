//! Game-mode state on the client and its HUD: the scoreboard (Tab columns), checkpoint
//! banners, co-op downed / revive (world markers over downed teammates with the revive
//! prompt and progress, the local "DOWNED" panel, the reviver's progress bar), team wipes,
//! the extraction countdown, the grind wave block (wave, Moulds left, breather countdown, the
//! local best wave), the deathmatch clock and frag counter, the gun game tier block (tier,
//! weapon, next weapon, the leader), the difficulty tag, and the results screen when the match
//! ends.
//!
//! Fed by `GameEvent::Mode` ([`ModeEvent`], broadcast by the server) and the replicated
//! `player_flags::DOWNED`; countdowns are computed from server ticks with
//! `NetClient::server_tick_estimate`. See docs/style_modes.md.

use std::collections::BTreeMap;

use engine::audio::{Audio, MusicTrack, PlayParams, Sfx};
use engine::gfx::{Camera, UiDrawList};
use engine::ui::draw::{self, fade, mix};
use engine::ui::hud::format_timer;
use engine::ui::{Rect, TextStyle, Theme, Ui, UiDrawExt};
use glam::{Vec2, Vec3, vec2};
use shared::loadout::WeaponBuild;
use shared::lobby::GameMode;
use shared::protocol::{EntityState, PlayerInfo, player_flags};
use shared::sim::gungame::{Tier, tier_of};
use shared::sim::style::rank_name;
use shared::sim::{
    GrindEvent, GunGameEvent, LifeState, MatchPhase, ModeEvent, PlayerId, ScoreEntry, Scoreboard,
};
use shared::tuning::GunGameTuning;

use super::InGame;
use super::cues;
use super::hud::{Hud, project};
use super::remote_players::{RemotePlayers, player_color};
use super::scoreboard::{ScoreColumn, ScoreRow};
use super::style_hud::rank_text_color;
use crate::app::Common;
use crate::menus::results::ResultsScreen;
use crate::net::{NetClient, Session};

/// What the HUD needs from the game this frame.
pub struct ModeHudInfo<'a> {
    pub local: Option<PlayerId>,
    /// Server tick estimate (countdowns).
    pub server_tick: Option<f64>,
    pub tick_rate: u32,
    pub players: &'a BTreeMap<PlayerId, PlayerInfo>,
    pub camera: &'a Camera,
    /// The local player's feet and whether it is dead / downed.
    pub local_pos: Option<Vec3>,
    pub local_dead: bool,
    /// Remote players (interpolated entities).
    pub remote: &'a [EntityState],
    /// Revive range (m) from the tuning.
    pub revive_range: f32,
    /// Display name of the USE binding ("E").
    pub use_key: &'a str,
    pub time: f64,
    /// Grind: the best wave reached on this arena and difficulty (local record).
    pub best_wave: Option<u32>,
}

/// Mode state mirrored from the server's events.
#[derive(Default)]
pub struct ModeHud {
    pub scoreboard: Option<Scoreboard>,
    /// Downed players and the tick they respawn at.
    downed: BTreeMap<PlayerId, u32>,
    /// Revives in progress: target → (reviver, start tick, done tick).
    reviving: BTreeMap<PlayerId, (PlayerId, u32, u32)>,
    /// Extraction countdown: who started it, end tick.
    exit: Option<(PlayerId, u32)>,
    /// Team wipe: respawn tick.
    wipe: Option<u32>,
    /// Last whole second a countdown beeped at.
    last_beep: Option<i64>,
    /// The results screen (match over).
    pub results: Option<ResultsScreen>,
    /// Gun game: the server's ladder (from its tuning; banners, the HUD, score cells).
    pub gun_game: GunGameTuning,
}

/// Effects of a mode event the game has to apply.
#[derive(Default)]
pub struct ModeEffects {
    /// The results screen opened (switch the music, free the cursor).
    pub results_opened: bool,
    /// A new match started (after a restart): close results, reset the style HUD.
    pub match_restarted: bool,
}

fn name_of(players: &BTreeMap<PlayerId, PlayerInfo>, id: PlayerId) -> String {
    players
        .get(&id)
        .map_or_else(|| id.to_string(), |p| p.name.to_uppercase())
}

impl ModeHud {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mode(&self) -> Option<GameMode> {
        self.scoreboard.as_ref().map(|b| b.rules.kind)
    }

    /// Campaign: index of the last checkpoint reached.
    pub fn checkpoint(&self) -> Option<usize> {
        self.scoreboard
            .as_ref()
            .and_then(|b| b.checkpoint)
            .map(|c| c as usize)
    }

    pub fn results_open(&self) -> bool {
        self.results.is_some()
    }

    /// `id` is down (co-op), waiting for a revive.
    pub fn is_downed(&self, id: PlayerId) -> bool {
        self.downed.contains_key(&id)
    }

    /// A mode event from the server.
    #[allow(clippy::too_many_arguments)]
    pub fn on_event(
        &mut self,
        e: &ModeEvent,
        t: &Theme,
        hud: &mut Hud,
        audio: &mut Audio,
        players: &BTreeMap<PlayerId, PlayerInfo>,
        local: Option<PlayerId>,
        now: f64,
    ) -> ModeEffects {
        let mut fx = ModeEffects::default();
        let me = |id: PlayerId| Some(id) == local;
        // "YOU" in feed lines about the local player, like the kill feed.
        let who = |id: PlayerId| {
            if me(id) {
                "YOU".to_string()
            } else {
                name_of(players, id)
            }
        };
        match e {
            ModeEvent::Checkpoint { player, .. } => {
                hud.banner("CHECKPOINT", t.stamina, now);
                audio.play_2d(Sfx::Checkpoint);
                if !me(*player) && players.len() > 1 {
                    hud.event(
                        format!("{} REACHED A CHECKPOINT", name_of(players, *player)),
                        player_color(*player),
                        now,
                    );
                }
            }
            ModeEvent::Downed {
                player,
                respawn_tick,
            } => {
                self.downed.insert(*player, *respawn_tick);
                if me(*player) {
                    audio.play_2d(Sfx::Downed);
                } else {
                    audio.play(Sfx::Downed, PlayParams::default().with_volume(0.5));
                    hud.event(
                        format!("{} IS DOWN", name_of(players, *player)),
                        t.crimson_bright,
                        now,
                    );
                }
            }
            ModeEvent::Reviving {
                player,
                reviver,
                start_tick,
                done_tick,
            } => match reviver {
                Some(r) => {
                    self.reviving.insert(*player, (*r, *start_tick, *done_tick));
                }
                None => {
                    self.reviving.remove(player);
                }
            },
            ModeEvent::Revived { player, by } => {
                self.downed.remove(player);
                self.reviving.remove(player);
                audio.play_2d(Sfx::Revive);
                if me(*player) {
                    hud.banner("REVIVED", t.stamina, now);
                }
                hud.event(
                    format!("{} REVIVED {}", who(*by), who(*player)),
                    player_color(*by),
                    now,
                );
            }
            ModeEvent::TeamWiped { respawn_tick } => {
                self.downed.clear();
                self.reviving.clear();
                self.wipe = Some(*respawn_tick);
                hud.banner("TEAM WIPED", t.crimson_bright, now);
                audio.play_2d(Sfx::Death);
            }
            ModeEvent::ExitCountdown { player, end_tick } => {
                self.exit = Some((*player, *end_tick));
                self.last_beep = None;
                hud.banner("EXTRACTION", t.bone, now);
                audio.play_2d(Sfx::Checkpoint);
                hud.event(
                    format!("{} REACHED THE EXIT", who(*player)),
                    player_color(*player),
                    now,
                );
            }
            ModeEvent::Grind(g) => self.on_grind_event(g, t, hud, audio, now),
            ModeEvent::GunGame(g) => self.on_gun_game_event(g, t, hud, audio, players, local, now),
            ModeEvent::Scoreboard(board) => {
                let new_match = self.scoreboard.as_ref().is_some_and(|old| {
                    old.start_tick != board.start_tick
                        || (old.phase == MatchPhase::Ended && board.phase != MatchPhase::Ended)
                });
                if new_match {
                    self.exit = None;
                    self.wipe = None;
                    self.downed.clear();
                    self.reviving.clear();
                    if self.results.take().is_some() || board.phase == MatchPhase::Playing {
                        fx.match_restarted = true;
                    }
                }
                // Anyone the server no longer lists as downed is up again (grind: back at the
                // next wave without a revive).
                self.downed.retain(|id, _| {
                    board
                        .rows
                        .iter()
                        .any(|r| r.player == *id && r.life == LifeState::Downed)
                });
                let downed = &self.downed;
                self.reviving.retain(|id, _| downed.contains_key(id));
                if board.rows.iter().all(|r| r.life == LifeState::Alive) {
                    self.wipe = None;
                }
                self.scoreboard = Some(board.clone());
            }
            ModeEvent::Ended(results) => {
                self.exit = None;
                self.wipe = None;
                self.downed.clear();
                self.reviving.clear();
                audio.play_2d(match results.rules.kind {
                    GameMode::Campaign | GameMode::Grind => Sfx::LevelComplete,
                    GameMode::Deathmatch | GameMode::GunGame => Sfx::MatchEnd,
                });
                audio.set_music(MusicTrack::Results);
                let mut screen = ResultsScreen::new((**results).clone(), local, now);
                screen.finale = Some(self.gun_game.finale.weapon);
                self.results = Some(screen);
                fx.results_opened = true;
            }
        }
        fx
    }

    /// Remote players' downed pose from the replicated `DOWNED` flag.
    pub fn apply_visuals(remote: &mut RemotePlayers, players: &[EntityState]) {
        for e in players {
            remote.visual_mut(e.id).downed = e.flags & player_flags::DOWNED != 0;
        }
    }

    /// Draws the mode HUD (not the results screen, which is interactive).
    pub fn draw(
        &mut self,
        list: &mut UiDrawList,
        t: &Theme,
        info: &ModeHudInfo,
        audio: &mut Audio,
    ) {
        let c = Rect::from_min_size(Vec2::ZERO, list.canvas_size);
        let rate = info.tick_rate.max(1) as f64;
        let secs_until = |tick: u32| -> Option<f64> {
            info.server_tick
                .map(|now| ((tick as f64 - now) / rate).max(0.0))
        };
        let outline = fade(t.ink, 0.9);
        let Some(board) = &self.scoreboard else {
            return;
        };
        if board.phase == MatchPhase::Ended {
            return;
        }

        match board.rules.kind {
            GameMode::Deathmatch => self.draw_deathmatch(list, t, info, board, c),
            GameMode::GunGame => {
                self.draw_gun_game(list, t, info, board);
                draw_time_left(list, t, info, board, c);
            }
            GameMode::Campaign | GameMode::Grind => {
                if board.rows.len() > 1 {
                    self.draw_team(list, t, info, board, &secs_until);
                }
            }
        }
        if board.rules.kind == GameMode::Grind {
            // Breather countdown: a beep for each of the last three seconds.
            if let Some(whole) = self.draw_grind(list, t, info, board, &secs_until, c)
                && self.last_beep != Some(whole)
            {
                self.last_beep = Some(whole);
                audio.play(
                    Sfx::Countdown,
                    PlayParams::default().with_pitch(if whole == 1 { 1.5 } else { 1.0 }),
                );
            }
        } else if board.rules.kind.has_difficulty() {
            draw_difficulty_tag(list, t, board, c);
        }

        // Extraction countdown (co-op).
        if let Some((who, end)) = self.exit
            && let Some(left) = secs_until(end)
        {
            let whole = left.ceil() as i64;
            if whole <= 5 && whole > 0 && self.last_beep != Some(whole) {
                self.last_beep = Some(whole);
                audio.play(
                    Sfx::Countdown,
                    PlayParams::default().with_pitch(if whole == 1 { 1.5 } else { 1.0 }),
                );
            }
            let urgent = left < 3.5 && (info.time * 4.0).fract() < 0.5;
            let col = if urgent { t.crimson_bright } else { t.bone };
            let y = 46.0;
            list.text(
                vec2(c.center().x, y),
                &format!("EXTRACTION  {:.1}", left),
                &TextStyle::new(col).scale(2.0).centered().outline(outline),
            );
            let sub = if Some(who) == info.local {
                "WAIT FOR YOUR TEAM - OR DON'T".to_string()
            } else {
                format!("{} IS AT THE EXIT", name_of(info.players, who))
            };
            list.text(
                vec2(c.center().x, y + 18.0),
                &sub,
                &TextStyle::new(t.bone_dim).centered().outline(outline),
            );
        }

        // Team wipe. Everyone is dead now, so the death view right above counts down
        // ("RESPAWNING IN 2.5"); a second, rounded countdown here disagreed with it.
        if let Some(at) = self.wipe
            && secs_until(at).is_some()
        {
            list.text(
                vec2(c.center().x, c.center().y + 40.0),
                "REGROUPING AT THE CHECKPOINT",
                &TextStyle::new(t.bone).centered().outline(outline),
            );
        }

        self.draw_downed_markers(list, t, info, &secs_until);
        self.draw_local_downed(list, t, info, board, &secs_until, c);
        self.draw_reviving(list, t, info, c);
    }

    /// Top-left co-op team list: names with ALIVE / DOWNED n / DEAD.
    fn draw_team(
        &self,
        list: &mut UiDrawList,
        t: &Theme,
        info: &ModeHudInfo,
        board: &Scoreboard,
        secs_until: &dyn Fn(u32) -> Option<f64>,
    ) {
        let x = 12.0;
        let mut y = 10.0;
        for row in &board.rows {
            if Some(row.player) == info.local {
                continue;
            }
            let col = player_color(row.player);
            let (slot, ink) = (row.player.0 as usize, fade(t.ink, 0.85));
            cues::player_swatch(list, vec2(x, y), vec2(3.0, 7.0), slot, col, ink);
            let name = engine::ui::text::truncate(&name_of(info.players, row.player), 90.0, 1.0);
            list.text(
                vec2(x + 7.0, y),
                &name,
                &TextStyle::new(t.bone).outline(fade(t.ink, 0.85)),
            );
            let (status, scol) = match row.life {
                LifeState::Alive => (String::new(), t.bone_dim),
                LifeState::Downed => (
                    match self.downed.get(&row.player).and_then(|tk| secs_until(*tk)) {
                        Some(s) => format!("DOWN {:.0}", s.ceil()),
                        None => "DOWN".into(),
                    },
                    t.crimson_bright,
                ),
                LifeState::Dead => ("DEAD".into(), t.ash),
            };
            if !status.is_empty() {
                list.text(
                    vec2(x + 100.0, y),
                    &status,
                    &TextStyle::new(scol).outline(fade(t.ink, 0.85)),
                );
            }
            y += 11.0;
        }
    }

    /// Top-left deathmatch block: frags / limit, place, time left.
    fn draw_deathmatch(
        &self,
        list: &mut UiDrawList,
        t: &Theme,
        info: &ModeHudInfo,
        board: &Scoreboard,
        c: Rect,
    ) {
        let outline = fade(t.ink, 0.9);
        let mut rows = board.rows.clone();
        rows.sort_by(|a, b| b.frags.cmp(&a.frags).then(a.deaths.cmp(&b.deaths)));
        let mine = rows.iter().position(|r| Some(r.player) == info.local);
        let my_frags = mine.map_or(0, |i| rows[i].frags);
        let slab = Rect::new(vec2(8.0, 6.0), vec2(150.0, 46.0));
        draw::slanted(list, slab, true, fade(t.ink, 0.5));
        draw::fill(
            list,
            Rect::new(slab.min, vec2(slab.min.x + 2.0, slab.max.y)),
            t.crimson,
        );
        list.text(
            vec2(18.0, 10.0),
            "FRAGS",
            &TextStyle::new(t.bone_dim).outline(outline),
        );
        let limit = if board.rules.frag_limit > 0 {
            format!(" / {}", board.rules.frag_limit)
        } else {
            String::new()
        };
        let r = list.text(
            vec2(18.0, 21.0),
            &format!("{my_frags}"),
            &TextStyle::new(t.bone).scale(2.0).outline(outline),
        );
        list.text(
            vec2(r.max.x + 2.0, 28.0),
            &limit,
            &TextStyle::new(t.bone_dim).outline(outline),
        );
        if let Some(place) = mine {
            let tied = rows.iter().filter(|r| r.frags == my_frags).count() > 1;
            let text = match (place, tied) {
                (0, false) => "1ST".to_string(),
                (_, true) => "TIED".to_string(),
                (p, _) => format!("{}{}", p + 1, ordinal(p + 1)),
            };
            let col = if place == 0 && !tied {
                t.rank_color(7)
            } else {
                t.bone_dim
            };
            list.text(
                vec2(96.0, 10.0),
                &text,
                &TextStyle::new(col).scale(2.0).outline(outline),
            );
        }
        draw_time_left(list, t, info, board, c);
    }

    /// Gun game banners, sounds and feed lines: your promotions (the next weapon; the melee
    /// finale: "FINAL TIER: TRIMMER"), being punched down, others reaching the finale or
    /// punching someone down.
    #[allow(clippy::too_many_arguments)]
    fn on_gun_game_event(
        &mut self,
        e: &GunGameEvent,
        t: &Theme,
        hud: &mut Hud,
        audio: &mut Audio,
        players: &BTreeMap<PlayerId, PlayerInfo>,
        local: Option<PlayerId>,
        now: f64,
    ) {
        let me = |id: PlayerId| Some(id) == local;
        match *e {
            GunGameEvent::Promoted { player, tier, .. } => {
                let finale = tier_of(&self.gun_game, tier);
                let last = matches!(finale, Tier::Finale(_));
                if me(player) {
                    let label = tier_label(&self.gun_game, tier, true);
                    hud.banner(label, if last { t.crimson_bright } else { t.bone }, now);
                    audio.play(
                        Sfx::RankUp,
                        PlayParams::default().with_pitch(if last { 0.8 } else { 1.0 }),
                    );
                } else if let Tier::Finale(b) = finale {
                    hud.event(
                        format!(
                            "{} IS ON THE FINAL TIER ({})",
                            name_of(players, player),
                            b.weapon.name().to_uppercase()
                        ),
                        player_color(player),
                        now,
                    );
                    audio.play(Sfx::RankUp, PlayParams::default().with_volume(0.5));
                }
            }
            GunGameEvent::Demoted { player, by, .. } => {
                // (The tier block shows where you landed.)
                if me(player) {
                    hud.banner("PUNCHED DOWN", t.crimson_bright, now);
                    audio.play_2d(Sfx::RankDown);
                }
                let who = |id: PlayerId| {
                    if me(id) {
                        "YOU".to_string()
                    } else {
                        name_of(players, id)
                    }
                };
                hud.event(
                    format!("{} PUNCHED {} DOWN A TIER", who(by), who(player)),
                    player_color(by),
                    now,
                );
            }
        }
    }

    /// Top-left gun game block: your tier (of the ladder) and weapon, the next one, your place
    /// and the leader.
    fn draw_gun_game(
        &self,
        list: &mut UiDrawList,
        t: &Theme,
        info: &ModeHudInfo,
        board: &Scoreboard,
    ) {
        let g = &self.gun_game;
        let outline = fade(t.ink, 0.9);
        let rows = gun_game_order(&board.rows);
        let mine = rows.iter().position(|r| Some(r.player) == info.local);
        let tier = mine.map_or(0, |i| rows[i].tier);
        let slab = Rect::new(vec2(8.0, 6.0), vec2(196.0, 56.0));
        draw::slanted(list, slab, true, fade(t.ink, 0.5));
        draw::fill(
            list,
            Rect::new(slab.min, vec2(slab.min.x + 2.0, slab.max.y)),
            t.crimson,
        );
        list.text(
            vec2(18.0, 10.0),
            "TIER",
            &TextStyle::new(t.bone_dim).outline(outline),
        );
        let fist = matches!(tier_of(g, tier), Tier::Finale(_));
        let count = g.ladder.len() + 1;
        let r = list.text(
            vec2(18.0, 21.0),
            &format!("{}", (tier as usize + 1).min(count)),
            &TextStyle::new(t.bone).scale(2.0).outline(outline),
        );
        let r = list.text(
            vec2(r.max.x + 2.0, 28.0),
            &format!(" / {count}"),
            &TextStyle::new(t.bone_dim).outline(outline),
        );
        let (weapon, col) = match tier_of(g, tier) {
            Tier::Weapon(b) => (b.weapon.name().to_uppercase(), t.bone),
            Tier::Finale(b) => (
                b.weapon.name().to_uppercase(),
                if (info.time * 3.0).fract() < 0.5 {
                    t.crimson_bright
                } else {
                    t.rank_color(7)
                },
            ),
            Tier::Won => ("WON".to_string(), t.rank_color(7)),
        };
        list.text(
            vec2(r.max.x + 8.0, 21.0),
            &weapon,
            &TextStyle::new(col).scale(2.0).outline(outline),
        );
        let next = match tier_of(g, tier.saturating_add(1)) {
            _ if fist => "A KILL TO WIN".to_string(),
            Tier::Weapon(b) => format!("NEXT: {}", b.weapon.name().to_uppercase()),
            Tier::Finale(b) => format!("NEXT: {}", b.weapon.name().to_uppercase()),
            Tier::Won => String::new(),
        };
        list.text(
            vec2(18.0, 42.0),
            &next,
            &TextStyle::new(if fist { t.crimson_bright } else { t.bone_dim }).outline(outline),
        );
        // Place (top right of the slab) and the leader under it.
        if let Some(place) = mine {
            let tied = rows
                .iter()
                .filter(|r| r.tier == rows[place].tier && r.kills == rows[place].kills)
                .count()
                > 1;
            let text = match (place, tied) {
                (0, false) => "1ST".to_string(),
                (_, true) => "TIED".to_string(),
                (p, _) => format!("{}{}", p + 1, ordinal(p + 1)),
            };
            let col = if place == 0 && !tied {
                t.rank_color(7)
            } else {
                t.bone_dim
            };
            list.text(
                vec2(slab.max.x - 6.0, 10.0),
                &text,
                &TextStyle::new(col).right().outline(outline),
            );
        }
        if let Some(leader) = rows.first() {
            let text = if mine == Some(0) {
                match rows.get(1) {
                    Some(second) => format!(
                        "YOU LEAD - NEXT BEST: {} TIER {}",
                        name_of(info.players, second.player),
                        second.tier as usize + 1
                    ),
                    None => "YOU LEAD".to_string(),
                }
            } else {
                format!(
                    "LEADER: {} - {}",
                    name_of(info.players, leader.player),
                    tier_label(g, leader.tier, false)
                )
            };
            list.text(
                vec2(12.0, slab.max.y + 4.0),
                &engine::ui::text::truncate(&text, 260.0, 1.0),
                &TextStyle::new(player_color(leader.player)).outline(outline),
            );
        }
    }

    /// Grind banners and sounds: waves starting (HEAVY / BOSS waves louder and lower), waves
    /// cleared, the overrun.
    fn on_grind_event(
        &mut self,
        e: &GrindEvent,
        t: &Theme,
        hud: &mut Hud,
        audio: &mut Audio,
        now: f64,
    ) {
        match e {
            GrindEvent::WaveStarted {
                wave, heavy, boss, ..
            } => {
                let (text, col, pitch) = if *boss {
                    (format!("BOSS WAVE {wave}"), t.crimson_bright, 0.7)
                } else if *heavy {
                    (format!("HEAVY WAVE {wave}"), t.crimson_bright, 0.85)
                } else {
                    (format!("WAVE {wave}"), t.bone, 1.0)
                };
                hud.banner(text, col, now);
                audio.play(Sfx::WaveStart, PlayParams::default().with_pitch(pitch));
                self.last_beep = None;
            }
            GrindEvent::WaveCleared { wave, .. } => {
                hud.banner(format!("WAVE {wave} CLEARED"), t.stamina, now);
                audio.play_2d(Sfx::EncounterClear);
                self.last_beep = None;
            }
            GrindEvent::Overrun { .. } => {
                // Nobody is left to revive anyone.
                self.downed.clear();
                self.reviving.clear();
                hud.banner("OVERRUN", t.crimson_bright, now);
                audio.play_2d(Sfx::Death);
            }
        }
    }

    /// Grind block, top centre: the wave and the Moulds left (HEAVY / BOSS tags), or the
    /// breather countdown to the next wave; difficulty and the local best wave under it.
    /// Returns the countdown's whole second while it is in its last three (the beeps).
    fn draw_grind(
        &self,
        list: &mut UiDrawList,
        t: &Theme,
        info: &ModeHudInfo,
        board: &Scoreboard,
        secs_until: &dyn Fn(u32) -> Option<f64>,
        c: Rect,
    ) -> Option<i64> {
        let g = board.grind?;
        let outline = fade(t.ink, 0.9);
        let cx = c.center().x;
        let y = 28.0;
        let mut sub = vec![board.rules.difficulty.title().to_string()];
        let mut beep = None;
        if let Some(at) = g.next_wave_tick
            && g.overrun_tick.is_none()
        {
            let left = secs_until(at).unwrap_or(0.0);
            let whole = left.ceil() as i64;
            if (1..=3).contains(&whole) {
                beep = Some(whole);
            }
            let urgent = whole <= 3;
            list.text(
                vec2(cx, y),
                &format!("WAVE {} IN {}", g.wave + 1, whole.max(1)),
                &TextStyle::new(if urgent { t.crimson_bright } else { t.bone })
                    .scale(2.0)
                    .centered()
                    .outline(outline),
            );
            if g.wave > 0 {
                sub.push(format!("WAVE {} CLEARED", g.wave));
            }
        } else if g.wave > 0 {
            let (tag, col) = if g.boss {
                ("  BOSS", t.crimson_bright)
            } else if g.heavy {
                ("  HEAVY", t.crimson_bright)
            } else {
                ("", t.bone)
            };
            let style = TextStyle::new(t.bone).scale(2.0).outline(outline);
            let main = format!("WAVE {}", g.wave);
            let w =
                engine::ui::text::line_width(&main, 2.0) + engine::ui::text::line_width(tag, 1.0);
            let r = list.text(vec2((cx - w * 0.5).round(), y), &main, &style);
            if !tag.is_empty() {
                list.text(
                    vec2(r.max.x, y + 7.0),
                    tag,
                    &TextStyle::new(col).outline(outline),
                );
            }
            sub.push(if g.overrun_tick.is_some() {
                "OVERRUN".to_string()
            } else if g.next_wave_tick.is_some() {
                format!("WAVE {} CLEARED", g.wave)
            } else {
                format!("{} LEFT", g.left)
            });
        }
        if let Some(best) = info.best_wave.filter(|b| *b > 0) {
            sub.push(format!("BEST: WAVE {best}"));
        }
        list.text(
            vec2(cx, y + 18.0),
            &sub.join("  //  "),
            &TextStyle::new(t.bone_dim).centered().outline(outline),
        );
        beep
    }

    /// Markers over downed teammates: DOWNED, time left, revive progress, the prompt.
    fn draw_downed_markers(
        &self,
        list: &mut UiDrawList,
        t: &Theme,
        info: &ModeHudInfo,
        secs_until: &dyn Fn(u32) -> Option<f64>,
    ) {
        let canvas = list.canvas_size;
        for e in info.remote {
            if e.flags & player_flags::DOWNED == 0 {
                continue;
            }
            let Some(id) = e.id.as_player() else {
                continue;
            };
            let anchor = e.position + Vec3::Y * 1.0;
            let Some(p) = project(info.camera, canvas, anchor) else {
                continue;
            };
            let p = p.round();
            let dist = info
                .local_pos
                .map_or(f32::INFINITY, |l| l.distance(e.position));
            let pulse = ((info.time * 5.0).sin() as f32 * 0.5 + 0.5) * 0.5 + 0.5;
            // Cross marker.
            let col = mix(t.crimson_bright, t.bone, 0.3 * pulse);
            draw::fill(
                list,
                Rect::from_center_size(p, vec2(9.0, 3.0)).expand(1.0),
                fade(t.ink, 0.8),
            );
            draw::fill(
                list,
                Rect::from_center_size(p, vec2(3.0, 9.0)).expand(1.0),
                fade(t.ink, 0.8),
            );
            draw::fill(list, Rect::from_center_size(p, vec2(9.0, 3.0)), col);
            draw::fill(list, Rect::from_center_size(p, vec2(3.0, 9.0)), col);
            let style = TextStyle::new(t.bone).centered().outline(fade(t.ink, 0.9));
            let left = self
                .downed
                .get(&id)
                .and_then(|tk| secs_until(*tk))
                .map(|s| format!("  {:.0}", s.ceil()))
                .unwrap_or_default();
            list.text(
                p - vec2(0.0, 18.0),
                &format!("{} DOWN{}", name_of(info.players, id), left),
                &style,
            );
            if let Some((reviver, start, done)) = self.reviving.get(&id) {
                let frac = progress(info, *start, *done);
                progress_bar(
                    list,
                    t,
                    Rect::from_center_size(p + vec2(0.0, 10.0), vec2(40.0, 3.0)),
                    frac,
                    t.stamina,
                );
                if Some(*reviver) != info.local {
                    list.text(
                        p + vec2(0.0, 16.0),
                        &format!("{} REVIVING", name_of(info.players, *reviver)),
                        &TextStyle::new(t.stamina)
                            .centered()
                            .outline(fade(t.ink, 0.9)),
                    );
                }
            } else if !info.local_dead && dist <= info.revive_range {
                let blink = (info.time * 2.5).fract() < 0.7;
                list.text(
                    p + vec2(0.0, 10.0),
                    &format!("HOLD [{}] TO REVIVE", info.use_key),
                    &TextStyle::new(if blink { t.bone } else { t.bone_dim })
                        .centered()
                        .outline(fade(t.ink, 0.9)),
                );
            } else if !info.local_dead {
                list.text(
                    p + vec2(0.0, 10.0),
                    &format!("{:.0} M", dist),
                    &TextStyle::new(t.bone_dim)
                        .centered()
                        .outline(fade(t.ink, 0.9)),
                );
            }
        }
    }

    /// The local player is down: big panel with the respawn timer and revive progress.
    fn draw_local_downed(
        &self,
        list: &mut UiDrawList,
        t: &Theme,
        info: &ModeHudInfo,
        board: &Scoreboard,
        secs_until: &dyn Fn(u32) -> Option<f64>,
        c: Rect,
    ) {
        let Some(me) = info.local else {
            return;
        };
        let outline = fade(t.ink, 0.9);
        let y = c.max.y - 112.0;
        if let Some(tick) = self.downed.get(&me) {
            let panel = Rect::from_center_size(vec2(c.center().x, y + 14.0), vec2(230.0, 44.0));
            draw::slanted(list, panel, true, fade(t.ink, 0.6));
            draw::hline(
                list,
                panel.min.x + 10.0,
                panel.max.x,
                panel.min.y,
                t.crimson_bright,
            );
            let blink = (info.time * 1.5).fract() < 0.75;
            list.text(
                vec2(c.center().x, y),
                "YOU ARE DOWN",
                &TextStyle::new(if blink { t.crimson_bright } else { t.crimson })
                    .scale(2.0)
                    .centered()
                    .outline(outline),
            );
            let grind = board.rules.kind == GameMode::Grind;
            let line = match self.reviving.get(&me) {
                Some((r, start, done)) => {
                    let frac = progress(info, *start, *done);
                    progress_bar(
                        list,
                        t,
                        Rect::from_center_size(vec2(c.center().x, y + 30.0), vec2(160.0, 4.0)),
                        frac,
                        t.stamina,
                    );
                    format!("{} IS REVIVING YOU", name_of(info.players, *r))
                }
                None => match secs_until(*tick) {
                    Some(s) if grind => format!("OUT IN {:.0}  -  HOLD ON FOR A REVIVE", s.ceil()),
                    Some(s) => format!("CHECKPOINT IN {:.0}  -  HOLD ON FOR A REVIVE", s.ceil()),
                    None => "HOLD ON FOR A REVIVE".into(),
                },
            };
            list.text(
                vec2(c.center().x, y + 17.0),
                &line,
                &TextStyle::new(t.bone).centered().outline(outline),
            );
        } else if info.local_dead
            && board.rules.kind == GameMode::Grind
            && let Some(g) = board.grind.filter(|g| g.overrun_tick.is_none())
        {
            // Out until the next wave starts.
            let text = match g.next_wave_tick {
                Some(_) => format!("BACK FOR WAVE {}", g.wave + 1),
                None if board.rows.len() > 1 => "OUT UNTIL THE WAVE IS CLEARED".into(),
                None => "OUT".into(),
            };
            list.text(
                vec2(c.center().x, y + 10.0),
                &text,
                &TextStyle::new(t.bone_dim).centered().outline(outline),
            );
        } else if info.local_dead && board.rules.kind == GameMode::Campaign && self.wipe.is_none() {
            let text = if board.rows.len() <= 1 {
                "BACK TO THE CHECKPOINT"
            } else {
                "RESPAWNING AT THE CHECKPOINT"
            };
            list.text(
                vec2(c.center().x, y + 10.0),
                text,
                &TextStyle::new(t.bone_dim).centered().outline(outline),
            );
        }
    }

    /// The local player is reviving someone: progress bar under the crosshair.
    fn draw_reviving(&self, list: &mut UiDrawList, t: &Theme, info: &ModeHudInfo, c: Rect) {
        let Some(me) = info.local else {
            return;
        };
        let Some((target, (_, start, done))) = self.reviving.iter().find(|(_, r)| r.0 == me) else {
            return;
        };
        let frac = progress(info, *start, *done);
        let y = c.center().y + 26.0;
        list.text(
            vec2(c.center().x, y),
            &format!("REVIVING {}", name_of(info.players, *target)),
            &TextStyle::new(t.stamina)
                .centered()
                .outline(fade(t.ink, 0.9)),
        );
        progress_bar(
            list,
            t,
            Rect::from_center_size(vec2(c.center().x, y + 14.0), vec2(120.0, 4.0)),
            frac,
            t.stamina,
        );
    }
}

// ---- glue with the in-game screen (called from `ingame/mod.rs`) ------------------------------

impl InGame {
    /// `GameEvent::Mode` from the server.
    pub(super) fn on_mode_event(&mut self, e: &ModeEvent, common: &mut Common) {
        let net = self.session.as_ref().map(Session::client);
        let empty = BTreeMap::new();
        let players = net.map_or(&empty, NetClient::players);
        let local = net.and_then(NetClient::local_player_id);
        // Grind: the local record for this arena and difficulty (the results show the old one
        // and NEW BEST).
        let mut previous_best = None;
        if let ModeEvent::Ended(r) = e
            && let Some(run) = r.grind
            && let Some(level) = net.and_then(NetClient::world).map(|w| w.level_name.clone())
        {
            let s = &mut common.settings;
            previous_best = Some(s.best_grind_wave(&level, r.rules.difficulty).unwrap_or(0));
            s.record_grind_wave(&level, r.rules.difficulty, run.wave);
            common.settings_save = true;
        }
        let fx = self.modes.on_event(
            e,
            &common.theme,
            &mut self.hud,
            &mut common.audio,
            players,
            local,
            self.time,
        );
        if fx.results_opened {
            // The results screen owns the cursor and the keys now.
            self.resume();
            self.chat.close();
            if let Some(screen) = &mut self.modes.results {
                screen.previous_best_wave = previous_best;
            }
        }
        if fx.match_restarted {
            self.style.reset();
            common
                .audio
                .set_music(Common::game_music(self.world.kind()));
        }
    }

    /// The clock the HUD feeds, banners and the results reveal run on (tools / screenshots that
    /// inject events).
    pub fn hud_clock(&self) -> f64 {
        self.time
    }

    /// Shows `results` on the results screen (tools / screenshots; the server's
    /// `ModeEvent::Ended` normally does this). Drawn while the lobby phase is `Results`.
    pub fn show_results(&mut self, results: shared::sim::MatchResults) {
        let local = self.net().and_then(NetClient::local_player_id);
        let mut screen = ResultsScreen::new(results, local, self.time);
        screen.finale = Some(self.modes.gun_game.finale.weapon);
        self.modes.results = Some(screen);
    }

    /// Per-frame mode upkeep before the world is drawn: checkpoint props, downed poses.
    pub(super) fn update_mode_visuals(&mut self) {
        self.world.checkpoint_reached = self.modes.checkpoint();
        if let Some(net) = self.session.as_ref().map(Session::client) {
            let players: Vec<EntityState> = net.remote_players().copied().collect();
            ModeHud::apply_visuals(&mut self.remote, &players);
        }
    }

    /// Style meter + game-mode HUD (drawn after the main HUD).
    pub(super) fn draw_mode_hud(
        &mut self,
        common: &mut Common,
        list: &mut UiDrawList,
        now: f64,
        dt: f32,
    ) {
        let Some(net) = self.session.as_ref().map(Session::client) else {
            return;
        };
        let tuning = net.tuning();
        // Gun game: the server's ladder (it is part of the tuning it sent).
        if self.modes.gun_game != tuning.gun_game {
            self.modes.gun_game = tuning.gun_game.clone();
        }
        let weapons = *self.combat.predictor.state();
        self.style.draw(
            list,
            &common.theme,
            net.own_style(),
            &tuning.style,
            weapons.build(),
            weapons.mode,
            Some(&mut common.audio),
            now,
            dt,
        );
        let local = net.local_player_id();
        let local_dead = self.modes.scoreboard.as_ref().is_some_and(|b| {
            b.rows
                .iter()
                .any(|r| Some(r.player) == local && r.life != LifeState::Alive)
        });
        let remote: Vec<EntityState> = net.remote_players().copied().collect();
        let use_key = common
            .settings
            .bindings
            .display(crate::controls::Action::Use);
        let best_wave = self
            .modes
            .scoreboard
            .as_ref()
            .filter(|b| b.rules.kind == GameMode::Grind)
            .zip(net.world())
            .and_then(|(b, w)| {
                common
                    .settings
                    .best_grind_wave(&w.level_name, b.rules.difficulty)
            });
        let info = ModeHudInfo {
            local,
            server_tick: net.server_tick_estimate(),
            tick_rate: net.tick_rate(),
            players: net.players(),
            camera: &self.last_camera,
            local_pos: net.local_state().map(|s| s.position),
            local_dead,
            remote: &remote,
            revive_range: tuning.modes.revive_range,
            use_key: &use_key,
            time: now,
            best_wave,
        };
        self.modes
            .draw(list, &common.theme, &info, &mut common.audio);
    }

    /// The local player is downed (co-op), waiting for a revive.
    pub(super) fn local_downed(&self) -> bool {
        let local = self
            .session
            .as_ref()
            .map(Session::client)
            .and_then(NetClient::local_player_id);
        local.is_some_and(|id| self.modes.is_downed(id))
    }

    /// Music intensity: the combat layer's (alert enemies, recent fighting) nudged up by the
    /// style rank (MERCILESS and above play everything).
    pub(super) fn mode_music_intensity(&self) -> f32 {
        self.style.music_floor().max(self.combat.music)
    }

    /// Fills the score cells (columns from `scoreboard::columns_for`: FRAGS, KILLS, DEATHS,
    /// REVIVES, RANK, STYLE) and the status of one row from the server's scoreboard.
    pub(super) fn fill_score_row(&self, columns: &[ScoreColumn], row: &mut ScoreRow, id: PlayerId) {
        let Some((board, e)) = self
            .modes
            .scoreboard
            .as_ref()
            .and_then(|b| Some((b, b.rows.iter().find(|r| r.player == id)?)))
        else {
            return;
        };
        let theme = Theme::default();
        row.cell_colors = vec![None; columns.len()];
        for (i, c) in columns.iter().enumerate() {
            let text = match c.title {
                "FRAGS" => {
                    if e.frags < 0 {
                        row.cell_colors[i] = Some(theme.crimson_bright);
                    }
                    e.frags.to_string()
                }
                "KILLS" => e.kills.to_string(),
                "DEATHS" => e.deaths.to_string(),
                "REVIVES" => e.revives.to_string(),
                "TIER" => tier_label(&self.modes.gun_game, e.tier, false),
                "RANK" => {
                    row.cell_colors[i] = Some(rank_text_color(&theme, e.rank, self.time));
                    rank_name(e.rank).to_string()
                }
                "STYLE" => e.style.to_string(),
                _ => continue,
            };
            if let Some(cell) = row.cells.get_mut(i) {
                *cell = text;
            }
        }
        match e.life {
            LifeState::Downed => row.status = "DOWNED".into(),
            LifeState::Dead if board.rules.kind == GameMode::Grind => row.status = "OUT".into(),
            LifeState::Dead => row.status = "DEAD".into(),
            LifeState::Alive => {}
        }
    }

    /// Deathmatch: rows by frags (then style); gun game by tier (then kills); campaign keeps
    /// the player order.
    pub(super) fn sort_score_rows(&self, ids: &[PlayerId], rows: &mut Vec<ScoreRow>) {
        let Some(board) = self.modes.scoreboard.as_ref() else {
            return;
        };
        if board.rules.kind == GameMode::GunGame {
            let order: Vec<PlayerId> = gun_game_order(&board.rows)
                .iter()
                .map(|r| r.player)
                .collect();
            let rank = |id: &PlayerId| order.iter().position(|p| p == id).unwrap_or(usize::MAX);
            let mut paired: Vec<(PlayerId, ScoreRow)> =
                ids.iter().copied().zip(std::mem::take(rows)).collect();
            paired.sort_by_key(|p| rank(&p.0));
            *rows = paired.into_iter().map(|p| p.1).collect();
            return;
        }
        if board.rules.kind != GameMode::Deathmatch {
            return;
        }
        let key = |id: &PlayerId| {
            board
                .rows
                .iter()
                .find(|r| r.player == *id)
                .map_or((i32::MIN, 0), |r| (r.frags, r.style))
        };
        let mut paired: Vec<(PlayerId, ScoreRow)> =
            ids.iter().copied().zip(std::mem::take(rows)).collect();
        paired.sort_by_key(|p| std::cmp::Reverse(key(&p.0)));
        *rows = paired.into_iter().map(|p| p.1).collect();
    }

    /// The game-mode results screen inside the lobby's Results phase (`match_flow::results_ui`):
    /// the animated tally / standings. `None` without results data (the lobby's plain
    /// scoreboard results are shown then); otherwise the first sound of the frame and whether
    /// the reveal is done (then the lobby's buttons go under it).
    pub(super) fn draw_mode_results(
        &mut self,
        ui: &mut Ui,
        now: f64,
    ) -> Option<(Option<Sfx>, bool)> {
        let screen = self.modes.results.as_mut()?;
        let sounds = screen.draw(ui, now);
        Some((sounds.first().copied(), screen.finished(now)))
    }
}

/// Time left, top centre under the level timer slot (deathmatch, gun game).
fn draw_time_left(
    list: &mut UiDrawList,
    t: &Theme,
    info: &ModeHudInfo,
    board: &Scoreboard,
    c: Rect,
) {
    if board.rules.time_limit_secs <= 0.0 {
        return;
    }
    let Some(now) = info.server_tick else {
        return;
    };
    let elapsed = (now - board.start_tick as f64) / info.tick_rate.max(1) as f64;
    let left = (board.rules.time_limit_secs as f64 - elapsed).max(0.0);
    let col = if left < 30.0 && (info.time * 2.0).fract() < 0.5 {
        t.crimson_bright
    } else {
        t.bone_dim
    };
    let text = format!("TIME LEFT {}", &format_timer(left)[..5]);
    list.text(
        vec2(c.center().x, 30.0),
        &text,
        &TextStyle::new(col).centered().outline(fade(t.ink, 0.9)),
    );
}

/// Gun game standings: tier, then kills, then fewer deaths (the server's order).
fn gun_game_order(rows: &[ScoreEntry]) -> Vec<ScoreEntry> {
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| {
        b.tier
            .cmp(&a.tier)
            .then(b.kills.cmp(&a.kills))
            .then(a.deaths.cmp(&b.deaths))
            .then(a.player.cmp(&b.player))
    });
    rows
}

/// "TIER 4 - THRESHER" (`long`) / "4 THRESHER"; the melee finale "FINAL TIER: TRIMMER" /
/// "FINAL TRIMMER"; "WON".
pub(crate) fn tier_label(g: &GunGameTuning, tier: u8, long: bool) -> String {
    match tier_of(g, tier) {
        Tier::Weapon(WeaponBuild { weapon, .. }) => {
            let name = weapon.name().to_uppercase();
            if long {
                format!("TIER {} - {name}", tier as usize + 1)
            } else {
                format!("{} {name}", tier as usize + 1)
            }
        }
        Tier::Finale(WeaponBuild { weapon, .. }) if long => {
            format!("FINAL TIER: {}", weapon.name().to_uppercase())
        }
        Tier::Finale(WeaponBuild { weapon, .. }) => {
            format!("FINAL {}", weapon.name().to_uppercase())
        }
        Tier::Won => "WON".to_string(),
    }
}

/// The PvE difficulty under the level timer (campaign; grind shows it in its wave block).
fn draw_difficulty_tag(list: &mut UiDrawList, t: &Theme, board: &Scoreboard, c: Rect) {
    list.text(
        vec2(c.center().x, 30.0),
        board.rules.difficulty.title(),
        &TextStyle::new(t.bone_dim)
            .centered()
            .outline(fade(t.ink, 0.9)),
    );
}

fn ordinal(n: usize) -> &'static str {
    match (n % 10, n % 100) {
        (1, 11) | (2, 12) | (3, 13) => "TH",
        (1, _) => "ST",
        (2, _) => "ND",
        (3, _) => "RD",
        _ => "TH",
    }
}

fn progress(info: &ModeHudInfo, start: u32, done: u32) -> f32 {
    let Some(now) = info.server_tick else {
        return 0.0;
    };
    let span = done.wrapping_sub(start).max(1) as f64;
    ((now - start as f64) / span).clamp(0.0, 1.0) as f32
}

fn progress_bar(list: &mut UiDrawList, t: &Theme, rect: Rect, frac: f32, color: glam::Vec4) {
    draw::fill(list, rect.expand(1.0), fade(t.ink, 0.85));
    draw::fill(list, rect, fade(color, 0.2));
    let w = (rect.width() * frac.clamp(0.0, 1.0)).round();
    draw::fill(
        list,
        Rect::new(rect.min, vec2(rect.min.x + w, rect.max.y)),
        color,
    );
    if frac > 0.0 {
        draw::fill(
            list,
            Rect::new(
                vec2(rect.min.x + w - 1.0, rect.min.y - 1.0),
                vec2(rect.min.x + w + 1.0, rect.max.y + 1.0),
            ),
            t.bone,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinals() {
        assert_eq!(ordinal(1), "ST");
        assert_eq!(ordinal(2), "ND");
        assert_eq!(ordinal(3), "RD");
        assert_eq!(ordinal(4), "TH");
        assert_eq!(ordinal(11), "TH");
    }
}
