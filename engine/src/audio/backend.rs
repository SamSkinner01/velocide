//! Everything that talks to kira: the mixer graph, sound-effect voices, the music player (a
//! custom kira [`Sound`] that plays all stems of a track from one playhead) and a master
//! limiter.
//!
//! Mixer graph:
//! ```text
//! sfx track  (volume = sfx)   ──┐
//!                               ├─► main track (volume = master) ─► peak limiter ─► device
//! music track (volume = music) ─┘
//! ```

use std::f32::consts::FRAC_PI_2;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};
use kira::effect::{Effect, EffectBuilder};
use kira::info::Info;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::sound::{PlaybackState, Sound, SoundData};
use kira::track::{MainTrackBuilder, TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Frame, Panning, Tween};

use super::PlayParams;
use super::dsp::{Rng, SAMPLE_RATE, db_to_gain};
use super::music::{MusicData, MusicTrack, STEM_COUNT, render_track, stem_gains};
use super::sfx::{Sfx, render_variant};
use super::spatial::{Spatial, spatialize};

/// Hard cap on simultaneous sound effects (kira's sfx track is sized a little above this).
pub const MAX_VOICES: usize = 96;
/// Music track change crossfade.
pub const CROSSFADE_SECS: f32 = 1.5;
/// Intensity smoothing time constants: rising (combat starts) and falling (it ends).
pub const INTENSITY_RISE_SECS: f32 = 0.6;
pub const INTENSITY_FALL_SECS: f32 = 3.0;
/// Per-stem gain ramp inside the audio thread (de-zippering only; the slow fades come from the
/// smoothed intensity).
const STEM_RAMP_SECS: f32 = 0.25;

fn amp_to_db(amp: f32) -> Decibels {
    if amp <= 0.001 || !amp.is_finite() {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * amp.log10())
    }
}

fn tween(ms: u64) -> Tween {
    Tween {
        duration: Duration::from_millis(ms),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------------------------
// Master limiter
// ---------------------------------------------------------------------------------------------

/// Stereo-linked peak limiter with instant attack: whatever piles up (twenty explosions on one
/// frame), the output never exceeds the threshold. Also scrubs non-finite samples.
pub(crate) struct LimiterBuilder {
    pub threshold: f32,
    pub release_secs: f32,
}

struct Limiter {
    threshold: f32,
    release_secs: f32,
    coef: f32,
    gain: f32,
}

impl Limiter {
    fn set_rate(&mut self, sample_rate: u32) {
        self.coef = (-1.0 / (self.release_secs * sample_rate.max(1) as f32)).exp();
    }
}

impl EffectBuilder for LimiterBuilder {
    type Handle = ();

    fn build(self) -> (Box<dyn Effect>, ()) {
        let mut limiter = Limiter {
            threshold: self.threshold,
            release_secs: self.release_secs,
            coef: 0.0,
            gain: 1.0,
        };
        limiter.set_rate(48_000);
        (Box::new(limiter), ())
    }
}

impl Effect for Limiter {
    fn init(&mut self, sample_rate: u32, _internal_buffer_size: usize) {
        self.set_rate(sample_rate);
    }

    fn on_change_sample_rate(&mut self, sample_rate: u32) {
        self.set_rate(sample_rate);
    }

    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &Info) {
        for frame in input {
            if !frame.left.is_finite() || !frame.right.is_finite() {
                *frame = Frame::ZERO;
                continue;
            }
            let peak = frame.left.abs().max(frame.right.abs());
            let needed = if peak > self.threshold {
                self.threshold / peak
            } else {
                1.0
            };
            self.gain = if needed < self.gain {
                needed
            } else {
                needed + (self.gain - needed) * self.coef
            };
            *frame *= self.gain;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Music player
// ---------------------------------------------------------------------------------------------

/// A track converted for playback: 16-bit, frame-major (`[frame][stem][channel]`) so one
/// playhead reads every stem from adjacent memory.
pub(crate) struct MusicPcm {
    pub track: MusicTrack,
    samples: Vec<i16>,
    len: usize,
    /// Intro spill heard on the first pass only (same layout), starting at `loop_start`.
    tail: Vec<i16>,
    tail_len: usize,
    loop_start: usize,
    sample_rate: f64,
    pub intensity_floor: f32,
}

impl MusicPcm {
    pub fn from_data(data: &MusicData) -> Self {
        let len = data.len();
        let tail_len = data
            .stems
            .iter()
            .map(|s| s.first_pass_tail.len())
            .max()
            .unwrap_or(0);
        let interleave = |frames_of: &dyn Fn(&super::music::Stem) -> &[[f32; 2]], n: usize| {
            let mut out = vec![0i16; n * STEM_COUNT * 2];
            for (s, stem) in data.stems.iter().enumerate().take(STEM_COUNT) {
                for (i, frame) in frames_of(stem).iter().enumerate().take(n) {
                    for (c, v) in frame.iter().enumerate() {
                        out[(i * STEM_COUNT + s) * 2 + c] =
                            (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                    }
                }
            }
            out
        };
        Self {
            track: data.track,
            samples: interleave(&|s| &s.frames, len),
            len,
            tail: interleave(&|s| &s.first_pass_tail, tail_len),
            tail_len,
            loop_start: data.loop_start.min(len.saturating_sub(1)),
            sample_rate: super::music::MUSIC_SAMPLE_RATE as f64,
            intensity_floor: data.intensity_floor,
        }
    }

    fn frame(&self, i: usize) -> &[i16] {
        &self.samples[i * STEM_COUNT * 2..(i + 1) * STEM_COUNT * 2]
    }

    fn tail_frame(&self, i: usize) -> &[i16] {
        &self.tail[i * STEM_COUNT * 2..(i + 1) * STEM_COUNT * 2]
    }
}

/// Lock-free controls shared between the game thread and a playing [`MusicSound`]. Dropping
/// every game-side reference releases the sound: it fades out and unloads itself.
pub(crate) struct MusicControl {
    stem_targets: [AtomicU32; STEM_COUNT],
    fade_target: AtomicU32,
    fade_secs: AtomicU32,
    /// Playhead in frames (diagnostics and tests).
    position: AtomicU64,
}

fn load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

fn store(a: &AtomicU32, v: f32) {
    a.store(v.to_bits(), Ordering::Relaxed);
}

impl MusicControl {
    pub fn new(gains: [f32; STEM_COUNT]) -> Self {
        Self {
            stem_targets: gains.map(|g| AtomicU32::new(g.to_bits())),
            fade_target: AtomicU32::new(0f32.to_bits()),
            fade_secs: AtomicU32::new(CROSSFADE_SECS.to_bits()),
            position: AtomicU64::new(0),
        }
    }

    pub fn set_stem_targets(&self, gains: [f32; STEM_COUNT]) {
        for (a, g) in self.stem_targets.iter().zip(gains) {
            store(a, g.clamp(0.0, 1.0));
        }
    }

    pub fn fade_to(&self, target: f32, secs: f32) {
        store(&self.fade_secs, secs.max(0.001));
        store(&self.fade_target, target.clamp(0.0, 1.0));
    }

    pub fn position(&self) -> u64 {
        self.position.load(Ordering::Relaxed)
    }
}

pub(crate) struct MusicSoundData {
    pub pcm: Arc<MusicPcm>,
    pub control: Arc<MusicControl>,
}

impl SoundData for MusicSoundData {
    type Error = ();
    type Handle = ();

    fn into_sound(self) -> Result<(Box<dyn Sound>, ()), ()> {
        Ok((Box::new(MusicSound::new(self.pcm, self.control)), ()))
    }
}

/// Plays every stem of one track in lockstep, looping sample-accurately, with per-stem gain
/// ramps and an equal-power fade for crossfades.
pub(crate) struct MusicSound {
    pcm: Arc<MusicPcm>,
    control: Arc<MusicControl>,
    pos: f64,
    /// True until the playhead wraps for the first time (the intro tail plays meanwhile).
    first_pass: bool,
    gains: [f32; STEM_COUNT],
    fade: f32,
    finished: bool,
}

impl MusicSound {
    pub fn new(pcm: Arc<MusicPcm>, control: Arc<MusicControl>) -> Self {
        let gains = control.stem_targets.each_ref().map(load);
        Self {
            pcm,
            control,
            pos: 0.0,
            first_pass: true,
            gains,
            fade: 0.0,
            finished: false,
        }
    }
}

fn approach(current: f32, target: f32, step: f32) -> f32 {
    if current < target {
        (current + step).min(target)
    } else {
        (current - step).max(target)
    }
}

impl Sound for MusicSound {
    fn process(&mut self, out: &mut [Frame], dt: f64, _info: &Info) {
        if self.finished || self.pcm.len == 0 {
            out.fill(Frame::ZERO);
            self.finished = true;
            return;
        }
        let released = Arc::strong_count(&self.control) == 1;
        let fade_target = if released {
            0.0
        } else {
            load(&self.control.fade_target)
        };
        let fade_step = dt as f32 / load(&self.control.fade_secs).max(0.001);
        let gain_step = dt as f32 / STEM_RAMP_SECS;
        let targets = self.control.stem_targets.each_ref().map(load);
        let advance = self.pcm.sample_rate * dt;
        let len = self.pcm.len as f64;
        let scale = 1.0 / 32768.0;

        for frame in out.iter_mut() {
            self.fade = approach(self.fade, fade_target, fade_step);
            for (g, t) in self.gains.iter_mut().zip(targets) {
                *g = approach(*g, t, gain_step);
            }
            let i0 = self.pos as usize;
            let frac = (self.pos - i0 as f64) as f32;
            let i1 = if i0 + 1 >= self.pcm.len {
                self.pcm.loop_start
            } else {
                i0 + 1
            };
            let (a, b) = (self.pcm.frame(i0), self.pcm.frame(i1));
            let tail = match i0.checked_sub(self.pcm.loop_start) {
                Some(k) if self.first_pass && k + 1 < self.pcm.tail_len => {
                    Some((self.pcm.tail_frame(k), self.pcm.tail_frame(k + 1)))
                }
                _ => None,
            };
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for (s, &g) in self.gains.iter().enumerate() {
                if g <= 0.0 {
                    continue;
                }
                let (mut al, mut ar) = (a[s * 2] as f32, a[s * 2 + 1] as f32);
                let (mut bl, mut br) = (b[s * 2] as f32, b[s * 2 + 1] as f32);
                if let Some((ta, tb)) = tail {
                    al += ta[s * 2] as f32;
                    ar += ta[s * 2 + 1] as f32;
                    bl += tb[s * 2] as f32;
                    br += tb[s * 2 + 1] as f32;
                }
                l += g * (al + (bl - al) * frac);
                r += g * (ar + (br - ar) * frac);
            }
            let f = (self.fade * FRAC_PI_2).sin() * scale;
            *frame = Frame::new(l * f, r * f);
            self.pos += advance;
            if self.pos >= len {
                self.pos = self.pcm.loop_start as f64 + (self.pos - len);
                self.first_pass = false;
            }
        }
        self.control
            .position
            .store(self.pos as u64, Ordering::Relaxed);
        if self.fade <= 0.0 && fade_target <= 0.0 {
            self.finished = true;
        }
    }

    fn finished(&self) -> bool {
        self.finished
    }
}

struct Playing {
    track: MusicTrack,
    control: Arc<MusicControl>,
    floor: f32,
}

struct MusicState {
    incoming: Receiver<Arc<MusicPcm>>,
    loaded: [Option<Arc<MusicPcm>>; MusicTrack::ALL.len()],
    wanted: MusicTrack,
    playing: Option<Playing>,
    intensity_target: f32,
    intensity: f32,
}

/// Renders every playable track on its own background thread.
fn spawn_music_generation() -> Receiver<Arc<MusicPcm>> {
    let (tx, rx): (Sender<Arc<MusicPcm>>, _) = channel();
    for track in MusicTrack::PLAYABLE {
        let tx = tx.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("music-gen-{track:?}"))
            .spawn(move || {
                let start = Instant::now();
                if let Some(data) = render_track(track) {
                    let pcm = MusicPcm::from_data(&data);
                    log::debug!(
                        "audio: generated {track:?} music ({:.1} s loop) in {} ms",
                        data.len() as f32 / super::music::MUSIC_SAMPLE_RATE as f32,
                        start.elapsed().as_millis()
                    );
                    // The receiver is gone if Audio was dropped meanwhile; nothing to do then.
                    let _ = tx.send(Arc::new(pcm));
                }
            });
        if let Err(e) = spawned {
            log::warn!("audio: could not spawn music generator for {track:?}: {e}");
        }
    }
    rx
}

// ---------------------------------------------------------------------------------------------
// Sound effects
// ---------------------------------------------------------------------------------------------

/// Renders every sound effect variation, spread over a few threads. Index: `[sfx][variant]`.
pub(crate) fn generate_sfx() -> Vec<Vec<Vec<f32>>> {
    let jobs: Vec<(Sfx, u8)> = Sfx::ALL
        .iter()
        .flat_map(|&s| (0..s.info().variants).map(move |v| (s, v)))
        .collect();
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(1, 8);
    let mut out: Vec<Vec<Vec<f32>>> = Sfx::ALL
        .iter()
        .map(|s| vec![Vec::new(); s.info().variants as usize])
        .collect();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|t| {
                let jobs = &jobs;
                scope.spawn(move || {
                    jobs.iter()
                        .skip(t)
                        .step_by(threads)
                        .map(|&(s, v)| (s, v, render_variant(s, v)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for worker in workers {
            match worker.join() {
                Ok(done) => {
                    for (s, v, buf) in done {
                        out[s.index()][v as usize] = buf;
                    }
                }
                Err(_) => {
                    log::warn!("audio: a sound generation thread panicked; some sounds are silent")
                }
            }
        }
    });
    out
}

fn to_sound_data(samples: &[f32]) -> StaticSoundData {
    StaticSoundData {
        sample_rate: SAMPLE_RATE,
        frames: samples.iter().map(|&s| Frame::from_mono(s)).collect(),
        settings: StaticSoundSettings::default(),
        slice: None,
    }
}

struct Voice {
    sfx: Sfx,
    handle: StaticSoundHandle,
    started: f64,
    /// Requested volume before spatial attenuation.
    base_gain: f32,
    /// Currently applied gain and pan.
    gain: f32,
    pan: f32,
    position: Option<Vec3>,
}

// ---------------------------------------------------------------------------------------------
// Backend
// ---------------------------------------------------------------------------------------------

pub(crate) struct Backend {
    manager: AudioManager<DefaultBackend>,
    sfx_track: TrackHandle,
    music_track: TrackHandle,
    /// `[sfx index][variant]`; an empty list means the sound failed to generate.
    sounds: Vec<Vec<StaticSoundData>>,
    voices: Vec<Voice>,
    music: MusicState,
    rng: Rng,
    time: f64,
    listener_pos: Vec3,
    listener_rot: Quat,
}

impl Backend {
    pub fn new() -> Result<Backend, String> {
        let settings = AudioManagerSettings::<DefaultBackend> {
            main_track_builder: MainTrackBuilder::new().with_effect(LimiterBuilder {
                threshold: db_to_gain(-1.0),
                release_secs: 0.15,
            }),
            ..Default::default()
        };
        let mut manager =
            AudioManager::<DefaultBackend>::new(settings).map_err(|e| e.to_string())?;
        let sfx_track = manager
            .add_sub_track(TrackBuilder::new().sound_capacity(MAX_VOICES + 32))
            .map_err(|e| e.to_string())?;
        let music_track = manager
            .add_sub_track(TrackBuilder::new().sound_capacity(16))
            .map_err(|e| e.to_string())?;

        // Music renders in the background while the sound effects render here.
        let incoming = spawn_music_generation();
        let start = Instant::now();
        let sounds = generate_sfx()
            .into_iter()
            .map(|variants| {
                variants
                    .iter()
                    .filter(|v| !v.is_empty())
                    .map(|v| to_sound_data(v))
                    .collect()
            })
            .collect();
        log::info!(
            "audio: generated {} sound effects in {} ms",
            Sfx::COUNT,
            start.elapsed().as_millis()
        );

        Ok(Backend {
            manager,
            sfx_track,
            music_track,
            sounds,
            voices: Vec::with_capacity(MAX_VOICES),
            music: MusicState {
                incoming,
                loaded: Default::default(),
                wanted: MusicTrack::None,
                playing: None,
                intensity_target: 0.0,
                intensity: 0.0,
            },
            rng: Rng::new(0xA0D10),
            time: 0.0,
            listener_pos: Vec3::ZERO,
            listener_rot: Quat::IDENTITY,
        })
    }

    pub fn play(&mut self, sfx: Sfx, params: PlayParams) {
        let info = sfx.info();
        let count = self.sounds[sfx.index()].len();
        if count == 0 {
            return;
        }
        let spatial = match params.position {
            Some(p) => spatialize(self.listener_pos, self.listener_rot, p),
            None => Spatial::CENTER,
        };
        let base_gain =
            params.volume.max(0.0) * db_to_gain(self.rng.bipolar() * info.volume_jitter_db);
        let gain = base_gain * spatial.gain;
        if gain.is_nan() || gain <= 0.001 {
            return; // inaudible
        }
        // Drop near-simultaneous repeats unless the new one is clearly louder.
        if info.min_interval > 0.0
            && self.voices.iter().any(|v| {
                v.sfx == sfx
                    && self.time - v.started < info.min_interval as f64
                    && v.gain >= gain * 0.7
            })
        {
            return;
        }
        // Per-sound voice limit, then the global cap: fade out the oldest.
        let mut same = self.voices.iter().filter(|v| v.sfx == sfx).count();
        while same >= info.max_voices as usize {
            if let Some(i) = self.voices.iter().position(|v| v.sfx == sfx) {
                self.voices.remove(i).handle.stop(tween(30));
            }
            same -= 1;
        }
        if self.voices.len() >= MAX_VOICES {
            self.voices.remove(0).handle.stop(tween(30));
        }

        let pitch = params.pitch.clamp(0.125, 8.0) * (1.0 + self.rng.bipolar() * info.pitch_jitter);
        let variant = (self.rng.f32() * count as f32) as usize % count;
        let data = self.sounds[sfx.index()][variant]
            .volume(amp_to_db(gain))
            .playback_rate(pitch as f64)
            .panning(Panning(spatial.pan));
        match self.sfx_track.play(data) {
            Ok(handle) => self.voices.push(Voice {
                sfx,
                handle,
                started: self.time,
                base_gain,
                gain,
                pan: spatial.pan,
                position: params.position,
            }),
            Err(e) => log::debug!("audio: {sfx:?} not played: {e}"),
        }
    }

    pub fn stop(&mut self, sfx: Sfx) {
        self.voices.retain_mut(|v| {
            if v.sfx == sfx {
                v.handle.stop(tween(40));
            }
            v.sfx != sfx
        });
    }

    pub fn set_listener(&mut self, position: Vec3, rotation: Quat) {
        if position.is_finite() && rotation.is_finite() && rotation.length_squared() > 1e-8 {
            self.listener_pos = position;
            self.listener_rot = rotation.normalize();
        }
    }

    pub fn set_volumes(&mut self, master: f32, sfx: f32, music: f32) {
        let v = |x: f32| {
            amp_to_db(if x.is_finite() {
                x.clamp(0.0, 1.0)
            } else {
                0.0
            })
        };
        self.manager.main_track().set_volume(v(master), tween(50));
        self.sfx_track.set_volume(v(sfx), tween(50));
        self.music_track.set_volume(v(music), tween(50));
    }

    pub fn set_music(&mut self, track: MusicTrack) {
        if self.music.wanted != track {
            self.music.wanted = track;
            self.start_wanted_music();
        }
    }

    pub fn set_music_intensity(&mut self, intensity: f32) {
        if intensity.is_finite() {
            self.music.intensity_target = intensity.clamp(0.0, 1.0);
        }
    }

    pub fn music_ready(&self, track: MusicTrack) -> bool {
        track == MusicTrack::None || self.music.loaded[track.index()].is_some()
    }

    pub fn music_position(&self) -> Option<u64> {
        self.music.playing.as_ref().map(|p| p.control.position())
    }

    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }

    pub fn update(&mut self, dt: f32) {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.5)
        } else {
            0.0
        };
        self.time += dt as f64;

        // Finished sounds go; playing spatial ones follow the (moving) listener.
        self.voices
            .retain(|v| v.handle.state() != PlaybackState::Stopped);
        for v in &mut self.voices {
            let Some(pos) = v.position else { continue };
            let s = spatialize(self.listener_pos, self.listener_rot, pos);
            let gain = v.base_gain * s.gain;
            let db_change = (amp_to_db(gain).0 - amp_to_db(v.gain).0).abs();
            if db_change > 0.5 || (s.pan - v.pan).abs() > 0.03 {
                v.handle.set_volume(amp_to_db(gain), tween(50));
                v.handle.set_panning(Panning(s.pan), tween(50));
                v.gain = gain;
                v.pan = s.pan;
            }
        }

        // Music generated in the background arrives here.
        let mut arrived = false;
        while let Ok(pcm) = self.music.incoming.try_recv() {
            let i = pcm.track.index();
            self.music.loaded[i] = Some(pcm);
            arrived = true;
        }
        if arrived && self.music.playing.is_none() {
            self.start_wanted_music();
        }

        // Smoothed intensity drives the stem gains.
        let m = &mut self.music;
        let tau = if m.intensity_target > m.intensity {
            INTENSITY_RISE_SECS
        } else {
            INTENSITY_FALL_SECS
        };
        m.intensity += (m.intensity_target - m.intensity) * (1.0 - (-dt / tau).exp());
        if let Some(p) = &m.playing {
            p.control
                .set_stem_targets(stem_gains(m.intensity.max(p.floor)));
        }
    }

    fn start_wanted_music(&mut self) {
        let wanted = self.music.wanted;
        if self
            .music
            .playing
            .as_ref()
            .is_some_and(|p| p.track == wanted)
        {
            return;
        }
        if let Some(old) = self.music.playing.take() {
            // Dropping our reference releases it; it fades out over the crossfade time.
            old.control.fade_to(0.0, CROSSFADE_SECS);
        }
        let Some(pcm) = self.music.loaded.get(wanted.index()).cloned().flatten() else {
            return; // MusicTrack::None, or still generating (started from update() later)
        };
        let floor = pcm.intensity_floor;
        let control = Arc::new(MusicControl::new(stem_gains(
            self.music.intensity.max(floor),
        )));
        control.fade_to(1.0, CROSSFADE_SECS);
        let data = MusicSoundData {
            pcm,
            control: control.clone(),
        };
        match self.music_track.play(data) {
            Ok(()) => {
                self.music.playing = Some(Playing {
                    track: wanted,
                    control,
                    floor,
                })
            }
            Err(e) => log::warn!("audio: could not start {wanted:?} music: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::music::{MusicData, Stem, StemKind};
    use kira::info::MockInfoBuilder;

    fn test_pcm(len: usize, loop_start: usize) -> Arc<MusicPcm> {
        // Each stem holds a ramp so the playhead position is observable in the output.
        let stems = StemKind::ALL
            .iter()
            .map(|&kind| Stem {
                kind,
                frames: (0..len).map(|i| [i as f32 / len as f32 * 0.5; 2]).collect(),
                first_pass_tail: Vec::new(),
            })
            .collect();
        let data = MusicData {
            track: MusicTrack::Campaign,
            bpm: 150.0,
            bars: 1,
            bar_frames: len,
            loop_start,
            intensity_floor: 0.0,
            stems,
        };
        Arc::new(MusicPcm::from_data(&data))
    }

    #[test]
    fn music_sound_loops_and_fades() {
        let pcm = test_pcm(100, 20);
        let control = Arc::new(MusicControl::new([1.0, 0.0, 0.0, 0.0, 0.0, 0.0]));
        control.fade_to(1.0, 0.001);
        let mut sound = MusicSound::new(pcm, control.clone());
        let info = MockInfoBuilder::new().build();
        let mut out = vec![Frame::ZERO; 250];
        // dt = 1 / 44100 -> exactly one source frame per output frame.
        sound.process(&mut out, 1.0 / SAMPLE_RATE as f64, &info);
        // 250 frames: 100 to the end, then 150 more within the 80-frame loop region -> 20 + 70.
        assert_eq!(control.position(), 90);
        assert!(
            out[150].left > 0.0
                && out
                    .iter()
                    .all(|f| f.left.is_finite() && f.left.abs() <= 1.0)
        );
        assert!(!sound.finished());

        // Releasing the control fades the sound out and lets kira unload it.
        drop(control);
        let mut out = vec![Frame::ZERO; 64];
        for _ in 0..200 {
            sound.process(&mut out, 1.0 / SAMPLE_RATE as f64, &info);
        }
        assert!(sound.finished());
    }

    #[test]
    fn limiter_caps_peaks() {
        let (mut effect, ()) = LimiterBuilder {
            threshold: 0.9,
            release_secs: 0.1,
        }
        .build();
        effect.init(48_000, 128);
        let info = MockInfoBuilder::new().build();
        let mut frames: Vec<Frame> = (0..512)
            .map(|i| Frame::from_mono((i as f32 * 0.1).sin() * 4.0))
            .collect();
        frames[10] = Frame::new(f32::NAN, 0.0);
        effect.process(&mut frames, 1.0 / 48_000.0, &info);
        assert!(
            frames
                .iter()
                .all(|f| f.left.is_finite() && f.left.abs() <= 0.9 + 1e-6)
        );
    }
}
