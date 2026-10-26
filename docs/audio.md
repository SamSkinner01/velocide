# Audio (`engine::audio`)

Everything you hear in VELOCIDE is synthesised at startup: no audio files ship with the game.
Sound effects are rendered in about 50 ms. Music tracks render on background threads in about
0.25–0.5 s each and start playing as soon as they are ready. Playback uses kira 0.12.

| file | contents |
|------|----------|
| `engine/src/audio/mod.rs` | public API: `Audio`, `PlayParams`, re-exports |
| `engine/src/audio/dsp.rs` | DSP toolkit: RNG, oscillators, envelopes, filters, distortion, bitcrush, echo/reverb, mixing, loudness |
| `engine/src/audio/sfx.rs` | `Sfx` list, per-sound mixing metadata (`SfxInfo`), synthesis recipes |
| `engine/src/audio/music.rs` | `MusicTrack`, stems, intensity → stem gains, the four tracks |
| `engine/src/audio/spatial.rs` | distance attenuation and panning (pure math, unit-tested) |
| `engine/src/audio/backend.rs` | kira glue: mixer graph, voices, music player `Sound`, master limiter |
| `engine/src/audio/wav.rs` | 16-bit WAV writer |
| `engine/examples/audio_demo.rs` | WAV export + level table, device playback demo, startup benchmark |

## API

```rust
pub enum Sfx { UiMove, UiSelect, ..., MatchEnd }   // Copy, Eq, Hash, Ord, Debug, Serialize, Deserialize
impl Sfx {
    pub const ALL: &'static [Sfx];
    pub const COUNT: usize;
    pub fn index(self) -> usize;
    pub fn name(self) -> String;
    pub fn info(self) -> SfxInfo;           // level, voice limit, jitter, min interval, variants
}

pub struct PlayParams { pub volume: f32, pub pitch: f32, pub position: Option<Vec3> } // Default: 1, 1, None
impl PlayParams {
    pub fn at(position: Vec3) -> Self;
    pub fn with_volume(self, volume: f32) -> Self;
    pub fn with_pitch(self, pitch: f32) -> Self;
}

pub enum MusicTrack { None, Menu, Campaign, Arena, Results }  // Default = None, serde

pub struct Audio { .. }
impl Audio {
    pub fn new() -> Audio;                  // never fails: no device => warning + silent instance
    pub fn disabled() -> Audio;             // silent, never touches the device (tools/tests)
    pub fn is_enabled(&self) -> bool;
    pub fn play(&mut self, sfx: Sfx, params: PlayParams);
    pub fn play_2d(&mut self, sfx: Sfx);
    pub fn play_at(&mut self, sfx: Sfx, position: Vec3);
    pub fn stop(&mut self, sfx: Sfx);       // fade out all instances (released charge, ended slide)
    pub fn set_listener(&mut self, position: Vec3, rotation: Quat); // camera: looks down -Z, +Y up
    pub fn set_volumes(&mut self, master: f32, sfx: f32, music: f32); // linear 0..=1 amplitude
    pub fn set_music(&mut self, track: MusicTrack);     // 1.5 s crossfade
    pub fn set_music_intensity(&mut self, intensity: f32); // 0 calm .. 1 full combat, smoothed
    pub fn music_ready(&self, track: MusicTrack) -> bool;
    pub fn active_voices(&self) -> usize;   // debug overlay
    pub fn music_position(&self) -> Option<u64>;
    pub fn update(&mut self, dt: f32);      // once per frame
}
pub fn generate_all_sfx() -> Vec<Vec<Vec<f32>>>;        // [sfx][variant], mono 44.1 kHz
// engine::audio::sfx::{render_sfx, render_variant}, engine::audio::music::{render_track, stem_gains}
```

### Using it from the game

- **2D vs 3D.** Use `play_2d` for the local player's own sounds: weapons, movement, hurt,
  pickups, UI and game events. Use `play_at` for everything that happens somewhere else:
  enemies, other players' weapons and footsteps, impacts and explosions.
- **Listener.** Call `set_listener(camera_pos, camera_rot)` every frame before `update`.
- **Pitch.** `Ricochet` is designed to be chained at rising pitch, for example
  `pitch = 1.0 + 0.12 * bounce_index`. `Countdown` can be pitched up for the final beep.
- **Rapid fire is safe.** Per-sound voice limits steal the oldest instance, and a minimum
  interval merges repeats that land in the same frame. Twelve Bellows pellets produce one
  `Hitmarker`, not twelve stacked ones.
- **Intensity.** Set the intensity from raw game state every frame; it is smoothed internally.
  A suggested mapping:
  - 0 when exploring.
  - 0.35–0.45 when enemies are alerted.
  - 0.7 in an active fight.
  - 1.0 in a big wave, a boss fight, or at style rank MERCILESS or higher.
  - Drop it back to around 0.2 once an encounter is cleared.
- **Track per mode.** Main menu and lobby use `Menu`, campaign and co-op levels use `Campaign`,
  PvP uses `Arena`, and the results screen uses `Results`.
- **Long sounds.** `DischargeCharge` (1.3 s rising whine that holds after 1 s) and `Slide`
  (0.6 s) are one-shots. Call `stop` when the charge is released or cancelled, or the slide
  ends. `GravewellHum` (1.6 s) covers a planted Gravewell's whole pull. `WispHum` (0.5 s, soft
  in and out) and `GnatBuzz` (0.6 s) are meant to be re-triggered periodically at the moving
  wisp / Gnat.

## Architecture

```text
sfx track   (volume = sfx, up to 96 voices) ──┐
                                              ├─► main track (volume = master) ─► peak limiter (-1 dBFS) ─► device
music track (volume = music)  ────────────────┘
```

- **Startup** (`Audio::new`):
  1. Opens the device and builds the mixer.
  2. Spawns one thread per music track.
  3. Renders all sound-effect variations on up to 8 worker threads and wraps each in a kira
     `StaticSoundData`.
  4. Returns without waiting for the music.

  Each finished track is sent over a channel, picked up by `update()`, and started if it is
  the wanted track.
- **Music player.** A custom kira `Sound` (`MusicSound`) plays all six stems of a track from
  one shared playhead, so the stems are sample-locked by construction and the loop is
  sample-exact.
  - Stems are stored as 16-bit frame-major PCM (`[frame][stem][channel]`) and resampled
    linearly to the device rate.
  - The game thread steers it through lock-free atomics: per-stem gain targets, a fade target
    and a fade time.
  - Inside the audio thread, gains ramp linearly (0.25 s per full swing) to avoid zipper
    noise. Fades use an equal-power curve.
  - Dropping the game-side control makes the sound fade out and unload itself. A track change
    is a 1.5 s equal-power crossfade.
- **Voices.** Every played sound effect records its handle, start time, base gain and position.
  - `update()` drops finished voices.
  - It re-spatialises playing 3D voices against the current listener. The emitter stays where
    it started; the update uses 50 ms tweens and only fires when the change exceeds 0.5 dB or
    0.03 pan.
  - Each play applies the sound's random pitch (±`pitch_jitter`) and volume (±1 dB) variation
    and picks a random variant.
- **Master limiter.** A custom kira `Effect`: a stereo-linked peak limiter with instant attack
  and 150 ms release at -1 dBFS. It also replaces non-finite samples with silence. Twenty
  simultaneous explosions duck instead of clipping.

### 3D positional audio (`spatial.rs`)

Attenuation and panning are computed by us, not by kira's spatial tracks, which keeps the code
simple and allocation-free per sound.

- Within **1 m** of the listener a sound is non-spatial: centred, full volume. Spatialisation
  fades in over the next metre so nothing jumps at the threshold.
- Volume is full up to **2 m**. Beyond that it rolls off with inverse distance,
  `2 / (2 + 0.6·(d − 2))`, multiplied by `1 − ((d−2)/58)²` so it reaches exactly 0 at
  **60 m**. That gives about −11 dB at 10 m, −22 dB at 30 m and −34 dB at 50 m.
- Pan is the listener-space x component of the direction to the source, limited to ±0.85.
  Sounds are never hard-panned.
- Sources directly behind the listener are up to 25 % quieter, a cheap front/back cue.

## Sound design

All recipes live in `sfx.rs::recipe` and are built from a handful of generators:

- `thump`: pitch-swept sine.
- `noise`: SVF-filtered noise with an exponential cutoff sweep.
- `metal`: inharmonic sine partials with independent decays.
- `fm`: two-operator FM.
- `whoosh`: band-passed noise swell.
- `crackle`: sparse impulses.
- `growl`: jittery saw through two formants that follow the pitch.
- `chord`: detuned saw stacks.
- `notes`: melodic lines.
- Weapon helpers: `pluck` (Karplus–Strong string), `zap` (pitch-jumping saw buzz + noise,
  high-passed: electric arcs), `glass_spray` (scattered tiny glassy pings), `quill_crack`,
  `bellows_core` / `bellows_wheeze`, `envelope` (fades a crackle's pops, not just their
  density).

Post effects are `drive` (soft clip), `crush` (bit depth plus sample-and-hold rate reduction,
for the PS1 crunch), `echo`, `reverb` (Freeverb-style) and filters. Every sound is seeded from
its index, so generation is deterministic.

- **UI.** Tiny square-wave blips. `UiMove` is crushed to 6 bits, `UiSelect` is a rising fifth
  with a short echo, `UiBack` a falling fourth, and `ChatMessage` a two-note FM bell.
- **Movement.** The Shrikes wear brass-buckled harness and hobnailed boots, so every footstep and landing carries a small
  metallic tick on top of a thud.
  - `Footstep` has 4 variants.
  - `HeavyLand` and `SlamImpact` stack a sub thump, a crunchy low-passed noise burst, an
    inharmonic clank and debris crackle, crushed to 10 bits.
  - `Dash` is a bright band-passed burst with a falling "fwip" and a jet saw.
  - `Slam` is a falling whoosh and dive tone.
  - `JumpPad` is a sproingy rising sine with a wide whoosh.
- **Player.**
  - `Hurt` is a distorted, 6-bit-crushed "damage glitch".
  - `Heal` is a resonant slurp with a rising chirp and sparkle.
  - `Death` and `Downed` are stuttering power-downs.
  - `Respawn` and `Revive` are rising swells and arpeggios.
  - `NoStamina` is a dull double buzz.
- **Weapons.** Each weapon has its own material, so the four read apart instantly: the
  Quill is a dry string-and-whip crack, the Bellows breathes air, the Arclash buzzes and
  crackles, the Tumbler is hollow iron.
  - **Quill.** `QuillFire` is a whip-crack of band-passed noise (7→2.2 kHz), the snap of a
    Karplus–Strong string at 820 Hz, a tiny metallic tick and a small high thump: 0.13 s,
    centroid ≈ 6.8 kHz, no boom. `BrandFire` adds a branding-iron hiss and sputter;
    `Branded` (at the branded target) is a searing sizzle over a low, ominous toll (155 Hz
    with a rubbing minor second). `SplinterFire` is a lower, heavier crack after a glassy
    pre-tick; `SplinterBurst` (at the impact) is a spray of 16 tiny glass pings.
  - **Bellows.** `BellowsBlast` is a breathy low "whumpf" (low-passed noise 2.6 kHz→280 Hz
    over a 150→40 Hz body) with a short crack, then the bellows refilling: a reedy in-draw
    through a sweeping formant with a leather creak. `BellowsGust` has no crack at all: a
    pressure pop, then a band-passed rush of air that swells for 45 ms and keeps blowing
    (half its energy above 250 Hz, against 93 % below for the blast). `BothBarrels` is the
    loudest weapon (−10): two blast cores 25 ms apart (the second high-passed so the bodies
    don't phase-lock into a late swell), a clean 95→28 Hz sub added after the distortion,
    a room tail and a longer wheeze. `Deflect` is a rising "fwip" into a bright 2.8 kHz
    inharmonic ping.
  - **Arclash.** Arcs are a saw buzz whose pitch jumps randomly every 1.5 ms, plus noise,
    high-passed at 700 Hz. `ArcZap` (85 ms, 3 variants, 8 voices, 20 ms minimum interval) is
    the per-zap tick at 10/s; `ArcChain` is a higher zap with an upward square chirp.
    `WispLaunch` is an electric thoom with a rising buzz, `WispHum` a soft 98/147 Hz buzz with
    23 Hz flutter and crackle, `WispZap` a sharp crackle-snap. `DischargeCharge` is a pulsing
    capacitor whine (380 Hz → 3.1 kHz over 1 s, then holding, pulse rate 8→40 Hz) over a mains
    hum with growing crackle; `DischargeRelease` is a thunder-crack, an electric boom (thump +
    FM) and a sizzling tail.
  - **Tumbler.** `BombLaunch` is a hollow triangle "bonk" over a thump, then three ratchet
    clicks of the drum turning. `BombBounce` is a short iron clank on a thud (2 variants, wide
    pitch jitter). `BreachBlast` is tighter and drier than `Explosion` (0.9 s against 2.1 s): a
    hard crack, a short boom, crunch and a little debris. `GravewellLaunch` is a deep thunk
    with the canister's clamps locking; `GravewellHum` a 1.6 s sub drone rising 48→74 Hz with
    a warble and band noise sliding down (air being sucked in); `GravewellImplode` a reversed,
    exponentially growing suck-in that stops dead on a deep compressed thump.
  - **Thorn.** `ThornThrow` is a thin whistling "fwit"; `ThornPlant` a glassy "tunk" plus a
    quivering plucked twang (330 Hz with a 26 Hz wobble); `ThornShatter` a small glass crumble
    and fizzle.
  - **Explosion** is a crack, a 90→28 Hz sub boom, a low-pass sweep from 6 kHz down to 250 Hz,
    a rumble, crackle, and a reverb tail (2.1 s).
- **Parry.** The loudest and brightest sound in the game (spectral centroid 6.2 kHz): a
  transient crack, a long inharmonic clang (C6 with partials at ×1.5, 2.76, 5.4, 8.9 and 13.3),
  a beating 3.52 kHz shimmer, a fast whoosh, then saturation and a reverb tail. Nothing else
  sounds like it.
- **Ricochet.** A thorn shattering as the shot re-fires from it: a burst of high noise and a
  few glass pings, then a bright crystalline chime (≈1.8–2 kHz, near-harmonic partials and a
  slowly beating doubled fundamental) with a short echo. It is pitched, so chaining it at
  rising pitch reads as a combo. 2 variants, 8 voices, no minimum interval.
- **Hit feedback.** `Hitmarker` is a crisp 2.6 kHz tick. `Headshot` is a brighter double tick
  plus a splat. `KillConfirm` is a thud plus a two-note FM ping.
- **Enemy telegraphs.** Every attack has an unmistakable windup.
  - `HuskWindup` is a rising formant screech (≈140→420 Hz pitch, formants 775→1530 Hz) that
    ends as the swipe lands.
  - `SpitterWindup` is a gurgle rising 160→450 Hz.
  - **`BastionTelegraph`** is a two-octave rising siren (260→1040 Hz with 14 Hz tremolo) over a
    rumble. It ends on a bright 1.57 kHz "ding" at 0.7 s, which is the cue that means "parry
    now". It is mixed among the loudest sounds.
- **Enemy hits and deaths.** Gory, wet sounds from resonant low-passed noise ("squelch").
  `EnemyHurt` and `Gib` have 3 variants each, and `EnemyDeath` adds a sinking groan.
- **Game.**
  - `RankUp` is a rising A-major arpeggio (A4–C♯5–E5–A5) whose last note holds with vibrato
    and a third on top, plus shimmer and echo: triumphant and short (1.2 s).
  - `RankDown` is a soft falling minor third.
  - `WaveStart` is a distorted power-chord horn hit with a crash.
  - `EncounterClear`, `LevelComplete` and `Checkpoint` are major swells and bells.
  - `DoorLock` is a heavy clank with an alarm. `DoorOpen` is a hydraulic hiss, servo and
    clunk.
  - `Countdown` is a clean 880 Hz beep. `MatchStart` and `MatchEnd` are a stab and a falling
    horn with a gong.

## Music

Each track is a short loop split into six stems. Tempos are chosen so that a 16th note is a
whole number of samples at 44.1 kHz, which makes every loop an exact number of bars. Notes and
effect tails that run past the loop end are folded back to the loop start, so the loop is
seamless; a test checks the seam jump against the track's own 99.9th-percentile sample step.

| track | tempo | key / feel | loop | intensity floor |
|-------|-------|-----------|------|-----------------|
| `Menu` | 90 BPM | A minor, ominous; half-time drums, droning guitars, FM-bell melody with echo | 8 bars (21.3 s) | 0.35 |
| `Campaign` | 150 BPM | E Phrygian (E E F E C D E B♭) industrial breakbeat-metal; galloping bass and palm-muted guitars | 8 bars (12.8 s) | 0 |
| `Arena` | 175 BPM | D minor drum'n'bass-metal; reese bass, two-step breaks, saw lead | 8 bars (11.0 s) | 0 |
| `Results` | 126 BPM | 1-bar sting (Am add9 + crash + tom roll), then a 4-bar Am–F–C–G cool-down loop with arpeggios | 5 bars (9.5 s), loops from bar 2 | 1 (all stems) |

The intensity floor means the menu always has its heartbeat bass and sparse percussion, and the
results screen plays every stem.

For `Results`, whatever rings out of the sting (pad release, crash, reverb) is kept as a
separate per-stem *first-pass tail*. It is heard only the first time through the loop, so the
wrap from the last bar back to bar 2 stays click-free.

### Stems and intensity thresholds

`stem_gains(i)` returns, for each stem, `smoothstep(lo, hi, i)`:

| stem | contents | fades in over |
|------|----------|---------------|
| `pad` | detuned-saw chords with a slowly breathing low-pass, sine sub, reverb | always on |
| `bass` | gallop / reese / heartbeat bass | 0.20 – 0.35 |
| `drums_light` | kick on the main beats, closed hats, ghost backbeat | 0.20 – 0.35 |
| `drums_full` | syncopated kicks, snare and ghost notes, anvil clanks, crashes, fills | 0.50 – 0.62 |
| `guitar` | double-tracked (L/R, 4 ms apart) distorted power chords from saw stacks, palm mutes | 0.55 – 0.70 |
| `lead` | square/saw lead or FM bells, ping-pong dotted-eighth echo | 0.85 – 0.97 |

That gives four states: about 0 is the pad only, about 0.4 adds bass and light drums, about
0.75 adds full drums and guitars, and 1 adds the lead.

The effective intensity is `max(smoothed, floor)`. Smoothing uses a time constant of 0.6 s
when rising and 3 s when falling, so combat kicks in fast and the extra layers drain away
after an encounter.

## Mixing levels

Every sound is normalised to its own loudness target: the K-weighted (ITU BS.1770-style)
loudness of its loudest 50 ms window, in LU (`dsp::loudness`). Using 50 ms windows makes short
transients and long sounds comparable. Transients are not pushed more than about 4 dB into a
soft-knee limiter, and every peak stays below 0 dBFS (the loudest is −0.2 dBFS).

| family | target (LU, 50 ms) |
|--------|--------------------|
| Parry, Explosion, Both Barrels, Bastion slam | −10 |
| Bastion telegraph, Bellows blast, Breach, Discharge release, Gravewell implosion | −11 … −12 |
| Slam impact, death | −12 |
| Quill shots, Gust, launchers (bomb, wisp, gravewell), deflect, punch hit, wave/match stingers | −13 … −15 |
| Enemy windups (telegraphs), Branded | −14 … −15 |
| Hurt, rank up, kill confirm, enemy attacks, splinter burst, wisp zap, thorn plant | −14 … −17 |
| Arc zaps (10/s), bomb bounce, dash, heavy landings, slam, wall jump, jump pad | −16 … −19 |
| Hitmarker, enemy hurt, UI select/back, chat, Gravewell hum, thorn throw / shatter, Discharge charge | −18 … −20 |
| Jump, land, slide, gnat buzz, wisp hum, weapon/mode switch, UI move | −20 … −22 |
| Footsteps (constant, 4 voices) | −25 |

Each track is mastered so the full mix's loudest 400 ms sits at **−21 LU** (Campaign and
Arena), −23 (Menu) or −22 (Results), with peaks at −5 dBFS or lower. In practice:

- Full-combat music measures about −21 LU momentary and about −17 to −18.5 LU over its loudest
  50 ms. On the same 50 ms scale, Quill shots sit about 3 LU above it, the Bellows about 6–7,
  and Both Barrels, the parry and the Bastion telegraph about 7.
- The pad alone measures about −29 to −30 LU.

The music stays clearly under the gameplay-critical cues even at full music volume.

The volume buses (`set_volumes`) are linear amplitude, converted to dB with 50 ms tweens.

Measured with `audio_demo --wav-dir` (excerpt):

```text
name                           secs  peak dB   rms dB   LU 50ms  LU 400ms
UiMove                         0.05    -13.8    -22.6     -22.0     -22.0
Footstep                       0.12    -13.1    -27.1     -25.0     -28.7
QuillFire                      0.13     -4.3    -18.4     -14.0     -18.3
BellowsBlast                   0.75     -5.5    -16.1     -11.0     -17.2
BothBarrels                    1.01     -3.2    -14.7     -10.0     -15.1
ArcZap                         0.09     -1.2    -21.7     -17.0     -19.1
DischargeRelease               1.30     -5.8    -18.8     -11.0     -16.6
Explosion                      2.12     -0.9    -15.8     -10.0     -14.1
Parry                          2.20     -3.7    -18.0     -10.0     -10.9
Ricochet                       0.91     -5.7    -24.6     -15.0     -19.0
Hitmarker                      0.05     -8.9    -22.7     -20.0     -20.0
BastionTelegraph               1.30     -9.0    -17.1     -11.0     -14.0
RankUp                         1.17     -5.0    -19.2     -14.0     -16.3
Campaign_mix_calm             25.60    -14.0    -29.1     -27.5     -28.9
Campaign_mix_mid              25.60    -10.3    -25.1     -20.9     -25.7
Campaign_mix_full             25.60     -5.4    -21.8     -17.0     -21.0
Arena_mix_full                21.94     -7.6    -22.6     -18.6     -20.8
Menu_mix_full                 42.67     -8.4    -25.1     -18.7     -23.0
```

## Performance and memory

Measured on a dev build (engine at opt-level 1, 16-core machine):

| work | time |
|------|------|
| all 74 sound effects (85 variations), 8 threads | 30–60 ms |
| `Audio::new()` returns (device open + sfx) | 60–160 ms |
| Menu / Campaign / Arena / Results music, each on its own background thread, in parallel | ~0.39 / 0.30 / 0.26 / 0.25 s |
| everything ready | ~0.4–0.5 s |

The single biggest optimisation was caching filter coefficients: the SVF and one-pole filters
only recompute `tan`/`exp` when the cutoff changes. That made generation about 5–8× faster.

Memory use:

- Music: about 58 MB (2.4 M frames × 6 stems × stereo × 16-bit).
- Sound effects: about 18 MB (52 s of audio held as kira stereo `f32` frames).

## Tools and tests

```text
cargo run -p engine --example audio_demo -- --wav-dir out/audio   # WAVs + peak/RMS/loudness table
cargo run -p engine --example audio_demo -- --play [--section sfx|music|spatial]
cargo run -p engine --example audio_demo -- --bench               # startup timings
```

The `--wav-dir` export writes every sound-effect variation, every music stem, and full mixes at
calm, mid and full intensity. The mixes are rendered by the same playback simulation the player
uses (`MusicData::playback`), playing through the loop seam once.

Unit tests (`cargo test -p engine audio`) check that:

- every sound effect is finite, has a peak ≤ 1, a sensible duration and audible loudness, and
  stays within its target;
- generation is deterministic;
- every stem is exactly `bars × bar_frames` long, finite and peak ≤ 1;
- loop seams are smooth;
- stem gains follow intensity;
- the music `Sound` loops (including `loop_start`), fades and releases;
- the limiter caps peaks and removes NaN;
- spatial attenuation and panning behave (orientation, continuity at 1 m, monotonic falloff);
- the WAV header is consistent;
- `Audio::disabled()` is a no-op and `Audio::new()` never panics.

## Known limitations

- 3D emitters stay where the sound started; only the listener is tracked while a sound plays.
  There is no Doppler, occlusion or reverb zones. "Behind" is only a gain cue, not filtering.
- Volume buses are linear amplitude, as the contract says, so a settings slider will feel
  bunched at the top. Square the slider value in the UI if that matters.
- Linear-interpolation resampling of music to the device rate gives slight high-frequency
  imaging at 48 kHz. It is inaudible under the deliberately lo-fi sound.
- Each track's loop is fixed at 8 bars (5 for Results). Variety comes from the intensity
  layers, not from arrangement changes.
- The cpal backend may print ALSA/PipeWire diagnostics to stderr when no device exists; the
  game itself only logs one warning.
