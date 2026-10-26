//! Sound effects: the [`Sfx`] list, per-sound mixing metadata and the synthesis recipe of every
//! sound.
//!
//! Each recipe layers a few simple generators (pitch-swept thumps, filtered noise, inharmonic
//! metal partials, FM, formant growls) and post effects, then [`render_variant`] normalises the
//! result to the sound's loudness target from [`Sfx::info`]. Recipes are seeded from the sound's
//! index, so generation is fully deterministic.

use serde::{Deserialize, Serialize};

use super::dsp::{
    Osc, Rng, SR, Svf, Wave, adsr, bell, crush, db_to_gain, drive, echo, fade_out, glide, highpass,
    layer, lerp, loudness, lowpass, midi_hz, mix_at, peak, perc, remove_dc, render, reverb,
    scale, smoothstep, soft_limit, sweep, trim_tail,
};

/// Every sound effect in the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Sfx {
    // UI
    UiMove,
    UiSelect,
    UiBack,
    ChatMessage,
    // Movement
    Footstep,
    Jump,
    Land,
    HeavyLand,
    Dash,
    Slide,
    Slam,
    SlamImpact,
    WallJump,
    JumpPad,
    // Player
    Hurt,
    Heal,
    Death,
    Respawn,
    Downed,
    Revive,
    NoStamina,
    // Weapons
    LancetFire,
    LancetCharge,
    LancetBeam,
    LancetFan,
    ThresherFire,
    ThresherSlug,
    FlakLaunch,
    RiveterFire,
    SawLaunch,
    SawBounce,
    VolleyFire,
    CinderFire,
    StickyArm,
    StickyDetonate,
    MortarFire,
    Explosion,
    WeaponSwitch,
    ModeSwitch,
    Punch,
    PunchHit,
    Parry,
    PrismThrow,
    Ricochet,
    Hitmarker,
    Headshot,
    KillConfirm,
    // Enemies (the Moulds, docs/roster.md): alert, wind-up telegraphs, strikes, projectiles.
    GnawerAlert,
    GnawerBiteWindup,
    GnawerBite,
    GnawerPounceWindup,
    GnawerPounce,
    CenserAlert,
    CenserWhirl,
    CenserLob,
    CenserSwingWindup,
    CenserSwing,
    EmberFlight,
    EmberBurst,
    HarrierAlert,
    HarrierShriek,
    HarrierDive,
    HarrierSwat,
    GaolerAlert,
    GaolerBashWindup,
    GaolerBash,
    GaolerChain,
    GaolerSweep,
    GaolerRushWindup,
    GaolerRush,
    GaolerStep,
    HeronAlert,
    HeronLaser,
    HeronLock,
    HeronNeedle,
    HeronKickWindup,
    HeronKick,
    /// A hit cracks a Mould: wax snapping, a sizzle.
    EnemyHurt,
    /// A Mould slumps and melts.
    EnemyDeath,
    GaolerDeath,
    /// The heat leaving a dead Mould: one big ember burst.
    EmberRelease,
    Gib,
    EnemySpawn,
    ProjectileImpact,
    // Game
    RankUp,
    RankDown,
    StyleBonus,
    DoorLock,
    DoorOpen,
    Checkpoint,
    WaveStart,
    EncounterClear,
    LevelComplete,
    Countdown,
    MatchStart,
    MatchEnd,
}

/// Mixing and playback metadata of one sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfxInfo {
    /// Loudness target: K-weighted loudness of the loudest 50 ms window, in LUFS-style units
    /// ([`super::dsp::loudness`]). Peaks are soft-limited to stay below 0 dBFS.
    pub level_db: f32,
    /// Maximum simultaneous instances; the oldest one is faded out when exceeded.
    pub max_voices: u8,
    /// Random pitch variation per play (± fraction).
    pub pitch_jitter: f32,
    /// Random volume variation per play (± dB).
    pub volume_jitter_db: f32,
    /// A repeat within this many seconds of the previous start is dropped (ten shotgun pellets
    /// make one hitmarker, not ten stacked ones).
    pub min_interval: f32,
    /// Number of pre-generated variations picked at random on play.
    pub variants: u8,
}

const fn info(level_db: f32, max_voices: u8, pitch_jitter: f32, min_interval: f32) -> SfxInfo {
    SfxInfo {
        level_db,
        max_voices,
        pitch_jitter,
        volume_jitter_db: if pitch_jitter > 0.0 { 1.0 } else { 0.0 },
        min_interval,
        variants: 1,
    }
}

const fn variants(mut i: SfxInfo, n: u8) -> SfxInfo {
    i.variants = n;
    i
}

impl Sfx {
    pub const ALL: &'static [Sfx] = &[
        Sfx::UiMove,
        Sfx::UiSelect,
        Sfx::UiBack,
        Sfx::ChatMessage,
        Sfx::Footstep,
        Sfx::Jump,
        Sfx::Land,
        Sfx::HeavyLand,
        Sfx::Dash,
        Sfx::Slide,
        Sfx::Slam,
        Sfx::SlamImpact,
        Sfx::WallJump,
        Sfx::JumpPad,
        Sfx::Hurt,
        Sfx::Heal,
        Sfx::Death,
        Sfx::Respawn,
        Sfx::Downed,
        Sfx::Revive,
        Sfx::NoStamina,
        Sfx::LancetFire,
        Sfx::LancetCharge,
        Sfx::LancetBeam,
        Sfx::LancetFan,
        Sfx::ThresherFire,
        Sfx::ThresherSlug,
        Sfx::FlakLaunch,
        Sfx::RiveterFire,
        Sfx::SawLaunch,
        Sfx::SawBounce,
        Sfx::VolleyFire,
        Sfx::CinderFire,
        Sfx::StickyArm,
        Sfx::StickyDetonate,
        Sfx::MortarFire,
        Sfx::Explosion,
        Sfx::WeaponSwitch,
        Sfx::ModeSwitch,
        Sfx::Punch,
        Sfx::PunchHit,
        Sfx::Parry,
        Sfx::PrismThrow,
        Sfx::Ricochet,
        Sfx::Hitmarker,
        Sfx::Headshot,
        Sfx::KillConfirm,
        Sfx::GnawerAlert,
        Sfx::GnawerBiteWindup,
        Sfx::GnawerBite,
        Sfx::GnawerPounceWindup,
        Sfx::GnawerPounce,
        Sfx::CenserAlert,
        Sfx::CenserWhirl,
        Sfx::CenserLob,
        Sfx::CenserSwingWindup,
        Sfx::CenserSwing,
        Sfx::EmberFlight,
        Sfx::EmberBurst,
        Sfx::HarrierAlert,
        Sfx::HarrierShriek,
        Sfx::HarrierDive,
        Sfx::HarrierSwat,
        Sfx::GaolerAlert,
        Sfx::GaolerBashWindup,
        Sfx::GaolerBash,
        Sfx::GaolerChain,
        Sfx::GaolerSweep,
        Sfx::GaolerRushWindup,
        Sfx::GaolerRush,
        Sfx::GaolerStep,
        Sfx::HeronAlert,
        Sfx::HeronLaser,
        Sfx::HeronLock,
        Sfx::HeronNeedle,
        Sfx::HeronKickWindup,
        Sfx::HeronKick,
        Sfx::EnemyHurt,
        Sfx::EnemyDeath,
        Sfx::GaolerDeath,
        Sfx::EmberRelease,
        Sfx::Gib,
        Sfx::EnemySpawn,
        Sfx::ProjectileImpact,
        Sfx::RankUp,
        Sfx::RankDown,
        Sfx::StyleBonus,
        Sfx::DoorLock,
        Sfx::DoorOpen,
        Sfx::Checkpoint,
        Sfx::WaveStart,
        Sfx::EncounterClear,
        Sfx::LevelComplete,
        Sfx::Countdown,
        Sfx::MatchStart,
        Sfx::MatchEnd,
    ];

    /// Number of sounds; `sfx as usize` is a valid index below this.
    pub const COUNT: usize = Sfx::ALL.len();

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> String {
        format!("{self:?}")
    }

    /// Mixing levels and voice limits. See `docs/audio.md` for the reasoning behind the levels.
    pub fn info(self) -> SfxInfo {
        use Sfx::*;
        match self {
            UiMove => info(-22.0, 2, 0.0, 0.03),
            UiSelect => info(-18.0, 2, 0.0, 0.03),
            UiBack => info(-19.0, 2, 0.0, 0.03),
            ChatMessage => info(-19.0, 2, 0.0, 0.1),

            Footstep => variants(info(-25.0, 4, 0.08, 0.05), 4),
            Jump => info(-22.0, 3, 0.05, 0.05),
            Land => info(-21.0, 3, 0.06, 0.06),
            HeavyLand => info(-16.0, 2, 0.04, 0.1),
            Dash => info(-17.0, 3, 0.05, 0.05),
            Slide => info(-21.0, 2, 0.04, 0.1),
            Slam => info(-18.0, 2, 0.03, 0.1),
            SlamImpact => info(-12.0, 2, 0.03, 0.1),
            WallJump => info(-19.0, 3, 0.05, 0.05),
            JumpPad => info(-17.0, 3, 0.03, 0.08),

            Hurt => info(-16.0, 3, 0.06, 0.06),
            Heal => info(-20.0, 3, 0.04, 0.05),
            Death => info(-12.0, 3, 0.0, 0.1),
            Respawn => info(-17.0, 3, 0.0, 0.1),
            Downed => info(-14.0, 3, 0.0, 0.1),
            Revive => info(-16.0, 3, 0.0, 0.1),
            NoStamina => info(-21.0, 1, 0.0, 0.15),

            LancetFire => info(-14.0, 6, 0.05, 0.0),
            LancetCharge => info(-19.0, 2, 0.0, 0.05),
            LancetBeam => info(-12.0, 3, 0.03, 0.0),
            LancetFan => info(-13.0, 4, 0.04, 0.0),
            ThresherFire => info(-11.0, 4, 0.04, 0.0),
            ThresherSlug => info(-11.0, 3, 0.04, 0.0),
            FlakLaunch => info(-15.0, 3, 0.05, 0.0),
            RiveterFire => variants(info(-17.0, 6, 0.06, 0.0), 3),
            SawLaunch => info(-15.0, 3, 0.05, 0.0),
            SawBounce => info(-17.0, 4, 0.08, 0.03),
            VolleyFire => info(-14.0, 3, 0.04, 0.0),
            CinderFire => info(-13.0, 3, 0.04, 0.0),
            StickyArm => info(-19.0, 4, 0.05, 0.03),
            StickyDetonate => info(-18.0, 2, 0.0, 0.05),
            MortarFire => info(-13.0, 3, 0.04, 0.0),
            Explosion => info(-10.0, 5, 0.07, 0.03),
            WeaponSwitch => info(-20.0, 2, 0.03, 0.03),
            ModeSwitch => info(-21.0, 2, 0.03, 0.03),
            Punch => info(-19.0, 3, 0.07, 0.03),
            PunchHit => info(-13.0, 3, 0.06, 0.03),
            Parry => info(-10.0, 3, 0.02, 0.05),
            PrismThrow => info(-19.0, 3, 0.04, 0.03),
            Ricochet => variants(info(-15.0, 8, 0.02, 0.0), 2),
            Hitmarker => info(-20.0, 3, 0.03, 0.035),
            Headshot => info(-17.0, 3, 0.02, 0.035),
            KillConfirm => info(-16.0, 3, 0.0, 0.04),

            GnawerAlert => variants(info(-18.0, 4, 0.1, 0.1), 2),
            GnawerBiteWindup => info(-14.0, 6, 0.06, 0.05),
            GnawerBite => info(-15.0, 6, 0.08, 0.03),
            GnawerPounceWindup => info(-14.0, 4, 0.06, 0.05),
            GnawerPounce => info(-15.0, 4, 0.06, 0.05),
            CenserAlert => info(-17.0, 3, 0.06, 0.1),
            CenserWhirl => info(-13.0, 4, 0.03, 0.05),
            CenserLob => info(-15.0, 6, 0.08, 0.03),
            CenserSwingWindup => info(-15.0, 3, 0.05, 0.05),
            CenserSwing => info(-14.0, 3, 0.06, 0.05),
            EmberFlight => info(-22.0, 6, 0.12, 0.03),
            EmberBurst => variants(info(-13.0, 6, 0.08, 0.02), 2),
            HarrierAlert => info(-17.0, 4, 0.1, 0.1),
            HarrierShriek => info(-12.0, 4, 0.05, 0.05),
            HarrierDive => info(-14.0, 4, 0.05, 0.05),
            HarrierSwat => info(-12.0, 3, 0.04, 0.05),
            GaolerAlert => info(-14.0, 2, 0.04, 0.2),
            GaolerBashWindup => info(-11.0, 3, 0.0, 0.2),
            GaolerBash => info(-10.0, 3, 0.03, 0.1),
            GaolerChain => info(-12.0, 3, 0.02, 0.2),
            GaolerSweep => info(-11.0, 3, 0.03, 0.1),
            GaolerRushWindup => info(-13.0, 3, 0.03, 0.2),
            GaolerRush => info(-12.0, 3, 0.03, 0.1),
            GaolerStep => variants(info(-18.0, 4, 0.06, 0.1), 2),
            HeronAlert => info(-16.0, 3, 0.06, 0.1),
            HeronLaser => info(-15.0, 4, 0.0, 0.1),
            HeronLock => info(-12.0, 4, 0.0, 0.05),
            HeronNeedle => info(-12.0, 4, 0.04, 0.03),
            HeronKickWindup => info(-17.0, 3, 0.06, 0.05),
            HeronKick => info(-15.0, 3, 0.06, 0.05),
            EnemyHurt => variants(info(-19.0, 6, 0.1, 0.03), 3),
            EnemyDeath => variants(info(-15.0, 5, 0.1, 0.03), 2),
            GaolerDeath => info(-11.0, 2, 0.03, 0.1),
            EmberRelease => info(-15.0, 5, 0.08, 0.03),
            Gib => variants(info(-16.0, 5, 0.08, 0.03), 3),
            EnemySpawn => info(-16.0, 4, 0.05, 0.05),
            ProjectileImpact => variants(info(-18.0, 6, 0.1, 0.02), 2),

            RankUp => info(-14.0, 2, 0.0, 0.1),
            RankDown => info(-19.0, 2, 0.0, 0.1),
            StyleBonus => info(-20.0, 3, 0.0, 0.05),
            DoorLock => info(-14.0, 2, 0.0, 0.1),
            DoorOpen => info(-16.0, 3, 0.0, 0.1),
            Checkpoint => info(-16.0, 1, 0.0, 0.2),
            WaveStart => info(-13.0, 1, 0.0, 0.2),
            EncounterClear => info(-14.0, 1, 0.0, 0.2),
            LevelComplete => info(-14.0, 1, 0.0, 0.2),
            Countdown => info(-16.0, 2, 0.0, 0.1),
            MatchStart => info(-13.0, 1, 0.0, 0.2),
            MatchEnd => info(-13.0, 1, 0.0, 0.2),
        }
    }
}

/// Renders the first variation of `sfx` (mono, [`super::dsp::SAMPLE_RATE`]), normalised.
pub fn render_sfx(sfx: Sfx) -> Vec<f32> {
    render_variant(sfx, 0)
}

/// Renders variation `variant` of `sfx` (mono, [`super::dsp::SAMPLE_RATE`]), normalised to its
/// loudness target with peaks below 0 dBFS. Deterministic.
pub fn render_variant(sfx: Sfx, variant: u8) -> Vec<f32> {
    let seed = 0x5EED_0000 + (sfx.index() as u64) * 131 + variant as u64 * 7919;
    let mut rng = Rng::new(seed);
    let mut buf = recipe(sfx, &mut rng);
    finish(&mut buf, sfx.info().level_db);
    buf
}

/// Final mastering of a sound: DC removal, tail trim, loudness normalisation, soft peak limiting.
fn finish(buf: &mut Vec<f32>, level_db: f32) {
    for s in buf.iter_mut() {
        if !s.is_finite() {
            *s = 0.0;
        }
    }
    remove_dc(buf);
    trim_tail(buf, 0.0015, 0.03);

    // Loudness: K-weighted, loudest 50 ms window.
    let measured = loudness(buf, SR, 0.05);
    let p = peak(buf).max(1e-6);
    // Never push transients more than ~4 dB into the limiter.
    let gain = db_to_gain(level_db - measured).min(1.6 / p);
    scale(buf, gain);
    soft_limit(buf, 0.7, 0.98);
    fade_out(buf, 0.004);
}

// ---------------------------------------------------------------------------------------------
// Building blocks
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub(super) enum Pass {
    Low,
    Band,
    High,
}

/// Pitch-swept sine: kick drums, booms, body thumps.
pub(super) fn thump(dur: f32, f0: f32, f1: f32, pitch_tau: f32, decay: f32) -> Vec<f32> {
    let mut osc = Osc::new(Wave::Sine);
    render(dur, |t, _| osc.next(glide(t, f0, f1, pitch_tau)) * perc(t, 0.0008, decay))
}

/// Filtered white noise, cutoff sweeping exponentially `f0 → f1` over `dur`.
#[allow(clippy::too_many_arguments)]
pub(super) fn noise(rng: &mut Rng, dur: f32, pass: Pass, f0: f32, f1: f32, q: f32, attack: f32, decay: f32) -> Vec<f32> {
    let mut n = Rng::new(rng.next_u64());
    let mut svf = Svf::default();
    render(dur, |t, _| {
        let o = svf.process(n.bipolar(), sweep(t, f0, f1, dur), q, SR);
        let y = match pass {
            Pass::Low => o.low,
            Pass::Band => o.band,
            Pass::High => o.high,
        };
        y * perc(t, attack, decay)
    })
}

/// Broadband transient.
pub(super) fn click(rng: &mut Rng, decay: f32) -> Vec<f32> {
    noise(rng, decay * 6.0 + 0.002, Pass::High, 2500.0, 2500.0, 0.6, 0.0002, decay)
}

/// Oscillator with an exponential pitch sweep and a percussive envelope.
#[allow(clippy::too_many_arguments)]
pub(super) fn tone(dur: f32, wave: Wave, f0: f32, f1: f32, sweep_dur: f32, attack: f32, decay: f32) -> Vec<f32> {
    let mut osc = Osc::new(wave);
    render(dur, |t, _| osc.next(sweep(t, f0, f1, sweep_dur)) * perc(t, attack, decay))
}

/// Inharmonic partials `(ratio, amplitude, decay)` over `base`: bells, clangs, pings.
pub(super) fn metal(dur: f32, base: f32, partials: &[(f32, f32, f32)]) -> Vec<f32> {
    let mut oscs: Vec<Osc> = (0..partials.len())
        .map(|i| Osc::new(Wave::Sine).phase(i as f32 * 0.31))
        .collect();
    render(dur, |t, _| {
        partials
            .iter()
            .zip(oscs.iter_mut())
            .map(|(&(ratio, amp, decay), osc)| osc.next(base * ratio) * amp * perc(t, 0.0005, decay))
            .sum()
    })
}

/// Two-operator FM. Carrier sweeps `fc0 → fc1`; the modulation index decays from `i0` to `i1`.
#[allow(clippy::too_many_arguments)]
pub(super) fn fm(dur: f32, fc0: f32, fc1: f32, ratio: f32, i0: f32, i1: f32, attack: f32, decay: f32) -> Vec<f32> {
    let mut car = Osc::new(Wave::Sine);
    let mut modu = Osc::new(Wave::Sine);
    render(dur, |t, _| {
        let fc = sweep(t, fc0, fc1, dur);
        let index = i1 + (i0 - i1) * (-t / (decay * 0.6).max(1e-3)).exp();
        let m = modu.next(fc * ratio) * index;
        car.next_pm(fc, m / std::f32::consts::TAU) * perc(t, attack, decay)
    })
}

/// Band-passed noise with a smooth swell: air movement.
pub(super) fn whoosh(rng: &mut Rng, dur: f32, f0: f32, f1: f32, q: f32) -> Vec<f32> {
    let mut n = Rng::new(rng.next_u64());
    let mut svf = Svf::default();
    render(dur, |t, _| {
        let env = bell(t, dur).powf(0.7);
        svf.process(n.bipolar(), sweep(t, f0, f1, dur), q, SR).band * env
    })
}

/// Sparse random crackles, density falling from `density` per second with time constant `decay`.
pub(super) fn crackle(rng: &mut Rng, dur: f32, density: f32, decay: f32) -> Vec<f32> {
    let mut n = Rng::new(rng.next_u64());
    let mut tail = 0.0f32;
    let mut sign = 1.0f32;
    let mut buf = render(dur, |t, _| {
        if n.chance(density * (-t / decay).exp() / SR) {
            tail = n.range(0.3, 1.0);
            sign = if n.chance(0.5) { 1.0 } else { -1.0 };
        }
        tail *= 0.93;
        sign = -sign;
        tail * sign
    });
    highpass(&mut buf, 800.0);
    buf
}

/// Voice-like growl: jittery saw through two formant band-passes. The formants follow the pitch
/// sweep (by its square root), so rising growls read as rising screeches.
#[allow(clippy::too_many_arguments)]
fn growl(rng: &mut Rng, dur: f32, f0: f32, f1: f32, formants: [f32; 2], jitter: f32, attack: f32, decay: f32) -> Vec<f32> {
    let mut n = Rng::new(rng.next_u64());
    let mut osc = Osc::new(Wave::Saw);
    let mut sub = Osc::new(Wave::Square);
    let (mut a, mut b) = (Svf::default(), Svf::default());
    let mut wobble = 0.0f32;
    render(dur, |t, _| {
        wobble += (n.bipolar() - wobble) * 0.003;
        let base = sweep(t, f0, f1, dur);
        let f = base * (1.0 + wobble * jitter * 6.0);
        let x = osc.next(f) + sub.next(f * 0.5) * 0.4 + n.bipolar() * 0.2;
        let shift = (base / f0).sqrt();
        let y = a.bandpass(x, formants[0] * shift, 3.0) + b.bandpass(x, formants[1] * shift, 4.0) * 0.7;
        y * perc(t, attack, decay)
    })
}

/// Detuned saw stack chord through a swept low-pass. `notes` are MIDI numbers.
#[allow(clippy::too_many_arguments)]
fn chord(notes: &[f32], dur: f32, wave: Wave, cutoff0: f32, cutoff1: f32, attack: f32, decay: f32, release_at: f32) -> Vec<f32> {
    let mut oscs: Vec<(Osc, f32)> = Vec::new();
    for (i, &n) in notes.iter().enumerate() {
        for (j, cents) in [-9.0f32, 0.0, 8.0].iter().enumerate() {
            let f = midi_hz(n) * 2f32.powf(cents / 1200.0);
            oscs.push((Osc::new(wave).phase((i * 3 + j) as f32 * 0.137), f));
        }
    }
    let norm = 1.0 / (oscs.len() as f32).sqrt();
    let mut lp = Svf::default();
    render(dur, |t, _| {
        let x: f32 = oscs.iter_mut().map(|(o, f)| o.next(*f)).sum::<f32>() * norm;
        let env = perc(t, attack, decay) * (1.0 - smoothstep(release_at, dur, t));
        lp.lowpass(x, sweep(t, cutoff0, cutoff1, dur * 0.6), 0.9) * env
    })
}

/// A melodic line: `(midi, start, length)` notes on two slightly detuned oscillators.
fn notes(seq: &[(f32, f32, f32)], wave: Wave, attack: f32, release: f32, cutoff: f32, vibrato: f32) -> Vec<f32> {
    let mut out = Vec::new();
    for &(note, start, len) in seq {
        let f = midi_hz(note);
        let mut a = Osc::new(wave);
        let mut b = Osc::new(wave).phase(0.37);
        let mut vib = Osc::new(Wave::Sine);
        let mut lp = Svf::default();
        let buf = render(len + release, |t, _| {
            let v = 1.0 + vib.next(6.5) * vibrato * smoothstep(0.08, 0.25, t);
            let x = a.next(f * v) * 0.6 + b.next(f * v * 1.004) * 0.4;
            lp.lowpass(x, cutoff, 0.8) * adsr(t, attack, 0.08, 0.75, release, len)
        });
        mix_at(&mut out, &buf, 1.0, start);
    }
    out
}

/// Amplitude stutter: chops the signal with a square gate at `rate` Hz after `from` seconds.
fn stutter(buf: &mut [f32], rate: f32, from: f32) {
    for (i, s) in buf.iter_mut().enumerate() {
        let t = i as f32 / SR;
        if t > from && (t * rate).fract() > 0.55 {
            *s *= 0.15;
        }
    }
}

/// Shared core of the hitscan guns: transient, body thump, crack and noise tail.
struct Gun {
    body_hz: f32,
    body_decay: f32,
    crack_hz: f32,
    crack_decay: f32,
    tail_hz: f32,
    tail_decay: f32,
}

impl Gun {
    fn render(&self, rng: &mut Rng) -> Vec<f32> {
        let click = click(rng, 0.003);
        let body = thump(self.body_decay * 5.0, self.body_hz * 2.6, self.body_hz, 0.012, self.body_decay);
        let crack = noise(rng, self.crack_decay * 6.0, Pass::Band, self.crack_hz, self.crack_hz * 0.6, 0.9, 0.0003, self.crack_decay);
        let tail = noise(rng, self.tail_decay * 5.0, Pass::Low, self.tail_hz, self.tail_hz * 0.25, 0.7, 0.001, self.tail_decay);
        layer(&[(&click, 0.8, 0.0), (&body, 1.0, 0.0), (&crack, 0.9, 0.0), (&tail, 0.6, 0.0)])
    }
}

// ---------------------------------------------------------------------------------------------
// Recipes
// ---------------------------------------------------------------------------------------------

fn recipe(sfx: Sfx, r: &mut Rng) -> Vec<f32> {
    use Sfx::*;
    match sfx {
        UiMove => {
            let mut b = tone(0.05, Wave::Square, 1250.0, 1250.0, 0.01, 0.001, 0.015);
            lowpass(&mut b, 5000.0);
            crush(&mut b, 6, 2.0);
            b
        }
        UiSelect => {
            let mut b = notes(&[(84.0, 0.0, 0.045), (91.0, 0.05, 0.08)], Wave::Square, 0.002, 0.04, 5000.0, 0.0);
            echo(&mut b, 0.07, 0.25, 0.25, 3000.0, 0.15);
            b
        }
        UiBack => notes(&[(86.0, 0.0, 0.045), (79.0, 0.05, 0.07)], Wave::Triangle, 0.002, 0.05, 4000.0, 0.0),
        ChatMessage => {
            let a = fm(0.35, 880.0, 880.0, 3.5, 2.0, 0.2, 0.002, 0.12);
            let b = fm(0.45, 1318.5, 1318.5, 3.5, 2.0, 0.2, 0.002, 0.16);
            layer(&[(&a, 0.8, 0.0), (&b, 0.8, 0.09)])
        }

        // ----- movement -------------------------------------------------------------------
        Footstep => {
            let thud = thump(0.12, r.range(150.0, 175.0), 70.0, 0.01, 0.028);
            let scuff_hz = r.range(2600.0, 3400.0);
            let scuff = noise(r, 0.08, Pass::Low, scuff_hz, 900.0, 0.7, 0.001, 0.016);
            let base = r.range(2300.0, 2900.0);
            let tick = metal(0.08, base, &[(1.0, 1.0, 0.012), (1.47, 0.6, 0.009), (2.31, 0.4, 0.006)]);
            layer(&[(&thud, 1.0, 0.0), (&scuff, 0.7, 0.0), (&tick, 0.22, 0.002)])
        }
        Jump => {
            let air = whoosh(r, 0.22, 700.0, 2600.0, 1.2);
            let servo = tone(0.15, Wave::Triangle, 280.0, 560.0, 0.12, 0.002, 0.05);
            let push = thump(0.1, 200.0, 110.0, 0.01, 0.025);
            layer(&[(&air, 1.0, 0.0), (&servo, 0.25, 0.0), (&push, 0.5, 0.0)])
        }
        Land => {
            let thud = thump(0.2, 140.0, 50.0, 0.015, 0.055);
            let crunch = noise(r, 0.12, Pass::Low, 2200.0, 500.0, 0.7, 0.001, 0.03);
            let tick = metal(0.1, 1900.0, &[(1.0, 1.0, 0.02), (1.53, 0.6, 0.015)]);
            layer(&[(&thud, 1.0, 0.0), (&crunch, 0.6, 0.0), (&tick, 0.15, 0.003)])
        }
        HeavyLand => {
            let thud = thump(0.5, 110.0, 38.0, 0.03, 0.13);
            let crunch = noise(r, 0.3, Pass::Low, 2500.0, 300.0, 0.7, 0.001, 0.07);
            let clank = metal(0.5, 420.0, &[(1.0, 1.0, 0.15), (2.37, 0.7, 0.1), (3.91, 0.4, 0.06), (5.2, 0.3, 0.04)]);
            let mut b = layer(&[(&thud, 1.0, 0.0), (&crunch, 0.7, 0.0), (&clank, 0.3, 0.0)]);
            drive(&mut b, 2.0);
            crush(&mut b, 10, 1.5);
            b
        }
        Dash => {
            let air = noise(r, 0.3, Pass::Band, 1500.0, 5200.0, 1.1, 0.004, 0.08);
            let mut jet = tone(0.28, Wave::Saw, 220.0, 90.0, 0.2, 0.002, 0.07);
            lowpass(&mut jet, 1200.0);
            let fwip = tone(0.14, Wave::Sine, 1400.0, 300.0, 0.1, 0.001, 0.04);
            let mut b = layer(&[(&air, 1.0, 0.0), (&jet, 0.5, 0.0), (&fwip, 0.35, 0.0)]);
            drive(&mut b, 1.5);
            b
        }
        Slide => {
            let mut n = Rng::new(r.next_u64());
            let mut svf = Svf::default();
            let mut grit = 0.5f32;
            let dur = 0.6;
            let scrape = render(dur, |t, _| {
                grit += (n.f32() - grit) * 0.02;
                let env = smoothstep(0.0, 0.03, t) * (1.0 - smoothstep(0.35, dur, t));
                svf.process(n.bipolar(), 2800.0 + 600.0 * grit, 1.5, SR).band * env * (0.5 + grit)
            });
            let mut grind = fm(dur, 180.0, 150.0, 7.1, 3.0, 2.0, 0.03, 0.3);
            lowpass(&mut grind, 2500.0);
            layer(&[(&scrape, 1.0, 0.0), (&grind, 0.25, 0.0)])
        }
        Slam => {
            let air = whoosh(r, 0.35, 2800.0, 350.0, 1.4);
            let mut dive = tone(0.35, Wave::Saw, 900.0, 120.0, 0.3, 0.01, 0.15);
            lowpass(&mut dive, 1800.0);
            let mut b = layer(&[(&air, 1.0, 0.0), (&dive, 0.35, 0.0)]);
            drive(&mut b, 1.4);
            b
        }
        SlamImpact => {
            let boom = thump(0.8, 120.0, 30.0, 0.04, 0.2);
            let crunch = noise(r, 0.6, Pass::Low, 4000.0, 250.0, 0.8, 0.0005, 0.11);
            let clank = metal(0.7, 180.0, &[(1.0, 1.0, 0.3), (2.43, 0.8, 0.2), (3.9, 0.5, 0.12), (6.1, 0.3, 0.07)]);
            let debris = crackle(r, 0.5, 300.0, 0.15);
            let mut b = layer(&[(&boom, 1.2, 0.0), (&crunch, 0.8, 0.0), (&clank, 0.3, 0.0), (&debris, 0.3, 0.02)]);
            drive(&mut b, 2.5);
            crush(&mut b, 10, 2.0);
            reverb(&mut b, 0.5, 0.5, 0.2, 0.4);
            b
        }
        WallJump => {
            let kick = thump(0.12, 220.0, 100.0, 0.01, 0.035);
            let ping = metal(0.3, 1700.0, &[(1.0, 1.0, 0.06), (1.62, 0.6, 0.045), (2.71, 0.4, 0.03)]);
            let air = whoosh(r, 0.22, 800.0, 3000.0, 1.1);
            layer(&[(&kick, 0.8, 0.0), (&ping, 0.35, 0.0), (&air, 0.8, 0.02)])
        }
        JumpPad => {
            let mut osc = Osc::new(Wave::Sine);
            let boing = render(0.5, |t, _| {
                let f = sweep(t, 80.0, 700.0, 0.3) * (1.0 + 0.04 * (t * 18.0 * std::f32::consts::TAU).sin());
                osc.next(f) * perc(t, 0.002, 0.18)
            });
            let air = whoosh(r, 0.5, 500.0, 6000.0, 1.0);
            let push = thump(0.2, 150.0, 60.0, 0.015, 0.05);
            layer(&[(&boing, 0.6, 0.0), (&air, 1.0, 0.0), (&push, 0.8, 0.0)])
        }

        // ----- player ---------------------------------------------------------------------
        Hurt => {
            let crunch = noise(r, 0.18, Pass::Band, 1800.0, 600.0, 0.9, 0.0005, 0.05);
            let glitch = tone(0.16, Wave::Square, 420.0, 130.0, 0.12, 0.001, 0.05);
            let hit = thump(0.12, 180.0, 80.0, 0.01, 0.03);
            let mut b = layer(&[(&crunch, 1.0, 0.0), (&glitch, 0.45, 0.0), (&hit, 0.7, 0.0)]);
            drive(&mut b, 3.0);
            crush(&mut b, 6, 3.0);
            b
        }
        Heal => {
            let slurp = noise(r, 0.3, Pass::Low, 300.0, 1400.0, 6.0, 0.01, 0.1);
            let rise = tone(0.3, Wave::Sine, 520.0, 1040.0, 0.25, 0.02, 0.12);
            let sparkle = fm(0.25, 2093.0, 2093.0, 2.0, 1.0, 0.1, 0.001, 0.07);
            layer(&[(&slurp, 0.8, 0.0), (&rise, 0.4, 0.0), (&sparkle, 0.25, 0.12)])
        }
        Death => {
            let mut fall = tone(1.4, Wave::Saw, 380.0, 35.0, 1.2, 0.001, 0.5);
            lowpass(&mut fall, 2500.0);
            stutter(&mut fall, 18.0, 0.3);
            let crunch = noise(r, 0.9, Pass::Low, 3000.0, 200.0, 0.8, 0.0005, 0.25);
            let boom = thump(1.2, 90.0, 28.0, 0.08, 0.3);
            let mut b = layer(&[(&fall, 0.6, 0.0), (&crunch, 0.9, 0.0), (&boom, 1.0, 0.0)]);
            drive(&mut b, 2.5);
            crush(&mut b, 8, 2.0);
            reverb(&mut b, 0.6, 0.5, 0.2, 0.5);
            b
        }
        Respawn => {
            let mut oscs = [Osc::new(Wave::Saw), Osc::new(Wave::Saw).phase(0.3), Osc::new(Wave::Saw).phase(0.6)];
            let mut lp = Svf::default();
            let swell = render(0.8, |t, _| {
                let f = sweep(t, 110.0, 440.0, 0.7);
                let x = oscs[0].next(f) + oscs[1].next(f * 1.007) + oscs[2].next(f * 0.995);
                let env = smoothstep(0.0, 0.55, t) * (1.0 - smoothstep(0.62, 0.8, t));
                lp.lowpass(x * 0.4, sweep(t, 300.0, 6000.0, 0.7), 1.2) * env
            });
            let air = whoosh(r, 0.7, 300.0, 8000.0, 0.9);
            let ping = fm(0.6, 1760.0, 1760.0, 2.0, 1.5, 0.1, 0.001, 0.2);
            let mut b = layer(&[(&swell, 0.8, 0.0), (&air, 0.5, 0.0), (&ping, 0.5, 0.62)]);
            reverb(&mut b, 0.6, 0.4, 0.25, 0.5);
            b
        }
        Downed => {
            let mut fall = tone(0.9, Wave::Square, 330.0, 70.0, 0.7, 0.002, 0.35);
            lowpass(&mut fall, 2200.0);
            stutter(&mut fall, 12.0, 0.15);
            let hit = thump(0.5, 120.0, 40.0, 0.03, 0.12);
            let mut b = layer(&[(&fall, 0.7, 0.0), (&hit, 1.0, 0.0)]);
            crush(&mut b, 7, 2.0);
            reverb(&mut b, 0.5, 0.5, 0.15, 0.3);
            b
        }
        Revive => {
            let arp = notes(
                &[(57.0, 0.0, 0.05), (64.0, 0.06, 0.05), (69.0, 0.12, 0.05), (76.0, 0.18, 0.3)],
                Wave::Square,
                0.002,
                0.12,
                4000.0,
                0.006,
            );
            let shimmer = notes(&[(88.0, 0.18, 0.3)], Wave::Triangle, 0.01, 0.15, 8000.0, 0.01);
            let mut b = layer(&[(&arp, 1.0, 0.0), (&shimmer, 0.3, 0.0)]);
            echo(&mut b, 0.09, 0.3, 0.3, 3000.0, 0.3);
            b
        }
        NoStamina => {
            let mut buzz = notes(&[(39.0, 0.0, 0.06), (39.0, 0.09, 0.06)], Wave::Square, 0.002, 0.02, 1800.0, 0.0);
            drive(&mut buzz, 2.0);
            buzz
        }

        // ----- weapons --------------------------------------------------------------------
        LancetFire => {
            let shot = Gun { body_hz: 120.0, body_decay: 0.045, crack_hz: 2400.0, crack_decay: 0.022, tail_hz: 2500.0, tail_decay: 0.06 }.render(r);
            let zap = tone(0.06, Wave::Square, 2400.0, 500.0, 0.04, 0.0005, 0.018);
            let mut b = layer(&[(&shot, 1.0, 0.0), (&zap, 0.25, 0.0)]);
            drive(&mut b, 1.8);
            crush(&mut b, 12, 1.0);
            b
        }
        LancetCharge => {
            let dur = 1.1;
            let mut saw = Osc::new(Wave::Saw);
            let mut sine = Osc::new(Wave::Sine);
            let mut trem = Osc::new(Wave::Sine);
            let mut lp = Svf::default();
            let whine = render(dur, |t, _| {
                let f = sweep(t, 180.0, 1400.0, 1.0);
                let x = saw.next(f) * 0.5 + sine.next(f * 2.0) * 0.3;
                let am = 0.7 + 0.3 * trem.next(lerp(6.0, 30.0, t / dur));
                let env = smoothstep(0.0, 0.9, t) * (1.0 - smoothstep(1.0, dur, t));
                lp.lowpass(x, sweep(t, 600.0, 7000.0, 1.0), 1.5) * am * env
            });
            let mut sizzle = crackle(r, dur, 40.0, -2.0);
            for (i, s) in sizzle.iter_mut().enumerate() {
                *s *= smoothstep(0.2, 1.0, i as f32 / SR);
            }
            layer(&[(&whine, 1.0, 0.0), (&sizzle, 0.2, 0.0)])
        }
        LancetBeam => {
            let zap = fm(0.8, 1100.0, 240.0, 1.5, 6.0, 0.5, 0.001, 0.22);
            let body = thump(0.4, 180.0, 45.0, 0.02, 0.11);
            let crack = noise(r, 0.25, Pass::Band, 3000.0, 800.0, 0.8, 0.0005, 0.07);
            let snap = click(r, 0.003);
            let mut b = layer(&[(&zap, 0.8, 0.0), (&body, 1.0, 0.0), (&crack, 0.7, 0.0), (&snap, 0.7, 0.0)]);
            drive(&mut b, 2.0);
            echo(&mut b, 0.09, 0.35, 0.3, 2500.0, 0.4);
            b
        }
        LancetFan => {
            let mut b = Vec::new();
            for (k, (gain, at)) in [(1.0, 0.0), (0.85, 0.028), (0.75, 0.056)].into_iter().enumerate() {
                let shot = Gun {
                    body_hz: 125.0 + k as f32 * 12.0,
                    body_decay: 0.04,
                    crack_hz: 2600.0 + k as f32 * 300.0,
                    crack_decay: 0.02,
                    tail_hz: 2600.0,
                    tail_decay: 0.05,
                }
                .render(r);
                mix_at(&mut b, &shot, gain, at);
            }
            drive(&mut b, 2.0);
            crush(&mut b, 12, 1.0);
            b
        }
        ThresherFire => {
            let snap = click(r, 0.004);
            let body = thump(0.4, 160.0, 42.0, 0.018, 0.09);
            let blast = noise(r, 0.45, Pass::Low, 7000.0, 700.0, 0.7, 0.0005, 0.085);
            let crunch = noise(r, 0.2, Pass::Band, 900.0, 700.0, 1.2, 0.0005, 0.045);
            let room = noise(r, 0.6, Pass::Low, 1200.0, 300.0, 0.7, 0.01, 0.2);
            let mut b = layer(&[(&snap, 0.8, 0.0), (&body, 1.0, 0.0), (&blast, 0.9, 0.0), (&crunch, 0.6, 0.0), (&room, 0.25, 0.0)]);
            drive(&mut b, 3.0);
            crush(&mut b, 11, 1.5);
            reverb(&mut b, 0.3, 0.5, 0.12, 0.3);
            b
        }
        ThresherSlug => {
            let snap = click(r, 0.003);
            let body = thump(0.6, 120.0, 32.0, 0.025, 0.14);
            let crack = noise(r, 0.15, Pass::Band, 1600.0, 1000.0, 1.0, 0.0003, 0.03);
            let tail = noise(r, 0.6, Pass::Low, 900.0, 300.0, 0.7, 0.002, 0.18);
            let casing = metal(0.15, 3100.0, &[(1.0, 1.0, 0.03), (1.71, 0.6, 0.02)]);
            let mut b = layer(&[(&snap, 0.9, 0.0), (&body, 1.1, 0.0), (&crack, 0.9, 0.0), (&tail, 0.5, 0.0), (&casing, 0.08, 0.28)]);
            drive(&mut b, 3.5);
            echo(&mut b, 0.12, 0.3, 0.22, 1500.0, 0.4);
            b
        }
        FlakLaunch => {
            let hollow = tone(0.3, Wave::Triangle, 340.0, 110.0, 0.08, 0.001, 0.07);
            let puff = noise(r, 0.25, Pass::Low, 1500.0, 400.0, 0.8, 0.001, 0.05);
            let push = thump(0.25, 180.0, 70.0, 0.015, 0.06);
            let clink = metal(0.15, 2900.0, &[(1.0, 1.0, 0.04), (1.37, 0.5, 0.03)]);
            layer(&[(&hollow, 0.8, 0.0), (&puff, 0.8, 0.0), (&push, 0.8, 0.0), (&clink, 0.12, 0.04)])
        }
        RiveterFire => {
            let hiss = noise(r, 0.06, Pass::High, 2500.0, 2500.0, 0.7, 0.0003, 0.011);
            let base = r.range(3100.0, 3500.0);
            let tick = metal(0.06, base, &[(1.0, 0.8, 0.01), (1.41, 0.5, 0.008), (2.13, 0.4, 0.006)]);
            let punch = thump(0.08, r.range(300.0, 340.0), 140.0, 0.006, 0.02);
            let air = noise(r, 0.1, Pass::Band, 6000.0, 4000.0, 0.7, 0.001, 0.025);
            let mut b = layer(&[(&hiss, 0.8, 0.0), (&tick, 0.4, 0.0), (&punch, 0.9, 0.0), (&air, 0.3, 0.0)]);
            drive(&mut b, 2.0);
            b
        }
        SawLaunch => {
            let mut car = Osc::new(Wave::Sine);
            let mut modu = Osc::new(Wave::Sine);
            let mut teeth = Osc::new(Wave::Triangle);
            let whir = render(0.6, |t, _| {
                let f = sweep(t, 950.0, 820.0, 0.6);
                let m = modu.next(f * 2.76) * 2.0;
                let am = 0.55 + 0.45 * teeth.next(38.0);
                car.next_pm(f, m / std::f32::consts::TAU) * am * perc(t, 0.01, 0.3)
            });
            let push = thump(0.2, 200.0, 80.0, 0.012, 0.05);
            let snap = click(r, 0.003);
            layer(&[(&whir, 0.7, 0.0), (&push, 0.9, 0.0), (&snap, 0.5, 0.0)])
        }
        SawBounce => {
            let ping = metal(0.3, 1900.0, &[(1.0, 1.0, 0.08), (2.21, 0.7, 0.06), (3.6, 0.5, 0.04), (5.1, 0.3, 0.03)]);
            let snap = click(r, 0.002);
            let whir = fm(0.2, 900.0, 800.0, 2.76, 1.5, 1.0, 0.005, 0.07);
            layer(&[(&ping, 0.8, 0.0), (&snap, 0.6, 0.0), (&whir, 0.3, 0.01)])
        }
        VolleyFire => {
            let mut b = Vec::new();
            for _ in 0..8 {
                let base = r.range(2800.0, 3800.0);
                let tick = metal(0.05, base, &[(1.0, 0.8, 0.008), (1.41, 0.5, 0.006)]);
                let pop = thump(0.06, r.range(280.0, 360.0), 140.0, 0.005, 0.015);
                let at = r.range(0.0, 0.07);
                mix_at(&mut b, &tick, 0.3, at);
                mix_at(&mut b, &pop, 0.5, at);
            }
            let burst = noise(r, 0.25, Pass::Band, 3000.0, 1500.0, 0.8, 0.002, 0.06);
            let push = thump(0.2, 170.0, 60.0, 0.012, 0.05);
            mix_at(&mut b, &burst, 0.8, 0.0);
            mix_at(&mut b, &push, 0.9, 0.0);
            drive(&mut b, 2.0);
            b
        }
        CinderFire => {
            let body = thump(0.5, 140.0, 45.0, 0.02, 0.11);
            let puff = noise(r, 0.3, Pass::Low, 2500.0, 500.0, 0.8, 0.0005, 0.06);
            let rocket = noise(r, 0.5, Pass::Band, 1200.0, 2400.0, 1.5, 0.02, 0.18);
            let mut rumble = tone(0.4, Wave::Saw, 90.0, 70.0, 0.4, 0.005, 0.15);
            lowpass(&mut rumble, 400.0);
            let mut b = layer(&[(&body, 1.0, 0.0), (&puff, 0.8, 0.0), (&rocket, 0.5, 0.0), (&rumble, 0.4, 0.0)]);
            drive(&mut b, 2.5);
            b
        }
        StickyArm => {
            let squish = noise(r, 0.12, Pass::Low, 600.0, 250.0, 5.0, 0.001, 0.035);
            let beep = notes(&[(93.0, 0.08, 0.035), (93.0, 0.16, 0.035)], Wave::Square, 0.001, 0.01, 6000.0, 0.0);
            layer(&[(&squish, 1.0, 0.0), (&beep, 0.35, 0.0)])
        }
        StickyDetonate => {
            let snap = click(r, 0.002);
            let beep = tone(0.12, Wave::Square, 1500.0, 3000.0, 0.08, 0.001, 0.05);
            layer(&[(&snap, 0.7, 0.0), (&beep, 0.5, 0.0)])
        }
        MortarFire => {
            let body = thump(0.7, 110.0, 35.0, 0.03, 0.15);
            let tube = tone(0.4, Wave::Triangle, 200.0, 120.0, 0.1, 0.001, 0.1);
            let puff = noise(r, 0.4, Pass::Low, 2000.0, 300.0, 0.8, 0.0005, 0.08);
            let mut whistle_osc = Osc::new(Wave::Sine);
            let whistle = render(1.0, |t, _| whistle_osc.next(sweep(t, 2200.0, 1500.0, 1.0)) * bell(t, 1.0) * 0.5);
            let mut b = layer(&[(&body, 1.0, 0.0), (&tube, 0.6, 0.0), (&puff, 0.8, 0.0), (&whistle, 0.12, 0.05)]);
            drive(&mut b, 2.2);
            b
        }
        Explosion => {
            let crack = noise(r, 0.05, Pass::High, 1000.0, 1000.0, 0.6, 0.0002, 0.008);
            let boom = thump(2.0, 90.0, 28.0, 0.06, 0.42);
            let body = noise(r, 1.4, Pass::Low, 6000.0, 250.0, 0.8, 0.001, 0.22);
            let rumble = noise(r, 2.0, Pass::Low, 180.0, 80.0, 1.0, 0.01, 0.55);
            let debris = crackle(r, 1.6, 120.0, 0.4);
            let mut b = layer(&[(&crack, 1.0, 0.0), (&boom, 1.2, 0.0), (&body, 1.0, 0.0), (&rumble, 0.8, 0.0), (&debris, 0.5, 0.01)]);
            drive(&mut b, 2.2);
            crush(&mut b, 10, 2.0);
            reverb(&mut b, 0.6, 0.6, 0.18, 0.6);
            b
        }
        WeaponSwitch => {
            let first = metal(0.06, 2200.0, &[(1.0, 1.0, 0.012), (1.6, 0.6, 0.009)]);
            let slide = noise(r, 0.1, Pass::Band, 1800.0, 2400.0, 2.0, 0.02, 0.03);
            let clack = metal(0.08, 1500.0, &[(1.0, 1.0, 0.018), (2.2, 0.5, 0.012)]);
            let thud = thump(0.08, 260.0, 160.0, 0.008, 0.02);
            layer(&[(&first, 0.6, 0.0), (&slide, 0.5, 0.03), (&clack, 0.7, 0.11), (&thud, 0.6, 0.11)])
        }
        ModeSwitch => {
            let mut servo = tone(0.08, Wave::Square, 500.0, 900.0, 0.06, 0.002, 0.03);
            lowpass(&mut servo, 2000.0);
            let snap = metal(0.05, 3000.0, &[(1.0, 1.0, 0.01), (1.5, 0.5, 0.007)]);
            layer(&[(&servo, 0.6, 0.0), (&snap, 0.6, 0.07)])
        }
        Punch => {
            let air = whoosh(r, 0.16, 600.0, 2200.0, 1.0);
            let thud = thump(0.1, 170.0, 80.0, 0.01, 0.03);
            layer(&[(&air, 1.0, 0.0), (&thud, 0.4, 0.06)])
        }
        PunchHit => {
            let body = thump(0.3, 190.0, 50.0, 0.012, 0.06);
            let crunch = noise(r, 0.15, Pass::Low, 4000.0, 700.0, 0.8, 0.0005, 0.03);
            let snap = click(r, 0.003);
            let clang = metal(0.2, 700.0, &[(1.0, 1.0, 0.05), (2.3, 0.6, 0.035), (3.7, 0.4, 0.02)]);
            let mut b = layer(&[(&body, 1.0, 0.0), (&crunch, 0.8, 0.0), (&snap, 0.6, 0.0), (&clang, 0.25, 0.0)]);
            drive(&mut b, 4.0);
            crush(&mut b, 9, 2.0);
            b
        }
        Parry => {
            // Bright inharmonic clang with a long ring, a fast whoosh and a beating high shimmer:
            // nothing else in the game sounds like it.
            let snap = noise(r, 0.03, Pass::High, 3000.0, 3000.0, 0.6, 0.0001, 0.003);
            let clang = metal(
                1.6,
                1046.5,
                &[
                    (1.0, 1.0, 0.55),
                    (1.507, 0.5, 0.6),
                    (2.76, 0.8, 0.4),
                    (5.40, 0.55, 0.28),
                    (8.93, 0.35, 0.18),
                    (13.34, 0.2, 0.1),
                ],
            );
            let ring = {
                let (mut a, mut b) = (Osc::new(Wave::Sine), Osc::new(Wave::Sine));
                render(1.2, |t, _| (a.next(3520.0) + b.next(3528.0)) * 0.5 * perc(t, 0.001, 0.6))
            };
            let air = whoosh(r, 0.25, 5000.0, 900.0, 1.0);
            let body = thump(0.2, 300.0, 120.0, 0.01, 0.05);
            let mut b = layer(&[(&snap, 0.9, 0.0), (&clang, 0.9, 0.0), (&ring, 0.25, 0.0), (&air, 0.7, 0.0), (&body, 0.5, 0.0)]);
            for s in b.iter_mut() {
                *s = super::dsp::soft_clip(*s * 1.5);
            }
            reverb(&mut b, 0.7, 0.3, 0.25, 0.6);
            b
        }
        PrismThrow => {
            let glass = fm(0.45, 2637.0, 2637.0, 3.5, 3.0, 0.2, 0.002, 0.15);
            let glass2 = fm(0.4, 3951.0, 3951.0, 3.5, 2.5, 0.2, 0.002, 0.12);
            let air = whoosh(r, 0.3, 1500.0, 5000.0, 1.0);
            let mut b = layer(&[(&glass, 0.6, 0.0), (&glass2, 0.4, 0.04), (&air, 0.8, 0.0)]);
            echo(&mut b, 0.08, 0.3, 0.25, 5000.0, 0.3);
            b
        }
        Ricochet => {
            let start = r.range(5000.0, 5400.0);
            let mut osc = Osc::new(Wave::Sine);
            let mut wob = Osc::new(Wave::Sine);
            let zing = render(0.5, |t, _| {
                let f = glide(t, start, 2300.0, 0.12) * (1.0 + 0.015 * wob.next(40.0));
                osc.next(f) * perc(t, 0.002, 0.15)
            });
            let ping = metal(0.3, 3100.0, &[(1.0, 1.0, 0.1), (1.59, 0.5, 0.07), (2.31, 0.4, 0.05)]);
            let snap = click(r, 0.002);
            let mut b = layer(&[(&zing, 0.8, 0.0), (&ping, 0.4, 0.0), (&snap, 0.6, 0.0)]);
            echo(&mut b, 0.07, 0.3, 0.25, 4000.0, 0.3);
            b
        }
        Hitmarker => {
            let tick = tone(0.05, Wave::Sine, 2600.0, 2600.0, 0.01, 0.0005, 0.014);
            let snap = noise(r, 0.02, Pass::High, 6000.0, 6000.0, 0.6, 0.0001, 0.002);
            let edge = tone(0.03, Wave::Square, 5200.0, 5200.0, 0.01, 0.0005, 0.006);
            layer(&[(&tick, 1.0, 0.0), (&snap, 0.6, 0.0), (&edge, 0.12, 0.0)])
        }
        Headshot => {
            let a = tone(0.05, Wave::Sine, 3200.0, 3200.0, 0.01, 0.0005, 0.014);
            let b2 = tone(0.07, Wave::Sine, 4300.0, 4300.0, 0.01, 0.0005, 0.02);
            let splat = noise(r, 0.15, Pass::Band, 1500.0, 800.0, 1.0, 0.0005, 0.03);
            let thud = thump(0.12, 250.0, 120.0, 0.01, 0.03);
            let mut b = layer(&[(&a, 0.8, 0.0), (&b2, 0.8, 0.03), (&splat, 0.6, 0.0), (&thud, 0.6, 0.0)]);
            crush(&mut b, 9, 1.5);
            b
        }
        KillConfirm => {
            let thud = thump(0.15, 200.0, 80.0, 0.01, 0.035);
            let a = fm(0.25, 1568.0, 1568.0, 2.0, 1.5, 0.2, 0.001, 0.1);
            let b2 = fm(0.35, 2093.0, 2093.0, 2.0, 1.5, 0.2, 0.001, 0.14);
            layer(&[(&thud, 0.7, 0.0), (&a, 0.5, 0.0), (&b2, 0.5, 0.06)])
        }

        // ----- enemies (the Moulds) ---------------------------------------------------------
        GnawerAlert => {
            // A chittering snarl: a jittery growl chopped into a chatter, brass teeth clicking.
            let f0 = r.range(140.0, 170.0);
            let mut snarl = growl(r, 0.6, f0, 110.0, [850.0, 1600.0], 0.12, 0.02, 0.25);
            stutter(&mut snarl, 26.0, 0.05);
            let clicks = crackle(r, 0.55, 70.0, 0.3);
            let mut b = layer(&[(&snarl, 1.0, 0.0), (&clicks, 0.6, 0.04)]);
            drive(&mut b, 2.5);
            b
        }
        GnawerBiteWindup => {
            // The jaw unhinging: a rising chitter that ends as the bite lands.
            let dur = 0.42;
            let mut chitter = growl(r, dur, 180.0, 560.0, [1000.0, 2100.0], 0.06, 0.25, 0.2);
            stutter(&mut chitter, 34.0, 0.0);
            let hiss = noise(r, dur, Pass::Band, 1500.0, 4200.0, 1.4, 0.3, 0.15);
            let mut b = layer(&[(&chitter, 1.0, 0.0), (&hiss, 0.35, 0.0)]);
            drive(&mut b, 2.0);
            b
        }
        GnawerBite => {
            // Brass teeth slamming shut.
            let clack = metal(0.12, 1900.0, &[(1.0, 1.0, 0.03), (1.48, 0.7, 0.025), (2.9, 0.4, 0.015)]);
            let snap = click(r, 0.003);
            let thud = thump(0.15, 220.0, 90.0, 0.01, 0.04);
            let grunt = growl(r, 0.12, 160.0, 120.0, [700.0, 1200.0], 0.05, 0.004, 0.04);
            layer(&[(&clack, 0.8, 0.0), (&snap, 0.8, 0.0), (&thud, 0.7, 0.0), (&grunt, 0.4, 0.0)])
        }
        GnawerPounceWindup => {
            // Coiling: a low growl swelling, claws scraping the floor.
            let dur = 0.56;
            let growl_ = growl(r, dur, 80.0, 150.0, [500.0, 900.0], 0.1, 0.35, 0.25);
            let scrape = noise(r, dur, Pass::Band, 2500.0, 1200.0, 3.0, 0.1, 0.3);
            let mut b = layer(&[(&growl_, 1.0, 0.0), (&scrape, 0.4, 0.05)]);
            drive(&mut b, 2.2);
            b
        }
        GnawerPounce => {
            let air = whoosh(r, 0.35, 900.0, 3000.0, 1.0);
            let snarl = growl(r, 0.3, 220.0, 320.0, [900.0, 1700.0], 0.1, 0.01, 0.12);
            let push = thump(0.2, 140.0, 60.0, 0.02, 0.05);
            layer(&[(&air, 1.0, 0.0), (&snarl, 0.6, 0.0), (&push, 0.6, 0.0)])
        }
        CenserAlert => {
            // The brazier flaring up: a breathy whoomph, embers crackling, a low drone.
            let dur = 0.9;
            let mut n = Rng::new(r.next_u64());
            let mut lp = Svf::default();
            let whoomph = render(dur, |t, _| {
                let env = smoothstep(0.0, 0.12, t) * (-(t - 0.12).max(0.0) / 0.25).exp();
                lp.lowpass(n.bipolar(), sweep(t, 300.0, 1400.0, 0.2), 0.8) * env
            });
            let embers = crackle(r, dur, 120.0, 0.4);
            let mut drone = chord(&[38.0, 45.0], dur, Wave::Saw, 300.0, 600.0, 0.15, 0.5, 0.6);
            lowpass(&mut drone, 700.0);
            layer(&[(&whoomph, 1.0, 0.0), (&embers, 0.5, 0.05), (&drone, 0.35, 0.0)])
        }
        CenserWhirl => {
            // The censer whirled overhead: whooshes coming faster and faster, the chain
            // rattling, the embers hissing brighter. Ends as the bombs fly.
            let dur = 0.82;
            let mut n = Rng::new(r.next_u64());
            let mut bp = Svf::default();
            let mut phase = 0.0f32;
            let whirl = render(dur, |t, _| {
                // Revolutions per second rise from 2 to 9.
                phase += sweep(t, 2.0, 9.0, dur) / SR;
                let pulse = (phase * std::f32::consts::TAU).sin().max(0.0).powi(2);
                bp.bandpass(n.bipolar(), 600.0 + 1400.0 * pulse, 1.5) * (0.3 + pulse) * smoothstep(0.0, 0.2, t)
            });
            let rattle = crackle(r, dur, 260.0, 2.0);
            let hiss = noise(r, dur, Pass::High, 3000.0, 6000.0, 0.7, 0.5, 0.2);
            let mut b = layer(&[(&whirl, 1.0, 0.0), (&rattle, 0.45, 0.0), (&hiss, 0.25, 0.0)]);
            fade_out(&mut b, 0.03);
            b
        }
        CenserLob => {
            // A bomb flung up out of the censer: a dull thump and a rising whoosh.
            let thud = thump(0.2, 180.0, 70.0, 0.015, 0.05);
            let up = whoosh(r, 0.3, 700.0, 2200.0, 1.2);
            let clank = metal(0.15, 900.0, &[(1.0, 1.0, 0.05), (2.3, 0.5, 0.03)]);
            layer(&[(&thud, 1.0, 0.0), (&up, 0.7, 0.02), (&clank, 0.25, 0.0)])
        }
        CenserSwingWindup => {
            // Drawing the censer back: the chain rattles taut, the joint creaks.
            let dur = 0.5;
            let rattle = crackle(r, dur, 180.0, 1.0);
            let mut creak = tone(dur, Wave::Saw, 220.0, 160.0, dur, 0.1, 0.25);
            lowpass(&mut creak, 1200.0);
            stutter(&mut creak, 40.0, 0.0);
            layer(&[(&rattle, 0.8, 0.0), (&creak, 0.35, 0.05)])
        }
        CenserSwing => {
            let air = whoosh(r, 0.32, 2400.0, 500.0, 1.3);
            let clang = metal(0.5, 520.0, &[(1.0, 1.0, 0.2), (2.4, 0.6, 0.12), (3.9, 0.3, 0.08)]);
            let rattle = crackle(r, 0.3, 300.0, 0.15);
            layer(&[(&air, 1.0, 0.0), (&clang, 0.3, 0.12), (&rattle, 0.4, 0.0)])
        }
        EmberFlight => {
            // A fuse burning on a lump of slag: crackle and a thin hiss.
            let dur = 0.9;
            let fuse = crackle(r, dur, 400.0, 2.0);
            let hiss = noise(r, dur, Pass::Band, 4500.0, 3000.0, 2.0, 0.05, 0.6);
            layer(&[(&fuse, 0.8, 0.0), (&hiss, 0.4, 0.0)])
        }
        EmberBurst => {
            // An ember bomb bursting: a sharp pop, a spray of sizzling embers, no big boom.
            let pop = thump(0.3, r.range(200.0, 240.0), 60.0, 0.01, 0.07);
            let crack = noise(r, 0.2, Pass::Band, 2500.0, 900.0, 0.9, 0.0003, 0.04);
            let spray = crackle(r, 0.7, 500.0, 0.2);
            let sizzle = noise(r, 0.8, Pass::High, 5000.0, 3000.0, 0.7, 0.01, 0.3);
            let mut b = layer(&[(&pop, 1.0, 0.0), (&crack, 0.8, 0.0), (&spray, 0.5, 0.01), (&sizzle, 0.3, 0.02)]);
            drive(&mut b, 2.0);
            b
        }
        HarrierAlert => {
            // A carrion screech: high formants, a lot of jitter.
            let mut b = growl(r, 0.55, 420.0, 300.0, [1800.0, 3100.0], 0.2, 0.02, 0.2);
            let mut rasp = noise(r, 0.5, Pass::Band, 2800.0, 1800.0, 2.0, 0.02, 0.2);
            stutter(&mut rasp, 18.0, 0.0);
            b = layer(&[(&b, 1.0, 0.0), (&rasp, 0.4, 0.0)]);
            drive(&mut b, 2.5);
            b
        }
        HarrierShriek => {
            // Rearing up to dive: a piercing rising shriek that peaks as it drops.
            let dur = 0.72;
            let scream = growl(r, dur, 380.0, 1100.0, [2000.0, 3600.0], 0.08, 0.3, 0.25);
            let tone_ = fm(dur, 900.0, 2400.0, 1.5, 3.0, 1.0, 0.3, 0.3);
            let mut b = layer(&[(&scream, 1.0, 0.0), (&tone_, 0.35, 0.0)]);
            drive(&mut b, 2.0);
            b
        }
        HarrierDive => {
            // The wind tearing past a falling body.
            let rush = whoosh(r, 0.62, 400.0, 3500.0, 2.0);
            let flap = thump(0.12, 120.0, 70.0, 0.01, 0.03);
            let mut b = layer(&[(&rush, 1.0, 0.0), (&flap, 0.6, 0.0)]);
            highpass(&mut b, 150.0);
            b
        }
        HarrierSwat => {
            // Punched out of the air: wax crunching, a flapping tumble.
            let crunch = noise(r, 0.25, Pass::Low, 3500.0, 600.0, 0.8, 0.0005, 0.05);
            let thud = thump(0.25, 180.0, 60.0, 0.01, 0.07);
            let mut flaps = noise(r, 0.5, Pass::Band, 900.0, 500.0, 1.0, 0.02, 0.25);
            stutter(&mut flaps, 11.0, 0.08);
            let squawk = growl(r, 0.25, 520.0, 300.0, [1800.0, 2900.0], 0.2, 0.01, 0.1);
            layer(&[(&crunch, 0.9, 0.0), (&thud, 1.0, 0.0), (&flaps, 0.5, 0.05), (&squawk, 0.5, 0.02)])
        }
        GaolerAlert => {
            // A deep iron bellow through the grille, the chain shaken.
            let bellow = growl(r, 1.1, 55.0, 48.0, [300.0, 650.0], 0.05, 0.15, 0.5);
            let rattle = crackle(r, 0.8, 200.0, 0.4);
            let mut b = layer(&[(&bellow, 1.0, 0.0), (&rattle, 0.4, 0.1)]);
            drive(&mut b, 2.0);
            reverb(&mut b, 0.6, 0.5, 0.2, 0.5);
            b
        }
        GaolerBashWindup => {
            // The door drawn back: iron grinding up in pitch, then two bright strikes on the
            // bell at 0.65 s, the start of the parry window, and a shimmer until the bash.
            let dur = 1.0;
            let cue = 0.65;
            let mut n = Rng::new(r.next_u64());
            let mut bp = Svf::default();
            let grind = render(dur, |t, _| {
                let env = smoothstep(0.0, cue, t).powf(0.7) * (1.0 - smoothstep(cue, dur, t) * 0.7);
                bp.bandpass(n.bipolar(), sweep(t, 300.0, 1500.0, cue), 6.0) * env
            });
            let mut rumble = {
                let mut o = Osc::new(Wave::Saw);
                render(dur, |t, _| o.next(sweep(t, 45.0, 60.0, cue)) * smoothstep(0.0, cue, t) * (1.0 - smoothstep(cue, dur, t)))
            };
            lowpass(&mut rumble, 250.0);
            let ting = metal(0.5, 2093.0, &[(1.0, 1.0, 0.15), (2.92, 0.5, 0.08), (5.1, 0.3, 0.05)]);
            let shimmer = metal(0.4, 3136.0, &[(1.0, 0.5, 0.25), (1.5, 0.3, 0.2)]);
            let mut b = layer(&[(&grind, 0.8, 0.0), (&rumble, 0.6, 0.0), (&ting, 0.9, cue), (&ting, 0.7, cue + 0.09), (&shimmer, 0.4, cue + 0.12)]);
            drive(&mut b, 1.8);
            b
        }
        GaolerBash => {
            // A cell door slammed into you: a huge iron clang over a boom.
            let boom = thump(1.0, 85.0, 28.0, 0.04, 0.3);
            let clang = metal(1.1, 165.0, &[(1.0, 1.0, 0.45), (2.31, 0.8, 0.3), (3.62, 0.6, 0.22), (5.7, 0.4, 0.12)]);
            let crunch = noise(r, 0.6, Pass::Low, 3000.0, 200.0, 0.8, 0.0005, 0.15);
            let mut b = layer(&[(&boom, 1.2, 0.0), (&clang, 0.55, 0.0), (&crunch, 0.7, 0.0)]);
            drive(&mut b, 2.5);
            reverb(&mut b, 0.6, 0.5, 0.2, 0.6);
            b
        }
        GaolerChain => {
            // The chain swung up to speed, low: heavy links rattling faster, a whirr rising,
            // a red-hot hiss.
            let dur = 0.82;
            let mut n = Rng::new(r.next_u64());
            let mut bp = Svf::default();
            let mut phase = 0.0f32;
            let whirr = render(dur, |t, _| {
                phase += sweep(t, 1.5, 6.0, dur) / SR;
                let pulse = (phase * std::f32::consts::TAU).sin().max(0.0);
                bp.bandpass(n.bipolar(), 250.0 + 600.0 * pulse, 2.0) * (0.3 + pulse) * smoothstep(0.0, 0.25, t)
            });
            let links = crackle(r, dur, 220.0, 3.0);
            let hiss = noise(r, dur, Pass::High, 2500.0, 5000.0, 0.8, 0.4, 0.25);
            let mut b = layer(&[(&whirr, 1.0, 0.0), (&links, 0.6, 0.0), (&hiss, 0.25, 0.0)]);
            fade_out(&mut b, 0.03);
            b
        }
        GaolerSweep => {
            let air = whoosh(r, 0.5, 300.0, 1800.0, 1.5);
            let links = crackle(r, 0.5, 400.0, 0.2);
            let scrape = noise(r, 0.4, Pass::Band, 1800.0, 700.0, 4.0, 0.01, 0.15);
            let thud = thump(0.3, 90.0, 40.0, 0.02, 0.08);
            layer(&[(&air, 1.0, 0.0), (&links, 0.6, 0.0), (&scrape, 0.4, 0.05), (&thud, 0.6, 0.0)])
        }
        GaolerRushWindup => {
            // A snort through the grille, iron feet scraping, a rumble building.
            let dur = 0.9;
            let snort = noise(r, 0.35, Pass::Low, 900.0, 300.0, 1.5, 0.02, 0.1);
            let scrape = noise(r, dur, Pass::Band, 1400.0, 700.0, 3.0, 0.2, 0.4);
            let mut rumble = {
                let mut o = Osc::new(Wave::Saw);
                render(dur, |t, _| o.next(sweep(t, 40.0, 65.0, dur)) * smoothstep(0.0, dur, t))
            };
            lowpass(&mut rumble, 220.0);
            layer(&[(&snort, 1.0, 0.0), (&snort, 0.6, 0.45), (&scrape, 0.4, 0.1), (&rumble, 0.8, 0.0)])
        }
        GaolerRush => {
            // Charging: iron footfalls coming faster.
            let mut b = Vec::new();
            for (k, at) in [0.0f32, 0.18, 0.33, 0.46, 0.57, 0.67].iter().enumerate() {
                let step = thump(0.25, 100.0, 35.0, 0.02, 0.07);
                let clank = metal(0.15, 280.0 + 20.0 * k as f32, &[(1.0, 1.0, 0.06), (2.7, 0.5, 0.04)]);
                mix_at(&mut b, &step, 1.0, *at);
                mix_at(&mut b, &clank, 0.25, *at);
            }
            let rattle = crackle(r, 0.8, 250.0, 1.0);
            mix_at(&mut b, &rattle, 0.4, 0.0);
            b
        }
        GaolerStep => {
            let thud = thump(0.4, r.range(85.0, 100.0), 35.0, 0.03, 0.1);
            let clank = metal(0.25, r.range(260.0, 320.0), &[(1.0, 1.0, 0.1), (2.6, 0.6, 0.07), (4.1, 0.3, 0.04)]);
            let jingle = crackle(r, 0.25, 160.0, 0.08);
            layer(&[(&thud, 1.0, 0.0), (&clank, 0.35, 0.0), (&jingle, 0.3, 0.02)])
        }
        HeronAlert => {
            // A creaking metal call: two hoarse honks through a brass throat.
            let honk = |r: &mut Rng, f: f32| {
                let h = fm(0.22, f, f * 0.9, 1.5, 4.0, 2.0, 0.01, 0.12);
                let rasp = noise(r, 0.22, Pass::Band, f * 3.0, f * 2.0, 3.0, 0.01, 0.1);
                layer(&[(&h, 1.0, 0.0), (&rasp, 0.4, 0.0)])
            };
            let a = honk(r, 420.0);
            let b2 = honk(r, 360.0);
            let mut b = layer(&[(&a, 1.0, 0.0), (&b2, 0.9, 0.26)]);
            drive(&mut b, 2.0);
            b
        }
        HeronLaser => {
            // The aim laser: an electric hum with a whine climbing towards the lock (1.3 s).
            let dur = 1.3;
            let (mut saw, mut sine, mut vib) = (Osc::new(Wave::Saw), Osc::new(Wave::Sine), Osc::new(Wave::Sine));
            let mut bp = Svf::default();
            let hum = render(dur, |t, _| {
                let f = 110.0 * (1.0 + 0.01 * vib.next(7.0));
                let x = saw.next(f) * 0.6 + sine.next(f * 4.0) * 0.4;
                bp.bandpass(x, 700.0, 1.2) * smoothstep(0.0, 0.08, t) * (1.0 - smoothstep(1.22, dur, t))
            });
            let mut whine_osc = Osc::new(Wave::Sine);
            let whine = render(dur, |t, _| {
                whine_osc.next(sweep(t, 900.0, 2600.0, 1.05)) * smoothstep(0.1, 1.05, t) * (1.0 - smoothstep(1.2, dur, t))
            });
            layer(&[(&hum, 1.0, 0.0), (&whine, 0.25, 0.0)])
        }
        HeronLock => {
            // Locked on: a hard mechanical click and a short high double beep.
            let clk = click(r, 0.002);
            let latch = metal(0.08, 3200.0, &[(1.0, 1.0, 0.02), (1.7, 0.6, 0.015)]);
            let beep = tone(0.06, Wave::Square, 2637.0, 2637.0, 0.01, 0.001, 0.05);
            let mut b = layer(&[(&clk, 1.0, 0.0), (&latch, 0.7, 0.0), (&beep, 0.35, 0.03), (&beep, 0.35, 0.11)]);
            lowpass(&mut b, 9000.0);
            b
        }
        HeronNeedle => {
            // The needle leaving: a hard crack and a zip falling away.
            let crack = noise(r, 0.08, Pass::High, 3500.0, 3500.0, 0.7, 0.0002, 0.012);
            let zip = tone(0.25, Wave::Saw, 6000.0, 900.0, 0.2, 0.0005, 0.08);
            let thump_ = thump(0.12, 300.0, 120.0, 0.008, 0.03);
            let mut b = layer(&[(&crack, 1.0, 0.0), (&zip, 0.35, 0.0), (&thump_, 0.6, 0.0)]);
            lowpass(&mut b, 11000.0);
            echo(&mut b, 0.09, 0.25, 0.25, 4000.0, 0.2);
            b
        }
        HeronKickWindup => {
            // A brass joint cocking: a squeaky creak.
            let dur = 0.4;
            let mut creak = tone(dur, Wave::Saw, 380.0, 260.0, dur, 0.05, 0.2);
            highpass(&mut creak, 500.0);
            lowpass(&mut creak, 1600.0);
            stutter(&mut creak, 55.0, 0.0);
            let tick = metal(0.06, 2400.0, &[(1.0, 1.0, 0.02)]);
            layer(&[(&creak, 1.0, 0.0), (&tick, 0.4, 0.3)])
        }
        HeronKick => {
            let air = whoosh(r, 0.22, 2000.0, 600.0, 1.4);
            let thud = thump(0.2, 160.0, 60.0, 0.01, 0.05);
            let clank = metal(0.2, 700.0, &[(1.0, 1.0, 0.08), (2.2, 0.5, 0.05)]);
            layer(&[(&air, 1.0, 0.0), (&thud, 0.8, 0.08), (&clank, 0.3, 0.08)])
        }
        EnemyHurt => {
            // Wax cracking under the hit, a puff of hiss from the heat inside.
            let (crack_hz, snap_hz) = (r.range(2800.0, 3600.0), r.range(1100.0, 1500.0));
            let crack = noise(r, 0.06, Pass::High, crack_hz, 2000.0, 0.8, 0.0003, 0.012);
            let snap = noise(r, 0.15, Pass::Band, snap_hz, 600.0, 1.5, 0.0005, 0.03);
            let thud = thump(0.12, r.range(150.0, 190.0), 70.0, 0.01, 0.03);
            let hiss = noise(r, 0.25, Pass::High, 4500.0, 3500.0, 0.7, 0.01, 0.08);
            layer(&[(&crack, 0.8, 0.0), (&snap, 0.7, 0.0), (&thud, 0.8, 0.0), (&hiss, 0.25, 0.02)])
        }
        EnemyDeath => {
            // A Mould gives out: wax splitting, a slag thud as it slumps, a long sizzle as it
            // melts.
            let split_hz = r.range(1500.0, 2000.0);
            let split = noise(r, 0.3, Pass::Band, split_hz, 500.0, 1.2, 0.0005, 0.07);
            let thud = thump(0.45, r.range(110.0, 140.0), 38.0, 0.02, 0.12);
            let creak = growl(r, 0.5, 120.0, 60.0, [400.0, 800.0], 0.08, 0.02, 0.22);
            let sizzle = noise(r, 1.2, Pass::High, 4000.0, 2500.0, 0.7, 0.05, 0.45);
            let pops = crackle(r, 1.0, 90.0, 0.5);
            let mut b = layer(&[(&split, 0.8, 0.0), (&thud, 1.0, 0.05), (&creak, 0.35, 0.0), (&sizzle, 0.35, 0.1), (&pops, 0.4, 0.1)]);
            drive(&mut b, 1.8);
            reverb(&mut b, 0.4, 0.6, 0.12, 0.3);
            b
        }
        GaolerDeath => {
            // The jailer falls: the door crashes down, iron collapses, the lantern hisses out.
            let boom = thump(1.2, 70.0, 24.0, 0.05, 0.35);
            let crash = metal(1.4, 140.0, &[(1.0, 1.0, 0.6), (2.2, 0.8, 0.4), (3.5, 0.5, 0.3), (6.1, 0.3, 0.15)]);
            let chain = crackle(r, 1.0, 300.0, 0.4);
            let hiss = noise(r, 1.6, Pass::High, 4000.0, 2000.0, 0.7, 0.2, 0.6);
            let mut b = layer(&[(&boom, 1.2, 0.0), (&crash, 0.5, 0.05), (&chain, 0.5, 0.1), (&hiss, 0.35, 0.3)]);
            drive(&mut b, 2.2);
            reverb(&mut b, 0.7, 0.5, 0.22, 0.7);
            b
        }
        EmberRelease => {
            // The heat leaving a dead Mould: a soft whoomph and a fountain of crackling embers.
            let dur = 1.1;
            let mut n = Rng::new(r.next_u64());
            let mut lp = Svf::default();
            let whoomph = render(dur, |t, _| {
                let env = smoothstep(0.0, 0.06, t) * (-(t - 0.06).max(0.0) / 0.18).exp();
                lp.lowpass(n.bipolar(), sweep(t, 250.0, 2200.0, 0.15), 0.9) * env
            });
            let rise = whoosh(r, 0.8, 500.0, 4000.0, 0.8);
            let embers = crackle(r, dur, 350.0, 0.35);
            layer(&[(&whoomph, 1.0, 0.0), (&rise, 0.4, 0.05), (&embers, 0.6, 0.05)])
        }
        Gib => {
            let mut b = Vec::new();
            let blobs = 5 + (r.f32() * 3.0) as usize;
            for k in 0..blobs {
                let f0 = r.range(600.0, 2000.0);
                let (q, decay) = (r.range(5.0, 9.0), r.range(0.025, 0.055));
                let blob = noise(r, 0.12, Pass::Low, f0, f0 / 3.0, q, 0.001, decay);
                let at = if k == 0 { 0.0 } else { r.range(0.0, 0.35) };
                mix_at(&mut b, &blob, r.range(0.5, 1.0), at);
            }
            let thud = thump(0.2, 140.0, 60.0, 0.01, 0.05);
            mix_at(&mut b, &thud, 0.7, 0.0);
            b
        }
        EnemySpawn => {
            let dur = 0.9;
            let mut n = Rng::new(r.next_u64());
            let mut lp = Svf::default();
            let swell = render(dur, |t, _| {
                let env = (t / 0.6).min(1.0).powi(2) * (1.0 - smoothstep(0.6, 0.66, t));
                lp.lowpass(n.bipolar(), sweep(t, 200.0, 6000.0, 0.6), 1.5) * env
            });
            let mut drone = chord(&[33.0, 40.0], dur, Wave::Saw, 200.0, 800.0, 0.3, 0.5, 0.6);
            lowpass(&mut drone, 900.0);
            let zap = fm(0.3, 900.0, 200.0, 1.41, 4.0, 0.5, 0.001, 0.08);
            let thud = thump(0.3, 140.0, 50.0, 0.02, 0.07);
            layer(&[(&swell, 0.6, 0.0), (&drone, 0.4, 0.0), (&zap, 0.6, 0.6), (&thud, 0.8, 0.6)])
        }
        ProjectileImpact => {
            let splash_hz = r.range(1400.0, 1800.0);
            let splash = noise(r, 0.18, Pass::Band, splash_hz, 700.0, 1.0, 0.0005, 0.04);
            let thud = thump(0.15, 180.0, 70.0, 0.01, 0.04);
            let squelch = noise(r, 0.12, Pass::Low, 1200.0, 300.0, 5.0, 0.001, 0.03);
            layer(&[(&splash, 1.0, 0.0), (&thud, 0.7, 0.0), (&squelch, 0.4, 0.0)])
        }

        // ----- game -----------------------------------------------------------------------
        RankUp => {
            // A-major arpeggio up an octave, last note held with vibrato plus a third on top.
            let arp = [(69.0, 0.0, 0.05), (73.0, 0.055, 0.05), (76.0, 0.11, 0.05), (81.0, 0.165, 0.45)];
            let lead = notes(&arp, Wave::Saw, 0.002, 0.15, 4500.0, 0.008);
            let square = notes(&arp, Wave::Square, 0.002, 0.15, 3000.0, 0.008);
            let third = notes(&[(85.0, 0.165, 0.45)], Wave::Square, 0.01, 0.15, 3500.0, 0.008);
            let shimmer = noise(r, 0.6, Pass::High, 6000.0, 6000.0, 0.6, 0.002, 0.25);
            // A saw's fundamental is in anti-phase with a square's: invert the square so the
            // layers reinforce instead of cancelling.
            let mut b = layer(&[(&lead, 0.6, 0.0), (&square, -0.35, 0.0), (&third, 0.3, 0.0), (&shimmer, 0.15, 0.165)]);
            echo(&mut b, 0.11, 0.3, 0.3, 4000.0, 0.4);
            b
        }
        RankDown => {
            let mut osc = Osc::new(Wave::Triangle);
            let mut b = render(0.45, |t, _| {
                let note = if t < 0.12 { 76.0 } else { 72.0 - (t - 0.12) * 1.5 };
                osc.next(midi_hz(note)) * adsr(t, 0.004, 0.05, 0.7, 0.12, 0.3)
            });
            lowpass(&mut b, 2000.0);
            b
        }
        StyleBonus => {
            let a = fm(0.15, 1975.5, 1975.5, 2.0, 1.2, 0.1, 0.001, 0.06);
            let b2 = fm(0.25, 2637.0, 2637.0, 2.0, 1.2, 0.1, 0.001, 0.08);
            layer(&[(&a, 0.7, 0.0), (&b2, 0.7, 0.05)])
        }
        DoorLock => {
            let clank = metal(0.8, 160.0, &[(1.0, 1.0, 0.25), (2.33, 0.7, 0.18), (3.71, 0.5, 0.12), (5.9, 0.3, 0.08)]);
            let thud = thump(0.4, 100.0, 40.0, 0.02, 0.1);
            let mut alarm = notes(&[(57.0, 0.15, 0.13), (57.0, 0.4, 0.13)], Wave::Square, 0.003, 0.03, 1500.0, 0.0);
            crush(&mut alarm, 7, 2.0);
            layer(&[(&clank, 0.6, 0.0), (&thud, 1.0, 0.0), (&alarm, 0.35, 0.0)])
        }
        DoorOpen => {
            let hiss = noise(r, 0.6, Pass::Band, 4000.0, 3000.0, 0.8, 0.05, 0.25);
            let mut whir = tone(0.7, Wave::Saw, 120.0, 240.0, 0.6, 0.05, 0.4);
            lowpass(&mut whir, 900.0);
            let clunk = thump(0.3, 130.0, 50.0, 0.015, 0.07);
            let clank = metal(0.3, 400.0, &[(1.0, 1.0, 0.08), (2.5, 0.5, 0.05)]);
            layer(&[(&hiss, 0.5, 0.0), (&whir, 0.5, 0.0), (&clunk, 1.0, 0.65), (&clank, 0.3, 0.65)])
        }
        Checkpoint => {
            let mut b = Vec::new();
            for (k, note) in [88.0, 92.0, 95.0].into_iter().enumerate() {
                let bell_tone = fm(0.7, midi_hz(note), midi_hz(note), 3.0, 1.5, 0.1, 0.001, 0.3);
                mix_at(&mut b, &bell_tone, 0.6, k as f32 * 0.08);
            }
            reverb(&mut b, 0.6, 0.4, 0.25, 0.5);
            b
        }
        WaveStart => {
            let horn = chord(&[40.0, 47.0, 52.0], 1.0, Wave::Saw, 3000.0, 600.0, 0.01, 0.4, 0.8);
            let hit = thump(0.5, 120.0, 40.0, 0.02, 0.12);
            let crash = noise(r, 1.0, Pass::High, 5000.0, 5000.0, 0.6, 0.001, 0.35);
            let mut b = layer(&[(&horn, 1.0, 0.0), (&hit, 1.0, 0.0), (&crash, 0.3, 0.0)]);
            drive(&mut b, 3.0);
            b
        }
        EncounterClear => {
            let swell = chord(&[52.0, 56.0, 59.0, 64.0], 1.4, Wave::Saw, 800.0, 4000.0, 0.05, 0.7, 1.1);
            let top = fm(1.0, midi_hz(88.0), midi_hz(88.0), 3.0, 1.5, 0.1, 0.002, 0.35);
            let crash = noise(r, 1.2, Pass::High, 5000.0, 5000.0, 0.6, 0.002, 0.4);
            let mut b = layer(&[(&swell, 1.0, 0.0), (&top, 0.35, 0.05), (&crash, 0.2, 0.0)]);
            reverb(&mut b, 0.6, 0.4, 0.2, 0.5);
            b
        }
        LevelComplete => {
            let run = notes(
                &[(64.0, 0.0, 0.1), (68.0, 0.12, 0.1), (71.0, 0.24, 0.1), (76.0, 0.36, 0.1)],
                Wave::Square,
                0.003,
                0.06,
                4000.0,
                0.0,
            );
            let fin = chord(&[76.0, 80.0, 83.0, 88.0], 1.4, Wave::Saw, 1500.0, 5000.0, 0.01, 0.6, 1.0);
            let crash = noise(r, 1.2, Pass::High, 5000.0, 5000.0, 0.6, 0.002, 0.45);
            let hit = thump(0.4, 120.0, 45.0, 0.02, 0.1);
            let mut b = layer(&[(&run, 0.6, 0.0), (&fin, 0.9, 0.48), (&crash, 0.25, 0.48), (&hit, 0.8, 0.48)]);
            reverb(&mut b, 0.6, 0.4, 0.2, 0.6);
            b
        }
        Countdown => notes(&[(81.0, 0.0, 0.11)], Wave::Square, 0.002, 0.02, 3000.0, 0.0),
        MatchStart => {
            let beep = notes(&[(93.0, 0.0, 0.35)], Wave::Square, 0.002, 0.05, 3500.0, 0.0);
            let stab = chord(&[45.0, 52.0, 57.0], 0.8, Wave::Saw, 3000.0, 700.0, 0.005, 0.3, 0.6);
            let crash = noise(r, 0.8, Pass::High, 5000.0, 5000.0, 0.6, 0.001, 0.3);
            let mut b = layer(&[(&beep, 0.4, 0.0), (&stab, 1.0, 0.0), (&crash, 0.25, 0.0)]);
            drive(&mut b, 2.0);
            b
        }
        MatchEnd => {
            let mut horn = chord(&[45.0, 52.0, 57.0, 60.0], 1.6, Wave::Saw, 2500.0, 500.0, 0.01, 0.7, 1.2);
            // Droop the whole chord down a semitone by resampling.
            let len = horn.len();
            let drooped: Vec<f32> = (0..len)
                .map(|i| {
                    let t = i as f32 / len as f32;
                    let src = i as f32 * (1.0 - 0.06 * t * 0.5);
                    horn[(src as usize).min(len - 1)]
                })
                .collect();
            horn = drooped;
            let gong = metal(1.8, 110.0, &[(1.0, 1.0, 1.0), (2.14, 0.7, 0.7), (3.47, 0.5, 0.5), (5.03, 0.3, 0.3)]);
            let mut b = layer(&[(&horn, 0.8, 0.0), (&gong, 0.5, 0.0)]);
            reverb(&mut b, 0.7, 0.5, 0.2, 0.6);
            b
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::dsp::{max_window_rms, samples};

    #[test]
    fn all_list_matches_indices() {
        for (i, sfx) in Sfx::ALL.iter().enumerate() {
            assert_eq!(sfx.index(), i, "{sfx:?} out of order in Sfx::ALL");
        }
    }

    #[test]
    fn every_sfx_renders_clean_audio() {
        for &sfx in Sfx::ALL {
            for v in 0..sfx.info().variants {
                let buf = render_variant(sfx, v);
                let secs = buf.len() as f32 / SR;
                assert!((0.03..=4.0).contains(&secs), "{sfx:?} duration {secs}");
                assert!(buf.iter().all(|s| s.is_finite()), "{sfx:?} has NaN/inf");
                let p = peak(&buf);
                assert!(p <= 1.0, "{sfx:?} clips: peak {p}");
                let loud = max_window_rms(&buf, samples(0.05));
                assert!(loud > db_to_gain(-40.0), "{sfx:?} too quiet: {loud}");
                let measured = loudness(&buf, SR, 0.05);
                assert!(measured <= sfx.info().level_db + 0.5, "{sfx:?} louder than its target");
            }
        }
    }

    #[test]
    fn generation_is_deterministic() {
        for &sfx in &[Sfx::Explosion, Sfx::Parry, Sfx::Gib, Sfx::Footstep] {
            assert_eq!(render_sfx(sfx), render_sfx(sfx));
        }
        assert_ne!(render_variant(Sfx::Footstep, 0), render_variant(Sfx::Footstep, 1));
    }

    #[test]
    fn serde_roundtrip() {
        let s = ron::to_string(&Sfx::GaolerBashWindup).expect("serialize");
        let back: Sfx = ron::from_str(&s).expect("deserialize");
        assert_eq!(back, Sfx::GaolerBashWindup);
    }
}
