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
  in and out) is meant to be re-triggered periodically at the moving wisp. `HeronLaser` (1.3 s)
  is exactly the Heron's aim wind-up: play it on the wind-up event, `HeronLock` when
  `AIM_LOCKED` appears.

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
  The player sounds follow the wick (`docs/premise.md`): a Shrike is a flame behind a mask, not
  a machine, and nothing about it is wet.
  - `Hurt` is a crunchy hit with the wick guttering under it (band-passed noise whose level
    flickers ~30 times a second, the `gutter` building block), crushed to 6 bits.
  - `Heal` is *kindling*: the wick drawing embers in. A breath of air pulled in and rising
    (band-passed noise sweeping 450 Hz → 3 kHz), crackles thickening as the embers arrive, then
    the flame catching at ~0.22 s (a warm low whoomph, a soft 196 → 262 Hz tone and a bright FM
    glint), timed to the spark stream reaching the lantern.
  - `Death` is the wick snuffed: a long gutter sinking 1.1 kHz → 160 Hz and stuttering over a
    heavy fall and boom, ending in a low puff. `Downed` is the wick sinking to a low flicker.
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
    over a 150→40 Hz body; core weight 1.15 since the shotguns pass, `docs/balance.md`) with a
    short crack and the pellets crunching out (1.4 kHz→600 Hz), a clean 90→30 Hz sub boom
    under the distortion (−10 LU, was −11 without the crunch and the sub), then the bellows
    refilling: a reedy in-draw
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
- **Arsenal** (the expansion, `docs/expansion.md`; the block marked `arsenal-client` in
  `sfx.rs`). The six new weapons keep a material each, like the first four: the Lancet is thin
  steel, the Thresher a rougher Bellows with a ratchet, the Riveter compressed air and hammered
  metal, the Cinderbox fizzing clay rockets, the Longwick wood and string, the Kiln fire.
  - **Lancet.** `LancetFire` (0.09 s) is a tight band-passed "tsik" (9→5 kHz), a 3.3 kHz steel
    ping and a small high thump: thinner and higher than the Quill's crack.
  - **Thresher.** `ThresherFire` is a shorter Bellows core (weight 0.95, was 0.8) with a crack
    and the pellets rattling, then two ratchet clicks at 0.09 / 0.13 s, the drum turning one
    shell, over a short clean 110→40 Hz sub (−11 LU, was −12).
  - **Riveter.** `RiveterFire` (0.07 s, 8 voices, for eleven a second) is a burst of high-passed
    air and a hard 1.45 kHz "tak" over a knock; `RivetImpact` (2 variants) a bright clink and a
    short sizzle.
  - **Cinderbox.** `CinderboxFire` is one rocket (play it per rocket, three per salvo): a pop
    from the clay tube and a rising fizzing rush. `CinderBlast` is a small, dry burst (0.6 s) so
    three in a row stay three.
  - **Longwick.** `BowDraw` (0.75 s) is wax-hardened wood creaking: stick-slip grains through a
    band-pass, coming faster as the draw deepens, over the string's rising tension.
    `BowFullDraw` is the wick string flaring (a soft whoomph) and ringing a clear 1.76 kHz note.
    `BowRelease` is a low 98 Hz Karplus–Strong twang, the limbs' thwack and the arrow's hiss;
    `TripleNock` three twangs a chord apart; `PinningRelease` a heavier 82 Hz twang and a chain
    rattling out. `ArrowImpact` (2 variants) is a wooden thunk and a quivering pluck,
    `PinThunk` a heavier thunk with the chain going taut, `EmberArrowBurst` a crack, a whoomph
    of flame and a shower of embers.
  - **Kiln.** `KilnIgnite` is the spray catching; `KilnRoar` (0.55 s) a breathy rumble with
    crackle, soft at both ends: re-trigger it every ~0.35 s while the Kiln fires for one
    continuous roar.
  - **Abilities.** `SkewerCharge` (1.2 s, stop it on release) is a glassy whine climbing 600 Hz
    → 2.4 kHz and holding with a quickening tremolo; `SkewerFire` a heavy, lowered Quill crack
    with a long ringing zing and an echo. `FlurryFire` is three needles 0.06 s apart, each a
    little higher. `SlugFire` a heavy Bellows core with a crack and the barrel ringing.
    `FlakFire` thumps the shell out with its timer whistling up; `FlakBurst` is a crack in the
    air and a ring of pellets (a glass-spray of pings) hissing out. `VolleyFire` rips ten hard
    ticks out in 0.12 s over a gust. `FlareLaunch` is a pop and a fizzing hiss; `FlareBurn`
    (1.6 s) the patch catching and crackling. `LimpetThrow`, `LimpetStick` (a clank and four
    claw ticks), `LimpetBlink` (a tiny crushed square beep at 1.76 kHz, for the armed lamp) and
    `LimpetDetonate` (a double crack and a boom). `MortarLaunch` is a deep tube "thoomp",
    `MortarBlast` the arsenal's biggest boom (2.5 s, as loud as `Explosion`).
  - **Status.** `BurnSizzle` (0.6 s) is a fizzing sizzle, a guttering flame and pops; re-trigger
    it while something burns.
- **Melee and knacks** (the refit, `docs/refit.md`; a block marked `arsenal-client: melee and
  knacks` at the end of `Sfx`). The Trimmer is shear steel, the Tamper a foundry tool, the
  knacks glass, wax, clay and soot.
  - **Trimmer.** `TrimmerSlash` (0.23 s, 2 variants, one per slash: three a second) is the
    shears' snip (a 3.6–4.2 kHz inharmonic tick), a whoosh sweeping 1.4 → 5.2 kHz and a thin
    2.5 kHz ring riding on it, high-passed. `TrimmerHit` (2 variants) a crisp band-passed slice,
    a split and a short steel scrape, driven and crushed.
  - **Tamper.** `TamperSwing` a deep rush of air (180 → 900 Hz) with the haft creaking (a
    stuttered saw). `TamperHit` (1 s) the rammer's foot driving home: a 95 → 34 Hz packed
    thud, sand crunching, the iron head ringing once (~245 Hz partials), grit, driven, crushed
    and a short room.
  - **Abilities.** `Lunge` a hard rush and a blade drawn along steel; `Whirl` (0.65 s) air
    whooshing round twice over a spinning saw drone; `HurlThrow` a fling and the piece
    whirring away (a band-passed noise fluttering at 14 Hz); `HurlHit` (2 variants) a chop and
    a ringing, stuttering judder; `Upswing` a rising whoosh into a heavy upward heave.
  - **Knacks.** `KnackThrow` a short lob and a clink (snare, pot); `SnareSet` a wax thud and the jaws ratcheting open; `SnareArmed` two small
    bright ticks as its lamp catches; `SnareSpring` the jaws biting shut (a hard ~1 kHz steel
    bite) and a chain jerking taut; `SootBurst` (1.4 s) a dull clay pop, shards and soot
    billowing out, low-passed; `TallowSkin` a soft sizzling slide and a waxy settle under an
    FM shimmer; `Surge` the wick flaring (a fwoom) and a hard rush of air.
- **The rework** (`docs/rework.md`, area client-a; a block marked `client-a: the rework` at the
  end of `Sfx`). The Lamplighter is glass and brass lit by a wick, the Wickline a burning cord,
  the Tally chalk, the Waxwork hot wax, the arrows' tricks air and light, the Riposte steel.
  - **Lamplighter.** `LamplighterFocus` (1.4 s, stop it on release, like `SkewerCharge`): the
    focus ring ratcheting round faster and faster (bright 2.6 kHz ticks, 0.12 → 0.05 s apart),
    a glassy triangle hum climbing 320 Hz → 1.3 kHz over 0.9 s then holding with a tremble,
    and the wick hissing up under it. `LamplighterFull` one clear lens note (C7 with a soft
    twelfth) over the flame's whoomph, with an echo. `LamplighterFire` (−11, the loudest gun:
    a focused shot is rare and should carry) a low, hard crack, a bright hiss falling 7 →
    1.8 kHz down the beam, a 150 → 38 Hz boom and the brass tube ringing (880 Hz partials) into
    a reverb tail; the client plays it quieter and higher for a tap. `LamplighterTell` (0.6 s,
    soft both ends, −21) a faint high glassy shimmer (A7 and E8 with a slow wobble) played at
    another player's Lamplighter every 0.55 s while it focuses, louder with the focus: the
    sniper can be heard as well as seen.
  - **Precision abilities.** `WicklineFire` a whip-crack, a falling saw zip and the cord's
    crackle; `WicklineZip` the yank: a low plucked twang as the cord snaps taut, a tug and a
    hard rush of air. `TallyMark` a dry chalk stroke and a small 2.9 kHz tick per target
    tallied; `TallyShot` (2 variants) a short dry crack with a little weight, one per shot of
    the release a few hundredths of a second apart. `WaxworkCast` a thick glugging pour (a
    band-passed noise throbbing at 9 → 14 Hz) with a sizzle, then a hollow knock as the double
    sets; `WaxworkBurst` a wet sag, then a soft boom, hot wax splattering and a long sizzle.
  - **The arrows.** `Blink` a short sucking rush that swells to its end (a whoosh run
    backwards), a pop where you arrive and a bright FM shimmer. `BeaconLight` a whoomph and a
    bright E6 bell that keeps ringing, with an echo; `BeaconHum` (1.2 s, soft both ends: re-
    trigger while a beacon burns) a low pulsing triangle hum with a faint ring on it.
  - **Melee.** `HurlReturn` the piece's whirr dipping as it turns and rising again as it comes
    back; `HurlCatch` the glove slapping round the haft and a short ring. `RiposteGuard` steel
    scraping on steel, a set and a held ring; `RiposteBlock` (2 variants) a hard bright clang
    (780–860 Hz inharmonic partials) and a burst of sparks, driven; `RiposteCounter` a heavy
    whoosh let go all at once and a ringing edge.
- **The rework** (`docs/rework.md`; a block marked `client-b` at the end of `Sfx`). Glass and
  kindling for the draught and the relays, cord and wax for Scatter, fire and steam for Rapid,
  paper for the kite.
  - **The Kindle Flask.** `KindleDrink` (about a second) the stopper popped (a falling sine
    tick), two quick wet gulps, then the kindling catching in the wick (a low whoomph opening
    up, a crackle) into a bright two-note triangle chime (D6, A6) with a short echo: the haste
    is on;
    `KindleTick` the heal tick every second, a soft fifth (G6, D7) on a breath of flame, quiet
    (−21) so it repeats without nagging; `HasteFade` the wick settling back (a falling gutter
    and a sigh) as the haste runs out.
  - **Thorn relays.** Rays keep the `Ricochet` chime; `ThornBat` (2 variants) a glass crack,
    a woody knock, shards and a whip away with a ~1.8 kHz ring (a blow or blast batting the
    thorn); `ThornTurn` (2 variants) the spike shattering in a glassy ring and the shot's rush
    bending away (a band-passed whoosh sweeping up then down) with an echo.
  - **Bolas.** `BolasThrow` a fling and the weights whirring away (band-passed noise
    fluttering at 11 Hz); `BolasBind` the cord whipping round twice and the wax weights
    thudding together, driven; `BolasLink` the cord snapping taut, a low plucked G2 twang
    (Karplus–Strong) over a hum; `BolasShare` a short low thrum when damage runs along it.
  - **Bulwark.** `BulwarkRaise` a deep scrape-thud as the slab is heaved up, a hiss as it
    hardens and cooling ticks; `BulwarkAbsorb` (2 variants) a dull packed wax thunk and a
    crackle, driven and crushed; `BulwarkCrack` a sharp dry split running away over a groan;
    `BulwarkShatter` (−11) a heavy crack and boom with hard wax chunks raining down and a short
    room; `BulwarkDrop` a soft slump.
  - **Wisp, Stoke, Lantern.** `WispFling` an FM whine rising 0.5 → 1.8 kHz over a crackle and
    a fwoosh; `StokeIgnite` a bellows' breath roaring into the fire (a low-passed rush opening
    up, a crackle, a saw rumble); `StokeVent` (about 1.3 s) a hard burst of steam settling into
    a long hiss, then the metal ticking four times as it cools; `LanternSet` a brass clank, the
    glass rattling, the taper catching; `LanternOut` the flame guttering away and a glass tick.
    A lantern's shots are its carrier's primary sound at 55 %.
  - **Kite.** `KiteLaunch` oiled paper snapping taut twice and the ember heart catching (a
    fwoom); `KiteFlutter` (0.7 s, soft at both ends: re-trigger every 0.55 s while it flies)
    paper fluttering at ~22 Hz over a soft ember crackle. It bursts with the explosion.
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
- **Enemies (the Moulds).** Each type has its own alert bark, a wind-up cue per attack (each
  ends as the attack lands), strike sounds, and the projectiles' flight / burst. The client
  picks them by attack name (`client::ingame::combat::enemy_fx::attack_sfx`).
  - Gnawer: `GnawerAlert` a chittering snarl (a jittery growl chopped into a chatter, teeth
    clicking, 2 variants); `GnawerBiteWindup` a rising chitter (180→560 Hz) with a hiss;
    `GnawerBite` brass teeth clacking shut; `GnawerPounceWindup` a low coiling growl with claws
    scraping; `GnawerPounce` a whoosh and a snarl.
  - Censer: `CenserAlert` the brazier flaring (a breathy whoomph, crackling embers, a low
    drone); `CenserWhirl` (0.82 s) whooshes coming faster and faster (2→9 revolutions per
    second) over a chain rattle and a brightening hiss; `CenserLob` a thump and a rising whoosh
    per bomb; `CenserSwingWindup` / `CenserSwing` chain rattle and creak, then a whoosh and a
    clang; `EmberFlight` a fuse crackle (played when a bomb appears); `EmberBurst` a pop, a spray
    of sizzling embers, no big boom (2 variants).
  - Harrier: `HarrierAlert` a carrion screech; **`HarrierShriek`** (0.72 s) a piercing shriek
    rising 380→1100 Hz that peaks as it drops; `HarrierDive` the wind tearing past;
    `HarrierSwat` wax crunching, a thud, flapping and a squawk.
  - Gaoler: `GaolerAlert` a deep iron bellow through the grille with the chain shaken;
    **`GaolerBashWindup`** (1.0 s) iron grinding up in pitch with a rumble, then **two bright
    bell strikes at 0.65 s — the start of the parry window** — and a shimmer until the bash
    (mixed among the loudest sounds); `GaolerBash` a cell door slammed: a huge clang over a
    boom; `GaolerChain` (0.82 s) the chain spun up to speed (heavy links rattling faster, a
    whirr, a hiss); `GaolerSweep`; `GaolerRushWindup` a snort through the grille, iron feet
    scraping, a rumble; `GaolerRush` footfalls coming faster; `GaolerStep` (2 variants, played
    on each step).
  - Heron: `HeronAlert` two hoarse honks through a brass throat; **`HeronLaser`** (1.3 s) an
    electric hum with a whine climbing towards the lock; **`HeronLock`** a hard mechanical click
    and a short high double beep (dodge now); `HeronNeedle` a hard crack and a zip falling
    away; `HeronKickWindup` a squeaky brass joint; `HeronKick`.
  - The third roster (appended in a `// enemies` block of `Sfx`, docs/enemies.md):
    Bellwether: `BellwetherAlert` a hollow groan through the hood over a humming bell;
    **`BellwetherTollWindup`** (1.6 s, exactly the toll's wind-up) the yoke creaking and a
    bowed-metal hum swelling to a shriek, cut before the strike; **`BellwetherToll`** one deep
    bronze strike, inharmonic partials beating over a sub boom, ~3 s (among the loudest enemy
    sounds: it means every Mould near it just got faster); `BellCrack` the bell shot mid-swing,
    a crushed, buzzing clank (also when a Bellwether dies); `Frenzied` a hiss of flaring heat
    and a snarl on each Mould the toll takes; `BellwetherClapperWindup` / `BellwetherClapper`.
    Snuffer: `SnufferAlert` dry brass chittering (2 variants); `SnufferPounceWindup` a hiss
    rising under the lifted cap with clicking legs (also its nip's wind-up); `SnufferLatch` a
    muffled clank and a flame pressed out; `SnufferSmother` a muffled gutter, repeated every
    0.55 s to the player it holds (non-spatial); `SnufferThrown` a ringing cap and skittering
    legs; `SnufferNip` a brass snip. Smelter: `SmelterAlert` a furnace roar and a bellow;
    **`SmelterPourWindup`** (1.1 s) iron grinding and slag bubbling up to the lip;
    `SmelterPour` a molten gush hissing along the floor; `SlagBurn` (2 variants) a sizzle when a
    line ignites or a Smelter's crucible spills; `SmelterSwingWindup` / `SmelterSwing` (the
    heave); `SmelterStep` (2 variants) footfalls with a slosh. `EffigyHit` (2 variants) a thunk
    into packed wax and the post creaking (instead of `EnemyHurt` on the training dummy).
    Getting there (`docs/enemies.md` "Leaps, pads, melting"): `MouldMelt` (2 variants) a wet
    sag falling away, the heat hissing into the floor and the wax bubbling as a stranded Mould
    slumps into its puddle; `MouldRise` (2 variants) bubbling welling up, a wet groan climbing
    and a slap as it stands out of a new one (both pitched by the Mould's size); a leap takes
    off with `Jump` and lands with `Land` (`HeavyLand` for the heavies), pitched down.
- **Feel** (`docs/feel.md`). `ShotPunch` (3 variants) is layered under every own shot with a
  kick of 0.3 or more, at 0.4 + 0.6 × kick volume: a 150 → 52 Hz kick-drum thump, a snap and a
  low noise body, driven, so light guns still land with weight. `ParryImpact` (−11) goes under
  `Parry`: a 120 → 38 Hz boom, a crack and a shimmer rising 1.4 → 4.2 kHz. `FullDrawGlint`
  (−13) two bright pings a fifth apart (G6, D7) over a sparkle, with `BowFullDraw` when the bow
  reaches full draw. `WeakPointTick` (−16) a hard metallic tick on a confirmed weak-point /
  head hit (with `Headshot`) or weak-point kill (with `KillConfirm`).
- **Enemy hits and deaths.** The Moulds don't bleed: wax cracking, a hiss of heat, slag thuds,
  sizzle. `EnemyHurt` (3 variants) is a wax snap, a thud and a puff of hiss; `EnemyDeath`
  (2 variants, pitched by the client for the body's size) wax splitting, a slag thud and a long
  sizzle as it melts; `GaolerDeath` the door crashing down, iron collapsing, the lantern hissing
  out; `EmberRelease` the heat leaving: a soft whoomph and a fountain of crackling embers.
  `ProjectileImpact` (a needle or ember bomb hitting the level) is a crack, a thud and a short
  sizzle. There is no gore sound (the old `Gib` is gone; removing it re-seeded the sounds after
  it in `Sfx::ALL`, which only changes their random detail).
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
  - Play of the game (docs/potg.md): `PotgStinger` is the title card slamming in — a short
    riser, then a sub boom, a detuned brass stab in D minor (add 9) closing down, the foundry
    bell and a crash of sparks; `PotgWhoosh` the cut to the replay (rising air, a soft thud);
    `PotgSlowmo` the drop into slow motion on the last kill (a deep falling tone over a rumble).

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
| Parry, Explosion, Both Barrels, Gaoler bash, Bellows blast | −10 |
| Gaoler bash wind-up (parry cue), Gaoler death, sweep, Breach, Discharge release, Gravewell implosion | −11 … −12 |
| Harrier shriek / swat, Heron lock / needle, Gaoler chain / rush | −12 … −13 |
| Slam impact, death | −12 |
| Quill shots, Gust, launchers (bomb, wisp, gravewell), deflect, punch hit, wave/match stingers | −13 … −15 |
| Enemy windups (telegraphs), Branded | −14 … −15 |
| Hurt, rank up, kill confirm, enemy attacks, splinter burst, wisp zap, thorn plant | −14 … −17 |
| Arc zaps (10/s), bomb bounce, dash, heavy landings, slam, wall jump, jump pad | −16 … −19 |
| Hitmarker, enemy hurt, Gnawer bark, Gaoler steps, UI select/back, chat, Gravewell hum, thorn throw / shatter, Discharge charge | −18 … −20 |
| Jump, land, slide, ember fuse, wisp hum, weapon/mode switch, UI move | −20 … −22 |
| Footsteps (constant, 4 voices) | −25 |
| Arsenal: Mortar blast | −10 |
| Arsenal: Slug, Limpet detonation, Thresher, Skewer, Flak burst, Volley, Cinder blast | −11 … −12 |
| Arsenal: Mortar launch, Triple Nock, ember burst, bow release, Pinning release, pin thunk, Flurry, Flak shell | −13 … −14 |
| Arsenal: Lancet, Cinderbox rocket, Flare launch, Kiln ignite, arrow impact, limpet stick | −15 … −16 |
| Arsenal: Riveter (11/s), Kiln roar, full draw, Flare burn, limpet throw | −17 … −18 |
| Arsenal: bow draw, Skewer charge, rivet impact, burn sizzle | −19 |
| Arsenal: limpet blink | −22 |
| Melee: Tamper hit | −11 |
| Melee: Snare spring, Whirl, Upswing | −12 … −13 |
| Melee: Trimmer hit, Tamper swing, Lunge, hurl hit, soot burst | −14 |
| Melee: hurl throw, Tallow Skin, Surge, Trimmer slash (3/s) | −15 … −16 |
| Melee: snare set, knack throw | −17 … −18 |
| Melee: snare armed | −20 |
| Rework: Bulwark shatter | −11 |
| Rework: Bolas bind, Bulwark crack, Stoke ignite | −14 |
| Rework: Kindle drink, thorn bat / turn, Bolas link, Bulwark raise / absorb, Wisp fling, Kite launch | −15 |
| Rework: Bolas throw, Lantern set, Stoke vent | −16 |
| Rework: Lantern out, Bulwark drop | −18 … −19 |
| Rework: haste fade, Bolas share, Kindle tick, Kite flutter | −20 … −21 |
| Rework: Lamplighter shot | −11 |
| Rework: Waxwork burst, Riposte block / counter | −13 |
| Rework: Tally shot, Blink, Lamplighter full focus, Wickline, Beacon light | −14 … −15 |
| Rework: Waxwork cast, hurl catch, Riposte guard | −16 … −17 |
| Rework: tally mark, hurl return | −18 |
| Rework: Lamplighter focus | −19 |
| Rework: Lamplighter tell, beacon hum | −21 |

Each track is mastered so the full mix's loudest 400 ms sits at **−21 LU** (Campaign and
Arena), −23 (Menu) or −22 (Results), with peaks at −5 dBFS or lower. In practice:

- Full-combat music measures about −21 LU momentary and about −17 to −18.5 LU over its loudest
  50 ms. On the same 50 ms scale, Quill shots sit about 3 LU above it, the Bellows about 6–7,
  and Both Barrels, the parry and the Gaoler's bash wind-up about 7.
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
GaolerBashWindup               1.24     -9.1    -19.2     -11.0     -14.6
HarrierShriek                  0.72     -7.3    -17.5     -12.0     -14.1
HeronLaser                     1.30     -9.5    -18.1     -15.0     -15.4
HeronLock                      0.17     -1.1    -17.9     -12.0     -15.0
CenserWhirl                    0.82     -4.1    -20.7     -13.0     -16.8
EmberBurst                     0.82     -7.4    -20.3     -13.0     -17.4
EnemyDeath                     1.41     -7.2    -20.6     -15.0     -17.4
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
