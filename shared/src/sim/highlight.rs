//! Play of the game (PvP modes): every player's bursts of kills, scored as highlights.
//! `docs/potg.md` has the whole feature; this module is the pure scoring part.
//!
//! A **burst** is one player's run of kills where each kill comes within
//! [`HighlightRules::chain_gap`] of the previous one and the run spans at most
//! [`HighlightRules::max_span`]. It **closes** once its last kill is `chain_gap` old (nothing can
//! join it any more) or when the next kill would make it too long; a closed burst becomes a
//! [`Highlight`]: whose, the ticks of its first and last moment, a score and a title
//! ("TRIPLE KILL", "LONGSHOT 87m", "AIRBORNE DOUBLE").
//!
//! **Score**: every kill is worth its base (a frag, or less for a Mould) plus what made it
//! special — headshot, the killer airborne, the victim airborne, distance (per metre past
//! `distance_free`, plus LONGSHOT at `longshot_distance` for hitscan shots and arrows), a
//! returned projectile, thorn ricochets, a slam or a punch, a close call, the killer's style
//! rank — plus a tempo bonus for kills that follow quickly and the burst's parries; the sum is
//! multiplied by `1 + chain × (kills − 1)`, so multi-kills win.
//!
//! The server feeds a [`HighlightTracker`] in PvP modes (`server/src/highlights.rs`): a closed
//! burst that beats the best so far is announced (`GameEvent::HighlightMark`) so every client
//! keeps that stretch of its recording; at match end [`HighlightTracker::finish`] closes
//! everything still open and picks the play of the game (`GameEvent::PlayOfTheGame`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::combat::{Attack, DamageFlags, KillInfo};
use super::style::is_longshot;
use super::{HitTarget, PlayerId};
use crate::protocol::kind;

/// One player's best moment: a burst of kills from `start_tick` to `end_tick` (server ticks of
/// the first and last moment that counted: kills, a parry just before them).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub player: PlayerId,
    pub start_tick: u32,
    pub end_tick: u32,
    pub score: f32,
    /// "TRIPLE KILL", "LONGSHOT 87m", "AIRBORNE DOUBLE"... (ASCII, short).
    pub title: String,
}

/// Timing and weights of highlight scoring (starting values; see the module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct HighlightRules {
    /// A kill joins its killer's open burst if it comes within this many seconds of the
    /// burst's last kill; a burst closes once its last kill is this old.
    pub chain_gap: f32,
    /// A burst spans at most this long (first to last kill).
    pub max_span: f32,
    /// Base points of a player kill ...
    pub frag: f32,
    /// ... and of a Mould kill (PvP modes rarely have any).
    pub enemy_kill: f32,
    /// Burst total × (1 + `chain` × (kills − 1)).
    pub chain: f32,
    /// Up to this much per kill after the first, the sooner it follows the previous one
    /// (linear down to 0 at `chain_gap`).
    pub quick: f32,
    pub headshot: f32,
    /// The killer was in the air.
    pub airborne: f32,
    /// The victim was in the air.
    pub airshot: f32,
    /// Per metre of kill distance beyond `distance_free` (up to `distance_cap`).
    pub per_metre: f32,
    pub distance_free: f32,
    pub distance_cap: f32,
    /// LONGSHOT: a hitscan / arrow kill from at least `longshot_distance` metres.
    pub longshot: f32,
    pub longshot_distance: f32,
    /// A kill with a projectile the killer had parried / deflected back.
    pub returned: f32,
    /// Per thorn bounce of the killing shot.
    pub ricochet: f32,
    pub slam: f32,
    pub fist: f32,
    /// A kill while the killer's health is below `close_call_health`.
    pub close_call: f32,
    pub close_call_health: f32,
    /// Per style rank of the killer at the time of the kill (DULL 0 .. VELOCIDE 7).
    pub per_rank: f32,
    /// Per parry during the burst (or within `chain_gap` before it).
    pub parry: f32,
}

impl Default for HighlightRules {
    fn default() -> Self {
        Self {
            chain_gap: 3.0,
            max_span: 10.0,
            frag: 100.0,
            enemy_kill: 25.0,
            chain: 0.35,
            quick: 60.0,
            headshot: 40.0,
            airborne: 50.0,
            airshot: 25.0,
            per_metre: 1.5,
            distance_free: 15.0,
            distance_cap: 120.0,
            longshot: 50.0,
            longshot_distance: 40.0,
            returned: 90.0,
            ricochet: 45.0,
            slam: 50.0,
            fist: 40.0,
            close_call: 40.0,
            close_call_health: 30.0,
            per_rank: 8.0,
            parry: 35.0,
        }
    }
}

/// What the scoring keeps of one kill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KillNote {
    pub tick: u32,
    pub frag: bool,
    pub headshot: bool,
    pub airborne: bool,
    pub airshot: bool,
    pub distance: f32,
    pub longshot: bool,
    pub returned: bool,
    pub ricochets: u8,
    pub slam: bool,
    pub fist: bool,
    pub close_call: bool,
    pub rank: u8,
}

impl KillNote {
    /// The kill's note and its killer, `None` for kills that don't count (no player killer,
    /// suicides).
    pub fn from_kill(k: &KillInfo, rank: u8, rules: &HighlightRules) -> Option<(PlayerId, Self)> {
        let killer = k.killer.player()?;
        if k.victim == HitTarget::Player(killer) {
            return None;
        }
        let distance = if k.distance.is_finite() {
            k.distance.max(0.0)
        } else {
            0.0
        };
        Some((
            killer,
            KillNote {
                tick: k.tick,
                frag: k.victim_kind == kind::PLAYER,
                headshot: k.flags.contains(DamageFlags::HEADSHOT),
                airborne: k.killer_airborne,
                airshot: k.victim_airborne,
                distance,
                longshot: is_longshot(k, rules.longshot_distance),
                returned: k.parried_projectile,
                ricochets: k.ricochets,
                slam: k.attack == Attack::Slam,
                fist: k.attack == Attack::Fist,
                close_call: k.killer_health > 0.0 && k.killer_health < rules.close_call_health,
                rank: rank.min(7),
            },
        ))
    }

    /// Points of this kill on its own (no tempo, chain or parries).
    pub fn points(&self, r: &HighlightRules) -> f32 {
        let flag = |on: bool, points: f32| if on { points } else { 0.0 };
        let metres = (self.distance.min(r.distance_cap) - r.distance_free).max(0.0);
        flag(self.frag, r.frag)
            + flag(!self.frag, r.enemy_kill)
            + flag(self.headshot, r.headshot)
            + flag(self.airborne, r.airborne)
            + flag(self.airshot, r.airshot)
            + metres * r.per_metre
            + flag(self.longshot, r.longshot)
            + flag(self.returned, r.returned)
            + self.ricochets as f32 * r.ricochet
            + flag(self.slam, r.slam)
            + flag(self.fist, r.fist)
            + flag(self.close_call, r.close_call)
            + self.rank as f32 * r.per_rank
    }
}

/// Score of a burst of `kills` (ascending ticks) with `parries` parries in it.
pub fn burst_score(kills: &[KillNote], parries: usize, r: &HighlightRules, tick_rate: u32) -> f32 {
    if kills.is_empty() {
        return 0.0;
    }
    let rate = tick_rate.max(1) as f32;
    let mut total: f32 = kills.iter().map(|k| k.points(r)).sum();
    for w in kills.windows(2) {
        let gap = w[1].tick.saturating_sub(w[0].tick) as f32 / rate;
        total += r.quick * (1.0 - gap / r.chain_gap.max(1e-3)).clamp(0.0, 1.0);
    }
    total += parries as f32 * r.parry;
    total * (1.0 + r.chain * (kills.len() - 1) as f32)
}

/// The title of a burst: the multi-kill word, sharpened by what every kill had in common
/// ("AIRBORNE DOUBLE", "HEADSHOT TRIPLE"), or for a single kill what made it special
/// ("LONGSHOT 87m", "RETURN TO SENDER", "AIRBORNE HEADSHOT").
pub fn burst_title(kills: &[KillNote], parries: usize) -> String {
    let all = |f: fn(&KillNote) -> bool| !kills.is_empty() && kills.iter().all(f);
    match kills.len() {
        0 => String::new(),
        1 => {
            let k = &kills[0];
            let metres = k.distance.round() as u32;
            if k.returned {
                "RETURN TO SENDER".into()
            } else if k.ricochets > 1 {
                format!("RICOCHET x{}", k.ricochets)
            } else if k.ricochets == 1 {
                "RICOCHET".into()
            } else if k.longshot {
                format!("LONGSHOT {metres}m")
            } else if k.slam {
                "SLAM KILL".into()
            } else if k.airborne && k.headshot {
                "AIRBORNE HEADSHOT".into()
            } else if k.airborne {
                "AIRKILL".into()
            } else if k.fist {
                "FISTED".into()
            } else if parries > 0 {
                "PARRY AND PUNISH".into()
            } else if k.close_call {
                "CLOSE CALL".into()
            } else if k.headshot {
                "HEADSHOT".into()
            } else if k.frag {
                "FRAG".into()
            } else {
                "KILL".into()
            }
        }
        n => {
            let word = match n {
                2 => "DOUBLE".to_string(),
                3 => "TRIPLE".to_string(),
                4 => "QUAD".to_string(),
                5 => "PENTA".to_string(),
                _ => format!("RAMPAGE x{n}"),
            };
            let prefix = if all(|k| k.airborne) {
                Some("AIRBORNE")
            } else if all(|k| k.longshot) {
                Some("LONGSHOT")
            } else if all(|k| k.headshot) {
                Some("HEADSHOT")
            } else if all(|k| k.ricochets > 0) {
                Some("RICOCHET")
            } else {
                None
            };
            match prefix {
                Some(p) => format!("{p} {word}"),
                None if n <= 5 => format!("{word} KILL"),
                None => word,
            }
        }
    }
}

/// One player's open burst.
#[derive(Clone, Debug, Default)]
struct Burst {
    kills: Vec<KillNote>,
}

/// Closed highlights kept (best first): the play of the game can fall back to the next best
/// when its player left.
const KEPT: usize = 16;

/// Tracks every player's bursts and the best highlights of a match (server-side, PvP).
#[derive(Clone, Debug)]
pub struct HighlightTracker {
    rules: HighlightRules,
    tick_rate: u32,
    open: BTreeMap<PlayerId, Burst>,
    /// Recent parry ticks per player.
    parries: BTreeMap<PlayerId, Vec<u32>>,
    /// Closed highlights, best first.
    closed: Vec<Highlight>,
    /// A new best closed since the last [`HighlightTracker::update`].
    announce: Option<Highlight>,
}

impl HighlightTracker {
    pub fn new(rules: HighlightRules, tick_rate: u32) -> Self {
        Self {
            rules,
            tick_rate: tick_rate.max(1),
            open: BTreeMap::new(),
            parries: BTreeMap::new(),
            closed: Vec::new(),
            announce: None,
        }
    }

    pub fn rules(&self) -> &HighlightRules {
        &self.rules
    }

    /// Changes the rules (tuning reload) for the kills to come; what is recorded stays.
    pub fn rules_mut(&mut self) -> &mut HighlightRules {
        &mut self.rules
    }

    /// Forgets everything (a new match).
    pub fn reset(&mut self) {
        self.open.clear();
        self.parries.clear();
        self.closed.clear();
        self.announce = None;
    }

    fn ticks(&self, secs: f32) -> u32 {
        (secs.max(0.0) * self.tick_rate as f32).round() as u32
    }

    /// The best closed highlight so far.
    pub fn best(&self) -> Option<&Highlight> {
        self.closed.first()
    }

    /// Bursts still open (tests, debugging).
    pub fn open_bursts(&self) -> usize {
        self.open.len()
    }

    /// A kill (`SimEvent::Killed`); `rank` is the killer's style rank at the time. Kills
    /// without a player killer and suicides are ignored.
    pub fn kill(&mut self, k: &KillInfo, rank: u8) {
        let Some((killer, note)) = KillNote::from_kill(k, rank, &self.rules) else {
            return;
        };
        let (gap, span) = (
            self.ticks(self.rules.chain_gap),
            self.ticks(self.rules.max_span),
        );
        let breaks = self.open.get(&killer).is_some_and(|b| {
            let first = b.kills.first().map_or(note.tick, |k| k.tick);
            let last = b.kills.last().map_or(note.tick, |k| k.tick);
            note.tick.saturating_sub(last) > gap || note.tick.saturating_sub(first) > span
        });
        if breaks && let Some(b) = self.open.remove(&killer) {
            self.close(killer, b);
        }
        self.open.entry(killer).or_default().kills.push(note);
    }

    /// A successful parry (`SimEvent::Parried`): adds to the player's burst around it.
    pub fn parry(&mut self, player: PlayerId, tick: u32) {
        let keep = self.ticks(self.rules.max_span + 2.0 * self.rules.chain_gap);
        let list = self.parries.entry(player).or_default();
        list.retain(|t| tick.saturating_sub(*t) <= keep);
        list.push(tick);
    }

    /// Closes bursts whose last kill is `chain_gap` old at `tick`. Returns a highlight to
    /// announce when one of them (or a burst broken by a later kill) beat the best so far.
    pub fn update(&mut self, tick: u32) -> Option<Highlight> {
        let gap = self.ticks(self.rules.chain_gap);
        let done: Vec<PlayerId> = self
            .open
            .iter()
            .filter(|(_, b)| {
                b.kills
                    .last()
                    .is_none_or(|k| tick.saturating_sub(k.tick) >= gap)
            })
            .map(|(p, _)| *p)
            .collect();
        for p in done {
            if let Some(b) = self.open.remove(&p) {
                self.close(p, b);
            }
        }
        self.announce.take()
    }

    /// The match is over: closes every burst and returns the play of the game — the best
    /// highlight of a player still `present`, else the best of anyone; `None` without kills.
    pub fn finish(&mut self, present: impl Fn(PlayerId) -> bool) -> Option<Highlight> {
        let open = std::mem::take(&mut self.open);
        for (p, b) in open {
            self.close(p, b);
        }
        self.announce = None;
        self.closed
            .iter()
            .find(|h| present(h.player))
            .or_else(|| self.closed.first())
            .cloned()
    }

    fn close(&mut self, player: PlayerId, burst: Burst) {
        let (Some(first), Some(last)) = (burst.kills.first(), burst.kills.last()) else {
            return;
        };
        let gap = self.ticks(self.rules.chain_gap);
        let (first, last) = (first.tick, last.tick);
        // Parries during the burst, or shortly before its first kill ("parry and punish"); the
        // highlight starts at the earliest of them.
        let parries: Vec<u32> = self.parries.get(&player).map_or(Vec::new(), |list| {
            list.iter()
                .copied()
                .filter(|&t| t.saturating_add(gap) >= first && t <= last)
                .collect()
        });
        let h = Highlight {
            player,
            start_tick: parries.iter().copied().fold(first, u32::min),
            end_tick: last,
            score: burst_score(&burst.kills, parries.len(), &self.rules, self.tick_rate),
            title: burst_title(&burst.kills, parries.len()),
        };
        let new_best = self.closed.first().is_none_or(|b| h.score > b.score);
        let at = self
            .closed
            .iter()
            .position(|c| h.score > c.score)
            .unwrap_or(self.closed.len());
        self.closed.insert(at, h.clone());
        self.closed.truncate(KEPT);
        if new_best {
            self.announce = Some(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::sim::HitPart;
    use crate::sim::combat::Owner;

    const RATE: u32 = 60;

    fn secs(s: f32) -> u32 {
        (s * RATE as f32).round() as u32
    }

    /// Player `by` kills player `victim` at `tick` with a Quill body shot from 10 m.
    fn frag(by: u8, victim: u8, tick: u32) -> KillInfo {
        KillInfo {
            victim: HitTarget::Player(PlayerId(victim)),
            victim_kind: kind::PLAYER,
            killer: Owner::Player(PlayerId(by)),
            attack: Attack::Quill,
            flags: DamageFlags::HITSCAN,
            part: HitPart::Body,
            position: Vec3::ZERO,
            ricochets: 0,
            killer_airborne: false,
            victim_airborne: false,
            parried_projectile: false,
            killer_health: 100.0,
            killer_effective_max: 100.0,
            overkill: 0.0,
            distance: 10.0,
            weapon: Some(crate::loadout::WeaponKind::Quill),
            tick,
        }
    }

    fn tracker() -> HighlightTracker {
        HighlightTracker::new(HighlightRules::default(), RATE)
    }

    fn note(f: impl Fn(&mut KillNote)) -> KillNote {
        let mut k = KillNote {
            tick: 0,
            frag: true,
            headshot: false,
            airborne: false,
            airshot: false,
            distance: 10.0,
            longshot: false,
            returned: false,
            ricochets: 0,
            slam: false,
            fist: false,
            close_call: false,
            rank: 0,
        };
        f(&mut k);
        k
    }

    #[test]
    fn a_plain_frag_is_a_highlight_once_its_burst_closes() {
        let mut t = tracker();
        t.kill(&frag(1, 2, 1000), 0);
        assert_eq!(
            t.update(1000 + secs(1.0)),
            None,
            "still open: a kill could join it"
        );
        let h = t.update(1000 + secs(3.0)).expect("closed and announced");
        assert_eq!(h.player, PlayerId(1));
        assert_eq!((h.start_tick, h.end_tick), (1000, 1000));
        assert_eq!(h.title, "FRAG");
        assert_eq!(h.score, 100.0);
        assert_eq!(t.best(), Some(&h));
        // Announced once.
        assert_eq!(t.update(1000 + secs(5.0)), None);
    }

    #[test]
    fn quick_kills_chain_into_a_multikill_that_beats_any_single_kill() {
        let mut t = tracker();
        for (i, at) in [0.0, 0.8, 1.9].into_iter().enumerate() {
            t.kill(&frag(1, 2 + i as u8, 600 + secs(at)), 0);
        }
        let h = t.update(600 + secs(5.0)).expect("closed");
        assert_eq!(h.title, "TRIPLE KILL");
        assert_eq!((h.start_tick, h.end_tick), (600, 600 + secs(1.9)));
        assert!(h.score > 3.0 * 100.0 * 1.6, "{}", h.score);
        // A flashy single kill: an 87 m airborne headshot at VELOCIDE.
        let mut k = frag(3, 1, 2000);
        k.distance = 87.0;
        k.killer_airborne = true;
        k.flags = DamageFlags::HITSCAN | DamageFlags::HEADSHOT;
        t.kill(&k, 7);
        assert_eq!(
            t.update(2000 + secs(4.0)),
            None,
            "not better than the triple"
        );
        assert_eq!(t.best().map(|b| b.player), Some(PlayerId(1)));
    }

    #[test]
    fn slow_kills_are_separate_bursts_and_bursts_have_a_maximum_span() {
        let mut t = tracker();
        // 4 s apart: the gap (3 s) splits them.
        t.kill(&frag(1, 2, 100), 0);
        t.kill(&frag(1, 3, 100 + secs(4.0)), 0);
        let first = t.update(100 + secs(4.0)).expect("the first one closed");
        assert_eq!(first.title, "FRAG");
        assert_eq!(t.open_bursts(), 1);
        // A kill every 2.5 s keeps chaining, but a burst stops at 10 s.
        let mut t = tracker();
        for i in 0..6 {
            t.kill(&frag(1, 2, 100 + secs(2.5 * i as f32)), 0);
        }
        let h = t.update(100 + secs(12.5)).expect("the first 10 s closed");
        assert_eq!(h.title, "PENTA KILL");
        assert_eq!(h.end_tick - h.start_tick, secs(10.0));
        assert_eq!(t.open_bursts(), 1, "the sixth kill started a new burst");
    }

    #[test]
    fn titles_name_what_made_it_special() {
        let air = note(|k| k.airborne = true);
        assert_eq!(burst_title(&[air, air], 0), "AIRBORNE DOUBLE");
        let head = note(|k| k.headshot = true);
        assert_eq!(burst_title(&[head, head, head], 0), "HEADSHOT TRIPLE");
        assert_eq!(burst_title(&[head, air, head, air], 0), "QUAD KILL");
        assert_eq!(burst_title(&[head; 7], 0), "HEADSHOT RAMPAGE x7");
        assert_eq!(burst_title(&[note(|_| {}); 6], 0), "RAMPAGE x6");
        let far = note(|k| {
            k.distance = 86.6;
            k.longshot = true;
        });
        assert_eq!(burst_title(&[far], 0), "LONGSHOT 87m");
        assert_eq!(
            burst_title(&[note(|k| k.returned = true)], 0),
            "RETURN TO SENDER"
        );
        assert_eq!(burst_title(&[note(|k| k.ricochets = 2)], 0), "RICOCHET x2");
        let air_head = note(|k| {
            k.airborne = true;
            k.headshot = true;
        });
        assert_eq!(burst_title(&[air_head], 0), "AIRBORNE HEADSHOT");
        assert_eq!(burst_title(&[note(|k| k.slam = true)], 0), "SLAM KILL");
        assert_eq!(burst_title(&[note(|_| {})], 1), "PARRY AND PUNISH");
        assert_eq!(burst_title(&[note(|k| k.frag = false)], 0), "KILL");
    }

    #[test]
    fn distance_style_and_tricks_add_up() {
        let r = HighlightRules::default();
        let (_, plain) = KillNote::from_kill(&frag(1, 2, 0), 0, &r).unwrap();
        assert_eq!(plain.points(&r), 100.0);
        // 87 m hitscan: distance points past 15 m plus LONGSHOT.
        let mut k = frag(1, 2, 0);
        k.distance = 87.0;
        let (_, far) = KillNote::from_kill(&k, 0, &r).unwrap();
        assert!(far.longshot);
        assert_eq!(far.points(&r), 100.0 + 72.0 * 1.5 + 50.0);
        // An explosion from that far is no LONGSHOT (only hitscan shots and arrows).
        k.flags = DamageFlags::PROJECTILE | DamageFlags::EXPLOSIVE;
        let (_, bomb) = KillNote::from_kill(&k, 0, &r).unwrap();
        assert!(!bomb.longshot);
        // Style rank, ricochets, a returned projectile.
        let mut k = frag(1, 2, 0);
        k.ricochets = 2;
        k.parried_projectile = true;
        let (_, trick) = KillNote::from_kill(&k, 4, &r).unwrap();
        assert_eq!(trick.points(&r), 100.0 + 2.0 * 45.0 + 90.0 + 4.0 * 8.0);
        // A tighter double scores more than a slower one, both more than two single frags.
        let quick = [plain, KillNote { tick: 10, ..plain }];
        let slow = [plain, KillNote { tick: 150, ..plain }];
        assert!(burst_score(&quick, 0, &r, RATE) > burst_score(&slow, 0, &r, RATE));
        assert!(burst_score(&slow, 0, &r, RATE) > 2.0 * 100.0);
    }

    #[test]
    fn suicides_and_world_deaths_dont_count() {
        let mut t = tracker();
        t.kill(&frag(1, 1, 100), 0);
        let mut world = frag(1, 2, 100);
        world.killer = Owner::World;
        world.attack = Attack::World;
        t.kill(&world, 0);
        assert_eq!(t.open_bursts(), 0);
        assert_eq!(t.finish(|_| true), None, "no kills, no play of the game");
    }

    #[test]
    fn only_new_bests_are_announced() {
        let mut t = tracker();
        // P1: a double. P2 (later): a single, then a triple.
        t.kill(&frag(1, 2, 0), 0);
        t.kill(&frag(1, 3, 30), 0);
        let h = t.update(30 + secs(3.0));
        assert_eq!(h.map(|h| h.player), Some(PlayerId(1)));
        t.kill(&frag(2, 1, 1000), 0);
        assert_eq!(
            t.update(1000 + secs(3.0)),
            None,
            "a single frag is not better"
        );
        for i in 0..3 {
            t.kill(&frag(2, 3, 2000 + 20 * i), 0);
        }
        let h = t.update(2040 + secs(3.0)).expect("the triple is");
        assert_eq!((h.player, h.title.as_str()), (PlayerId(2), "TRIPLE KILL"));
    }

    #[test]
    fn finish_closes_open_bursts_and_prefers_players_still_here() {
        let mut t = tracker();
        t.kill(&frag(1, 2, 0), 0);
        t.kill(&frag(1, 3, 20), 0);
        t.update(20 + secs(3.0));
        // The last kills of the match: P2's triple, still open when the frag limit hits.
        for i in 0..3 {
            t.kill(&frag(2, 1, 5000 + 15 * i), 0);
        }
        let potg = t.finish(|_| true).expect("play of the game");
        assert_eq!((potg.player, potg.end_tick), (PlayerId(2), 5030));
        assert_eq!(t.open_bursts(), 0);
        // P2 left before the end: P1's double instead.
        let potg = t.finish(|p| p != PlayerId(2)).expect("fallback");
        assert_eq!(potg.player, PlayerId(1));
        // Nobody is left at all: still the best one.
        assert_eq!(t.finish(|_| false).map(|h| h.player), Some(PlayerId(2)));
        t.reset();
        assert_eq!(t.best(), None);
    }

    #[test]
    fn a_parry_just_before_the_kills_counts_and_starts_the_highlight() {
        let mut t = tracker();
        t.parry(PlayerId(1), 900);
        t.kill(&frag(1, 2, 1000), 0);
        let h = t.update(1000 + secs(3.0)).expect("closed");
        assert_eq!(h.start_tick, 900);
        assert_eq!(h.title, "PARRY AND PUNISH");
        assert_eq!(h.score, 100.0 + 35.0);
        // A parry long before doesn't.
        let mut t = tracker();
        t.parry(PlayerId(1), 100);
        t.kill(&frag(1, 2, 1000), 0);
        let h = t.update(1000 + secs(3.0)).expect("closed");
        assert_eq!((h.start_tick, h.title.as_str()), (1000, "FRAG"));
    }
}
