//! Procedural music: each track is a short loop (8 bars) split into stems that are layered by
//! combat intensity.
//!
//! Every stem of a track has exactly the same length (a whole number of bars; tempos are chosen
//! so a 16th note is a whole number of samples), so stems played from one shared playhead stay
//! sample-locked forever. Notes and effect tails that run past the loop end are folded back to
//! the loop start, which makes the loop seamless.
//!
//! | stem          | fades in at intensity |
//! |---------------|-----------------------|
//! | `pad`         | always                |
//! | `bass`        | 0.20 – 0.35           |
//! | `drums_light` | 0.20 – 0.35           |
//! | `drums_full`  | 0.50 – 0.62           |
//! | `guitar`      | 0.55 – 0.70           |
//! | `lead`        | 0.85 – 0.97           |

use std::f32::consts::{FRAC_PI_4, SQRT_2, TAU};

use serde::{Deserialize, Serialize};

use super::dsp::{
    Biquad, Osc, Rng, SAMPLE_RATE, SR, Svf, Wave, adsr, db_to_gain, drive, layer, loudness,
    midi_hz, perc, reverb_with_rate, samples, smoothstep, soft_clip, soft_limit,
};
use super::sfx::{Pass, click, fm, metal, noise, thump};

/// Sample rate of the music stems (same as the sound effects).
pub const MUSIC_SAMPLE_RATE: u32 = SAMPLE_RATE;

/// Which music is playing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MusicTrack {
    #[default]
    None,
    /// Slow (90 BPM) ominous A-minor loop. Always plays with at least intensity 0.35.
    Menu,
    /// 150 BPM E-phrygian industrial breakbeat-metal for campaign levels.
    Campaign,
    /// 175 BPM D-minor drum'n'bass-metal for PvP arenas.
    Arena,
    /// 126 BPM: a one-bar sting, then a 4-bar cool-down loop. All stems always play.
    Results,
}

impl MusicTrack {
    pub const ALL: [MusicTrack; 5] = [
        MusicTrack::None,
        MusicTrack::Menu,
        MusicTrack::Campaign,
        MusicTrack::Arena,
        MusicTrack::Results,
    ];

    /// Every track that has music (all but `None`).
    pub const PLAYABLE: [MusicTrack; 4] = [
        MusicTrack::Menu,
        MusicTrack::Campaign,
        MusicTrack::Arena,
        MusicTrack::Results,
    ];

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StemKind {
    Pad,
    Bass,
    DrumsLight,
    DrumsFull,
    Guitar,
    Lead,
}

pub const STEM_COUNT: usize = 6;

impl StemKind {
    pub const ALL: [StemKind; STEM_COUNT] = [
        StemKind::Pad,
        StemKind::Bass,
        StemKind::DrumsLight,
        StemKind::DrumsFull,
        StemKind::Guitar,
        StemKind::Lead,
    ];

    pub fn name(self) -> &'static str {
        match self {
            StemKind::Pad => "pad",
            StemKind::Bass => "bass",
            StemKind::DrumsLight => "drums_light",
            StemKind::DrumsFull => "drums_full",
            StemKind::Guitar => "guitar",
            StemKind::Lead => "lead",
        }
    }

    /// Intensity window over which the stem fades in (smoothstep from `.0` to `.1`).
    pub fn fade_in_range(self) -> (f32, f32) {
        match self {
            StemKind::Pad => (-1.0, 0.0),
            StemKind::Bass | StemKind::DrumsLight => (0.2, 0.35),
            StemKind::DrumsFull => (0.5, 0.62),
            StemKind::Guitar => (0.55, 0.7),
            StemKind::Lead => (0.85, 0.97),
        }
    }
}

/// Target gain (0..1) of every stem at `intensity` (0 = calm, 1 = full combat).
pub fn stem_gains(intensity: f32) -> [f32; STEM_COUNT] {
    StemKind::ALL.map(|k| {
        let (a, b) = k.fade_in_range();
        smoothstep(a, b, intensity)
    })
}

/// One layer of a track: stereo frames at [`MUSIC_SAMPLE_RATE`].
pub struct Stem {
    pub kind: StemKind,
    pub frames: Vec<[f32; 2]>,
    /// Tails of the intro (sting) that ring into the loop region. They are heard only on the
    /// first pass, starting at `loop_start`, so the loop seam stays clean afterwards.
    pub first_pass_tail: Vec<[f32; 2]>,
}

/// A fully rendered track.
pub struct MusicData {
    pub track: MusicTrack,
    pub bpm: f32,
    pub bars: usize,
    pub bar_frames: usize,
    /// Frame the loop jumps back to (non-zero when the track opens with a one-off sting).
    pub loop_start: usize,
    /// The effective intensity never drops below this.
    pub intensity_floor: f32,
    /// One stem per [`StemKind`], in [`StemKind::ALL`] order, all the same length.
    pub stems: Vec<Stem>,
}

impl MusicData {
    /// Length in frames (identical for every stem).
    pub fn len(&self) -> usize {
        self.stems.first().map_or(0, |s| s.frames.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Sum of all stems at the given gains (the loop body, without the first-pass tail).
    pub fn mixdown(&self, gains: &[f32; STEM_COUNT]) -> Vec<[f32; 2]> {
        let mut out = vec![[0.0f32; 2]; self.len()];
        for (stem, &g) in self.stems.iter().zip(gains) {
            for (o, f) in out.iter_mut().zip(&stem.frames) {
                o[0] += f[0] * g;
                o[1] += f[1] * g;
            }
        }
        out
    }

    /// What the player outputs for the first `frames` frames at fixed stem gains: plays from the
    /// start, wraps to `loop_start` at the end, and adds the intro tail on the first pass only.
    pub fn playback(&self, gains: &[f32; STEM_COUNT], frames: usize) -> Vec<[f32; 2]> {
        let len = self.len();
        let mut out = Vec::with_capacity(frames);
        if len == 0 {
            return out;
        }
        let (mut pos, mut first_pass) = (0usize, true);
        for _ in 0..frames {
            let mut f = [0.0f32; 2];
            for (stem, &g) in self.stems.iter().zip(gains) {
                let s = stem.frames[pos];
                let t = match pos.checked_sub(self.loop_start) {
                    Some(k) if first_pass => stem.first_pass_tail.get(k).copied().unwrap_or_default(),
                    _ => [0.0; 2],
                };
                f[0] += (s[0] + t[0]) * g;
                f[1] += (s[1] + t[1]) * g;
            }
            out.push(f);
            pos += 1;
            if pos >= len {
                pos = self.loop_start;
                first_pass = false;
            }
        }
        out
    }
}

/// Renders a track (deterministic). `None` has no music.
pub fn render_track(track: MusicTrack) -> Option<MusicData> {
    match track {
        MusicTrack::None => None,
        MusicTrack::Menu => Some(menu()),
        MusicTrack::Campaign => Some(campaign()),
        MusicTrack::Arena => Some(arena()),
        MusicTrack::Results => Some(results()),
    }
}

// ---------------------------------------------------------------------------------------------
// Sequencing
// ---------------------------------------------------------------------------------------------

struct Song {
    bars: usize,
    intro_bars: usize,
    /// Samples per 16th note.
    step: usize,
    bpm: f32,
}

impl Song {
    fn new(bpm: f32, bars: usize, intro_bars: usize) -> Self {
        let step = (SR * 60.0 / bpm / 4.0).round() as usize;
        debug_assert!(
            ((SR * 60.0 / bpm / 4.0) - step as f32).abs() < 1e-3,
            "tempo {bpm} does not give whole-sample 16ths"
        );
        Self {
            bars,
            intro_bars,
            step,
            bpm,
        }
    }

    fn bar_frames(&self) -> usize {
        16 * self.step
    }

    fn len(&self) -> usize {
        self.bars * self.bar_frames()
    }

    fn loop_start(&self) -> usize {
        self.intro_bars * self.bar_frames()
    }

    fn at(&self, bar: usize, step: usize) -> usize {
        (bar * 16 + step) * self.step
    }

    fn secs(&self, steps: f32) -> f32 {
        steps * self.step as f32 / SR
    }
}

/// Parses a step pattern: `X` accent (1.0), `x` normal (0.7), `o` ghost (0.35), anything else is
/// a rest. Patterns may be longer than one bar.
fn hits(pattern: &str) -> Vec<(usize, f32)> {
    pattern
        .chars()
        .enumerate()
        .filter_map(|(i, c)| match c {
            'X' => Some((i, 1.0)),
            'x' => Some((i, 0.7)),
            'o' => Some((i, 0.35)),
            _ => None,
        })
        .collect()
}

/// Constant-power pan gains, normalised to 1 at the centre.
fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
    (a.cos() * SQRT_2, a.sin() * SQRT_2)
}

fn cents(c: f32) -> f32 {
    2f32.powf(c / 1200.0)
}

/// A stereo stem under construction: a linear "tape" with room for tails, folded into the loop
/// at the end.
struct Tape {
    l: Vec<f32>,
    r: Vec<f32>,
}

impl Tape {
    fn new(song: &Song) -> Self {
        let n = song.len() + samples(4.0);
        Self {
            l: vec![0.0; n],
            r: vec![0.0; n],
        }
    }

    fn grow(&mut self, n: usize) {
        if self.l.len() < n {
            self.l.resize(n, 0.0);
            self.r.resize(n, 0.0);
        }
    }

    fn add(&mut self, pos: usize, mono: &[f32], gain: f32, pan: f32) {
        let (gl, gr) = pan_gains(pan);
        self.grow(pos + mono.len());
        for (i, s) in mono.iter().enumerate() {
            self.l[pos + i] += s * gain * gl;
            self.r[pos + i] += s * gain * gr;
        }
    }

    fn add_stereo(&mut self, pos: usize, [l, r]: &[Vec<f32>; 2], gain: f32) {
        self.grow(pos + l.len().max(r.len()));
        for (i, s) in l.iter().enumerate() {
            self.l[pos + i] += s * gain;
        }
        for (i, s) in r.iter().enumerate() {
            self.r[pos + i] += s * gain;
        }
    }

    /// Ping-pong echo: repeats alternate left and right.
    fn echo(&mut self, delay: usize, feedback: f32, mix: f32) {
        let d = delay.max(1);
        let (mut dl, mut dr) = (vec![0.0f32; d], vec![0.0f32; d]);
        for i in 0..self.l.len() {
            let k = i % d;
            let input = (self.l[i] + self.r[i]) * 0.5;
            let (ol, or) = (dl[k], dr[k]);
            dl[k] = input + or * feedback;
            dr[k] = ol * feedback;
            self.l[i] += ol * mix;
            self.r[i] += or * mix;
        }
    }

    fn reverb(&mut self, size: f32, damp: f32, mix: f32) {
        reverb_with_rate(&mut self.l, size, damp, mix, 0.0, SR, 0);
        reverb_with_rate(&mut self.r, size, damp, mix, 0.0, SR, 23);
    }

    /// Wraps everything past the loop end back into the loop region and returns the frames.
    fn fold(mut self, song: &Song) -> Vec<[f32; 2]> {
        let len = song.len();
        let start = song.loop_start();
        let span = len - start;
        for i in len..self.l.len() {
            let target = start + (i - len) % span;
            self.l[target] += self.l[i];
            self.r[target] += self.r[i];
        }
        self.l.truncate(len);
        self.r.truncate(len);
        self.l.iter().zip(&self.r).map(|(&l, &r)| [l, r]).collect()
    }
}

struct Arrangement {
    song: Song,
    tapes: Vec<Tape>,
    /// Separate tapes for notes that start in the intro, so their tails can be kept out of the
    /// loop body (empty when the track has no intro).
    intro: Vec<Tape>,
}

impl Arrangement {
    fn new(song: Song) -> Self {
        let tapes = (0..STEM_COUNT).map(|_| Tape::new(&song)).collect();
        let intro = if song.intro_bars > 0 {
            (0..STEM_COUNT).map(|_| Tape::new(&song)).collect()
        } else {
            Vec::new()
        };
        Self { song, tapes, intro }
    }

    fn tape_at(&mut self, kind: StemKind, pos: usize) -> &mut Tape {
        if pos < self.song.loop_start() {
            &mut self.intro[kind as usize]
        } else {
            &mut self.tapes[kind as usize]
        }
    }

    /// Adds a mono sound starting at frame `pos`.
    fn add_at(&mut self, kind: StemKind, pos: usize, mono: &[f32], gain: f32, pan: f32) {
        self.tape_at(kind, pos).add(pos, mono, gain, pan);
    }

    fn add_stereo_at(&mut self, kind: StemKind, pos: usize, stereo: &[Vec<f32>; 2], gain: f32) {
        self.tape_at(kind, pos).add_stereo(pos, stereo, gain);
    }

    fn add(&mut self, kind: StemKind, bar: usize, step: usize, mono: &[f32], gain: f32, pan: f32) {
        let pos = self.song.at(bar, step);
        self.add_at(kind, pos, mono, gain, pan);
    }

    /// Runs a (linear) effect over a whole stem.
    fn effect(&mut self, kind: StemKind, f: impl Fn(&mut Tape)) {
        f(&mut self.tapes[kind as usize]);
        if let Some(tape) = self.intro.get_mut(kind as usize) {
            f(tape);
        }
    }

    /// Places `sample` at every hit of `pattern` in `bar`, scaled by hit velocity.
    fn pattern(&mut self, kind: StemKind, bar: usize, pattern: &str, sample: &[f32], gain: f32, pan: f32) {
        for (step, vel) in hits(pattern) {
            self.add(kind, bar, step, sample, gain * vel, pan);
        }
    }

    fn bar_secs(&self) -> f32 {
        self.song.secs(16.0)
    }

    /// Folds the tapes into loops and masters the track: the loudest 400 ms of the full mix sits
    /// at `level_db` (K-weighted momentary loudness) with peaks below -1 dBFS.
    fn finish(self, track: MusicTrack, intensity_floor: f32, level_db: f32) -> MusicData {
        let song = self.song;
        let (len, loop_start) = (song.len(), song.loop_start());
        let mut intro = self.intro.into_iter();
        let mut stems: Vec<Stem> = self
            .tapes
            .into_iter()
            .zip(StemKind::ALL)
            .map(|(tape, kind)| {
                let mut frames = tape.fold(&song);
                let mut first_pass_tail = Vec::new();
                if let Some(intro) = intro.next() {
                    // The intro itself goes into the body; what rings past it becomes the tail.
                    for (i, f) in frames.iter_mut().enumerate().take(loop_start) {
                        f[0] += intro.l[i];
                        f[1] += intro.r[i];
                    }
                    let end = intro.l.len().min(len);
                    let audible = (loop_start..end)
                        .rposition(|i| intro.l[i].abs().max(intro.r[i].abs()) > 1e-5)
                        .map_or(0, |n| n + 1);
                    first_pass_tail = (loop_start..loop_start + audible)
                        .map(|i| [intro.l[i], intro.r[i]])
                        .collect();
                }
                Stem {
                    kind,
                    frames,
                    first_pass_tail,
                }
            })
            .collect();

        let mut pk = 0.0f32;
        let mid: Vec<f32> = (0..len)
            .map(|i| {
                let (mut l, mut r) = (0.0f32, 0.0f32);
                for s in &stems {
                    l += s.frames[i][0];
                    r += s.frames[i][1];
                }
                pk = pk.max(l.abs()).max(r.abs());
                (l + r) * 0.5
            })
            .collect();
        let loud = loudness(&mid, SR, 0.4);
        let gain = db_to_gain(level_db - loud).min(0.89 / pk.max(1e-6));
        for stem in &mut stems {
            for f in stem.frames.iter_mut().chain(stem.first_pass_tail.iter_mut()) {
                f[0] *= gain;
                f[1] *= gain;
            }
            // Safety for stems that are louder alone than inside the mix.
            let mut flat: Vec<f32> = stem.frames.iter().flatten().copied().collect();
            soft_limit(&mut flat, 0.85, 0.98);
            for (f, pair) in stem.frames.iter_mut().zip(flat.chunks_exact(2)) {
                *f = [pair[0], pair[1]];
            }
        }

        MusicData {
            track,
            bpm: song.bpm,
            bars: song.bars,
            bar_frames: song.bar_frames(),
            loop_start,
            intensity_floor,
            stems,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Instruments
// ---------------------------------------------------------------------------------------------

fn kick(r: &mut Rng, tight: bool) -> Vec<f32> {
    let (decay, len) = if tight { (0.08, 0.35) } else { (0.16, 0.6) };
    let body = thump(len, 160.0, 48.0, 0.022, decay);
    let beater = click(r, 0.0015);
    let mut b = layer(&[(&body, 1.0, 0.0), (&beater, 0.5, 0.0)]);
    drive(&mut b, 1.8);
    b
}

fn snare(r: &mut Rng, big: bool) -> Vec<f32> {
    let body = thump(0.2, 260.0, 185.0, 0.01, 0.045);
    let rattle = noise(r, 0.35, Pass::Band, 5000.0, 2500.0, 0.7, 0.0005, if big { 0.12 } else { 0.07 });
    let crack = click(r, 0.002);
    let mut b = layer(&[(&body, 0.8, 0.0), (&rattle, 1.0, 0.0), (&crack, 0.5, 0.0)]);
    drive(&mut b, 2.0);
    if big {
        super::dsp::reverb(&mut b, 0.8, 0.4, 0.35, 1.2);
    }
    b
}

/// 808-style metallic hat: six detuned squares through a band-pass, plus noise.
fn hat(r: &mut Rng, decay: f32) -> Vec<f32> {
    let mut oscs = [263.0f32, 400.0, 421.0, 474.0, 587.0, 845.0].map(|f| (Osc::new(Wave::Square), f));
    let mut n = Rng::new(r.next_u64());
    let (mut bp, mut hp) = (Svf::default(), Svf::default());
    super::dsp::render(decay * 6.0 + 0.01, |t, _| {
        let x = oscs.iter_mut().map(|(o, f)| o.next(*f)).sum::<f32>() / 6.0 + n.bipolar() * 0.4;
        let y = bp.process(x, 10_000.0, 1.0, SR).band;
        hp.highpass(y, 7000.0, 0.7) * perc(t, 0.0005, decay)
    })
}

fn crash(r: &mut Rng) -> Vec<f32> {
    let shimmer = hat(r, 0.6);
    let wash = noise(r, 2.0, Pass::High, 4000.0, 3000.0, 0.6, 0.002, 0.5);
    layer(&[(&shimmer, 1.0, 0.0), (&wash, 0.5, 0.0)])
}

/// Industrial anvil hit.
fn clank(r: &mut Rng) -> Vec<f32> {
    let ring = metal(0.4, 520.0, &[(1.0, 1.0, 0.09), (2.41, 0.7, 0.06), (3.73, 0.5, 0.04), (5.6, 0.3, 0.03)]);
    let hit = click(r, 0.002);
    let mut b = layer(&[(&ring, 0.8, 0.0), (&hit, 0.6, 0.0)]);
    super::dsp::crush(&mut b, 8, 2.0);
    b
}

fn tom(r: &mut Rng, freq: f32) -> Vec<f32> {
    let body = thump(0.7, freq * 1.8, freq, 0.04, 0.2);
    let skin = noise(r, 0.2, Pass::Low, 1200.0, 400.0, 0.8, 0.001, 0.04);
    layer(&[(&body, 1.0, 0.0), (&skin, 0.4, 0.0)])
}

/// Growling 16th-note metal bass: saw + sub square through an enveloped low-pass, saturated.
fn bass_growl(freq: f32, secs: f32, vel: f32) -> Vec<f32> {
    let (mut saw, mut sq) = (Osc::new(Wave::Saw), Osc::new(Wave::Square).phase(0.25));
    let mut lp = Svf::default();
    super::dsp::render(secs + 0.05, |t, _| {
        let x = saw.next(freq) * 0.7 + sq.next(freq * 0.5) * 0.5;
        let fc = 180.0 + 1400.0 * vel * perc(t, 0.002, 0.07);
        soft_clip(lp.lowpass(x, fc, 1.3) * 2.2) * adsr(t, 0.002, 0.08, 0.8, 0.04, secs)
    })
}

/// Deep sine sub with a little harmonic grit (audible on small speakers).
fn bass_sub(freq: f32, secs: f32) -> Vec<f32> {
    let (mut s, mut tri) = (Osc::new(Wave::Sine), Osc::new(Wave::Triangle));
    super::dsp::render(secs + 0.3, |t, _| {
        let x = s.next(freq) + tri.next(freq * 2.0) * 0.25;
        soft_clip(x * 1.6) * adsr(t, 0.005, 0.3, 0.6, 0.3, secs)
    })
}

/// Detuned-saw "reese" bass for drum'n'bass.
fn bass_reese(freq: f32, secs: f32, vel: f32) -> Vec<f32> {
    let (mut a, mut b, mut sub) = (Osc::new(Wave::Saw), Osc::new(Wave::Saw).phase(0.5), Osc::new(Wave::Sine));
    let mut lp = Svf::default();
    super::dsp::render(secs + 0.06, |t, _| {
        let x = a.next(freq * cents(-14.0)) + b.next(freq * cents(14.0));
        let fc = 500.0 + 900.0 * vel * perc(t, 0.003, 0.15);
        let y = lp.lowpass(x * 0.5, fc, 1.1) + sub.next(freq * 0.5) * 0.6;
        soft_clip(y * 1.8) * adsr(t, 0.003, 0.1, 0.85, 0.05, secs)
    })
}

/// Distorted power chord (root, fifth, octave) from a stack of detuned saws. Palm-muted notes
/// are darker and short.
fn guitar(r: &mut Rng, root: f32, secs: f32, muted: bool, gain_drive: f32) -> Vec<f32> {
    let mut oscs = Vec::new();
    for ratio in [1.0f32, 1.4983, 2.0] {
        for c in [-7.0f32, 6.0] {
            oscs.push((Osc::new(Wave::Saw).phase(r.f32()), root * ratio * cents(c + r.range(-2.0, 2.0))));
        }
    }
    let (mut hp, mut pre, mut cab, mut cab2) = (Svf::default(), Svf::default(), Svf::default(), Svf::default());
    let mut scoop = Biquad::peaking(750.0, 0.8, -5.0, SR);
    let pre_fc = if muted { 1000.0 } else { 4000.0 };
    let len = secs + if muted { 0.03 } else { 0.15 };
    super::dsp::render(len, |t, _| {
        let x = oscs.iter_mut().map(|(o, f)| o.next(*f)).sum::<f32>() / 6.0;
        let x = pre.lowpass(hp.highpass(x, 90.0, 0.7), pre_fc, 0.7);
        let env = if muted {
            perc(t, 0.002, 0.07)
        } else {
            adsr(t, 0.003, 0.3, 0.75, 0.12, secs)
        };
        let d = soft_clip(x * gain_drive) * env;
        scoop.process(cab2.lowpass(cab.lowpass(d, 3400.0, 0.8), 5500.0, 0.7))
    })
}

#[derive(Clone, Copy)]
enum LeadStyle {
    /// Square + saw with delayed vibrato.
    Square,
    /// Aggressive detuned saws.
    Saw,
    /// Glassy FM bell.
    Bell,
    /// Short plucked square for arpeggios.
    Pluck,
}

fn lead(freq: f32, secs: f32, style: LeadStyle) -> Vec<f32> {
    match style {
        LeadStyle::Square | LeadStyle::Saw => {
            let (mut a, mut b, mut vib) = (Osc::new(Wave::Square), Osc::new(Wave::Saw).phase(0.3), Osc::new(Wave::Sine));
            let mut lp = Svf::default();
            let saw = matches!(style, LeadStyle::Saw);
            super::dsp::render(secs + 0.08, |t, _| {
                let v = 1.0 + vib.next(5.5) * 0.006 * smoothstep(0.12, 0.3, t);
                let x = if saw {
                    a.next(freq * v * 0.5) * 0.3 + b.next(freq * v)
                } else {
                    a.next(freq * v) + b.next(freq * v * 1.003) * 0.5
                };
                let y = lp.lowpass(x * 0.5, if saw { 4500.0 } else { 3800.0 }, 1.1);
                soft_clip(y * if saw { 2.5 } else { 1.5 }) * adsr(t, 0.004, 0.1, 0.75, 0.06, secs)
            })
        }
        LeadStyle::Bell => fm(secs + 1.2, freq, freq, 3.5, 2.5, 0.3, 0.002, 0.6),
        LeadStyle::Pluck => {
            let mut sq = Osc::new(Wave::Square);
            let mut lp = Svf::default();
            super::dsp::render(secs + 0.2, |t, _| {
                lp.lowpass(sq.next(freq), 300.0 + 4000.0 * perc(t, 0.001, 0.05), 1.0) * perc(t, 0.001, 0.14)
            })
        }
    }
}

/// Sustained chord from detuned saws per channel (wide stereo) plus a sine sub, through a
/// slowly breathing low-pass.
fn pad(notes: &[f32], secs: f32, cutoff: f32, attack: f32, release: f32, seed: u64) -> [Vec<f32>; 2] {
    let mut r = Rng::new(seed);
    let mut out = [Vec::new(), Vec::new()];
    for (ch, buf) in out.iter_mut().enumerate() {
        let spread = 5.0 + 3.0 * ch as f32;
        let mut oscs: Vec<(Osc, f32)> = notes
            .iter()
            .flat_map(|&n| [-1.0f32, 1.0].map(|d| midi_hz(n) * cents(d * spread)))
            .map(|f| (Osc::new(Wave::Saw).phase(r.f32()), f))
            .collect();
        let norm = 1.0 / (oscs.len() as f32).sqrt();
        let mut sub = Osc::new(Wave::Sine);
        let sub_f = midi_hz(notes[0] - 12.0);
        let mut lp = Svf::default();
        let lfo_phase = ch as f32 * 1.7 + r.f32();
        let mut fc = cutoff;
        *buf = super::dsp::render(secs + release, |t, i| {
            let x = oscs.iter_mut().map(|(o, f)| o.next(*f)).sum::<f32>() * norm;
            // The slow filter LFO runs at control rate.
            if i % 64 == 0 {
                fc = cutoff * (1.0 + 0.35 * (t * 0.3 * TAU + lfo_phase).sin());
            }
            let y = lp.lowpass(x, fc, 0.9) + sub.next(sub_f) * 0.4;
            y * adsr(t, attack, 0.0, 1.0, release, secs)
        });
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Tracks
// ---------------------------------------------------------------------------------------------

/// `(step, length in steps, MIDI note)`.
type Phrase = &'static [(usize, usize, f32)];

fn place_melody(a: &mut Arrangement, phrases: &[Phrase], style: LeadStyle, gain: f32, pan: f32) {
    for (bar, phrase) in phrases.iter().enumerate() {
        for &(step, len, note) in *phrase {
            let n = lead(midi_hz(note), a.song.secs(len as f32), style);
            a.add(StemKind::Lead, bar, step, &n, gain, pan);
        }
    }
}

/// Double-tracked guitar: two separate takes hard-ish left and right, the right one a few ms late.
fn place_guitar(a: &mut Arrangement, r: &mut Rng, bar: usize, root_midi: f32, pattern: &str, gain: f32, gain_drive: f32) {
    let hs = hits(pattern);
    for (i, &(step, vel)) in hs.iter().enumerate() {
        let next = hs.get(i + 1).map_or(16, |h| h.0);
        let secs = a.song.secs((next - step) as f32);
        let muted = vel < 1.0;
        for (pan, delay) in [(-0.75f32, 0usize), (0.75, samples(0.004))] {
            let take = guitar(r, midi_hz(root_midi), secs, muted, gain_drive);
            let pos = a.song.at(bar, step) + delay;
            a.add_at(StemKind::Guitar, pos, &take, gain * vel.max(0.8), pan);
        }
    }
}

fn menu() -> MusicData {
    let mut a = Arrangement::new(Song::new(90.0, 8, 0));
    let mut r = Rng::new(0x3E2E);
    // A minor, ominous: Am Am F F Dm Dm E E (E major: harmonic-minor dread).
    let roots: [f32; 8] = [0.0, 0.0, -4.0, -4.0, 5.0, 5.0, 7.0, 7.0];
    let thirds: [f32; 8] = [3.0, 3.0, 4.0, 4.0, 3.0, 3.0, 4.0, 4.0];
    let bar = a.bar_secs();

    for b in 0..8 {
        let n = 45.0 + roots[b];
        let chord = pad(&[n, n + 7.0, n + 12.0, n + 12.0 + thirds[b]], bar, 700.0, 0.8, 1.2, r.next_u64());
        let pos = a.song.at(b, 0);
        a.add_stereo_at(StemKind::Pad, pos, &chord, 0.5);

        // Heartbeat sub bass.
        let f = midi_hz(33.0 + roots[b]);
        for (step, vel) in hits("X..x............") {
            let note = bass_sub(f, a.song.secs(2.0));
            a.add(StemKind::Bass, b, step, &note, 0.8 * vel, 0.0);
        }
    }

    let low_tom = tom(&mut r, 70.0);
    let anvil = clank(&mut r);
    let hat_c = hat(&mut r, 0.04);
    let kick_big = kick(&mut r, false);
    let snare_big = snare(&mut r, true);
    let cymbal = crash(&mut r);
    for b in 0..8 {
        a.pattern(StemKind::DrumsLight, b, "X.........o.....", &low_tom, 0.9, 0.0);
        a.pattern(StemKind::DrumsLight, b, "............x...", &anvil, 0.4, 0.35);
        a.pattern(StemKind::DrumsLight, b, "o.o.o.o.o.o.o.o.", &hat_c, 0.35, -0.3);
        // Half-time heavy beat.
        let k = if b == 7 { "X.....X...X.X..." } else { "X.........X....." };
        a.pattern(StemKind::DrumsFull, b, k, &kick_big, 1.0, 0.0);
        a.pattern(StemKind::DrumsFull, b, "........X.......", &snare_big, 0.8, 0.0);
        if b % 4 == 0 {
            a.pattern(StemKind::DrumsFull, b, "X...............", &cymbal, 0.4, 0.2);
        }
    }
    a.effect(StemKind::DrumsLight, |t| t.reverb(0.85, 0.4, 0.3));

    // Slow droning guitar swells, one chord per two bars.
    for b in (0..8).step_by(2) {
        let take_secs = a.bar_secs() * 2.0;
        for (pan, delay) in [(-0.6f32, 0usize), (0.6, samples(0.006))] {
            let mut take = guitar(&mut r, midi_hz(40.0 + roots[b] + 5.0), take_secs, false, 7.0);
            for (i, s) in take.iter_mut().enumerate() {
                *s *= smoothstep(0.0, 1.2, i as f32 / SR);
            }
            let pos = a.song.at(b, 0) + delay;
            a.add_at(StemKind::Guitar, pos, &take, 0.45, pan);
        }
    }

    let melody: [Phrase; 8] = [
        &[(0, 4, 81.0), (8, 4, 84.0)],
        &[(0, 6, 83.0), (8, 8, 79.0)],
        &[(0, 4, 77.0), (8, 4, 81.0)],
        &[(0, 8, 84.0)],
        &[(0, 4, 86.0), (8, 4, 81.0)],
        &[(0, 8, 77.0)],
        &[(0, 4, 83.0), (4, 4, 80.0), (8, 8, 76.0)],
        &[(0, 12, 76.0)],
    ];
    place_melody(&mut a, &melody, LeadStyle::Bell, 0.5, 0.15);
    let dotted_eighth = a.song.step * 3;
    a.effect(StemKind::Lead, |t| t.echo(dotted_eighth, 0.45, 0.4));
    a.effect(StemKind::Pad, |t| t.reverb(0.85, 0.5, 0.35));
    a.finish(MusicTrack::Menu, 0.35, -23.0)
}

fn campaign() -> MusicData {
    let mut a = Arrangement::new(Song::new(150.0, 8, 0));
    let mut r = Rng::new(0xCA4A_1601);
    // E phrygian: E E F E C D E Bb (the last a tritone away for tension).
    let roots: [f32; 8] = [0.0, 0.0, 1.0, 0.0, -4.0, -2.0, 0.0, -6.0];
    let thirds: [f32; 8] = [3.0, 3.0, 4.0, 3.0, 4.0, 3.0, 3.0, 4.0];
    let bar = a.bar_secs();

    for b in 0..8 {
        let n = 52.0 + roots[b];
        let chord = pad(&[n, n + 7.0, n + 12.0, n + 12.0 + thirds[b]], bar, 1000.0, 0.2, 0.5, r.next_u64());
        let pos = a.song.at(b, 0);
        a.add_stereo_at(StemKind::Pad, pos, &chord, 0.6);

        // Galloping bass.
        let f = midi_hz(40.0 + roots[b]);
        let pat = match b {
            7 => "X.X.X.X.XxXxXxXx",
            _ if b % 2 == 1 => "X.xxX.xxX.xxXxXx",
            _ => "X.xxX.xxX.xxX.xx",
        };
        for (step, vel) in hits(pat) {
            let note = bass_growl(f, a.song.secs(1.6), vel);
            a.add(StemKind::Bass, b, step, &note, 0.55 * vel, 0.0);
        }

        // Palm-muted gallop chugs with open accents; sustained chords on the tritone bar.
        let gpat = match b {
            3 => "X.xxX.xxX..xX.X.",
            7 => "X.......X.......",
            _ => "X.xxX.xxX.xxX.xx",
        };
        place_guitar(&mut a, &mut r, b, 40.0 + roots[b], gpat, 0.45, 9.0);
    }

    let kick_t = kick(&mut r, true);
    let snare_t = snare(&mut r, false);
    let hat_c = hat(&mut r, 0.03);
    let hat_o = hat(&mut r, 0.18);
    let cymbal = crash(&mut r);
    let anvil = clank(&mut r);
    for b in 0..8 {
        a.pattern(StemKind::DrumsLight, b, "X.......X.......", &kick_t, 0.9, 0.0);
        a.pattern(StemKind::DrumsLight, b, "x.x.x.x.x.x.x.x.", &hat_c, 0.3, 0.3);
        a.pattern(StemKind::DrumsLight, b, "....o.......o...", &snare_t, 0.6, 0.0);

        let (k, s) = if b == 7 {
            ("X.....X.X.X.X...", "....X..XX.XXXXXX")
        } else {
            ("......X..X...X..", "....X..o.o..X..o")
        };
        a.pattern(StemKind::DrumsFull, b, k, &kick_t, 0.9, 0.0);
        a.pattern(StemKind::DrumsFull, b, s, &snare_t, 0.85, 0.0);
        a.pattern(StemKind::DrumsFull, b, ".o.o.o.o.o.o.o.o", &hat_c, 0.3, 0.3);
        a.pattern(StemKind::DrumsFull, b, "...x.......x....", &anvil, 0.35, -0.4);
        a.pattern(StemKind::DrumsFull, b, "..............x.", &hat_o, 0.35, 0.3);
        if b % 4 == 0 {
            a.pattern(StemKind::DrumsFull, b, "X...............", &cymbal, 0.45, -0.2);
        }
    }

    let melody: [Phrase; 8] = [
        &[(0, 2, 76.0), (2, 2, 77.0), (4, 2, 79.0), (6, 1, 77.0), (7, 1, 76.0), (8, 4, 71.0), (12, 2, 72.0), (14, 2, 71.0)],
        &[(0, 3, 76.0), (3, 1, 74.0), (4, 2, 76.0), (6, 2, 79.0), (8, 2, 77.0), (10, 2, 76.0), (12, 4, 74.0)],
        &[(0, 3, 77.0), (3, 1, 79.0), (4, 2, 81.0), (6, 2, 79.0), (8, 4, 77.0), (12, 4, 72.0)],
        &[(0, 6, 76.0), (6, 2, 71.0), (8, 2, 76.0), (10, 2, 77.0), (12, 4, 76.0)],
        &[(0, 2, 72.0), (2, 2, 76.0), (4, 4, 79.0), (8, 2, 76.0), (10, 2, 79.0), (12, 4, 84.0)],
        &[(0, 2, 74.0), (2, 2, 77.0), (4, 4, 81.0), (8, 2, 77.0), (10, 2, 81.0), (12, 4, 86.0)],
        &[(0, 4, 88.0), (4, 2, 86.0), (6, 2, 84.0), (8, 4, 83.0), (12, 4, 79.0)],
        &[(0, 8, 82.0), (8, 4, 81.0), (12, 4, 77.0)],
    ];
    place_melody(&mut a, &melody, LeadStyle::Square, 0.5, 0.1);
    let dotted_eighth = a.song.step * 3;
    a.effect(StemKind::Lead, |t| t.echo(dotted_eighth, 0.3, 0.25));
    a.effect(StemKind::Pad, |t| t.reverb(0.7, 0.5, 0.3));
    a.finish(MusicTrack::Campaign, 0.0, -21.0)
}

fn arena() -> MusicData {
    let mut a = Arrangement::new(Song::new(175.0, 8, 0));
    let mut r = Rng::new(0xA2E7A);
    // D minor: Dm Dm Bb C Dm Dm Gm A.
    let roots: [f32; 8] = [0.0, 0.0, -4.0, -2.0, 0.0, 0.0, 5.0, 7.0];
    let thirds: [f32; 8] = [3.0, 3.0, 4.0, 4.0, 3.0, 3.0, 3.0, 4.0];
    let bar = a.bar_secs();

    for b in 0..8 {
        let n = 50.0 + roots[b];
        let chord = pad(&[n, n + 7.0, n + 12.0, n + 12.0 + thirds[b]], bar, 1100.0, 0.15, 0.4, r.next_u64());
        let pos = a.song.at(b, 0);
        a.add_stereo_at(StemKind::Pad, pos, &chord, 0.55);

        let f = midi_hz(38.0 + roots[b]);
        let pat = "X..X..X...X.X...";
        let hs = hits(pat);
        for (i, &(step, vel)) in hs.iter().enumerate() {
            let next = hs.get(i + 1).map_or(16, |h| h.0);
            let note = bass_reese(f, a.song.secs((next - step) as f32 * 0.9), vel);
            a.add(StemKind::Bass, b, step, &note, 0.5, 0.0);
        }

        let gpat = if b == 7 { "X...X...X.X.X.X." } else { "X..x..x.X..x.x.x" };
        place_guitar(&mut a, &mut r, b, 38.0 + roots[b] + 12.0, gpat, 0.42, 10.0);
    }

    let kick_t = kick(&mut r, true);
    let snare_t = snare(&mut r, false);
    let hat_c = hat(&mut r, 0.025);
    let ride = hat(&mut r, 0.12);
    let cymbal = crash(&mut r);
    let anvil = clank(&mut r);
    for b in 0..8 {
        a.pattern(StemKind::DrumsLight, b, "X.........X.....", &kick_t, 0.9, 0.0);
        a.pattern(StemKind::DrumsLight, b, "x.x.x.x.x.x.x.x.", &hat_c, 0.3, 0.25);
        a.pattern(StemKind::DrumsLight, b, "....o.......o...", &snare_t, 0.6, 0.0);

        let (k, s) = if b == 7 {
            ("X.X...X...X.X...", "....X.XoX.XXXXXX")
        } else if b % 2 == 1 {
            ("X.X.......XX....", "....X..o.o..X..o")
        } else {
            ("..X.......X.....", "....X..o....X.o.")
        };
        a.pattern(StemKind::DrumsFull, b, k, &kick_t, 0.85, 0.0);
        a.pattern(StemKind::DrumsFull, b, s, &snare_t, 0.85, 0.0);
        a.pattern(StemKind::DrumsFull, b, "x.x.x.x.x.x.x.x.", &ride, 0.25, -0.3);
        a.pattern(StemKind::DrumsFull, b, ".......x.......x", &anvil, 0.3, 0.4);
        if b % 4 == 0 {
            a.pattern(StemKind::DrumsFull, b, "X...............", &cymbal, 0.45, 0.2);
        }
    }

    let melody: [Phrase; 8] = [
        &[(0, 2, 74.0), (2, 2, 77.0), (4, 2, 81.0), (6, 2, 77.0), (8, 3, 86.0), (11, 1, 84.0), (12, 4, 81.0)],
        &[(0, 2, 79.0), (2, 2, 77.0), (4, 4, 76.0), (8, 2, 77.0), (10, 2, 76.0), (12, 4, 74.0)],
        &[(0, 3, 74.0), (3, 1, 77.0), (4, 4, 82.0), (8, 2, 81.0), (10, 2, 79.0), (12, 4, 77.0)],
        &[(0, 3, 76.0), (3, 1, 79.0), (4, 4, 84.0), (8, 2, 82.0), (10, 2, 81.0), (12, 4, 79.0)],
        &[(0, 2, 86.0), (2, 2, 84.0), (4, 2, 81.0), (6, 2, 77.0), (8, 4, 81.0), (12, 4, 74.0)],
        &[(0, 2, 74.0), (2, 2, 76.0), (4, 2, 77.0), (6, 2, 79.0), (8, 4, 81.0), (12, 4, 77.0)],
        &[(0, 4, 79.0), (4, 2, 82.0), (6, 2, 86.0), (8, 4, 91.0), (12, 4, 86.0)],
        &[(0, 4, 85.0), (4, 4, 81.0), (8, 4, 76.0), (12, 4, 73.0)],
    ];
    place_melody(&mut a, &melody, LeadStyle::Saw, 0.5, -0.1);
    let dotted_eighth = a.song.step * 3;
    a.effect(StemKind::Lead, |t| t.echo(dotted_eighth, 0.3, 0.22));
    a.effect(StemKind::Pad, |t| t.reverb(0.6, 0.5, 0.25));
    a.finish(MusicTrack::Arena, 0.0, -21.0)
}

fn results() -> MusicData {
    // One sting bar, then a 4-bar loop.
    let mut a = Arrangement::new(Song::new(126.0, 5, 1));
    let mut r = Rng::new(0x2E5017);
    // Sting: Am(add9). Loop: Am F C G.
    let roots: [f32; 5] = [0.0, 0.0, -4.0, 3.0, -2.0];
    let thirds: [f32; 5] = [3.0, 3.0, 4.0, 4.0, 4.0];
    let bar = a.bar_secs();

    for b in 0..5 {
        let n = 57.0 + roots[b];
        let mut notes = vec![n - 12.0, n, n + thirds[b], n + 7.0];
        if b == 0 {
            notes.push(n + 14.0);
        }
        let chord = pad(&notes, bar, 1600.0, if b == 0 { 0.005 } else { 0.1 }, 0.5, r.next_u64());
        let pos = a.song.at(b, 0);
        a.add_stereo_at(StemKind::Pad, pos, &chord, 0.35);

        let f = midi_hz(45.0 + roots[b]);
        let pat = if b == 0 { "X..............." } else { "X.x.X.x.X.x.X.x." };
        for (step, vel) in hits(pat) {
            let note = bass_growl(f, a.song.secs(if b == 0 { 12.0 } else { 1.8 }), vel * 0.6);
            a.add(StemKind::Bass, b, step, &note, 0.5 * vel, 0.0);
        }

        let gpat = if b == 0 { "X..............." } else { "X.......X......." };
        place_guitar(&mut a, &mut r, b, 45.0 + roots[b], gpat, 0.22, 5.0);

        // Arpeggio over the chord tones.
        if b > 0 {
            let tones = [n + 12.0, n + 12.0 + thirds[b], n + 19.0, n + 24.0];
            for step in 0..16 {
                let note = tones[[0, 1, 2, 3, 2, 1][step % 6]];
                let pluck = lead(midi_hz(note), a.song.secs(1.0), LeadStyle::Pluck);
                a.add(StemKind::Lead, b, step, &pluck, if step % 4 == 0 { 0.3 } else { 0.2 }, 0.2);
            }
        }
    }

    let kick_t = kick(&mut r, false);
    let snare_t = snare(&mut r, false);
    let hat_c = hat(&mut r, 0.035);
    let cymbal = crash(&mut r);
    let low_tom = tom(&mut r, 90.0);
    // Sting: kick + crash, tom roll swelling into the loop.
    a.pattern(StemKind::DrumsLight, 0, "X...............", &kick_t, 1.0, 0.0);
    a.pattern(StemKind::DrumsLight, 0, "X...............", &cymbal, 0.6, 0.0);
    a.pattern(StemKind::DrumsFull, 0, "........ooxxxXXX", &low_tom, 0.7, -0.2);
    for b in 1..5 {
        a.pattern(StemKind::DrumsLight, b, "X.......X.......", &kick_t, 0.8, 0.0);
        a.pattern(StemKind::DrumsLight, b, "x.x.x.x.x.x.x.x.", &hat_c, 0.25, 0.3);
        a.pattern(StemKind::DrumsFull, b, "....X..o....X...", &snare_t, 0.7, 0.0);
        a.pattern(StemKind::DrumsFull, b, "..........X.....", &kick_t, 0.6, 0.0);
    }
    let dotted_eighth = a.song.step * 3;
    a.effect(StemKind::Lead, |t| t.echo(dotted_eighth, 0.3, 0.25));
    a.effect(StemKind::Pad, |t| t.reverb(0.7, 0.4, 0.3));
    a.finish(MusicTrack::Results, 1.0, -22.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::dsp::peak;

    #[test]
    fn stem_gains_follow_intensity() {
        let calm = stem_gains(0.0);
        assert_eq!(calm[StemKind::Pad as usize], 1.0);
        assert!(calm[1..].iter().all(|&g| g == 0.0));
        let full = stem_gains(1.0);
        assert!(full.iter().all(|&g| g == 1.0));
        let mid = stem_gains(0.4);
        assert_eq!(mid[StemKind::Bass as usize], 1.0);
        assert_eq!(mid[StemKind::DrumsFull as usize], 0.0);
        assert_eq!(mid[StemKind::Lead as usize], 0.0);
    }

    #[test]
    fn tracks_loop_on_exact_bars_and_are_clean() {
        for track in MusicTrack::PLAYABLE {
            let data = render_track(track).expect("track renders");
            assert_eq!(data.stems.len(), STEM_COUNT);
            let bar = (SR * 60.0 / data.bpm * 4.0).round() as usize;
            assert_eq!(data.bar_frames, bar);
            assert_eq!(data.len(), bar * data.bars, "{track:?} not a whole number of bars");
            assert_eq!(data.loop_start % bar, 0);
            for stem in &data.stems {
                assert_eq!(stem.frames.len(), data.len(), "{track:?}/{}", stem.kind.name());
                let flat: Vec<f32> = stem.frames.iter().flatten().copied().collect();
                assert!(flat.iter().all(|s| s.is_finite()), "{track:?}/{} NaN", stem.kind.name());
                let p = peak(&flat);
                assert!(p <= 1.0 && p > 0.01, "{track:?}/{} peak {p}", stem.kind.name());
            }
            let mix: Vec<f32> = data.mixdown(&[1.0; STEM_COUNT]).into_iter().flatten().collect();
            assert!(peak(&mix) <= 1.0);
        }
    }

    #[test]
    fn loop_seams_are_smooth() {
        for track in MusicTrack::PLAYABLE {
            let data = render_track(track).expect("track renders");
            let len = data.len();
            let out = data.playback(&[1.0; STEM_COUNT], len + 2000);
            let mut diffs: Vec<f32> = out.windows(2).map(|w| (w[1][0] - w[0][0]).abs()).collect();
            let seam = diffs[len - 1];
            diffs.sort_unstable_by(f32::total_cmp);
            let p999 = diffs[diffs.len() * 999 / 1000];
            assert!(seam <= p999, "{track:?}: jump {seam} at the loop seam (99.9th pct {p999})");
        }
    }

    #[test]
    fn music_is_deterministic() {
        let a = render_track(MusicTrack::Results).expect("results");
        let b = render_track(MusicTrack::Results).expect("results");
        for (x, y) in a.stems.iter().zip(&b.stems) {
            assert!(x.frames == y.frames);
        }
    }

    #[test]
    fn none_has_no_music() {
        assert!(render_track(MusicTrack::None).is_none());
    }
}
