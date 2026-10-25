//! Audio: procedurally synthesised sound effects, layered music stems, 3D positional sound.
//!
//! Nothing is loaded from disk. [`Audio::new`] synthesises every [`Sfx`] (a few hundred ms,
//! spread over several threads) and starts background threads that render the music tracks;
//! each track becomes playable as soon as it is ready (see [`Audio::music_ready`]).
//!
//! - [`dsp`]: the synthesis toolkit (oscillators, envelopes, filters, distortion, bitcrush,
//!   echo/reverb, mixing and analysis helpers).
//! - [`sfx`]: the sound list, mixing metadata and per-sound recipes.
//! - [`music`]: tracks made of stems that layer in with combat intensity.
//! - [`spatial`]: distance attenuation and panning.
//! - [`wav`]: 16-bit WAV export (debugging, `examples/audio_demo.rs`).
//!
//! Typical use from the game:
//! ```no_run
//! use engine::audio::{Audio, MusicTrack, PlayParams, Sfx};
//! use engine::glam::{Quat, Vec3};
//!
//! let mut audio = Audio::new();
//! audio.set_music(MusicTrack::Campaign);
//! // every frame:
//! audio.set_listener(Vec3::ZERO, Quat::IDENTITY);
//! audio.set_music_intensity(0.8);
//! audio.play_2d(Sfx::BellowsBlast);
//! audio.play_at(Sfx::HuskWindup, Vec3::new(4.0, 0.0, -10.0));
//! audio.play(Sfx::Ricochet, PlayParams { pitch: 1.12, ..Default::default() });
//! audio.update(1.0 / 60.0);
//! ```
//! See `docs/audio.md` for the sound design and mixing notes.

mod backend;
pub mod dsp;
pub mod music;
pub mod sfx;
pub mod spatial;
pub mod wav;

use glam::{Quat, Vec3};

pub use music::{MusicTrack, StemKind};
pub use sfx::Sfx;

/// How to play one sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayParams {
    /// Linear volume multiplier (1 = the sound's mixed level).
    pub volume: f32,
    /// Playback-rate multiplier (2 = an octave up, and twice as fast).
    pub pitch: f32,
    /// World position for 3D sounds; `None` plays it centred and unattenuated (UI, own weapon).
    pub position: Option<Vec3>,
}

impl Default for PlayParams {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pitch: 1.0,
            position: None,
        }
    }
}

impl PlayParams {
    /// A 3D sound at `position` with default volume and pitch.
    pub fn at(position: Vec3) -> Self {
        Self {
            position: Some(position),
            ..Self::default()
        }
    }

    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }

    pub fn with_pitch(mut self, pitch: f32) -> Self {
        self.pitch = pitch;
        self
    }
}

/// The game's audio system. Construction never fails: without an audio device every method is a
/// silent no-op and [`Audio::is_enabled`] returns `false`.
pub struct Audio {
    backend: Option<backend::Backend>,
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

impl Audio {
    /// Opens the default output device and synthesises all sounds. Logs a warning and returns a
    /// disabled (silent) instance if there is no usable device.
    pub fn new() -> Audio {
        let start = std::time::Instant::now();
        match backend::Backend::new() {
            Ok(backend) => {
                log::info!("audio: ready in {} ms", start.elapsed().as_millis());
                Audio {
                    backend: Some(backend),
                }
            }
            Err(e) => {
                log::warn!("audio: no output device ({e}); running silently");
                Audio::disabled()
            }
        }
    }

    /// A silent instance that never touches the audio device (headless tools, tests).
    pub fn disabled() -> Audio {
        Audio { backend: None }
    }

    pub fn is_enabled(&self) -> bool {
        self.backend.is_some()
    }

    pub fn play(&mut self, sfx: Sfx, params: PlayParams) {
        if let Some(b) = &mut self.backend {
            b.play(sfx, params);
        }
    }

    /// Plays a non-positional sound (UI, the local player's own weapon and movement).
    pub fn play_2d(&mut self, sfx: Sfx) {
        self.play(sfx, PlayParams::default());
    }

    /// Plays a sound at a world position (enemies, other players, impacts).
    pub fn play_at(&mut self, sfx: Sfx, position: Vec3) {
        self.play(sfx, PlayParams::at(position));
    }

    /// Quickly fades out every playing instance of `sfx` (a released charge, an ended slide).
    pub fn stop(&mut self, sfx: Sfx) {
        if let Some(b) = &mut self.backend {
            b.stop(sfx);
        }
    }

    /// The listener (camera) transform. The camera looks down -Z with +Y up and +X to the right.
    pub fn set_listener(&mut self, position: Vec3, rotation: Quat) {
        if let Some(b) = &mut self.backend {
            b.set_listener(position, rotation);
        }
    }

    /// Linear volumes in `0..=1` for the master, sound-effect and music buses.
    pub fn set_volumes(&mut self, master: f32, sfx: f32, music: f32) {
        if let Some(b) = &mut self.backend {
            b.set_volumes(master, sfx, music);
        }
    }

    /// Switches music with a ~1.5 s crossfade. Setting the current track again does nothing; a
    /// track that is still being generated starts as soon as it is ready.
    pub fn set_music(&mut self, track: MusicTrack) {
        if let Some(b) = &mut self.backend {
            b.set_music(track);
        }
    }

    /// Combat intensity, `0` (calm: pad only) to `1` (full combat: every stem). Smoothed
    /// internally (fast rise, slow fall), so it can be set from raw game state every frame.
    pub fn set_music_intensity(&mut self, intensity: f32) {
        if let Some(b) = &mut self.backend {
            b.set_music_intensity(intensity);
        }
    }

    /// True once `track` has finished generating (always true for `None`, false if disabled).
    pub fn music_ready(&self, track: MusicTrack) -> bool {
        self.backend.as_ref().is_some_and(|b| b.music_ready(track))
    }

    /// Number of sound effects currently playing (debug overlay).
    pub fn active_voices(&self) -> usize {
        self.backend.as_ref().map_or(0, |b| b.active_voices())
    }

    /// Call once per frame: smooths music intensity, starts music that finished generating,
    /// re-pans playing 3D sounds for the current listener and drops finished voices.
    pub fn update(&mut self, dt: f32) {
        if let Some(b) = &mut self.backend {
            b.update(dt);
        }
    }

    /// Current music playhead in frames (diagnostics).
    pub fn music_position(&self) -> Option<u64> {
        self.backend.as_ref().and_then(|b| b.music_position())
    }
}

/// Renders every sound effect variation (`[sfx index][variant]`, mono at
/// [`dsp::SAMPLE_RATE`]) using several threads. This is what [`Audio::new`] does at startup.
pub fn generate_all_sfx() -> Vec<Vec<Vec<f32>>> {
    backend::generate_sfx()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_audio_is_a_silent_no_op() {
        let mut audio = Audio::disabled();
        assert!(!audio.is_enabled());
        audio.set_volumes(0.5, 1.0, 0.2);
        audio.set_listener(Vec3::ONE, Quat::IDENTITY);
        audio.set_music(MusicTrack::Arena);
        audio.set_music_intensity(f32::NAN);
        audio.play_2d(Sfx::Parry);
        audio.play_at(Sfx::Explosion, Vec3::new(f32::NAN, 0.0, 0.0));
        audio.stop(Sfx::Explosion);
        audio.update(1.0 / 60.0);
        assert_eq!(audio.active_voices(), 0);
        assert!(!audio.music_ready(MusicTrack::Arena));
    }

    /// Opens the real device when there is one; must never panic either way.
    #[test]
    fn new_never_panics() {
        let mut audio = Audio::new();
        audio.set_volumes(0.0, 0.0, 0.0); // keep the test run quiet
        audio.set_music(MusicTrack::Menu);
        audio.set_music_intensity(1.0);
        for &sfx in Sfx::ALL {
            audio.play_at(sfx, Vec3::new(3.0, 0.0, -2.0));
        }
        for _ in 0..10 {
            audio.update(1.0 / 60.0);
        }
        if audio.is_enabled() {
            assert!(audio.active_voices() > 0);
        }
        let before = audio.active_voices();
        audio.stop(Sfx::Explosion);
        assert!(audio.active_voices() <= before);
    }
}
