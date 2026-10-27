//! Balance per game mode (`docs/arsenal.md`, "Balance profiles").
//!
//! The weapons' base numbers live in `combat` and are the same everywhere. A
//! [`BalanceProfile`] adjusts them for one kind of match: per-weapon (primary fire) and
//! per-ability [`BalanceScale`] multipliers, then absolute `overrides` for single `combat`
//! fields where a multiplier can't express it (`"quill.brand_multiplier": 1.25`). (No Thorns
//! in PvP is not a number any more: the Thorn isn't `Knack::pvp_allowed`.)
//! `pve` applies to campaign and grind (and warm-ups before them), `pvp` to deathmatch and gun
//! game. [`Tuning::for_mode`] returns the effective tuning a match runs with; the server sends
//! that one to clients, so prediction always uses exactly what the server simulates.
//!
//! Which `combat` fields each multiplier scales is the table in
//! [`CombatTuning::scale_weapon`] / [`CombatTuning::scale_ability`]; the same table scales an
//! ability's damage when it sits on a weapon other than its home weapon (`ability_power`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{CombatTuning, MAX_MULT, Tuning, clamped};
use crate::loadout::{Ability, WeaponKind};
use crate::lobby::GameMode;

/// Override values are clamped to this magnitude (the fields they set are clamped again by
/// `CombatTuning::sanitize`).
const MAX_OVERRIDE: f32 = 1.0e6;

/// Multipliers for one weapon's primary fire or one ability (1.0 = unchanged).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BalanceScale {
    /// Every damage number of it (direct, pellets, splash, zaps).
    pub damage: f32,
    /// Fire interval (primary; the Longwick's draw time too) or cooldown (ability).
    pub interval: f32,
    /// Splash / area / cone radius.
    pub radius: f32,
    /// Knockback dealt to what it hits.
    pub knockback: f32,
    /// Projectile speed.
    pub speed: f32,
}

impl Default for BalanceScale {
    fn default() -> Self {
        Self::ONE
    }
}

impl BalanceScale {
    pub const ONE: BalanceScale = BalanceScale {
        damage: 1.0,
        interval: 1.0,
        radius: 1.0,
        knockback: 1.0,
        speed: 1.0,
    };

    /// Only the damage scaled.
    pub fn damage(damage: f32) -> Self {
        Self {
            damage,
            ..Self::ONE
        }
    }

    fn sanitize(&mut self, d: &BalanceScale) {
        let s = self;
        clamp!(s, d, 0.0, MAX_MULT; damage, interval, radius, knockback, speed);
    }
}

/// The adjustments of one kind of match. Missing entries are 1.0 everywhere.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BalanceProfile {
    pub weapons: BTreeMap<WeaponKind, BalanceScale>,
    pub abilities: BTreeMap<Ability, BalanceScale>,
    /// Absolute values for single numbers in `combat`, by field path relative to `combat`
    /// (`"thorn.charges"`, `"self_damage_fraction"`), applied after the multipliers. Integer
    /// fields take the rounded value.
    pub overrides: BTreeMap<String, f32>,
}

impl BalanceProfile {
    /// Applies the multipliers, then the overrides, to `c` (not sanitised: the caller does).
    /// An override naming no number in `combat` is skipped with a warning.
    pub fn apply(&self, c: &mut CombatTuning) {
        for (weapon, scale) in &self.weapons {
            c.scale_weapon(*weapon, scale);
        }
        for (ability, scale) in &self.abilities {
            c.scale_ability(*ability, scale);
        }
        for (path, value) in &self.overrides {
            if let Err(err) = c.set_path(path, *value) {
                log::warn!("balance override `{path}`: {err}");
            }
        }
    }

    /// Clamps multipliers to `0..=100` and overrides to ±1e6; NaN falls back to `d`'s value
    /// for that entry (1.0 / dropped when `d` has none).
    fn sanitize(&mut self, d: &BalanceProfile) {
        for (w, s) in &mut self.weapons {
            s.sanitize(d.weapons.get(w).unwrap_or(&BalanceScale::ONE));
        }
        for (a, s) in &mut self.abilities {
            s.sanitize(d.abilities.get(a).unwrap_or(&BalanceScale::ONE));
        }
        self.overrides
            .retain(|k, v| !v.is_nan() || d.overrides.contains_key(k));
        for (k, v) in &mut self.overrides {
            let default = d.overrides.get(k).copied().unwrap_or(0.0);
            *v = clamped(*v, -MAX_OVERRIDE, MAX_OVERRIDE, default);
        }
    }
}

/// The `balance` section: one profile for PvE matches, one for PvP.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BalanceTuning {
    /// Campaign, grind (co-op included) and warm-ups before them.
    pub pve: BalanceProfile,
    /// Deathmatch, gun game and warm-ups before them.
    pub pvp: BalanceProfile,
}

impl Default for BalanceTuning {
    /// PvE is the reference (nothing changed). PvP: tuned for time-to-kill between players
    /// (no Thorns either: `Knack::pvp_allowed`). The numbers and their reasoning:
    /// `docs/arsenal.md`.
    fn default() -> Self {
        let scale = |damage: f32| BalanceScale::damage(damage);
        let weapons = BTreeMap::from([
            (WeaponKind::Tumbler, scale(0.9)),
            (WeaponKind::Lancet, scale(0.85)),
            (WeaponKind::Bellows, scale(0.6)),
            (WeaponKind::Thresher, scale(0.75)),
            (WeaponKind::Cinderbox, scale(0.75)),
            (
                WeaponKind::Longwick,
                BalanceScale {
                    interval: 0.85,
                    ..scale(0.5)
                },
            ),
            (WeaponKind::Kiln, scale(0.9)),
            (WeaponKind::Trimmer, scale(0.84)),
            // Its abilities hit for a multiple of the swing: 59.5 keeps them under a one-shot.
            (WeaponKind::Tamper, scale(0.7)),
        ]);
        let abilities = BTreeMap::from([
            (Ability::BothBarrels, scale(0.25)),
            (Ability::Wisp, scale(0.5)),
            (Ability::Discharge, scale(0.9)),
            (Ability::Breach, scale(0.85)),
            (Ability::Gravewell, scale(0.75)),
            (Ability::Skewer, scale(0.58)),
            (Ability::Flurry, scale(0.6)),
            (Ability::Flak, scale(0.8)),
            (Ability::Volley, scale(0.65)),
            (Ability::Limpet, scale(0.75)),
            (
                Ability::Mortar,
                BalanceScale {
                    radius: 0.85,
                    ..scale(0.75)
                },
            ),
            (Ability::TripleNock, scale(0.55)),
            (Ability::EmberArrow, scale(0.5)),
        ]);
        let overrides = BTreeMap::from([
            ("quill.brand_multiplier".to_string(), 1.25),
            // Half damage for 3 s won every even duel.
            ("knacks.tallow_skin.damage_scale".to_string(), 0.7),
            ("knacks.tallow_skin.duration".to_string(), 2.0),
        ]);
        Self {
            pve: BalanceProfile::default(),
            pvp: BalanceProfile {
                weapons,
                abilities,
                overrides,
            },
        }
    }
}

impl BalanceTuning {
    /// No adjustments at all (what an effective tuning carries: it is already applied).
    pub fn neutral() -> Self {
        Self {
            pve: BalanceProfile::default(),
            pvp: BalanceProfile::default(),
        }
    }

    pub fn profile(&self, mode: GameMode) -> &BalanceProfile {
        if mode.is_pvp() { &self.pvp } else { &self.pve }
    }

    pub fn sanitize(&mut self) {
        let d = Self::default();
        self.pve.sanitize(&d.pve);
        self.pvp.sanitize(&d.pvp);
    }
}

impl Tuning {
    /// The effective tuning of a `mode` match: `combat` adjusted by the mode's balance profile
    /// (and sanitised). Its own `balance` is neutral, so applying it twice changes nothing.
    /// The server runs matches with it and sends it to clients.
    pub fn for_mode(&self, mode: GameMode) -> Tuning {
        let mut out = self.clone();
        self.balance.profile(mode).apply(&mut out.combat);
        out.combat.sanitize();
        out.balance = BalanceTuning::neutral();
        out
    }
}

impl CombatTuning {
    /// Scales the numbers of `weapon`'s primary fire (the table behind
    /// [`BalanceProfile::weapons`]).
    pub fn scale_weapon(&mut self, weapon: WeaponKind, s: &BalanceScale) {
        match weapon {
            WeaponKind::Quill => {
                let q = &mut self.quill;
                q.damage *= s.damage;
                q.interval *= s.interval;
            }
            WeaponKind::Bellows => {
                let b = &mut self.bellows;
                b.pellet_damage *= s.damage;
                b.interval *= s.interval;
                b.pellet_knockback *= s.knockback;
            }
            WeaponKind::Arclash => {
                let a = &mut self.arclash;
                a.zap_damage *= s.damage;
                a.zap_interval *= s.interval;
            }
            WeaponKind::Tumbler => {
                let t = &mut self.tumbler;
                t.bomb_damage *= s.damage;
                t.interval *= s.interval;
                t.bomb_radius *= s.radius;
                t.bomb_knockback *= s.knockback;
                t.bomb_speed *= s.speed;
            }
            WeaponKind::Lancet => {
                let l = &mut self.lancet;
                l.damage *= s.damage;
                l.interval *= s.interval;
            }
            WeaponKind::Thresher => {
                let t = &mut self.thresher;
                t.pellet_damage *= s.damage;
                t.interval *= s.interval;
                t.pellet_knockback *= s.knockback;
            }
            WeaponKind::Riveter => {
                let r = &mut self.riveter;
                r.damage *= s.damage;
                r.interval *= s.interval;
                r.rivet_speed *= s.speed;
            }
            WeaponKind::Cinderbox => {
                let c = &mut self.cinderbox;
                c.damage *= s.damage;
                c.interval *= s.interval;
                c.explosion_radius *= s.radius;
                c.knockback *= s.knockback;
                c.rocket_speed *= s.speed;
            }
            WeaponKind::Longwick => {
                let l = &mut self.longwick;
                l.min_damage *= s.damage;
                l.max_damage *= s.damage;
                l.draw_time *= s.interval;
                l.interval *= s.interval;
                l.min_speed *= s.speed;
                l.max_speed *= s.speed;
            }
            WeaponKind::Kiln => {
                let k = &mut self.kiln;
                k.damage *= s.damage;
                k.interval *= s.interval;
                k.range *= s.radius;
            }
            WeaponKind::Trimmer => {
                let m = &mut self.trimmer;
                m.damage *= s.damage;
                m.interval *= s.interval;
                m.reach *= s.radius;
                m.knockback *= s.knockback;
            }
            WeaponKind::Tamper => {
                let m = &mut self.tamper;
                m.damage *= s.damage;
                m.interval *= s.interval;
                m.reach *= s.radius;
                m.knockback *= s.knockback;
            }
        }
    }

    /// Scales the numbers of `ability` (the table behind [`BalanceProfile::abilities`] and
    /// `ability_power`).
    pub fn scale_ability(&mut self, ability: Ability, s: &BalanceScale) {
        match ability {
            Ability::Brand => {
                let q = &mut self.quill;
                q.brand_damage *= s.damage;
                q.brand_cooldown *= s.interval;
            }
            Ability::Splinter => {
                let q = &mut self.quill;
                q.splinter_damage *= s.damage;
                q.splinter_shard_damage *= s.damage;
                q.splinter_cooldown *= s.interval;
                q.splinter_range *= s.radius;
            }
            Ability::Gust => {
                let b = &mut self.bellows;
                b.gust_damage *= s.damage;
                b.gust_cooldown *= s.interval;
                b.gust_range *= s.radius;
                b.gust_knockback *= s.knockback;
                b.gust_up *= s.knockback;
            }
            Ability::BothBarrels => {
                let b = &mut self.bellows;
                b.both_pellet_damage *= s.damage;
                b.both_cooldown *= s.interval;
            }
            Ability::Wisp => {
                let a = &mut self.arclash;
                a.wisp_zap_damage *= s.damage;
                a.wisp_pop_damage *= s.damage;
                a.wisp_cooldown *= s.interval;
                a.wisp_zap_range *= s.radius;
                a.wisp_pop_radius *= s.radius;
                a.wisp_pop_knockback *= s.knockback;
                a.wisp_speed *= s.speed;
            }
            Ability::Discharge => {
                let a = &mut self.arclash;
                a.discharge_min_damage *= s.damage;
                a.discharge_max_damage *= s.damage;
                a.discharge_cooldown *= s.interval;
                a.discharge_min_radius *= s.radius;
                a.discharge_max_radius *= s.radius;
                a.discharge_min_knockback *= s.knockback;
                a.discharge_max_knockback *= s.knockback;
            }
            Ability::Breach => {
                let t = &mut self.tumbler;
                t.breach_damage *= s.damage;
                t.breach_cooldown *= s.interval;
                t.breach_radius *= s.radius;
                t.breach_knockback *= s.knockback;
            }
            Ability::Gravewell => {
                let t = &mut self.tumbler;
                t.gravewell_damage *= s.damage;
                t.gravewell_cooldown *= s.interval;
                t.gravewell_radius *= s.radius;
                t.gravewell_pull_radius *= s.radius;
                t.gravewell_knockback *= s.knockback;
                t.gravewell_speed *= s.speed;
            }
            Ability::Skewer => {
                let k = &mut self.skewer;
                k.min_damage *= s.damage;
                k.max_damage *= s.damage;
                k.cooldown *= s.interval;
            }
            Ability::Flurry => {
                let f = &mut self.flurry;
                f.damage_scale *= s.damage;
                f.cooldown *= s.interval;
            }
            Ability::Slug => {
                let g = &mut self.slug;
                g.damage *= s.damage;
                g.cooldown *= s.interval;
                g.knockback *= s.knockback;
            }
            Ability::Flak => {
                let f = &mut self.flak;
                f.pellet_damage *= s.damage;
                f.cooldown *= s.interval;
                f.pellet_range *= s.radius;
                f.shell_speed *= s.speed;
            }
            Ability::Volley => {
                let v = &mut self.volley;
                v.damage_scale *= s.damage;
                v.cooldown *= s.interval;
            }
            Ability::Flare => {
                let f = &mut self.flare;
                f.burn_rate *= s.damage;
                f.cooldown *= s.interval;
                f.patch_radius *= s.radius;
                f.speed *= s.speed;
            }
            Ability::Limpet => {
                let l = &mut self.limpet;
                l.damage *= s.damage;
                l.cooldown *= s.interval;
                l.explosion_radius *= s.radius;
                l.knockback *= s.knockback;
                l.speed *= s.speed;
            }
            Ability::Mortar => {
                let m = &mut self.mortar;
                m.damage *= s.damage;
                m.cooldown *= s.interval;
                m.explosion_radius *= s.radius;
                m.knockback *= s.knockback;
                m.speed *= s.speed;
            }
            Ability::TripleNock => {
                let t = &mut self.triple_nock;
                t.damage_scale *= s.damage;
                t.cooldown *= s.interval;
            }
            Ability::PinningArrow => {
                let p = &mut self.pinning_arrow;
                p.damage_scale *= s.damage;
                p.cooldown *= s.interval;
            }
            Ability::EmberArrow => {
                let e = &mut self.ember_arrow;
                e.damage_scale *= s.damage;
                e.damage *= s.damage;
                e.cooldown *= s.interval;
                e.explosion_radius *= s.radius;
                e.knockback *= s.knockback;
            }
            Ability::Lunge => {
                let l = &mut self.lunge;
                l.damage_scale *= s.damage;
                l.cooldown *= s.interval;
                l.reach *= s.radius;
                l.knockback *= s.knockback;
                l.speed *= s.speed;
            }
            Ability::Whirl => {
                let w = &mut self.whirl;
                w.damage_scale *= s.damage;
                w.cooldown *= s.interval;
                w.radius *= s.radius;
                w.knockback *= s.knockback;
            }
            Ability::Hurl => {
                let h = &mut self.hurl;
                h.damage_scale *= s.damage;
                h.cooldown *= s.interval;
                h.knockback *= s.knockback;
                h.speed *= s.speed;
            }
            Ability::Upswing => {
                let u = &mut self.upswing;
                u.damage_scale *= s.damage;
                u.cooldown *= s.interval;
                u.launch *= s.knockback;
            }
        }
    }

    /// Sets the number at `path` (field names relative to `combat`, joined by dots:
    /// `"thorn.charges"`) to `value`; integer fields take it rounded (negative → error).
    pub fn set_path(&mut self, path: &str, value: f32) -> Result<(), String> {
        if !value.is_finite() {
            return Err(format!("{value} is not a number"));
        }
        let text = ron::to_string(self).map_err(|e| e.to_string())?;
        let (start, end) = find_path(&text, path).ok_or("no such number in `combat`")?;
        let old = &text[start..end];
        if old.parse::<f64>().is_err() {
            return Err(format!("`{old}` is not a number"));
        }
        let new = if old.contains(['.', 'e', 'E']) {
            format!("{value:?}")
        } else if value >= 0.0 {
            format!("{}", value.round() as u64)
        } else {
            return Err(format!("{value} for a whole-number field"));
        };
        let patched = format!("{}{new}{}", &text[..start], &text[end..]);
        *self = ron::from_str(&patched).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Byte range of the value at `path` in compact RON (`(a:1,b:(c:2.0))`), if it is a scalar.
fn find_path(text: &str, path: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut open = text.find('(')?;
    let segments: Vec<&str> = path.split('.').collect();
    for (i, segment) in segments.iter().enumerate() {
        let mut depth = 0i32;
        let mut value = None;
        for (j, &b) in bytes.iter().enumerate().skip(open) {
            match b {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            let field_starts = depth == 1 && (j == open || b == b',');
            let rest = &text[j + 1..];
            if field_starts
                && let Some(after) = rest.strip_prefix(segment)
                && after.starts_with(':')
            {
                value = Some(j + 1 + segment.len() + 1);
                break;
            }
        }
        let start = value?;
        if i + 1 < segments.len() {
            if bytes.get(start) != Some(&b'(') {
                return None;
            }
            open = start;
        } else {
            let len = text[start..].find([',', ')'])?;
            return Some((start, start + len));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_set_single_numbers() {
        let mut c = CombatTuning::default();
        c.set_path("thorn.charges", 0.0).unwrap();
        assert_eq!(c.thorn.charges, 0);
        c.set_path("quill.damage", 30.5).unwrap();
        assert_eq!(c.quill.damage, 30.5);
        c.set_path("max_health", 150.0).unwrap();
        assert_eq!(c.max_health, 150.0);
        // `damage` is a prefix of other fields; only the exact one changes.
        assert_eq!(
            c.quill.brand_damage,
            CombatTuning::default().quill.brand_damage
        );
        let before = c.clone();
        for bad in ["thorn", "thorn.nothing", "nothing", "quill.damage.x", ""] {
            assert!(c.set_path(bad, 1.0).is_err(), "{bad}");
        }
        assert!(c.set_path("thorn.charges", -1.0).is_err());
        assert!(c.set_path("quill.damage", f32::NAN).is_err());
        assert!(c.set_path("parry_compensate_view_lag", 1.0).is_err());
        assert_eq!(c, before);
    }

    #[test]
    fn pve_is_the_reference_and_pvp_is_neutral_once_applied() {
        let t = Tuning::default();
        let pve = t.for_mode(GameMode::Campaign);
        assert_eq!(pve.combat, t.combat);
        assert_eq!(t.for_mode(GameMode::Grind).combat, t.combat);
        let pvp = t.for_mode(GameMode::Deathmatch);
        // No Thorns in PvP is the knack rule (`Knack::pvp_allowed`), not a tuning number.
        assert_eq!(pvp.combat.thorn.charges, t.combat.thorn.charges);
        assert_eq!(pvp.combat.quill.brand_multiplier, 1.25);
        // Applied once: an effective tuning is neutral.
        assert_eq!(pvp.for_mode(GameMode::Deathmatch), pvp);
        assert_eq!(pvp.balance, BalanceTuning::neutral());
    }

    #[test]
    fn multipliers_follow_the_table() {
        let mut t = Tuning::default();
        t.balance.pvp.weapons.insert(
            WeaponKind::Tumbler,
            BalanceScale {
                damage: 0.5,
                radius: 2.0,
                ..BalanceScale::ONE
            },
        );
        t.balance
            .pvp
            .abilities
            .insert(Ability::Brand, BalanceScale::damage(2.0));
        t.balance
            .pvp
            .overrides
            .insert("tumbler.bomb_damage".into(), 40.0);
        let d = CombatTuning::default();
        let pvp = t.for_mode(GameMode::Deathmatch).combat;
        // The override wins over the multiplier.
        assert_eq!(pvp.tumbler.bomb_damage, 40.0);
        assert_eq!(pvp.tumbler.bomb_radius, d.tumbler.bomb_radius * 2.0);
        assert_eq!(pvp.quill.brand_damage, d.quill.brand_damage * 2.0);
        assert_eq!(pvp.quill.damage, d.quill.damage);
        assert_eq!(t.for_mode(GameMode::Campaign).combat, d);
        // Every weapon and ability is in the table (a zero scale changes something).
        for w in WeaponKind::ALL {
            let mut c = CombatTuning::default();
            c.scale_weapon(w, &BalanceScale::damage(0.0));
            assert_ne!(c, d, "{w:?}");
        }
        for a in Ability::ALL {
            let mut c = CombatTuning::default();
            c.scale_ability(a, &BalanceScale::damage(0.0));
            assert_ne!(c, d, "{a:?}");
        }
    }

    #[test]
    fn profiles_read_from_ron() {
        let t: Tuning = ron::from_str(
            r#"(balance: (pvp: (
                weapons: { Longwick: (damage: 0.9) },
                abilities: { Mortar: (radius: 0.8, interval: 1.2) },
                overrides: { "thorn.charges": 1 },
            )))"#,
        )
        .unwrap();
        assert_eq!(t.balance.pvp.weapons[&WeaponKind::Longwick].damage, 0.9);
        assert_eq!(t.balance.pvp.weapons[&WeaponKind::Longwick].speed, 1.0);
        assert_eq!(t.balance.pvp.abilities[&Ability::Mortar].interval, 1.2);
        assert_eq!(t.for_mode(GameMode::Deathmatch).combat.thorn.charges, 1);
        assert_eq!(t.balance.pve, BalanceProfile::default());
    }
}
