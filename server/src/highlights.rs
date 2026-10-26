//! Play of the game (PvP modes, docs/potg.md): feeds every tick's kills and parries to the
//! highlight tracker (`shared::sim::highlight`), announces a new best highlight to everyone
//! (`GameEvent::HighlightMark`: clients keep that stretch of their recording) and, in the tick
//! the match ends, sends the play of the game (`GameEvent::PlayOfTheGame`) **before** the
//! results go out (`Server::send_mode_events` calls this first).

use shared::protocol::{GameEvent, ServerMessage};
use shared::sim::{HighlightRules, HighlightTracker, ModeEvent, SimEvent};

use crate::Server;

/// The tracker of the running match.
#[derive(Debug)]
pub struct Highlights {
    tracker: HighlightTracker,
    /// `ModeState::start_tick` of the match being tracked (a new match world / restart
    /// starts over).
    match_start: Option<u32>,
}

impl Highlights {
    pub fn new(tick_rate: u32) -> Self {
        Self {
            tracker: HighlightTracker::new(HighlightRules::default(), tick_rate),
            match_start: None,
        }
    }

    /// The tracker (tests, tools).
    pub fn tracker(&self) -> &HighlightTracker {
        &self.tracker
    }
}

impl Server {
    /// One tick's highlight bookkeeping (called by `send_mode_events` before it broadcasts the
    /// tick's mode events, so `PlayOfTheGame` arrives before `ModeEvent::Ended`).
    pub(crate) fn update_highlights(&mut self, events: &[SimEvent]) {
        let mode = &self.world.mode;
        // PvP matches only: not co-op, not a world running in the lobby phase (warm-up).
        if !mode.rules.kind.is_pvp() || !self.lobby_phase().level_running() {
            self.highlights.match_start = None;
            return;
        }
        if self.highlights.match_start != Some(mode.start_tick) {
            self.highlights.tracker.reset();
            self.highlights.match_start = Some(mode.start_tick);
        }
        // LONGSHOT is the style's threshold (tuning), so the title and the bonus agree.
        self.highlights.tracker.rules_mut().longshot_distance = self.tuning.style.longshot_distance;
        let mut ended = false;
        for event in events {
            match event {
                SimEvent::Killed(k) => {
                    let rank = k
                        .killer
                        .player()
                        .and_then(|p| self.world.style.get(p))
                        .map_or(0, |s| s.rank);
                    self.highlights.tracker.kill(k, rank);
                }
                SimEvent::Parried(p) => self.highlights.tracker.parry(p.player, p.tick),
                SimEvent::Mode(ModeEvent::Ended(_)) => ended = true,
                _ => {}
            }
        }
        if let Some(best) = self.highlights.tracker.update(self.world.tick) {
            log::info!(
                "highlight: {} {:?} {:.0} ({}..{})",
                best.player,
                best.title,
                best.score,
                best.start_tick,
                best.end_tick
            );
            self.broadcast(&ServerMessage::Game(GameEvent::HighlightMark(best)));
        }
        if ended {
            let players = &self.world.players;
            let potg = self.highlights.tracker.finish(|p| players.contains_key(&p));
            if let Some(potg) = potg {
                log::info!(
                    "play of the game: {} {:?} {:.0}",
                    potg.player,
                    potg.title,
                    potg.score
                );
                self.broadcast(&ServerMessage::Game(GameEvent::PlayOfTheGame(potg)));
            }
        }
    }
}
