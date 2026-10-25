//! Style and game-mode traffic: turns the simulation's [`SimEvent::Style`] /
//! [`SimEvent::Mode`] events into reliable messages (style → the owning player only, mode
//! events → everyone) and hands a finished match to the end-of-match hook. The rules
//! themselves live in `shared::sim::modes` / `shared::sim::style` (docs/style_modes.md).

use shared::protocol::GameEvent;
use shared::sim::{MatchResults, ModeEvent, PlayerId, SimEvent};

use crate::Server;

/// Reliable messages produced by one tick's style / mode events.
#[derive(Debug, Default)]
pub struct ModeRouted {
    pub broadcast: Vec<GameEvent>,
    pub to_player: Vec<(PlayerId, GameEvent)>,
    /// The match ended this tick.
    pub ended: Option<Box<MatchResults>>,
}

/// Splits one tick's events into style messages (owner only) and mode messages (everyone).
pub fn route_events(events: &[SimEvent]) -> ModeRouted {
    let mut routed = ModeRouted::default();
    for event in events {
        match event {
            SimEvent::Style(e) => routed.to_player.push((e.player(), GameEvent::Style(*e))),
            SimEvent::Mode(e) => {
                if let ModeEvent::Ended(results) = e {
                    routed.ended = Some(results.clone());
                }
                routed.broadcast.push(GameEvent::Mode(e.clone()));
            }
            _ => {}
        }
    }
    routed
}

impl Server {
    /// Sends one tick's style / mode messages (called by `run_tick`).
    pub(crate) fn send_mode_events(&mut self, events: &[SimEvent]) {
        let routed = route_events(events);
        for event in routed.broadcast {
            self.broadcast(&shared::protocol::ServerMessage::Game(event));
        }
        for (player, event) in routed.to_player {
            if let Some(client) = self.client_of(player) {
                self.send(client, &shared::protocol::ServerMessage::Game(event));
            }
        }
        if let Some(results) = routed.ended {
            self.on_match_ended(&results);
        }
    }

    /// The sim's mode rules ended the match (campaign exit, frag / time limit): the results
    /// went out with this tick's events; the lobby flow moves to its Results phase
    /// (`Server::end_match`, docs/lobby.md), which also returns everyone to the lobby later.
    fn on_match_ended(&mut self, results: &MatchResults) {
        log::info!(
            "match over ({:?}, {:.0} s): {}",
            results.reason,
            results.duration_secs,
            results
                .players
                .iter()
                .map(|p| format!(
                    "{} {}k/{}d/{}f {}pts{}",
                    p.name,
                    p.stats.kills,
                    p.stats.deaths,
                    p.stats.frags,
                    p.style_points,
                    p.grades
                        .map(|g| format!(" {}", g.overall.letter()))
                        .unwrap_or_default()
                ))
                .collect::<Vec<_>>()
                .join(", ")
        );
        self.end_match();
    }

    /// The lobby flow ended the match (host / admin `EndMatch`): end the sim's match too so
    /// results (grades, standings) are built and broadcast with the next tick.
    pub(crate) fn end_sim_match(&mut self) {
        let tuning = self.tuning.clone();
        self.world.end_match(&tuning, shared::sim::EndReason::Host);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::sim::{StyleBonus, StyleEvent};

    #[test]
    fn style_goes_to_its_owner_and_mode_to_everyone() {
        let events = vec![
            SimEvent::Style(StyleEvent::Bonus {
                player: PlayerId(2),
                bonus: StyleBonus::Parry,
                points: 60.0,
                count: 0,
            }),
            SimEvent::Mode(ModeEvent::Checkpoint {
                index: 1,
                player: PlayerId(0),
            }),
        ];
        let r = route_events(&events);
        assert_eq!(r.to_player.len(), 1);
        assert_eq!(r.to_player[0].0, PlayerId(2));
        assert_eq!(r.broadcast.len(), 1);
        assert!(r.ended.is_none());
    }
}
