//! A small DSP toolkit used to synthesise every sound at startup.
//!
//! Plain functions and tiny structs working on mono `f32` buffers. Generators take the sample
//! rate explicitly where it matters (sound effects run at [`SAMPLE_RATE`], music at
//! [`crate::audio::music::MUSIC_SAMPLE_RATE`]). Everything is deterministic: the only source of
//! randomness is the seeded [`Rng`].
//!
//! Waveforms are intentionally a little dirty (naive triangle, polyBLEP saw/square, sample & hold
//! noise) and [`crush`] gives the reduced bit-depth / sample-rate crunch of a 90s console.

use std::f32::consts::{PI, TAU};

/// Sample rate of all sound effects.
pub const SAMPLE_RATE: u32 = 44_100;
/// [`SAMPLE_RATE`] as `f32`.
pub const SR: f32 = SAMPLE_RATE as f32;

/// Number of sound-effect samples in `secs` seconds.
pub fn samples(secs: f32) -> usize {
    (secs.max(0.0) * SR).round() as usize
}

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn gain_to_db(gain: f32) -> f32 {
    20.0 * gain.max(1e-6).log10()
}

/// Frequency of a (fractional) MIDI note number. A4 = 69 = 440 Hz.
pub fn midi_hz(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

/// Hermite smoothstep of `x` between `e0` and `e1`.
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ---------------------------------------------------------------------------------------------
// Random numbers
// ---------------------------------------------------------------------------------------------

/// xorshift64* generator. Cheap, deterministic and good enough for noise.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // splitmix64 scramble so neighbouring seeds produce unrelated streams.
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        Self(z.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0, 1)`.
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }

    /// Uniform in `[-1, 1)`.
    pub fn bipolar(&mut self) -> f32 {
        self.f32() * 2.0 - 1.0
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }
}

// ---------------------------------------------------------------------------------------------
// Oscillators
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Wave {
    Sine,
    Triangle,
    Saw,
    Square,
    /// Sample & hold noise: a new random value every cycle, so the "frequency" sets how bright
    /// and grainy it is (at or above the sample rate it is plain white noise).
    Noise,
}

/// Phase-accumulator oscillator. The frequency is passed per sample, so sweeps, vibrato and FM
/// are just a matter of computing the frequency.
#[derive(Clone, Debug)]
pub struct Osc {
    pub wave: Wave,
    phase: f32,
    sample_rate: f32,
    held: f32,
    rng: Rng,
}

impl Osc {
    pub fn new(wave: Wave) -> Self {
        Self::with_rate(wave, SR)
    }

    pub fn with_rate(wave: Wave, sample_rate: f32) -> Self {
        Self {
            wave,
            phase: 0.0,
            sample_rate,
            held: 0.0,
            rng: Rng::new(0x05C1),
        }
    }

    /// Starting phase in cycles (`0..1`).
    pub fn phase(mut self, phase: f32) -> Self {
        self.phase = phase.rem_euclid(1.0);
        self
    }

    /// Seed for [`Wave::Noise`].
    pub fn seed(mut self, seed: u64) -> Self {
        self.rng = Rng::new(seed);
        self
    }

    /// Advances one sample at `freq` Hz and returns a value in roughly `[-1, 1]`.
    pub fn next(&mut self, freq: f32) -> f32 {
        let dt = (freq / self.sample_rate).clamp(0.0, 1.0);
        let p = self.phase;
        let out = match self.wave {
            Wave::Sine => (p * TAU).sin(),
            Wave::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
            Wave::Saw => 2.0 * p - 1.0 - poly_blep(p, dt),
            Wave::Square => {
                let naive = if p < 0.5 { 1.0 } else { -1.0 };
                naive + poly_blep(p, dt) - poly_blep((p + 0.5).fract(), dt)
            }
            Wave::Noise => self.held,
        };
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            if self.wave == Wave::Noise {
                self.held = self.rng.bipolar();
            }
        }
        out
    }

    /// Phase-modulated sine (for FM): `pm` is added to the phase in cycles.
    pub fn next_pm(&mut self, freq: f32, pm: f32) -> f32 {
        let out = ((self.phase + pm) * TAU).sin();
        self.phase = (self.phase + freq / self.sample_rate).fract();
        out
    }
}

/// Polynomial band-limited step correction for saw/square discontinuities.
fn poly_blep(t: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        0.0
    } else if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------------------------
// Envelopes and sweeps (all functions of time in seconds)
// ---------------------------------------------------------------------------------------------

/// Linear attack to 1, then exponential decay with time constant `decay`.
pub fn perc(t: f32, attack: f32, decay: f32) -> f32 {
    if t < 0.0 {
        0.0
    } else if t < attack {
        t / attack
    } else {
        (-(t - attack) / decay.max(1e-4)).exp()
    }
}

/// Classic ADSR for a note whose gate is held for `gate` seconds.
pub fn adsr(t: f32, attack: f32, decay: f32, sustain: f32, release: f32, gate: f32) -> f32 {
    let held = |t: f32| {
        if t < attack {
            t / attack.max(1e-5)
        } else if t < attack + decay {
            1.0 - (1.0 - sustain) * (t - attack) / decay.max(1e-5)
        } else {
            sustain
        }
    };
    if t < 0.0 {
        0.0
    } else if t < gate {
        held(t)
    } else {
        let r = t - gate;
        if r >= release {
            0.0
        } else {
            held(gate) * (1.0 - r / release.max(1e-5))
        }
    }
}

/// Exponential sweep from `f0` to `f1` over `dur` seconds, holding `f1` afterwards.
pub fn sweep(t: f32, f0: f32, f1: f32, dur: f32) -> f32 {
    let x = (t / dur.max(1e-5)).clamp(0.0, 1.0);
    f0 * (f1 / f0).powf(x)
}

/// Exponential glide from `f0` towards `f1` with time constant `tau` (kick-drum style).
pub fn glide(t: f32, f0: f32, f1: f32, tau: f32) -> f32 {
    f1 + (f0 - f1) * (-t.max(0.0) / tau.max(1e-5)).exp()
}

/// Smooth 0 → 1 → 0 window (sin²) over `dur` seconds.
pub fn bell(t: f32, dur: f32) -> f32 {
    if t <= 0.0 || t >= dur {
        0.0
    } else {
        (PI * t / dur).sin().powi(2)
    }
}

// ---------------------------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------------------------

/// One-pole low/high-pass, cheap enough to modulate per sample.
#[derive(Clone, Copy, Debug)]
pub struct OnePole {
    z: f32,
    /// Coefficient cache: recomputed only when the cutoff changes.
    cutoff: f32,
    a: f32,
}

impl Default for OnePole {
    fn default() -> Self {
        Self {
            z: 0.0,
            cutoff: f32::NAN,
            a: 0.0,
        }
    }
}

impl OnePole {
    pub fn lowpass(&mut self, x: f32, cutoff: f32, sample_rate: f32) -> f32 {
        if cutoff != self.cutoff {
            self.cutoff = cutoff;
            self.a = 1.0 - (-TAU * cutoff.max(1.0) / sample_rate).exp();
        }
        self.z += self.a * (x - self.z);
        self.z
    }

    pub fn highpass(&mut self, x: f32, cutoff: f32, sample_rate: f32) -> f32 {
        x - self.lowpass(x, cutoff, sample_rate)
    }
}

/// Topology-preserving state-variable filter (Simper). Stable under fast cutoff modulation, so
/// it is used for every swept filter.
#[derive(Clone, Copy, Debug)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    /// Coefficient cache keyed on (cutoff, q): static filters skip the per-sample `tan`.
    key: (f32, f32),
    coefs: (f32, f32, f32, f32),
}

#[derive(Clone, Copy, Debug)]
pub struct SvfOut {
    pub low: f32,
    /// Normalised band-pass (unity gain at the centre frequency).
    pub band: f32,
    pub high: f32,
}

impl Default for Svf {
    fn default() -> Self {
        Self {
            ic1: 0.0,
            ic2: 0.0,
            key: (f32::NAN, f32::NAN),
            coefs: (0.0, 0.0, 0.0, 0.0),
        }
    }
}

impl Svf {
    pub fn process(&mut self, x: f32, cutoff: f32, q: f32, sample_rate: f32) -> SvfOut {
        if (cutoff, q) != self.key {
            self.key = (cutoff, q);
            let g = (PI * cutoff.clamp(10.0, sample_rate * 0.48) / sample_rate).tan();
            let k = 1.0 / q.max(0.05);
            let a1 = 1.0 / (1.0 + g * (g + k));
            self.coefs = (k, a1, g * a1, g * g * a1);
        }
        let (k, a1, a2, a3) = self.coefs;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        SvfOut {
            low: v2,
            band: v1 * k,
            high: x - k * v1 - v2,
        }
    }

    pub fn lowpass(&mut self, x: f32, cutoff: f32, q: f32) -> f32 {
        self.process(x, cutoff, q, SR).low
    }

    pub fn bandpass(&mut self, x: f32, cutoff: f32, q: f32) -> f32 {
        self.process(x, cutoff, q, SR).band
    }

    pub fn highpass(&mut self, x: f32, cutoff: f32, q: f32) -> f32 {
        self.process(x, cutoff, q, SR).high
    }
}

/// RBJ-cookbook biquad with fixed coefficients (static EQ / tone shaping).
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn from_coefficients(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn omega(freq: f32, sample_rate: f32) -> (f32, f32) {
        let w = TAU * freq.clamp(10.0, sample_rate * 0.48) / sample_rate;
        (w.cos(), w.sin())
    }

    pub fn lowpass(freq: f32, q: f32, sample_rate: f32) -> Self {
        let (cos, sin) = Self::omega(freq, sample_rate);
        let alpha = sin / (2.0 * q);
        let b1 = 1.0 - cos;
        Self::from_coefficients(b1 / 2.0, b1, b1 / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    pub fn highpass(freq: f32, q: f32, sample_rate: f32) -> Self {
        let (cos, sin) = Self::omega(freq, sample_rate);
        let alpha = sin / (2.0 * q);
        let b1 = -(1.0 + cos);
        Self::from_coefficients(-b1 / 2.0, b1, -b1 / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    /// Peaking EQ: boosts/cuts `gain_db` around `freq`.
    pub fn peaking(freq: f32, q: f32, gain_db: f32, sample_rate: f32) -> Self {
        let (cos, sin) = Self::omega(freq, sample_rate);
        let a = 10f32.powf(gain_db / 40.0);
        let alpha = sin / (2.0 * q);
        Self::from_coefficients(
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
        )
    }

    /// High shelf (slope 1).
    pub fn high_shelf(freq: f32, gain_db: f32, sample_rate: f32) -> Self {
        let (cos, sin) = Self::omega(freq, sample_rate);
        let a = 10f32.powf(gain_db / 40.0);
        let alpha = sin / 2.0 * 2f32.sqrt();
        let sa = 2.0 * a.sqrt() * alpha;
        Self::from_coefficients(
            a * ((a + 1.0) + (a - 1.0) * cos + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - sa),
            (a + 1.0) - (a - 1.0) * cos + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - sa,
        )
    }

    /// Low shelf (slope 1).
    pub fn low_shelf(freq: f32, gain_db: f32, sample_rate: f32) -> Self {
        let (cos, sin) = Self::omega(freq, sample_rate);
        let a = 10f32.powf(gain_db / 40.0);
        let alpha = sin / 2.0 * 2f32.sqrt();
        let sa = 2.0 * a.sqrt() * alpha;
        Self::from_coefficients(
            a * ((a + 1.0) - (a - 1.0) * cos + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - sa),
            (a + 1.0) + (a - 1.0) * cos + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - sa,
        )
    }

    pub fn process(&mut self, x: f32) -> f32 {
        // Transposed direct form II.
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn run(mut self, buf: &mut [f32]) {
        for s in buf {
            *s = self.process(*s);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Waveshaping and lo-fi
// ---------------------------------------------------------------------------------------------

/// Smooth saturation (rational tanh approximation), output within `[-1, 1]`.
pub fn soft_clip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

/// Sine wavefolder: bright, metallic harmonics as `drive` increases.
pub fn fold(x: f32, drive: f32) -> f32 {
    (x * drive * PI * 0.5).sin()
}

/// Quantises `x` to a signed `bits`-bit grid.
pub fn quantize(x: f32, bits: u32) -> f32 {
    let steps = (1u32 << (bits.clamp(2, 24) - 1)) as f32;
    (x * steps).round() / steps
}

/// Saturates the whole buffer: `soft_clip(x * drive)`.
pub fn drive(buf: &mut [f32], drive: f32) {
    for s in buf {
        *s = soft_clip(*s * drive);
    }
}

/// Bit depth and sample-rate reduction: holds every input sample for `hold` samples (fractional
/// values allowed) and quantises to `bits`.
pub fn crush(buf: &mut [f32], bits: u32, hold: f32) {
    let hold = hold.max(1.0);
    let mut acc = hold;
    let mut held = 0.0;
    for s in buf {
        if acc >= hold {
            acc -= hold;
            held = quantize(*s, bits);
        }
        acc += 1.0;
        *s = held;
    }
}

/// Static one-pole low-pass over a buffer.
pub fn lowpass(buf: &mut [f32], cutoff: f32) {
    let mut f = OnePole::default();
    for s in buf {
        *s = f.lowpass(*s, cutoff, SR);
    }
}

/// Static one-pole high-pass over a buffer.
pub fn highpass(buf: &mut [f32], cutoff: f32) {
    let mut f = OnePole::default();
    for s in buf {
        *s = f.highpass(*s, cutoff, SR);
    }
}

// ---------------------------------------------------------------------------------------------
// Time-based effects
// ---------------------------------------------------------------------------------------------

/// Feedback delay ("echo") with a damped feedback path. Grows the buffer by `tail` seconds.
pub fn echo(buf: &mut Vec<f32>, delay: f32, feedback: f32, mix: f32, damp_hz: f32, tail: f32) {
    let d = samples(delay).max(1);
    buf.resize(buf.len() + samples(tail), 0.0);
    let mut line = vec![0.0f32; d];
    let mut damp = OnePole::default();
    for (i, s) in buf.iter_mut().enumerate() {
        let idx = i % d;
        let delayed = line[idx];
        line[idx] = *s + damp.lowpass(delayed, damp_hz, SR) * feedback;
        *s += delayed * mix;
    }
}

/// Small Schroeder/Freeverb-style reverb: 4 damped combs into 2 all-passes.
/// `size` in `0..1` sets the decay, `damp` in `0..1` darkens the tail, `mix` is the wet level.
/// Grows the buffer by `tail` seconds.
pub fn reverb(buf: &mut Vec<f32>, size: f32, damp: f32, mix: f32, tail: f32) {
    reverb_with_rate(buf, size, damp, mix, tail, SR, 0);
}

/// [`reverb`] at an arbitrary sample rate; `spread` offsets the delay lengths (stereo width).
pub fn reverb_with_rate(
    buf: &mut Vec<f32>,
    size: f32,
    damp: f32,
    mix: f32,
    tail: f32,
    sample_rate: f32,
    spread: usize,
) {
    const COMBS: [usize; 4] = [1116, 1188, 1277, 1356];
    const ALLPASSES: [usize; 2] = [556, 441];
    let scale = sample_rate / 44_100.0;
    let feedback = 0.7 + 0.28 * size.clamp(0.0, 1.0);
    let damp = damp.clamp(0.0, 0.95);
    buf.resize(buf.len() + (tail.max(0.0) * sample_rate) as usize, 0.0);

    let mut combs: Vec<(Vec<f32>, usize, f32)> = COMBS
        .iter()
        .map(|&n| (vec![0.0; ((n + spread) as f32 * scale) as usize + 1], 0, 0.0))
        .collect();
    let mut allpasses: Vec<(Vec<f32>, usize)> = ALLPASSES
        .iter()
        .map(|&n| (vec![0.0; ((n + spread) as f32 * scale) as usize + 1], 0))
        .collect();

    for s in buf.iter_mut() {
        let input = *s * 0.25;
        let mut wet = 0.0;
        for (line, pos, filter) in combs.iter_mut() {
            let out = line[*pos];
            *filter = out * (1.0 - damp) + *filter * damp;
            line[*pos] = input + *filter * feedback;
            *pos = (*pos + 1) % line.len();
            wet += out;
        }
        for (line, pos) in allpasses.iter_mut() {
            let delayed = line[*pos];
            let out = delayed - wet;
            line[*pos] = wet + delayed * 0.5;
            *pos = (*pos + 1) % line.len();
            wet = out;
        }
        *s += wet * mix;
    }
}

// ---------------------------------------------------------------------------------------------
// Buffers, mixing and analysis
// ---------------------------------------------------------------------------------------------

/// Renders `secs` seconds by calling `f(t, i)` for every sample (`t` in seconds).
pub fn render(secs: f32, mut f: impl FnMut(f32, usize) -> f32) -> Vec<f32> {
    (0..samples(secs)).map(|i| f(i as f32 / SR, i)).collect()
}

/// Adds `src * gain` into `dst` starting `at` seconds in, growing `dst` as needed.
pub fn mix_at(dst: &mut Vec<f32>, src: &[f32], gain: f32, at: f32) {
    let offset = samples(at);
    if dst.len() < offset + src.len() {
        dst.resize(offset + src.len(), 0.0);
    }
    for (d, s) in dst[offset..].iter_mut().zip(src) {
        *d += s * gain;
    }
}

/// Sums several `(buffer, gain, start_secs)` layers into one buffer.
pub fn layer(parts: &[(&[f32], f32, f32)]) -> Vec<f32> {
    let mut out = Vec::new();
    for &(buf, gain, at) in parts {
        mix_at(&mut out, buf, gain, at);
    }
    out
}

pub fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

pub fn rms(buf: &[f32]) -> f32 {
    if buf.is_empty() {
        return 0.0;
    }
    (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
}

/// Loudest RMS over any window of `window` samples (hop = window / 2): a cheap loudness measure
/// that treats short and long sounds fairly.
pub fn max_window_rms(buf: &[f32], window: usize) -> f32 {
    let window = window.max(1);
    if buf.len() <= window {
        return rms(buf);
    }
    let mut best = 0.0f32;
    let mut start = 0;
    while start + window <= buf.len() {
        best = best.max(rms(&buf[start..start + window]));
        start += window / 2;
    }
    best
}

/// Approximate ITU-R BS.1770 K-weighting: +4 dB high shelf above ~1.7 kHz and a 38 Hz
/// high-pass, so the measure follows perceived loudness rather than raw energy.
pub fn k_weighted(buf: &[f32], sample_rate: f32) -> Vec<f32> {
    let mut shelf = Biquad::high_shelf(1681.0, 4.0, sample_rate);
    let mut hp = Biquad::highpass(38.0, 0.5, sample_rate);
    buf.iter().map(|&x| hp.process(shelf.process(x))).collect()
}

/// Loudness in LUFS-style units (`-0.691 + 10·log10(mean square)` of the K-weighted signal) of
/// the loudest `window_secs` window. 0.4 s is EBU "momentary" loudness; sound effects are
/// normalised on 50 ms windows so short transients and long sounds are treated alike.
pub fn loudness(buf: &[f32], sample_rate: f32, window_secs: f32) -> f32 {
    let k = k_weighted(buf, sample_rate);
    let r = max_window_rms(&k, (window_secs * sample_rate) as usize);
    -0.691 + 20.0 * r.max(1e-9).log10()
}

pub fn scale(buf: &mut [f32], gain: f32) {
    for s in buf {
        *s *= gain;
    }
}

/// Removes DC offset with a gentle high-pass.
pub fn remove_dc(buf: &mut [f32]) {
    highpass(buf, 12.0);
}

pub fn fade_in(buf: &mut [f32], secs: f32) {
    let n = samples(secs).min(buf.len()).max(1);
    for (i, s) in buf.iter_mut().take(n).enumerate() {
        *s *= i as f32 / n as f32;
    }
}

pub fn fade_out(buf: &mut [f32], secs: f32) {
    let n = samples(secs).min(buf.len()).max(1);
    let len = buf.len();
    for (i, s) in buf[len - n..].iter_mut().enumerate() {
        *s *= 1.0 - (i + 1) as f32 / n as f32;
    }
}

/// Drops trailing samples quieter than `threshold` (relative to the buffer peak), keeping at
/// least `min_secs`, then applies a short fade so the cut is inaudible.
pub fn trim_tail(buf: &mut Vec<f32>, threshold: f32, min_secs: f32) {
    let limit = peak(buf) * threshold;
    let last = buf.iter().rposition(|s| s.abs() > limit).unwrap_or(0);
    let keep = (last + samples(0.01)).max(samples(min_secs)).min(buf.len());
    buf.truncate(keep);
    fade_out(buf, 0.01);
}

/// Soft-knee peak limiter for finished buffers: samples above `knee` are smoothly compressed so
/// the output never exceeds `ceiling`.
pub fn soft_limit(buf: &mut [f32], knee: f32, ceiling: f32) {
    let range = (ceiling - knee).max(1e-3);
    for s in buf {
        let a = s.abs();
        if a > knee {
            // soft_clip has unit slope at 0 and saturates at 1, so the curve is continuous at
            // the knee and approaches the ceiling asymptotically.
            let shaped = knee + range * soft_clip((a - knee) / range);
            *s = shaped.min(ceiling).copysign(*s);
        }
    }
}

/// Scales so the peak is exactly `target`.
pub fn normalize(buf: &mut [f32], target: f32) {
    let p = peak(buf);
    if p > 1e-9 {
        scale(buf, target / p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..1000 {
            let x = a.f32();
            assert_eq!(x, b.f32());
            assert!((0.0..1.0).contains(&x));
        }
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
    }

    #[test]
    fn oscillators_stay_bounded() {
        for wave in [Wave::Sine, Wave::Triangle, Wave::Saw, Wave::Square, Wave::Noise] {
            let mut osc = Osc::new(wave);
            for i in 0..10_000 {
                let f = 30.0 + i as f32;
                let v = osc.next(f);
                assert!(v.is_finite() && v.abs() <= 1.2, "{wave:?} {v}");
            }
        }
    }

    #[test]
    fn svf_lowpass_attenuates_highs() {
        let mut lo = Svf::default();
        let mut osc = Osc::new(Wave::Sine);
        let out: Vec<f32> = (0..20_000).map(|_| lo.lowpass(osc.next(8000.0), 300.0, 0.7)).collect();
        assert!(rms(&out[10_000..]) < 0.02);
        let mut lo = Svf::default();
        let mut osc = Osc::new(Wave::Sine);
        let out: Vec<f32> = (0..20_000).map(|_| lo.lowpass(osc.next(60.0), 3000.0, 0.7)).collect();
        assert!(rms(&out[10_000..]) > 0.6);
    }

    #[test]
    fn soft_limit_respects_ceiling() {
        let mut buf: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.01).sin() * 3.0).collect();
        soft_limit(&mut buf, 0.7, 0.98);
        assert!(peak(&buf) <= 0.98);
    }

    #[test]
    fn loudness_of_full_scale_sine_is_about_minus_three() {
        let mut osc = Osc::new(Wave::Sine);
        let buf: Vec<f32> = (0..44_100).map(|_| osc.next(1000.0)).collect();
        let l = loudness(&buf, SR, 0.4);
        assert!((l + 3.0).abs() < 1.0, "{l}");
    }

    #[test]
    fn crush_quantizes() {
        let mut buf = vec![0.123, 0.5, -0.77];
        crush(&mut buf, 4, 1.0);
        for s in buf {
            assert!(((s * 8.0).round() - s * 8.0).abs() < 1e-6);
        }
    }
}
