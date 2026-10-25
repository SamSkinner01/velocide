//! The style meter: per-player, server-authoritative scoring of flashy play.
//!
//! Every player has a [`PlayerStyle`]: a meter that fills the current rank's bar
//! (`style.points_per_rank`), ranks up when it overflows and decays continuously (faster at
//! higher ranks); falling below zero drops a rank. Bonuses come from the tick's combat events
//! ([`SimWorld::score_style`], run by `SimWorld::step_modes` at the end of every tick):
//!
//! | bonus | from |
//! |---|---|
//! | KILL / FRAG | `SimEvent::Killed` by the player (enemy / another player) |
//! | HEADSHOT, AIRKILL, AIRSHOT, RETURN TO SENDER, EXPLODED, SLAM KILL, FISTED | flags of the [`KillInfo`] |
//! | MULTIKILL | `n` kills within `multikill_window` (`multikill × (n − 1)` in total) |
//! | CLOSE CALL | a kill while the killer's health is below `close_call_health` (×1.5) |
//! | PARRY | `SimEvent::Parried` |
//! | RICOCHET | a ricochet shot dealt damage (+ per extra bounce) |
//!
//! Gains are multiplied by the **freshness** of the weapon mode / action ([`StyleKey`]) that
//! scored: each use costs `freshness_cost`, restores every other key a little and everything
//! recovers over time, so repeating the same thing is worth less and less ("- STALE").
//!
//! Results: [`StyleEvent`]s (sent reliably to the owning player only: the HUD feed, rank-up
//! banner and sounds), the replicated [`StyleView`] in the owner's `OwnState` (meter, rank,
//! freshness) and the rank on the scoreboard (`sim::modes`). Numbers: `StyleTuning`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::combat::{Attack, DamageFlags, KillInfo, Owner};
use super::{HitTarget, PlayerId, SimEvent, SimWorld};
use crate::protocol::kind;
use crate::tuning::{StyleTuning, Tuning};

/// Number of style ranks.
pub const RANK_COUNT: usize = 8;
/// The highest rank index.
pub const TOP_RANK: u8 = RANK_COUNT as u8 - 1;

/// Rank names, lowest to highest.
pub const RANK_NAMES: [&str; RANK_COUNT] = [
    "DULL",
    "CRUDE",
    "FERAL",
    "VICIOUS",
    "MERCILESS",
    "GRISLY",
    "SANGUINE",
    "VELOCIDE",
];

/// Display name of a rank (clamped).
pub fn rank_name(rank: u8) -> &'static str {
    RANK_NAMES[(rank as usize).min(RANK_COUNT - 1)]
}

/// One kind of style bonus (the style feed shows its [`StyleBonus::label`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StyleBonus {
    Kill,
    Frag,
    Headshot,
    Airkill,
    Airshot,
    Multikill,
    Parry,
    ReturnToSender,
    Ricochet,
    Exploded,
    SlamKill,
    Fisted,
    CloseCall,
    /// Informational: the gain was cut by staleness (`points` = how much was lost).
    Stale,
}

impl StyleBonus {
    pub fn label(self) -> &'static str {
        match self {
            StyleBonus::Kill => "KILL",
            StyleBonus::Frag => "FRAG",
            StyleBonus::Headshot => "HEADSHOT",
            StyleBonus::Airkill => "AIRKILL",
            StyleBonus::Airshot => "AIRSHOT",
            StyleBonus::Multikill => "MULTIKILL",
            StyleBonus::Parry => "PARRY",
            StyleBonus::ReturnToSender => "RETURN TO SENDER",
            StyleBonus::Ricochet => "RICOCHET",
            StyleBonus::Exploded => "EXPLODED",
            StyleBonus::SlamKill => "SLAM KILL",
            StyleBonus::Fisted => "FISTED",
            StyleBonus::CloseCall => "CLOSE CALL",
            StyleBonus::Stale => "STALE",
        }
    }
}

/// What freshness is tracked for: every weapon fire mode plus the fist, the slam and parries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StyleKey {
    Lancet,
    Overcharge,
    Fan,
    Thresher,
    Slug,
    Flak,
    Riveter,
    Sawblade,
    Volley,
    Cinderbox,
    Sticky,
    Mortar,
    Fist,
    Slam,
    Parry,
}

impl StyleKey {
    pub const COUNT: usize = 15;
    pub const ALL: [StyleKey; Self::COUNT] = [
        StyleKey::Lancet,
        StyleKey::Overcharge,
        StyleKey::Fan,
        StyleKey::Thresher,
        StyleKey::Slug,
        StyleKey::Flak,
        StyleKey::Riveter,
        StyleKey::Sawblade,
        StyleKey::Volley,
        StyleKey::Cinderbox,
        StyleKey::Sticky,
        StyleKey::Mortar,
        StyleKey::Fist,
        StyleKey::Slam,
        StyleKey::Parry,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The key an attack scores under (`None` for enemy attacks, the world, dev commands).
    pub fn from_attack(attack: Attack) -> Option<StyleKey> {
        Some(match attack {
            Attack::Lancet => StyleKey::Lancet,
            Attack::Overcharge => StyleKey::Overcharge,
            Attack::Fan => StyleKey::Fan,
            Attack::Pellet => StyleKey::Thresher,
            Attack::Slug => StyleKey::Slug,
            Attack::Flak => StyleKey::Flak,
            Attack::Rivet => StyleKey::Riveter,
            Attack::Sawblade => StyleKey::Sawblade,
            Attack::Volley => StyleKey::Volley,
            Attack::Rocket => StyleKey::Cinderbox,
            Attack::Sticky => StyleKey::Sticky,
            Attack::Mortar => StyleKey::Mortar,
            Attack::Fist => StyleKey::Fist,
            Attack::Slam => StyleKey::Slam,
            Attack::Enemy { .. } | Attack::World | Attack::Dev => return None,
        })
    }

    /// The key of a damage instance: a parried projectile scores as `Parry`.
    pub fn for_damage(attack: Attack, flags: DamageFlags, projectile: bool) -> Option<StyleKey> {
        if projectile && flags.contains(DamageFlags::PARRIED) {
            return Some(StyleKey::Parry);
        }
        Self::from_attack(attack)
    }

    /// The keys of a weapon slot (0 Lancet .. 3 Cinderbox): primary, alt A, alt B.
    pub fn weapon_keys(slot: u8) -> [StyleKey; 3] {
        let base = (slot.min(3) as usize) * 3;
        [Self::ALL[base], Self::ALL[base + 1], Self::ALL[base + 2]]
    }

    pub fn label(self) -> &'static str {
        match self {
            StyleKey::Lancet => "LANCET",
            StyleKey::Overcharge => "OVERCHARGE",
            StyleKey::Fan => "FAN",
            StyleKey::Thresher => "THRESHER",
            StyleKey::Slug => "SLUG",
            StyleKey::Flak => "FLAK",
            StyleKey::Riveter => "RIVETER",
            StyleKey::Sawblade => "SAWBLADE",
            StyleKey::Volley => "VOLLEY",
            StyleKey::Cinderbox => "CINDERBOX",
            StyleKey::Sticky => "STICKY",
            StyleKey::Mortar => "MORTAR",
            StyleKey::Fist => "FIST",
            StyleKey::Slam => "SLAM",
            StyleKey::Parry => "PARRY",
        }
    }
}

/// Something the style system tells the owning player (reliable `GameEvent::Style`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum StyleEvent {
    /// A bonus was scored: `points` after freshness / close call. `count`: multikill size,
    /// ricochet bounces (0 otherwise).
    Bonus {
        player: PlayerId,
        bonus: StyleBonus,
        points: f32,
        count: u8,
    },
    /// The rank changed (possibly by several ranks at once).
    Rank { player: PlayerId, rank: u8, up: bool },
}

impl StyleEvent {
    pub fn player(&self) -> PlayerId {
        match *self {
            StyleEvent::Bonus { player, .. } | StyleEvent::Rank { player, .. } => player,
        }
    }
}

/// The owner's view of their style (in `OwnState`, every snapshot; 18 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleView {
    pub rank: u8,
    /// Fill of the current rank's bar, 0..=255.
    pub meter: u8,
    /// Freshness per [`StyleKey`] (index order), 0..=255 = `freshness_min`..1.
    pub freshness: [u8; StyleKey::COUNT],
}

impl Default for StyleView {
    fn default() -> Self {
        Self {
            rank: 0,
            meter: 0,
            freshness: [255; StyleKey::COUNT],
        }
    }
}

impl StyleView {
    /// Fill of the current rank's bar, 0..=1.
    pub fn fraction(&self) -> f32 {
        self.meter as f32 / 255.0
    }

    /// Freshness of `key`, `freshness_min..=1`.
    pub fn freshness(&self, key: StyleKey, t: &StyleTuning) -> f32 {
        let q = self.freshness[key.index()] as f32 / 255.0;
        t.freshness_min + (1.0 - t.freshness_min) * q
    }

    pub fn is_stale(&self, key: StyleKey, t: &StyleTuning) -> bool {
        self.freshness(key, t) < t.stale_threshold
    }
}

/// Base points of a kill's bonuses (before freshness and close call), in feed order.
/// `victim_flying`: the victim hovers (no AIRSHOT for shooting gnats out of the air they live
/// in). `multikill`: kills by this killer within the window, this one included.
pub fn kill_bonuses(
    kill: &KillInfo,
    victim_flying: bool,
    multikill: u32,
    t: &StyleTuning,
) -> Vec<(StyleBonus, f32, u8)> {
    let mut out = Vec::with_capacity(4);
    if kill.victim_kind == kind::PLAYER {
        out.push((StyleBonus::Frag, t.frag, 0));
    } else {
        out.push((StyleBonus::Kill, t.kill, 0));
    }
    if kill.flags.contains(DamageFlags::HEADSHOT) {
        out.push((StyleBonus::Headshot, t.headshot, 0));
    }
    if kill.killer_airborne {
        out.push((StyleBonus::Airkill, t.airkill, 0));
    }
    if kill.victim_airborne && !victim_flying {
        out.push((StyleBonus::Airshot, t.airshot, 0));
    }
    if kill.parried_projectile {
        out.push((StyleBonus::ReturnToSender, t.return_to_sender, 0));
    }
    if kill.attack == Attack::Slam {
        out.push((StyleBonus::SlamKill, t.slam_kill, 0));
    } else if kill.flags.contains(DamageFlags::EXPLOSIVE) {
        out.push((StyleBonus::Exploded, t.exploded, 0));
    }
    if kill.attack == Attack::Fist {
        out.push((StyleBonus::Fisted, t.fisted, 0));
    }
    if multikill >= 2 {
        out.push((
            StyleBonus::Multikill,
            t.multikill,
            multikill.min(u8::MAX as u32) as u8,
        ));
    }
    out
}

/// Is this kill a CLOSE CALL (the killer alive but under `close_call_health`)?
pub fn is_close_call(kill: &KillInfo, t: &StyleTuning) -> bool {
    kill.killer.player().is_some()
        && kill.killer_health > 0.0
        && kill.killer_health < t.close_call_health
}

/// Points of a ricochet hit after `bounces` prism bounces.
pub fn ricochet_points(bounces: u8, t: &StyleTuning) -> f32 {
    t.ricochet + t.ricochet_per_bounce * bounces.saturating_sub(1) as f32
}

/// Meter decay (points/s) at `rank`: linear from `decay_lowest` to `decay_highest`.
pub fn decay_rate(rank: u8, t: &StyleTuning) -> f32 {
    let f = rank.min(TOP_RANK) as f32 / TOP_RANK as f32;
    t.decay_lowest + (t.decay_highest - t.decay_lowest) * f
}

/// One player's style state (server-side).
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerStyle {
    pub rank: u8,
    /// Points in the current rank's bar, `0..=points_per_rank`.
    pub meter: f32,
    /// Freshness per [`StyleKey`], `freshness_min..=1`.
    pub freshness: [f32; StyleKey::COUNT],
    /// Highest rank reached (results).
    pub best_rank: u8,
    /// Every point earned (results "style points").
    pub total: f32,
    /// Seconds left in which the current rank can't drop (after a rank-up).
    pub grace: f32,
    /// Ticks of this player's recent kills (multikill window).
    recent_kills: Vec<u32>,
    /// Freshness is charged once per key per tick (one rocket killing three is one use).
    last_use: Option<(StyleKey, u32)>,
}

impl Default for PlayerStyle {
    fn default() -> Self {
        Self {
            rank: 0,
            meter: 0.0,
            freshness: [1.0; StyleKey::COUNT],
            best_rank: 0,
            total: 0.0,
            grace: 0.0,
            recent_kills: Vec::new(),
            last_use: None,
        }
    }
}

impl PlayerStyle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fill of the current bar, 0..=1.
    pub fn fraction(&self, t: &StyleTuning) -> f32 {
        (self.meter / t.points_per_rank.max(1e-3)).clamp(0.0, 1.0)
    }

    /// Adds meter points; returns the number of ranks gained. At the top rank the bar stays
    /// full.
    pub fn add_points(&mut self, t: &StyleTuning, points: f32) -> u8 {
        if points.is_nan() || points <= 0.0 {
            return 0;
        }
        let per = t.points_per_rank.max(1.0);
        let before = self.rank;
        self.meter += points;
        while self.meter >= per {
            if self.rank >= TOP_RANK {
                self.meter = per;
                break;
            }
            self.rank += 1;
            self.meter -= per;
        }
        self.best_rank = self.best_rank.max(self.rank);
        if self.rank > before {
            self.grace = t.rank_grace;
        }
        self.rank - before
    }

    /// Decays the meter by `dt` seconds; returns true if that dropped a rank.
    pub fn decay(&mut self, t: &StyleTuning, dt: f32) -> bool {
        self.meter -= decay_rate(self.rank, t) * dt;
        if self.grace > 0.0 {
            // A fresh rank holds for a moment even with an empty bar.
            self.grace -= dt;
            self.meter = self.meter.max(0.0);
            return false;
        }
        if self.meter >= 0.0 {
            return false;
        }
        if self.rank == 0 {
            self.meter = 0.0;
            return false;
        }
        self.rank -= 1;
        self.meter = t.points_per_rank * t.rank_down_refill.clamp(0.0, 1.0);
        true
    }

    /// Recovers freshness over time.
    pub fn regen(&mut self, t: &StyleTuning, dt: f32) {
        for f in &mut self.freshness {
            *f = (*f + t.freshness_regen * dt).min(1.0);
        }
    }

    pub fn freshness(&self, key: StyleKey) -> f32 {
        self.freshness[key.index()]
    }

    /// Uses `key` at `tick`: returns its freshness *before* the use (the gain multiplier),
    /// then charges it and restores the others. Several uses of the same key in one tick
    /// are charged once.
    pub fn use_key(&mut self, t: &StyleTuning, key: StyleKey, tick: u32) -> f32 {
        if self.last_use == Some((key, tick)) {
            // Same tick, same key: the multiplier the first use got.
            return (self.freshness(key) + t.freshness_cost).min(1.0);
        }
        let before = self.freshness(key);
        for (i, f) in self.freshness.iter_mut().enumerate() {
            if i == key.index() {
                *f = (*f - t.freshness_cost).max(t.freshness_min);
            } else {
                *f = (*f + t.freshness_restore).min(1.0);
            }
        }
        self.last_use = Some((key, tick));
        before
    }

    /// Records a kill at `tick`; returns the number of kills within the window (this one
    /// included).
    pub fn register_kill(&mut self, tick: u32, window_ticks: u32) -> u32 {
        self.recent_kills
            .retain(|&k| tick.wrapping_sub(k) <= window_ticks);
        self.recent_kills.push(tick);
        self.recent_kills.len() as u32
    }

    /// Empties the meter (death). Returns true if the rank dropped.
    pub fn reset(&mut self) -> bool {
        let dropped = self.rank > 0;
        self.rank = 0;
        self.meter = 0.0;
        self.grace = 0.0;
        self.recent_kills.clear();
        dropped
    }

    /// Scores `items` (base points) used with `key`: applies freshness and the close-call
    /// multiplier, emits the bonus events (+ CLOSE CALL, - STALE) and the rank change.
    #[allow(clippy::too_many_arguments)]
    pub fn award(
        &mut self,
        t: &StyleTuning,
        player: PlayerId,
        key: Option<StyleKey>,
        items: &[(StyleBonus, f32, u8)],
        close_call: bool,
        tick: u32,
        out: &mut Vec<StyleEvent>,
    ) {
        if items.is_empty() {
            return;
        }
        let fresh = key.map_or(1.0, |k| self.use_key(t, k, tick));
        let mut total = 0.0;
        let mut base_total = 0.0;
        for &(bonus, base, count) in items {
            let points = base * fresh;
            total += points;
            base_total += base;
            out.push(StyleEvent::Bonus {
                player,
                bonus,
                points,
                count,
            });
        }
        if close_call {
            let extra = total * (t.close_call_multiplier - 1.0).max(0.0);
            total += extra;
            out.push(StyleEvent::Bonus {
                player,
                bonus: StyleBonus::CloseCall,
                points: extra,
                count: 0,
            });
        }
        if fresh < t.stale_threshold {
            out.push(StyleEvent::Bonus {
                player,
                bonus: StyleBonus::Stale,
                points: base_total - base_total * fresh,
                count: 0,
            });
        }
        self.total += total;
        let gained = self.add_points(t, total);
        if gained > 0 {
            out.push(StyleEvent::Rank {
                player,
                rank: self.rank,
                up: true,
            });
        }
    }

    /// The owner's replicated view.
    pub fn view(&self, t: &StyleTuning) -> StyleView {
        let span = (1.0 - t.freshness_min).max(1e-3);
        let mut freshness = [255u8; StyleKey::COUNT];
        for (q, f) in freshness.iter_mut().zip(self.freshness) {
            *q = (((f - t.freshness_min) / span).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        StyleView {
            rank: self.rank,
            meter: (self.fraction(t) * 255.0).round() as u8,
            freshness,
        }
    }
}

/// Every player's style (`SimWorld::style`).
#[derive(Clone, Debug, Default)]
pub struct StyleSystem {
    pub players: BTreeMap<PlayerId, PlayerStyle>,
}

impl StyleSystem {
    pub fn get(&self, id: PlayerId) -> Option<&PlayerStyle> {
        self.players.get(&id)
    }

    pub fn player(&mut self, id: PlayerId) -> &mut PlayerStyle {
        self.players.entry(id).or_default()
    }

    /// The owner's replicated view (default for unknown players).
    pub fn view(&self, id: PlayerId, t: &StyleTuning) -> StyleView {
        self.players
            .get(&id)
            .map_or_else(StyleView::default, |s| s.view(t))
    }

    pub fn clear(&mut self) {
        self.players.clear();
    }
}

impl SimWorld {
    /// Scores this tick's combat `events` for every player: kills, parries, ricochet hits.
    /// Returns the style events (also pushed as [`SimEvent::Style`] by the caller).
    pub(crate) fn score_style(&mut self, t: &Tuning, events: &[SimEvent]) -> Vec<StyleEvent> {
        let st = &t.style;
        let tick = self.tick;
        let window = (st.multikill_window.max(0.0) * self.tick_rate as f32).round() as u32;
        let mut out = Vec::new();
        // Ricochet hits, one award per shooter per tick (the deepest bounce counts).
        let mut ricochets: BTreeMap<PlayerId, (u8, Attack)> = BTreeMap::new();
        for event in events {
            match event {
                SimEvent::Killed(k) => {
                    let Owner::Player(killer) = k.killer else {
                        continue;
                    };
                    if k.victim == HitTarget::Player(killer) || !self.players.contains_key(&killer)
                    {
                        continue; // suicides score nothing
                    }
                    let flying = k.victim_kind != kind::PLAYER
                        && self.enemy_defs.by_kind(k.victim_kind).is_some_and(|d| {
                            matches!(d.locomotion, super::enemy::Locomotion::Flying { .. })
                        });
                    let key = if k.parried_projectile {
                        Some(StyleKey::Parry)
                    } else {
                        StyleKey::from_attack(k.attack)
                    };
                    let style = self.style.player(killer);
                    let n = style.register_kill(k.tick, window);
                    let items = kill_bonuses(k, flying, n, st);
                    let close = is_close_call(k, st);
                    style.award(st, killer, key, &items, close, tick, &mut out);
                }
                SimEvent::Parried(p) => {
                    if !self.players.contains_key(&p.player) {
                        continue;
                    }
                    let items = [(StyleBonus::Parry, st.parry, 0)];
                    self.style.player(p.player).award(
                        st,
                        p.player,
                        Some(StyleKey::Parry),
                        &items,
                        false,
                        tick,
                        &mut out,
                    );
                }
                SimEvent::Damage(d) => {
                    if let Owner::Player(shooter) = d.info.source
                        && d.info.flags.contains(DamageFlags::RICOCHET)
                        && d.blocked.is_none()
                        && d.dealt > 0.0
                        && d.target != HitTarget::Player(shooter)
                    {
                        let e = ricochets
                            .entry(shooter)
                            .or_insert((0, d.info.attack));
                        e.0 = e.0.max(d.info.ricochets.max(1));
                    }
                }
                _ => {}
            }
        }
        for (shooter, (bounces, attack)) in ricochets {
            if !self.players.contains_key(&shooter) {
                continue;
            }
            let items = [(StyleBonus::Ricochet, ricochet_points(bounces, st), bounces)];
            self.style.player(shooter).award(
                st,
                shooter,
                StyleKey::from_attack(attack),
                &items,
                false,
                tick,
                &mut out,
            );
        }
        out
    }

    /// Per-tick meter decay and freshness recovery. Returns rank-down events.
    pub(crate) fn decay_style(&mut self, t: &Tuning) -> Vec<StyleEvent> {
        let dt = self.dt();
        let mut out = Vec::new();
        for (&id, s) in self.style.players.iter_mut() {
            s.regen(&t.style, dt);
            if s.decay(&t.style, dt) {
                out.push(StyleEvent::Rank {
                    player: id,
                    rank: s.rank,
                    up: false,
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_names_and_keys_line_up() {
        assert_eq!(rank_name(0), "DULL");
        assert_eq!(rank_name(7), "VELOCIDE");
        assert_eq!(rank_name(200), "VELOCIDE");
        for (i, k) in StyleKey::ALL.iter().enumerate() {
            assert_eq!(k.index(), i);
        }
        assert_eq!(
            StyleKey::weapon_keys(1),
            [StyleKey::Thresher, StyleKey::Slug, StyleKey::Flak]
        );
    }

    #[test]
    fn view_round_trips_freshness() {
        let t = StyleTuning::default();
        let mut s = PlayerStyle::new();
        s.freshness[3] = t.freshness_min;
        s.freshness[4] = 0.6;
        let v = s.view(&t);
        assert_eq!(v.freshness[0], 255);
        assert_eq!(v.freshness[3], 0);
        assert!((v.freshness(StyleKey::Slug, &t) - 0.6).abs() < 0.01);
        assert!(v.is_stale(StyleKey::Thresher, &t));
        assert!(!v.is_stale(StyleKey::Slug, &t));
    }
}
