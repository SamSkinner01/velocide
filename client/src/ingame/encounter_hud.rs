//! Arena encounter presentation (see `docs/enemies.md`): a banner + sound for each encounter
//! `GameEvent` ("ARENA LOCKED", "WAVE 2/3", "CLEARED") and a small "WAVE 2/3 - 5 LEFT" readout
//! under the level timer, read from the replicated `kind::ENCOUNTER` entity (so late joiners
//! see it too).

use engine::audio::{Audio, Sfx};
use engine::gfx::UiDrawList;
use engine::ui::draw::fade;
use engine::ui::{TextStyle, Theme, UiDrawExt};
use glam::vec2;
use shared::protocol::{EntityState, GameEvent, encounter_flags, kind};

use super::hud::Hud;

/// Shows / plays an encounter event. Returns false (and does nothing) for other events.
pub fn on_event(event: &GameEvent, hud: &mut Hud, audio: &mut Audio, t: &Theme, now: f64) -> bool {
    match event {
        GameEvent::EncounterStarted { .. } => {
            hud.banner("ARENA LOCKED", t.crimson_bright, now);
            audio.play_2d(Sfx::DoorLock);
        }
        GameEvent::WaveStarted { wave, waves, .. } => {
            // Wave 1 follows "ARENA LOCKED" right away; announce the later ones.
            if *wave > 1 {
                hud.banner(format!("WAVE {wave}/{waves}"), t.bone, now);
            }
            audio.play_2d(Sfx::WaveStart);
        }
        GameEvent::EncounterCleared { .. } => {
            hud.banner("CLEARED", t.stamina, now);
            audio.play_2d(Sfx::EncounterClear);
            audio.play_2d(Sfx::DoorOpen);
        }
        _ => return false,
    }
    true
}

/// The readout for the active encounter, if any ("WAVE 2/3 - 5 LEFT").
pub fn readout(entities: &[EntityState]) -> Option<String> {
    let e = entities
        .iter()
        .find(|e| e.kind == kind::ENCOUNTER && e.flags & encounter_flags::ACTIVE != 0)?;
    let waves = e.aux & 0xFFFF;
    Some(if e.flags & encounter_flags::BETWEEN_WAVES != 0 {
        format!("WAVE {}/{} INCOMING", e.state, waves)
    } else {
        format!("WAVE {}/{} - {} LEFT", e.state, waves, e.health)
    })
}

/// Draws the readout under the timer (top centre).
pub fn draw(list: &mut UiDrawList, t: &Theme, entities: &[EntityState]) {
    let Some(text) = readout(entities) else {
        return;
    };
    let style = TextStyle::new(t.crimson_bright)
        .centered()
        .outline(fade(t.ink, 0.9));
    list.text(vec2(list.canvas_size.x * 0.5, 40.0), &text, &style);
}
