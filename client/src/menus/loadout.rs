//! LOADOUT screen (main menu, lobby, pause menu): choose the weapons on slot keys 1–4 from the
//! ten, and for each its two abilities from its class's pool, with names, blurbs, the class
//! and the base numbers; save, load, rename and delete named presets.
//!
//! The screen edits `Settings::loadout` directly (the caller saves the settings when the
//! player leaves and sends the loadout to the server: `NetClient::set_loadout`). Rules kept
//! while editing: no weapon twice (picking a weapon another slot holds swaps the two slots),
//! at least one weapon (the last one can't be emptied), the two abilities differ (picking the
//! other one swaps them).
//!
//! Numbers come from the tuning: in menus the local `assets/tuning.ron` in its PvE and PvP
//! balance (`Tuning::for_mode`, shown as `PVE / PVP`), in game too (a match runs one of the
//! two).

use engine::ui::{Rect, TextFilter, TextStyle, Ui};
use glam::vec2;
use shared::loadout::{Ability, LOADOUT_SLOTS, Loadout, WeaponKind};
use shared::lobby::GameMode;
use shared::tuning::{CombatTuning, Tuning};

use crate::settings::{LoadoutPreset, LoadoutSettings, MAX_PRESETS, PRESET_NAME_LEN};

/// What the screen did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoadoutResponse {
    /// The loadout or a preset changed: apply / send it.
    pub changed: bool,
    /// The player left the screen (BACK / Escape): save and close.
    pub back: bool,
}

/// Panel rectangle for the loadout screen on a canvas.
pub fn panel_rect(canvas: Rect) -> Rect {
    let size = vec2(560.0, 334.0).min(canvas.size() - vec2(16.0, 16.0));
    Rect::from_center_size(canvas.center(), size)
}

pub struct LoadoutMenu {
    /// The slot being edited (0..4).
    slot: usize,
    /// The preset the preset row works on.
    preset: usize,
    /// Stat columns: a label and the combat tuning its numbers come from.
    columns: Vec<(&'static str, CombatTuning)>,
    /// Shown above the buttons (in a match: when the change applies).
    note: Option<&'static str>,
}

impl Default for LoadoutMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadoutMenu {
    /// For the menus: PvE and PvP numbers from the local tuning file.
    pub fn new() -> Self {
        Self::with_tuning(&Tuning::load_or_default(&Tuning::default_path()))
    }

    /// PvE and PvP numbers from `tuning` (its balance profiles).
    pub fn with_tuning(tuning: &Tuning) -> Self {
        Self {
            slot: 0,
            preset: 0,
            columns: vec![
                ("PVE", tuning.for_mode(GameMode::Campaign).combat),
                ("PVP", tuning.for_mode(GameMode::Deathmatch).combat),
            ],
            note: None,
        }
    }

    /// Shows `note` above the buttons (in a match: when the change applies).
    pub fn with_note(mut self, note: &'static str) -> Self {
        self.note = Some(note);
        self
    }

    /// The slot being edited.
    pub fn slot(&self) -> usize {
        self.slot
    }

    /// Draws the screen in `rect`.
    pub fn ui(&mut self, ui: &mut Ui, s: &mut LoadoutSettings, rect: Rect) -> LoadoutResponse {
        let mut r = LoadoutResponse::default();
        self.slot = self.slot.min(LOADOUT_SLOTS - 1);
        self.preset = self.preset.min(s.presets.len().saturating_sub(1));
        ui.panel(rect, "LOADOUT", |ui| {
            ui.default_focus();
            let area = ui.available_rect();
            let bottom = 34.0;
            let split = area.min.x + (area.width() * 0.4).round();
            let left = Rect::new(area.min, vec2(split - 6.0, area.max.y - bottom));
            let right = Rect::new(vec2(split + 6.0, area.min.y), area.max - vec2(0.0, bottom));
            let foot = Rect::new(vec2(area.min.x, area.max.y - bottom + 4.0), area.max);
            ui.area(left, |ui| {
                self.slots_column(ui, &s.current);
                ui.spacing(4.0);
                r.changed |= self.presets_column(ui, s);
            });
            ui.area(right, |ui| {
                r.changed |= self.slot_details(ui, &mut s.current);
            });
            ui.area(foot, |ui| {
                let dim = TextStyle::new(ui.theme().ash);
                let hint = self
                    .note
                    .unwrap_or("SLOT KEYS 1-4 SELECT; PRESS AGAIN TO SWITCH ABILITY");
                ui.text(hint, &dim);
                ui.row(2, |ui| {
                    if ui.button("DEFAULT LOADOUT") {
                        s.current = Loadout::default();
                        r.changed = true;
                    }
                    if ui.button("BACK##loadout") {
                        r.back = true;
                    }
                });
            });
        });
        if ui.back_pressed() {
            r.back = true;
        }
        r
    }

    /// The four slots: key, weapon, its two abilities. Selecting one edits it on the right.
    fn slots_column(&mut self, ui: &mut Ui, loadout: &Loadout) {
        ui.heading("SLOTS");
        for i in 0..LOADOUT_SLOTS {
            let text = match loadout.slot(i as u8) {
                Some(b) => format!(
                    "{}  {}  {} / {}",
                    i + 1,
                    b.weapon.name().to_uppercase(),
                    short(b.abilities[0]),
                    short(b.abilities[1])
                ),
                None => format!("{}  (EMPTY)", i + 1),
            };
            if ui.selectable(&format!("{text}##slot{i}"), i == self.slot) {
                self.slot = i;
            }
        }
    }

    /// Preset picker, load / save / new / delete and the name field. Returns whether
    /// anything changed.
    fn presets_column(&mut self, ui: &mut Ui, s: &mut LoadoutSettings) -> bool {
        let mut changed = false;
        ui.heading("PRESETS");
        if s.presets.is_empty() {
            s.presets.push(LoadoutPreset {
                name: "PRESET 1".into(),
                loadout: s.current,
            });
        }
        let names: Vec<String> = s.presets.iter().map(|p| p.name.clone()).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        ui.choice("PRESET", &mut self.preset, &names);
        let preset = self.preset;
        ui.row(2, |ui| {
            if ui.button("LOAD") {
                s.current = s.presets[preset].loadout;
                changed = true;
            }
            if ui.button("SAVE") {
                s.presets[preset].loadout = s.current;
                changed = true;
            }
        });
        ui.row(2, |ui| {
            ui.enabled_scope(s.presets.len() < MAX_PRESETS, |ui| {
                if ui.button("NEW") {
                    s.presets.push(LoadoutPreset {
                        name: format!("PRESET {}", s.presets.len() + 1),
                        loadout: s.current,
                    });
                    self.preset = s.presets.len() - 1;
                    changed = true;
                }
            });
            ui.enabled_scope(s.presets.len() > 1, |ui| {
                if ui.button("DELETE") {
                    s.presets.remove(preset);
                    self.preset = preset.min(s.presets.len() - 1);
                    changed = true;
                }
            });
        });
        let last = s.presets.len() - 1;
        let p = &mut s.presets[self.preset.min(last)];
        let edit = ui.text_field("NAME", &mut p.name, PRESET_NAME_LEN, TextFilter::Name);
        if edit.changed {
            p.name = p.name.to_uppercase();
            changed = true;
        }
        changed
    }

    /// The selected slot: weapon, class and blurb, numbers, both abilities with their blurbs.
    fn slot_details(&mut self, ui: &mut Ui, loadout: &mut Loadout) -> bool {
        let mut changed = false;
        let slot = self.slot;
        ui.heading(&format!("SLOT {}", slot + 1));
        // Weapon (index 0 = empty).
        let mut options = vec!["(EMPTY)".to_string()];
        options.extend(WeaponKind::ALL.iter().map(|w| w.name().to_uppercase()));
        let options: Vec<&str> = options.iter().map(String::as_str).collect();
        let current = loadout.slots[slot];
        let mut index = current.map_or(0, |b| b.weapon.id() as usize + 1);
        if ui.choice("WEAPON", &mut index, &options) {
            let weapon = index
                .checked_sub(1)
                .and_then(|i| WeaponKind::from_id(i as u8));
            changed |= set_weapon(loadout, slot, weapon);
        }
        let dim = TextStyle::new(ui.theme().bone_dim);
        let Some(build) = loadout.slots[slot] else {
            ui.text("NOTHING ON THIS KEY: SWITCHING SKIPS IT.", &dim);
            return changed;
        };
        let w = build.weapon;
        ui.text(
            &format!("{} - {}", w.class().name(), w.blurb().to_uppercase()),
            &dim,
        );
        // Numbers, one column per tuning.
        let header: Vec<&str> = self.columns.iter().map(|(name, _)| *name).collect();
        ui.value_row("", &header.join(" / "));
        let damage: Vec<String> = self
            .columns
            .iter()
            .map(|(_, t)| damage_text(w, t))
            .collect();
        ui.value_row("DAMAGE", &damage.join(" / "));
        let rate: Vec<String> = self.columns.iter().map(|(_, t)| rate_text(w, t)).collect();
        ui.value_row("RATE", &rate.join(" / "));
        // Abilities from the class pool.
        let pool = w.abilities();
        let names: Vec<String> = pool.iter().map(|a| a.name().to_uppercase()).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        for k in 0..2 {
            let label = if k == 0 { "ABILITY A" } else { "ABILITY B" };
            let mut i = pool
                .iter()
                .position(|a| *a == build.abilities[k])
                .unwrap_or(0);
            if ui.choice(label, &mut i, &names) {
                changed |= set_ability(loadout, slot, k, pool[i]);
            }
            let a = loadout.slots[slot].map_or(pool[i], |b| b.abilities[k]);
            let cooldown: Vec<String> = self
                .columns
                .iter()
                .map(|(_, t)| format!("{:.1}S", a.cooldown(t)))
                .collect();
            ui.text(
                &format!("{} ({})", a.blurb().to_uppercase(), cooldown.join(" / ")),
                &dim,
            );
        }
        changed
    }
}

/// Puts `weapon` (or nothing) on `slot`. A weapon another slot holds swaps the two slots; the
/// last weapon can't be removed. Returns whether the loadout changed.
pub fn set_weapon(loadout: &mut Loadout, slot: usize, weapon: Option<WeaponKind>) -> bool {
    let Some(current) = loadout.slots.get(slot).copied() else {
        return false;
    };
    match weapon {
        None => {
            if current.is_none() || loadout.builds().count() <= 1 {
                return false;
            }
            loadout.slots[slot] = None;
        }
        Some(w) if current.is_some_and(|b| b.weapon == w) => return false,
        Some(w) => match loadout.slot_of(w) {
            Some(other) => loadout.slots.swap(slot, other as usize),
            None => loadout.slots[slot] = Some(w.default_build()),
        },
    }
    true
}

/// Sets ability `k` (0 = A, 1 = B) of `slot`'s build; picking the other ability swaps them.
/// Returns whether the loadout changed.
pub fn set_ability(loadout: &mut Loadout, slot: usize, k: usize, ability: Ability) -> bool {
    let Some(Some(build)) = loadout.slots.get_mut(slot) else {
        return false;
    };
    if k > 1 || !ability.fits(build.weapon) || build.abilities[k] == ability {
        return false;
    }
    if build.abilities[1 - k] == ability {
        build.abilities.swap(0, 1);
    } else {
        build.abilities[k] = ability;
    }
    true
}

/// A short ability name for the slot list ("BOTH BARRELS" → "BOTH BARRELS", "PINNING ARROW"
/// → "PINNING").
fn short(a: Ability) -> String {
    let name = a.name().to_uppercase();
    match a {
        Ability::PinningArrow | Ability::EmberArrow | Ability::TripleNock => {
            name.split(' ').next().unwrap_or_default().to_string()
        }
        _ => name,
    }
}

/// Damage of a primary in words: per shot, pellets × damage, the bow's draw range, the Kiln's
/// tick plus Burn.
pub fn damage_text(w: WeaponKind, t: &CombatTuning) -> String {
    let d = w.primary_damage(t);
    match w {
        WeaponKind::Bellows => format!("{}X{:.0}", t.bellows.pellets, d),
        WeaponKind::Thresher => format!("{}X{:.0}", t.thresher.pellets, d),
        WeaponKind::Cinderbox => format!("{}X{:.0}", t.cinderbox.rockets, d),
        WeaponKind::Longwick => format!("{:.0}-{:.0}", d, t.longwick.max_damage),
        WeaponKind::Kiln => format!("{:.0}+BURN", d),
        _ => format!("{d:.0}"),
    }
}

/// How often it fires: shots per second when fast, else the interval (the bow: its draw).
pub fn rate_text(w: WeaponKind, t: &CombatTuning) -> String {
    if w == WeaponKind::Longwick {
        return format!("DRAW {:.2}S", t.longwick.draw_time);
    }
    let interval = w.primary_interval(t).max(1e-3);
    if interval < 0.2 {
        format!("{:.0}/S", 1.0 / interval)
    } else {
        format!("{interval:.2}S")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_weapons_keeps_the_loadout_valid() {
        let mut l = Loadout::default();
        // A new weapon replaces the slot's with its default build.
        assert!(set_weapon(&mut l, 1, Some(WeaponKind::Longwick)));
        assert_eq!(l.slot(1), Some(&WeaponKind::Longwick.default_build()));
        // A weapon another slot holds swaps the two.
        let quill = *l.slot(0).unwrap();
        assert!(set_weapon(&mut l, 3, Some(WeaponKind::Quill)));
        assert_eq!(l.slot(3), Some(&quill));
        assert_eq!(l.slot(0).map(|b| b.weapon), Some(WeaponKind::Tumbler));
        assert!(l.is_valid());
        // Emptying works until one weapon is left.
        assert!(set_weapon(&mut l, 0, None));
        assert!(set_weapon(&mut l, 1, None));
        assert!(set_weapon(&mut l, 2, None));
        assert!(!set_weapon(&mut l, 3, None), "the last weapon stays");
        assert!(l.is_valid());
        assert!(
            !set_weapon(&mut l, 9, Some(WeaponKind::Kiln)),
            "no such slot"
        );
        // Same weapon again: nothing to do.
        assert!(!set_weapon(&mut l, 3, Some(WeaponKind::Quill)));
    }

    #[test]
    fn picking_abilities_swaps_and_refuses_other_classes() {
        let mut l = Loadout::single(WeaponKind::Longwick.default_build());
        let [a, b] = l.slot(0).unwrap().abilities;
        assert!(set_ability(&mut l, 0, 0, b));
        assert_eq!(l.slot(0).unwrap().abilities, [b, a], "swapped");
        assert!(set_ability(&mut l, 0, 1, Ability::EmberArrow));
        assert!(
            !set_ability(&mut l, 0, 1, Ability::Gust),
            "not a Draw ability"
        );
        assert!(!set_ability(&mut l, 1, 0, Ability::Brand), "empty slot");
        assert!(l.is_valid());
    }

    #[test]
    fn numbers_read_for_every_weapon() {
        let t = CombatTuning::default();
        for w in WeaponKind::ALL {
            let d = damage_text(w, &t);
            let r = rate_text(w, &t);
            assert!(!d.is_empty() && d.len() < 12, "{w:?}: {d}");
            assert!(!r.is_empty() && r.len() < 12, "{w:?}: {r}");
        }
        assert_eq!(damage_text(WeaponKind::Bellows, &t), "12X8");
        assert!(damage_text(WeaponKind::Longwick, &t).contains('-'));
    }
}
