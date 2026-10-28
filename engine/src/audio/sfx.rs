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
    samples, scale, smoothstep, soft_limit, sweep, trim_tail,
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
    QuillFire,
    BrandFire,
    Branded,
    SplinterFire,
    SplinterBurst,
    BellowsBlast,
    BellowsGust,
    BothBarrels,
    Deflect,
    ArcZap,
    ArcChain,
    WispLaunch,
    WispHum,
    WispZap,
    DischargeCharge,
    DischargeRelease,
    BombLaunch,
    BombBounce,
    BreachBlast,
    GravewellLaunch,
    GravewellHum,
    GravewellImplode,
    ThornThrow,
    ThornPlant,
    ThornShatter,
    Explosion,
    WeaponSwitch,
    ModeSwitch,
    Punch,
    PunchHit,
    Parry,
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
    // arsenal-client: the expansion arsenal's weapons, abilities and status effects
    // (docs/audio.md "Arsenal").
    /// The Lancet's needle: a tight high "tsik" and a steel ping.
    LancetFire,
    /// The Thresher's blast, then its drum ratcheting round one shell.
    ThresherFire,
    /// Pneumatic "pff-tak": air from the motor, the nose hammering a rivet out.
    RiveterFire,
    /// A hot rivet striking: a clink and a short sizzle.
    RivetImpact,
    /// One cinder rocket leaving its tube (three per salvo).
    CinderboxFire,
    /// A cinder rocket bursting: short and dry, so three don't mush together.
    CinderBlast,
    /// The Longwick drawn: wax-hardened wood creaking, tension rising.
    BowDraw,
    /// Full draw: the wick string flares and rings.
    BowFullDraw,
    /// The string's low twang and the arrow hissing away.
    BowRelease,
    /// An arrow thunking home and quivering.
    ArrowImpact,
    /// The Kiln's roar (0.55 s, soft both ends: re-trigger every ~0.35 s while spraying).
    KilnRoar,
    /// The Kiln's spray catching.
    KilnIgnite,
    /// Skewer charging: a glassy whine climbing then holding (stop it on release).
    SkewerCharge,
    /// Skewer: a heavy crack and a long ringing zing.
    SkewerFire,
    /// Flurry: three needles 0.06 s apart.
    FlurryFire,
    /// Slug: a deep crack-boom and the barrel ringing.
    SlugFire,
    /// Flak: the shell thumped out, its timer whistling.
    FlakFire,
    /// Flak airburst: a crack in the air and a ring of pellets hissing out.
    FlakBurst,
    /// Volley: ten shots ripped out at once.
    VolleyFire,
    /// Flare: a pop and a fizzing hiss climbing away.
    FlareLaunch,
    /// A flare's patch catching and burning.
    FlareBurn,
    /// A limpet thrown.
    LimpetThrow,
    /// A limpet's claws biting.
    LimpetStick,
    /// An armed limpet's lamp beeping.
    LimpetBlink,
    /// The limpets going off.
    LimpetDetonate,
    /// Mortar: a deep tube "thoomp".
    MortarLaunch,
    /// A mortar shell landing: the arsenal's biggest boom.
    MortarBlast,
    /// Triple Nock: three twangs and a wide hiss.
    TripleNock,
    /// Pinning Arrow: a heavier twang and a chain rattling away.
    PinningRelease,
    /// A pinning arrow driving home, its chain going taut.
    PinThunk,
    /// An ember arrow bursting into flame.
    EmberArrowBurst,
    /// Something burning (re-trigger while Burn lasts).
    BurnSizzle,
    // potg (play of the game, docs/potg.md)
    /// The "PLAY OF THE GAME" title card slamming in: boom, brass stab, foundry bell.
    PotgStinger,
    /// The card cutting to the replay.
    PotgWhoosh,
    /// The replay dropping into slow motion on the final kill.
    PotgSlowmo,
    // enemies: the third roster (docs/enemies.md)
    /// A Bellwether notices you: a hollow groan through the hood, the bell humming.
    BellwetherAlert,
    /// The 1.6 s toll wind-up: the yoke creaking, a bowed-metal hum swelling to a shriek.
    BellwetherTollWindup,
    /// The toll: one deep bronze strike (the sound that means "frenzy").
    BellwetherToll,
    /// The bell shot mid-swing: a cracked, dissonant clank, the toll cancelled.
    BellCrack,
    /// A Mould frenzied by the toll: a hiss of flaring heat and a snarl.
    Frenzied,
    BellwetherClapperWindup,
    BellwetherClapper,
    /// A Snuffer: dry brass chittering, legs ticking.
    SnufferAlert,
    /// Rearing to leap: a rising hiss under the cap, legs clicking.
    SnufferPounceWindup,
    /// It clamps on: a muffled brass clank and a flame snuffed.
    SnufferLatch,
    /// Smothering your wick: a muffled, guttering flame (repeats while latched on you).
    SnufferSmother,
    /// Punched / shaken off: a ringing clang and skittering legs.
    SnufferThrown,
    SnufferNip,
    /// A Smelter: a furnace roar through the grate, chains shaken.
    SmelterAlert,
    /// Tipping the crucible: iron grinding, slag bubbling up to the lip.
    SmelterPourWindup,
    /// The pour: a heavy molten gush hissing along the floor.
    SmelterPour,
    /// Standing in slag: a sharp sizzle.
    SlagBurn,
    SmelterSwingWindup,
    SmelterSwing,
    SmelterStep,
    /// A hit on the training dummy: a wax thunk and the post creaking.
    EffigyHit,
    /// A stranded Mould slumping into a wax puddle: a wet sag, its heat hissing into the floor.
    MouldMelt,
    /// A Mould rising out of a puddle where it re-formed: bubbling, a wet groan, a slap.
    MouldRise,

    // ----- feel (refit, area **feel**: docs/feel.md) -------------------------------------------
    /// Layered under every own shot (volume = the shot's kick): a tight low-end punch and a
    /// click, so light weapons still land with weight.
    ShotPunch,
    /// Layered under `Parry`: a deep impact boom and a rising shimmer.
    ParryImpact,
    /// The bow at full draw: a bright two-note glint, clearly audible over the creak.
    FullDrawGlint,
    /// A hit on a weak point / head confirmed by the server: a hard metallic tick.
    WeakPointTick,
    // arsenal-client: melee and knacks (docs/refit.md)
    /// A Trimmer slash: the shears snip and the blade whistles past (one per swing).
    TrimmerSlash,
    /// The Trimmer's hook biting into a target: a crisp slice and a scrape of steel.
    TrimmerHit,
    /// The Tamper's overhead swing: a deep rush of air, the haft creaking.
    TamperSwing,
    /// The Tamper landing: a packed thud, sand crunching, the iron head ringing.
    TamperHit,
    /// Lunge: a hard rush forward, a blade drawn along steel.
    Lunge,
    /// Whirl: air whooshing round twice over a spinning drone.
    Whirl,
    /// Hurl: a fling and the piece whirring away end over end.
    HurlThrow,
    /// A hurled blade / head biting into something: a chop and a ringing judder.
    HurlHit,
    /// Upswing: a rising whoosh into a heavy upward heave.
    Upswing,
    /// A knack thrown (flask, snare, soot pot): a short lob, a soft clink.
    KnackThrow,
    /// A Snare slapped down: a wax thud, the jaws ratcheting open.
    SnareSet,
    /// A Snare armed: its wick lamp catching.
    SnareArmed,
    /// A Snare sprung: the jaws biting shut, a chain jerking taut.
    SnareSpring,
    /// A Soot pot breaking: a clay pop and soot billowing out.
    SootBurst,
    /// Tallow Skin: a sizzling coat of hot tallow settling.
    TallowSkin,
    /// Surge: the wick flaring and a hard rush of air.
    Surge,
    // client-a: the rework's Lamplighter and its Precision / Draw / Melee abilities
    // (docs/rework.md, docs/audio.md "The rework").
    /// The Lamplighter focusing: its focus ring ratcheting faster, the wick hissing up and a
    /// glassy hum climbing as the lens comes true (1.4 s; stop it on release).
    LamplighterFocus,
    /// Full focus: one clear lens note and the flame's soft whoomph.
    LamplighterFull,
    /// The beam: a hard crack, a searing hiss down the line, a deep boom and a long tail.
    LamplighterFire,
    /// Someone else's Lamplighter focusing: a faint, high glassy shimmer (0.6 s, re-trigger
    /// at them while they focus).
    LamplighterTell,
    /// Wickline: a burning cord whipped out, crackling.
    WicklineFire,
    /// The Wickline's yank: the cord snapping taut and a hard rush along it.
    WicklineZip,
    /// Tally: a target tallied (a dry chalk stroke and a tick).
    TallyMark,
    /// One of the tally's shots (one per shot, a few hundredths of a second apart).
    TallyShot,
    /// Waxwork: hot wax poured into your shape and setting.
    WaxworkCast,
    /// The wax double slumping and bursting.
    WaxworkBurst,
    /// Blink Arrow: the step to the arrow (a sucking rush and a pop).
    Blink,
    /// Beacon Arrow: the beacon catching (a whoomph and a bright bell note).
    BeaconLight,
    /// A lit beacon's hum (1.2 s, soft both ends: re-trigger while it burns).
    BeaconHum,
    /// Hurl: the piece turning back in the air.
    HurlReturn,
    /// Hurl: the piece caught (a slap and a ring).
    HurlCatch,
    /// Riposte: the guard raised (steel scraping, a held ring).
    RiposteGuard,
    /// Riposte: a blow caught on the guard (a hard clang and sparks).
    RiposteBlock,
    /// Riposte: the counter-swing (a heavy whoosh and a ringing edge).
    RiposteCounter,
    // client-b: the rework's Kindle draught, Thorn relays and the Scatter / Rapid / Explosive
    // abilities (docs/rework.md, docs/audio.md "The rework").
    /// The Kindle Flask drunk: the stopper popped, two gulps, the kindling catching in the wick.
    KindleDrink,
    /// A Kindle heal tick while hasted: a soft warm chime on a breath of flame.
    KindleTick,
    /// The haste running out: the wick settling back down.
    HasteFade,
    /// A Thorn batted at a target by a blow or a blast: a glass crack and a whip away.
    ThornBat,
    /// A Thorn turning a projectile: a glassy ring and the shot bending away.
    ThornTurn,
    /// Bolas thrown: the weights whirring round on their cord.
    BolasThrow,
    /// Bolas binding: the cord whipping round, the wax weights thudding together.
    BolasBind,
    /// The cord snapping taut to a second target: a low twang.
    BolasLink,
    /// Damage carried along a bolas cord: a short thrum.
    BolasShare,
    /// Bulwark raised: a slab of wax heaved up, hissing as it hardens.
    BulwarkRaise,
    /// A blow taken by the slab: a dull wax thunk and a crackle.
    BulwarkAbsorb,
    /// The slab cracking further: a sharp split.
    BulwarkCrack,
    /// The slab shattering: hard wax bursting into chunks.
    BulwarkShatter,
    /// The slab lowered: a soft slump.
    BulwarkDrop,
    /// The Wisp flung off its orbit: a rising whine and a fwoosh.
    WispFling,
    /// Stoke: a bellows roar and the weapon's fire catching.
    StokeIgnite,
    /// Stoke spent: steam venting from the overheated weapon, metal ticking as it cools.
    StokeVent,
    /// A lantern set down: a clank, a glass rattle, its flame catching.
    LanternSet,
    /// A lantern guttering out: a dying flame and a glass tick.
    LanternOut,
    /// A kite launched: paper snapping taut and a fwoom.
    KiteLaunch,
    /// A steered kite fluttering (re-trigger while it flies).
    KiteFlutter,
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
    /// A repeat within this many seconds of the previous start is dropped (twelve Bellows pellets
    /// make one hitmarker, not twelve stacked ones).
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
        Sfx::QuillFire,
        Sfx::BrandFire,
        Sfx::Branded,
        Sfx::SplinterFire,
        Sfx::SplinterBurst,
        Sfx::BellowsBlast,
        Sfx::BellowsGust,
        Sfx::BothBarrels,
        Sfx::Deflect,
        Sfx::ArcZap,
        Sfx::ArcChain,
        Sfx::WispLaunch,
        Sfx::WispHum,
        Sfx::WispZap,
        Sfx::DischargeCharge,
        Sfx::DischargeRelease,
        Sfx::BombLaunch,
        Sfx::BombBounce,
        Sfx::BreachBlast,
        Sfx::GravewellLaunch,
        Sfx::GravewellHum,
        Sfx::GravewellImplode,
        Sfx::ThornThrow,
        Sfx::ThornPlant,
        Sfx::ThornShatter,
        Sfx::Explosion,
        Sfx::WeaponSwitch,
        Sfx::ModeSwitch,
        Sfx::Punch,
        Sfx::PunchHit,
        Sfx::Parry,
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
        // arsenal-client
        Sfx::LancetFire,
        Sfx::ThresherFire,
        Sfx::RiveterFire,
        Sfx::RivetImpact,
        Sfx::CinderboxFire,
        Sfx::CinderBlast,
        Sfx::BowDraw,
        Sfx::BowFullDraw,
        Sfx::BowRelease,
        Sfx::ArrowImpact,
        Sfx::KilnRoar,
        Sfx::KilnIgnite,
        Sfx::SkewerCharge,
        Sfx::SkewerFire,
        Sfx::FlurryFire,
        Sfx::SlugFire,
        Sfx::FlakFire,
        Sfx::FlakBurst,
        Sfx::VolleyFire,
        Sfx::FlareLaunch,
        Sfx::FlareBurn,
        Sfx::LimpetThrow,
        Sfx::LimpetStick,
        Sfx::LimpetBlink,
        Sfx::LimpetDetonate,
        Sfx::MortarLaunch,
        Sfx::MortarBlast,
        Sfx::TripleNock,
        Sfx::PinningRelease,
        Sfx::PinThunk,
        Sfx::EmberArrowBurst,
        Sfx::BurnSizzle,
        // potg
        Sfx::PotgStinger,
        Sfx::PotgWhoosh,
        Sfx::PotgSlowmo,
        // enemies
        Sfx::BellwetherAlert,
        Sfx::BellwetherTollWindup,
        Sfx::BellwetherToll,
        Sfx::BellCrack,
        Sfx::Frenzied,
        Sfx::BellwetherClapperWindup,
        Sfx::BellwetherClapper,
        Sfx::SnufferAlert,
        Sfx::SnufferPounceWindup,
        Sfx::SnufferLatch,
        Sfx::SnufferSmother,
        Sfx::SnufferThrown,
        Sfx::SnufferNip,
        Sfx::SmelterAlert,
        Sfx::SmelterPourWindup,
        Sfx::SmelterPour,
        Sfx::SlagBurn,
        Sfx::SmelterSwingWindup,
        Sfx::SmelterSwing,
        Sfx::SmelterStep,
        Sfx::EffigyHit,
        Sfx::MouldMelt,
        Sfx::MouldRise,
        // feel
        Sfx::ShotPunch,
        Sfx::ParryImpact,
        Sfx::FullDrawGlint,
        Sfx::WeakPointTick,
        // arsenal-client: melee and knacks
        Sfx::TrimmerSlash,
        Sfx::TrimmerHit,
        Sfx::TamperSwing,
        Sfx::TamperHit,
        Sfx::Lunge,
        Sfx::Whirl,
        Sfx::HurlThrow,
        Sfx::HurlHit,
        Sfx::Upswing,
        Sfx::KnackThrow,
        Sfx::SnareSet,
        Sfx::SnareArmed,
        Sfx::SnareSpring,
        Sfx::SootBurst,
        Sfx::TallowSkin,
        Sfx::Surge,
        Sfx::LamplighterFocus,
        Sfx::LamplighterFull,
        Sfx::LamplighterFire,
        Sfx::LamplighterTell,
        Sfx::WicklineFire,
        Sfx::WicklineZip,
        Sfx::TallyMark,
        Sfx::TallyShot,
        Sfx::WaxworkCast,
        Sfx::WaxworkBurst,
        Sfx::Blink,
        Sfx::BeaconLight,
        Sfx::BeaconHum,
        Sfx::HurlReturn,
        Sfx::HurlCatch,
        Sfx::RiposteGuard,
        Sfx::RiposteBlock,
        Sfx::RiposteCounter,
        Sfx::KindleDrink,
        Sfx::KindleTick,
        Sfx::HasteFade,
        Sfx::ThornBat,
        Sfx::ThornTurn,
        Sfx::BolasThrow,
        Sfx::BolasBind,
        Sfx::BolasLink,
        Sfx::BolasShare,
        Sfx::BulwarkRaise,
        Sfx::BulwarkAbsorb,
        Sfx::BulwarkCrack,
        Sfx::BulwarkShatter,
        Sfx::BulwarkDrop,
        Sfx::WispFling,
        Sfx::StokeIgnite,
        Sfx::StokeVent,
        Sfx::LanternSet,
        Sfx::LanternOut,
        Sfx::KiteLaunch,
        Sfx::KiteFlutter,
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

            QuillFire => info(-14.0, 6, 0.05, 0.0),
            BrandFire => info(-14.0, 3, 0.03, 0.0),
            Branded => info(-15.0, 4, 0.03, 0.05),
            SplinterFire => info(-13.0, 3, 0.04, 0.0),
            SplinterBurst => info(-16.0, 4, 0.06, 0.03),
            BellowsBlast => info(-10.0, 4, 0.04, 0.0),
            BellowsGust => info(-13.0, 3, 0.04, 0.0),
            BothBarrels => info(-10.0, 3, 0.03, 0.0),
            Deflect => info(-14.0, 4, 0.04, 0.03),
            ArcZap => variants(info(-17.0, 8, 0.08, 0.02), 3),
            ArcChain => info(-17.0, 6, 0.06, 0.02),
            WispLaunch => info(-15.0, 3, 0.04, 0.0),
            WispHum => info(-21.0, 4, 0.05, 0.1),
            WispZap => info(-16.0, 6, 0.08, 0.03),
            DischargeCharge => info(-19.0, 2, 0.0, 0.05),
            DischargeRelease => info(-11.0, 3, 0.03, 0.0),
            BombLaunch => info(-14.0, 3, 0.04, 0.0),
            BombBounce => variants(info(-17.0, 6, 0.1, 0.02), 2),
            BreachBlast => info(-11.0, 3, 0.04, 0.0),
            GravewellLaunch => info(-14.0, 3, 0.04, 0.0),
            GravewellHum => info(-18.0, 3, 0.0, 0.1),
            GravewellImplode => info(-11.0, 3, 0.03, 0.0),
            ThornThrow => info(-18.0, 3, 0.05, 0.03),
            ThornPlant => info(-16.0, 4, 0.06, 0.03),
            ThornShatter => info(-19.0, 4, 0.08, 0.03),
            Explosion => info(-10.0, 5, 0.07, 0.03),
            WeaponSwitch => info(-20.0, 2, 0.03, 0.03),
            ModeSwitch => info(-21.0, 2, 0.03, 0.03),
            Punch => info(-19.0, 3, 0.07, 0.03),
            PunchHit => info(-13.0, 3, 0.06, 0.03),
            Parry => info(-10.0, 3, 0.02, 0.05),
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

            // arsenal-client
            LancetFire => info(-15.0, 6, 0.06, 0.0),
            ThresherFire => info(-11.0, 4, 0.04, 0.0),
            RiveterFire => info(-17.0, 8, 0.06, 0.02),
            RivetImpact => variants(info(-19.0, 6, 0.1, 0.02), 2),
            CinderboxFire => info(-15.0, 6, 0.05, 0.02),
            CinderBlast => info(-12.0, 6, 0.07, 0.02),
            BowDraw => info(-19.0, 2, 0.03, 0.1),
            BowFullDraw => info(-17.0, 2, 0.0, 0.1),
            BowRelease => info(-14.0, 3, 0.04, 0.0),
            ArrowImpact => variants(info(-16.0, 6, 0.08, 0.02), 2),
            // Played per spray tick (12/s): merged into a roar every 0.3 s.
            KilnRoar => info(-17.0, 3, 0.04, 0.3),
            KilnIgnite => info(-15.0, 2, 0.04, 0.1),
            SkewerCharge => info(-19.0, 2, 0.0, 0.05),
            SkewerFire => info(-12.0, 3, 0.03, 0.0),
            // One sound per burst: its three shots are in the sound.
            FlurryFire => info(-14.0, 3, 0.04, 0.15),
            SlugFire => info(-11.0, 3, 0.03, 0.0),
            FlakFire => info(-14.0, 3, 0.04, 0.0),
            FlakBurst => info(-12.0, 4, 0.06, 0.02),
            VolleyFire => info(-12.0, 3, 0.04, 0.0),
            FlareLaunch => info(-15.0, 3, 0.05, 0.0),
            FlareBurn => info(-17.0, 4, 0.05, 0.1),
            LimpetThrow => info(-18.0, 3, 0.06, 0.03),
            LimpetStick => info(-16.0, 6, 0.06, 0.03),
            LimpetBlink => info(-22.0, 6, 0.0, 0.05),
            LimpetDetonate => info(-11.0, 4, 0.05, 0.02),
            MortarLaunch => info(-13.0, 3, 0.04, 0.0),
            MortarBlast => info(-10.0, 4, 0.05, 0.03),
            TripleNock => info(-13.0, 3, 0.03, 0.0),
            PinningRelease => info(-14.0, 3, 0.04, 0.0),
            PinThunk => info(-14.0, 4, 0.05, 0.03),
            EmberArrowBurst => info(-13.0, 4, 0.06, 0.02),
            BurnSizzle => info(-19.0, 6, 0.1, 0.08),
            // potg
            PotgStinger => info(-11.0, 1, 0.0, 0.5),
            PotgWhoosh => info(-16.0, 1, 0.0, 0.2),
            PotgSlowmo => info(-15.0, 1, 0.0, 0.3),

            // enemies
            BellwetherAlert => info(-16.0, 3, 0.05, 0.1),
            BellwetherTollWindup => info(-13.0, 3, 0.0, 0.2),
            BellwetherToll => info(-9.0, 3, 0.02, 0.2),
            BellCrack => info(-11.0, 3, 0.04, 0.1),
            Frenzied => info(-17.0, 4, 0.08, 0.06),
            BellwetherClapperWindup => info(-16.0, 3, 0.05, 0.05),
            BellwetherClapper => info(-14.0, 3, 0.06, 0.05),
            SnufferAlert => variants(info(-18.0, 4, 0.1, 0.1), 2),
            SnufferPounceWindup => info(-14.0, 4, 0.06, 0.05),
            SnufferLatch => info(-12.0, 3, 0.04, 0.05),
            SnufferSmother => info(-16.0, 2, 0.06, 0.2),
            SnufferThrown => info(-13.0, 3, 0.05, 0.05),
            SnufferNip => info(-17.0, 4, 0.08, 0.03),
            SmelterAlert => info(-14.0, 2, 0.04, 0.2),
            SmelterPourWindup => info(-13.0, 3, 0.0, 0.2),
            SmelterPour => info(-12.0, 3, 0.04, 0.1),
            SlagBurn => variants(info(-17.0, 4, 0.1, 0.12), 2),
            SmelterSwingWindup => info(-15.0, 3, 0.04, 0.1),
            SmelterSwing => info(-12.0, 3, 0.04, 0.1),
            SmelterStep => variants(info(-18.0, 4, 0.06, 0.1), 2),
            EffigyHit => variants(info(-18.0, 4, 0.1, 0.04), 2),
            MouldMelt => variants(info(-17.0, 4, 0.08, 0.1), 2),
            MouldRise => variants(info(-17.0, 4, 0.08, 0.1), 2),
            // feel
            ShotPunch => variants(info(-15.0, 4, 0.05, 0.02), 3),
            ParryImpact => info(-11.0, 2, 0.0, 0.08),
            FullDrawGlint => info(-13.0, 2, 0.0, 0.1),
            WeakPointTick => info(-16.0, 3, 0.03, 0.035),

            // arsenal-client: melee and knacks
            TrimmerSlash => variants(info(-16.0, 4, 0.06, 0.02), 2),
            TrimmerHit => variants(info(-14.0, 4, 0.06, 0.03), 2),
            TamperSwing => info(-14.0, 3, 0.04, 0.05),
            TamperHit => info(-11.0, 3, 0.04, 0.05),
            Lunge => info(-14.0, 3, 0.04, 0.05),
            Whirl => info(-13.0, 3, 0.03, 0.1),
            HurlThrow => info(-15.0, 3, 0.05, 0.03),
            HurlHit => variants(info(-14.0, 4, 0.06, 0.03), 2),
            Upswing => info(-13.0, 3, 0.04, 0.05),
            KnackThrow => info(-18.0, 3, 0.06, 0.05),
            SnareSet => info(-17.0, 3, 0.05, 0.05),
            SnareArmed => info(-20.0, 4, 0.03, 0.05),
            SnareSpring => info(-12.0, 3, 0.04, 0.05),
            SootBurst => info(-14.0, 3, 0.04, 0.1),
            TallowSkin => info(-15.0, 2, 0.03, 0.2),
            Surge => info(-15.0, 3, 0.05, 0.05),

            // client-a: the rework
            LamplighterFocus => info(-19.0, 2, 0.0, 0.05),
            LamplighterFull => info(-15.0, 2, 0.0, 0.1),
            // The loudest gun: a focused shot is rare and should be heard across the map.
            LamplighterFire => info(-11.0, 3, 0.03, 0.0),
            LamplighterTell => info(-21.0, 4, 0.02, 0.2),
            WicklineFire => info(-15.0, 3, 0.05, 0.03),
            WicklineZip => info(-15.0, 3, 0.04, 0.05),
            TallyMark => info(-18.0, 4, 0.04, 0.04),
            TallyShot => variants(info(-14.0, 4, 0.05, 0.02), 2),
            WaxworkCast => info(-16.0, 2, 0.03, 0.1),
            WaxworkBurst => info(-13.0, 3, 0.04, 0.05),
            Blink => info(-14.0, 3, 0.03, 0.05),
            BeaconLight => info(-15.0, 3, 0.03, 0.05),
            BeaconHum => info(-21.0, 3, 0.0, 0.2),
            HurlReturn => info(-18.0, 3, 0.05, 0.05),
            HurlCatch => info(-16.0, 3, 0.05, 0.05),
            RiposteGuard => info(-17.0, 2, 0.03, 0.05),
            RiposteBlock => variants(info(-13.0, 4, 0.05, 0.03), 2),
            RiposteCounter => info(-13.0, 3, 0.04, 0.05),
            // client-b: the rework
            KindleDrink => info(-15.0, 2, 0.03, 0.2),
            KindleTick => info(-21.0, 3, 0.03, 0.2),
            HasteFade => info(-20.0, 2, 0.03, 0.3),
            ThornBat => variants(info(-15.0, 6, 0.04, 0.0), 2),
            ThornTurn => variants(info(-15.0, 6, 0.04, 0.0), 2),
            BolasThrow => info(-16.0, 3, 0.05, 0.05),
            BolasBind => info(-14.0, 4, 0.05, 0.05),
            BolasLink => info(-15.0, 3, 0.03, 0.05),
            BolasShare => info(-20.0, 4, 0.08, 0.12),
            BulwarkRaise => info(-15.0, 3, 0.03, 0.1),
            BulwarkAbsorb => variants(info(-15.0, 4, 0.08, 0.04), 2),
            BulwarkCrack => info(-14.0, 3, 0.05, 0.08),
            BulwarkShatter => info(-11.0, 3, 0.03, 0.1),
            BulwarkDrop => info(-19.0, 3, 0.03, 0.1),
            WispFling => info(-15.0, 3, 0.04, 0.05),
            StokeIgnite => info(-14.0, 3, 0.03, 0.1),
            StokeVent => info(-16.0, 3, 0.03, 0.1),
            LanternSet => info(-16.0, 3, 0.04, 0.1),
            LanternOut => info(-18.0, 3, 0.04, 0.1),
            KiteLaunch => info(-15.0, 3, 0.04, 0.05),
            KiteFlutter => info(-21.0, 4, 0.06, 0.15),
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
    render(dur, |t, _| {
        osc.next(glide(t, f0, f1, pitch_tau)) * perc(t, 0.0008, decay)
    })
}

/// Filtered white noise, cutoff sweeping exponentially `f0 → f1` over `dur`.
#[allow(clippy::too_many_arguments)]
pub(super) fn noise(
    rng: &mut Rng,
    dur: f32,
    pass: Pass,
    f0: f32,
    f1: f32,
    q: f32,
    attack: f32,
    decay: f32,
) -> Vec<f32> {
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
    noise(
        rng,
        decay * 6.0 + 0.002,
        Pass::High,
        2500.0,
        2500.0,
        0.6,
        0.0002,
        decay,
    )
}

/// Oscillator with an exponential pitch sweep and a percussive envelope.
#[allow(clippy::too_many_arguments)]
pub(super) fn tone(
    dur: f32,
    wave: Wave,
    f0: f32,
    f1: f32,
    sweep_dur: f32,
    attack: f32,
    decay: f32,
) -> Vec<f32> {
    let mut osc = Osc::new(wave);
    render(dur, |t, _| {
        osc.next(sweep(t, f0, f1, sweep_dur)) * perc(t, attack, decay)
    })
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
            .map(|(&(ratio, amp, decay), osc)| {
                osc.next(base * ratio) * amp * perc(t, 0.0005, decay)
            })
            .sum()
    })
}

/// Two-operator FM. Carrier sweeps `fc0 → fc1`; the modulation index decays from `i0` to `i1`.
#[allow(clippy::too_many_arguments)]
pub(super) fn fm(
    dur: f32,
    fc0: f32,
    fc1: f32,
    ratio: f32,
    i0: f32,
    i1: f32,
    attack: f32,
    decay: f32,
) -> Vec<f32> {
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
fn growl(
    rng: &mut Rng,
    dur: f32,
    f0: f32,
    f1: f32,
    formants: [f32; 2],
    jitter: f32,
    attack: f32,
    decay: f32,
) -> Vec<f32> {
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
        let y =
            a.bandpass(x, formants[0] * shift, 3.0) + b.bandpass(x, formants[1] * shift, 4.0) * 0.7;
        y * perc(t, attack, decay)
    })
}

/// Detuned saw stack chord through a swept low-pass. `notes` are MIDI numbers.
#[allow(clippy::too_many_arguments)]
fn chord(
    notes: &[f32],
    dur: f32,
    wave: Wave,
    cutoff0: f32,
    cutoff1: f32,
    attack: f32,
    decay: f32,
    release_at: f32,
) -> Vec<f32> {
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
fn notes(
    seq: &[(f32, f32, f32)],
    wave: Wave,
    attack: f32,
    release: f32,
    cutoff: f32,
    vibrato: f32,
) -> Vec<f32> {
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

/// A flame guttering (the Shrike's wick): band-passed noise falling `f0 → f1` whose level dips
/// about `flutter` times a second, each dip a random depth, fading with `decay`.
fn gutter(rng: &mut Rng, dur: f32, f0: f32, f1: f32, flutter: f32, decay: f32) -> Vec<f32> {
    let mut n = Rng::new(rng.next_u64());
    let mut svf = Svf::default();
    let (mut phase, mut depth) = (0.0f32, 0.5f32);
    render(dur, |t, _| {
        phase += flutter / SR;
        if phase >= 1.0 {
            phase -= 1.0;
            depth = n.range(0.3, 0.95);
        }
        let flicker = 1.0 - depth * (std::f32::consts::PI * phase).sin();
        svf.process(n.bipolar(), sweep(t, f0, f1, dur), 1.3, SR)
            .band
            * flicker
            * perc(t, 0.002, decay)
    })
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

// ---- weapon building blocks -------------------------------------------------------------------

/// Karplus–Strong plucked string: a noise burst circulating in a delay line one period long,
/// averaged (low-passed) and damped by `damp` on every pass. Taut snaps and twangs.
fn pluck(rng: &mut Rng, dur: f32, freq: f32, damp: f32) -> Vec<f32> {
    let mut n = Rng::new(rng.next_u64());
    let period = ((SR / freq.max(20.0)).round() as usize).max(2);
    let mut line: Vec<f32> = (0..period).map(|_| n.bipolar()).collect();
    let mut i = 0;
    render(dur, |_, _| {
        let j = (i + 1) % period;
        let y = line[i];
        line[i] = (line[i] + line[j]) * 0.5 * damp;
        i = j;
        y
    })
}

/// Electric arc: a saw buzz sweeping `f0 → f1` whose pitch jumps at random every 1.5 ms
/// (± `jitter`), with raw noise (`grit`) on top, high-passed so it crackles instead of humming.
#[allow(clippy::too_many_arguments)]
fn zap(rng: &mut Rng, dur: f32, f0: f32, f1: f32, jitter: f32, grit: f32, decay: f32) -> Vec<f32> {
    let mut n = Rng::new(rng.next_u64());
    let mut osc = Osc::new(Wave::Saw);
    let mut hp = Svf::default();
    let mut jump = 1.0f32;
    let hold = samples(0.0015).max(1);
    render(dur, |t, i| {
        if i % hold == 0 {
            jump = 1.0 + n.bipolar() * jitter;
        }
        let x = osc.next(sweep(t, f0, f1, dur) * jump) + n.bipolar() * grit;
        hp.highpass(x, 700.0, 0.8) * perc(t, 0.0006, decay)
    })
}

/// Multiplies `buf` by a percussive envelope (crackles only thin out over time; this makes their
/// pops fade too).
fn envelope(mut buf: Vec<f32>, attack: f32, decay: f32) -> Vec<f32> {
    for (i, s) in buf.iter_mut().enumerate() {
        *s *= perc(i as f32 / SR, attack, decay);
    }
    buf
}

/// A spray of tiny glassy ticks: `count` short inharmonic pings (`lo..hi` Hz) scattered over
/// `spread` seconds, denser at the start and fading out.
fn glass_spray(rng: &mut Rng, count: usize, spread: f32, lo: f32, hi: f32) -> Vec<f32> {
    let mut b = Vec::new();
    for k in 0..count {
        let x = rng.f32();
        let at = spread * x * x;
        let base = rng.range(lo, hi);
        let decay = rng.range(0.004, 0.012);
        let tick = metal(
            decay * 6.0,
            base,
            &[
                (1.0, 1.0, decay),
                (1.62, 0.6, decay * 0.8),
                (2.71, 0.4, decay * 0.6),
            ],
        );
        let gain = 0.9 * (1.0 - 0.6 * k as f32 / count as f32);
        mix_at(&mut b, &tick, gain, at);
    }
    b
}

/// The Quill's shot: a whip-crack of air, the snap of a taut string and a tiny metallic tick
/// over a small, high body thump. `pitch` scales it (heavier modes go lower).
fn quill_crack(rng: &mut Rng, pitch: f32) -> Vec<f32> {
    let snap = click(rng, 0.0015);
    let whip = noise(
        rng,
        0.05,
        Pass::Band,
        7000.0 * pitch,
        2200.0 * pitch,
        1.4,
        0.0003,
        0.007,
    );
    let mut string = pluck(rng, 0.14, 820.0 * pitch, 0.985);
    for (i, s) in string.iter_mut().enumerate() {
        *s *= perc(i as f32 / SR, 0.0005, 0.035);
    }
    highpass(&mut string, 300.0);
    let tick = metal(
        0.05,
        4300.0 * pitch,
        &[(1.0, 1.0, 0.008), (1.83, 0.5, 0.006), (2.9, 0.3, 0.004)],
    );
    let body = thump(0.07, 420.0 * pitch, 160.0 * pitch, 0.006, 0.014);
    layer(&[
        (&snap, 0.8, 0.0),
        (&whip, 1.0, 0.0),
        (&string, 0.5, 0.002),
        (&tick, 0.3, 0.0),
        (&body, 0.8, 0.0),
    ])
}

/// Bellows blast core: a breathy, low "whumpf" of air behind the shot rather than a sharp
/// crack. `weight` > 1 = deeper and longer.
fn bellows_core(rng: &mut Rng, weight: f32) -> Vec<f32> {
    let snap = click(rng, 0.003);
    let body = thump(0.45 * weight, 150.0 / weight, 40.0, 0.02, 0.1 * weight);
    let whumpf = noise(
        rng,
        0.5 * weight,
        Pass::Low,
        2600.0,
        280.0,
        0.9,
        0.006,
        0.11 * weight,
    );
    let breath = noise(rng, 0.35, Pass::Band, 1100.0, 500.0, 1.1, 0.01, 0.08);
    let grit = noise(rng, 0.12, Pass::Band, 2400.0, 1600.0, 1.0, 0.0005, 0.025);
    let crack = noise(rng, 0.08, Pass::Band, 3200.0, 1800.0, 1.0, 0.0003, 0.015);
    layer(&[
        (&snap, 0.7, 0.0),
        (&crack, 0.6, 0.0),
        (&body, 1.1, 0.0),
        (&whumpf, 1.0, 0.0),
        (&breath, 0.5, 0.004),
        (&grit, 0.5, 0.0),
    ])
}

/// The leather bellows refilling after a shot: a reedy, breathy in-draw through a sweeping
/// formant, with a faint leather creak.
fn bellows_wheeze(rng: &mut Rng, dur: f32) -> Vec<f32> {
    let mut n = Rng::new(rng.next_u64());
    let mut reed = Osc::new(Wave::Saw);
    let (mut f1, mut f2) = (Svf::default(), Svf::default());
    let mut b = render(dur, |t, _| {
        let x = t / dur;
        let formant = 650.0 + 900.0 * (std::f32::consts::PI * x).sin();
        let src = reed.next(lerp(150.0, 175.0, x)) * 0.35 + n.bipolar();
        let y = f1.bandpass(src, formant, 4.0) + f2.bandpass(src, formant * 2.3, 5.0) * 0.5;
        y * bell(t, dur)
    });
    let creak = crackle(rng, dur, 60.0, dur);
    mix_at(&mut b, &creak, 0.15, 0.0);
    b
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
            let mut b = notes(
                &[(84.0, 0.0, 0.045), (91.0, 0.05, 0.08)],
                Wave::Square,
                0.002,
                0.04,
                5000.0,
                0.0,
            );
            echo(&mut b, 0.07, 0.25, 0.25, 3000.0, 0.15);
            b
        }
        UiBack => notes(
            &[(86.0, 0.0, 0.045), (79.0, 0.05, 0.07)],
            Wave::Triangle,
            0.002,
            0.05,
            4000.0,
            0.0,
        ),
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
            let tick = metal(
                0.08,
                base,
                &[(1.0, 1.0, 0.012), (1.47, 0.6, 0.009), (2.31, 0.4, 0.006)],
            );
            layer(&[(&thud, 1.0, 0.0), (&scuff, 0.7, 0.0), (&tick, 0.22, 0.002)])
        }
        Jump => {
            let air = whoosh(r, 0.22, 700.0, 2600.0, 1.2);
            let lift = tone(0.15, Wave::Triangle, 280.0, 560.0, 0.12, 0.002, 0.05);
            let push = thump(0.1, 200.0, 110.0, 0.01, 0.025);
            layer(&[(&air, 1.0, 0.0), (&lift, 0.25, 0.0), (&push, 0.5, 0.0)])
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
            let clank = metal(
                0.5,
                420.0,
                &[
                    (1.0, 1.0, 0.15),
                    (2.37, 0.7, 0.1),
                    (3.91, 0.4, 0.06),
                    (5.2, 0.3, 0.04),
                ],
            );
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
                svf.process(n.bipolar(), 2800.0 + 600.0 * grit, 1.5, SR)
                    .band
                    * env
                    * (0.5 + grit)
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
            let clank = metal(
                0.7,
                180.0,
                &[
                    (1.0, 1.0, 0.3),
                    (2.43, 0.8, 0.2),
                    (3.9, 0.5, 0.12),
                    (6.1, 0.3, 0.07),
                ],
            );
            let debris = crackle(r, 0.5, 300.0, 0.15);
            let mut b = layer(&[
                (&boom, 1.2, 0.0),
                (&crunch, 0.8, 0.0),
                (&clank, 0.3, 0.0),
                (&debris, 0.3, 0.02),
            ]);
            drive(&mut b, 2.5);
            crush(&mut b, 10, 2.0);
            reverb(&mut b, 0.5, 0.5, 0.2, 0.4);
            b
        }
        WallJump => {
            let kick = thump(0.12, 220.0, 100.0, 0.01, 0.035);
            let ping = metal(
                0.3,
                1700.0,
                &[(1.0, 1.0, 0.06), (1.62, 0.6, 0.045), (2.71, 0.4, 0.03)],
            );
            let air = whoosh(r, 0.22, 800.0, 3000.0, 1.1);
            layer(&[(&kick, 0.8, 0.0), (&ping, 0.35, 0.0), (&air, 0.8, 0.02)])
        }
        JumpPad => {
            let mut osc = Osc::new(Wave::Sine);
            let boing = render(0.5, |t, _| {
                let f = sweep(t, 80.0, 700.0, 0.3)
                    * (1.0 + 0.04 * (t * 18.0 * std::f32::consts::TAU).sin());
                osc.next(f) * perc(t, 0.002, 0.18)
            });
            let air = whoosh(r, 0.5, 500.0, 6000.0, 1.0);
            let push = thump(0.2, 150.0, 60.0, 0.015, 0.05);
            layer(&[(&boing, 0.6, 0.0), (&air, 1.0, 0.0), (&push, 0.8, 0.0)])
        }

        // ----- player ---------------------------------------------------------------------
        Hurt => {
            // A hit and the wick guttering under it.
            let crunch = noise(r, 0.18, Pass::Band, 1800.0, 600.0, 0.9, 0.0005, 0.05);
            let flame = gutter(r, 0.22, 1500.0, 450.0, 30.0, 0.07);
            let hit = thump(0.12, 180.0, 80.0, 0.01, 0.03);
            let mut b = layer(&[(&crunch, 1.0, 0.0), (&flame, 0.7, 0.0), (&hit, 0.7, 0.0)]);
            drive(&mut b, 3.0);
            crush(&mut b, 6, 3.0);
            b
        }
        Heal => {
            // Kindling: the wick drawing embers in. A breath of air pulled in and rising,
            // crackles getting denser as the embers arrive, then the flame catching with a
            // soft warm whoomph and a bright glint.
            let dur = 0.4;
            let mut n = Rng::new(r.next_u64());
            let mut bp = Svf::default();
            let inhale = render(dur, |t, _| {
                let env = smoothstep(0.0, 0.2, t) * (1.0 - smoothstep(0.21, 0.28, t));
                bp.process(n.bipolar(), sweep(t, 450.0, 3000.0, 0.22), 1.4, SR)
                    .band
                    * env
            });
            let sparks = envelope(crackle(r, 0.3, 220.0, -0.18), 0.02, 0.2);
            let catch = noise(r, 0.3, Pass::Low, 1100.0, 280.0, 0.9, 0.005, 0.07);
            let warmth = tone(0.3, Wave::Sine, 196.0, 262.0, 0.1, 0.01, 0.09);
            let glint = fm(0.2, 1568.0, 1568.0, 2.0, 0.8, 0.1, 0.001, 0.05);
            layer(&[
                (&inhale, 1.0, 0.0),
                (&sparks, 0.45, 0.04),
                (&catch, 0.9, 0.22),
                (&warmth, 0.35, 0.22),
                (&glint, 0.18, 0.23),
            ])
        }
        Death => {
            // The wick snuffed: the flame gutters lower and lower over a heavy fall, then goes
            // out with a puff.
            let mut flame = gutter(r, 1.1, 1100.0, 160.0, 16.0, 0.45);
            stutter(&mut flame, 18.0, 0.4);
            let snuff = noise(r, 0.3, Pass::Low, 1600.0, 220.0, 0.8, 0.002, 0.06);
            let crunch = noise(r, 0.9, Pass::Low, 3000.0, 200.0, 0.8, 0.0005, 0.25);
            let boom = thump(1.2, 90.0, 28.0, 0.08, 0.3);
            let mut b = layer(&[
                (&flame, 0.8, 0.0),
                (&crunch, 0.9, 0.0),
                (&boom, 1.0, 0.0),
                (&snuff, 0.6, 0.75),
            ]);
            drive(&mut b, 2.5);
            crush(&mut b, 8, 2.0);
            reverb(&mut b, 0.6, 0.5, 0.2, 0.5);
            b
        }
        Respawn => {
            let mut oscs = [
                Osc::new(Wave::Saw),
                Osc::new(Wave::Saw).phase(0.3),
                Osc::new(Wave::Saw).phase(0.6),
            ];
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
            // The wick sinks to a low, flickering ember (not out: a teammate can revive).
            let mut flame = gutter(r, 0.9, 900.0, 220.0, 12.0, 0.35);
            stutter(&mut flame, 12.0, 0.15);
            let hit = thump(0.5, 120.0, 40.0, 0.03, 0.12);
            let mut b = layer(&[(&flame, 0.8, 0.0), (&hit, 1.0, 0.0)]);
            crush(&mut b, 7, 2.0);
            reverb(&mut b, 0.5, 0.5, 0.15, 0.3);
            b
        }
        Revive => {
            let arp = notes(
                &[
                    (57.0, 0.0, 0.05),
                    (64.0, 0.06, 0.05),
                    (69.0, 0.12, 0.05),
                    (76.0, 0.18, 0.3),
                ],
                Wave::Square,
                0.002,
                0.12,
                4000.0,
                0.006,
            );
            let shimmer = notes(
                &[(88.0, 0.18, 0.3)],
                Wave::Triangle,
                0.01,
                0.15,
                8000.0,
                0.01,
            );
            let mut b = layer(&[(&arp, 1.0, 0.0), (&shimmer, 0.3, 0.0)]);
            echo(&mut b, 0.09, 0.3, 0.3, 3000.0, 0.3);
            b
        }
        NoStamina => {
            let mut buzz = notes(
                &[(39.0, 0.0, 0.06), (39.0, 0.09, 0.06)],
                Wave::Square,
                0.002,
                0.02,
                1800.0,
                0.0,
            );
            drive(&mut buzz, 2.0);
            buzz
        }

        // ----- weapons --------------------------------------------------------------------
        QuillFire => {
            let mut b = quill_crack(r, 1.0);
            drive(&mut b, 1.6);
            crush(&mut b, 12, 1.0);
            b
        }
        BrandFire => {
            let crack = quill_crack(r, 0.92);
            // Branding iron: a hot hiss with sputtering crackles that swells in, then fades.
            let hiss = noise(r, 0.45, Pass::High, 3800.0, 6500.0, 0.8, 0.02, 0.14);
            let sputter = envelope(crackle(r, 0.45, 400.0, 0.15), 0.01, 0.12);
            let heat = thump(0.3, 160.0, 70.0, 0.02, 0.07);
            let mut b = layer(&[
                (&crack, 1.0, 0.0),
                (&hiss, 0.55, 0.01),
                (&sputter, 0.35, 0.01),
                (&heat, 0.5, 0.0),
            ]);
            drive(&mut b, 1.5);
            b
        }
        Branded => {
            let sear = noise(r, 0.45, Pass::Band, 5000.0, 2200.0, 0.9, 0.003, 0.12);
            let sputter = envelope(crackle(r, 0.4, 600.0, 0.15), 0.005, 0.1);
            // A low, ominous toll: a minor second rubbing against the fundamental.
            let toll = metal(
                0.7,
                155.6,
                &[
                    (1.0, 1.0, 0.24),
                    (1.059, 0.5, 0.2),
                    (2.41, 0.45, 0.14),
                    (3.9, 0.25, 0.08),
                ],
            );
            let mut b = layer(&[(&sear, 0.7, 0.0), (&sputter, 0.3, 0.0), (&toll, 0.9, 0.0)]);
            reverb(&mut b, 0.4, 0.5, 0.15, 0.15);
            b
        }
        SplinterFire => {
            let glass = metal(
                0.06,
                5200.0,
                &[
                    (1.0, 1.0, 0.006),
                    (1.47, 0.7, 0.005),
                    (2.23, 0.5, 0.004),
                    (3.31, 0.3, 0.003),
                ],
            );
            let crack = quill_crack(r, 0.75);
            let body = thump(0.25, 210.0, 60.0, 0.012, 0.05);
            let mut b = layer(&[
                (&glass, 0.6, 0.0),
                (&crack, 1.0, 0.008),
                (&body, 0.9, 0.008),
            ]);
            drive(&mut b, 2.2);
            crush(&mut b, 11, 1.0);
            b
        }
        SplinterBurst => {
            let spray = glass_spray(r, 16, 0.2, 3200.0, 8000.0);
            let shatter = noise(r, 0.08, Pass::High, 5000.0, 3000.0, 0.7, 0.0003, 0.012);
            let mut b = layer(&[(&shatter, 0.7, 0.0), (&spray, 1.0, 0.0)]);
            echo(&mut b, 0.05, 0.25, 0.2, 6000.0, 0.15);
            b
        }
        BellowsBlast => {
            // Heavier since the shotguns pass (docs/balance.md): a deeper core, the pellets
            // crunching out of the barrels, and a clean sub boom under the distortion.
            let core = bellows_core(r, 1.15);
            let crunch = noise(r, 0.2, Pass::Band, 1400.0, 600.0, 1.0, 0.0005, 0.045);
            let mut b = layer(&[(&core, 1.0, 0.0), (&crunch, 0.5, 0.0)]);
            drive(&mut b, 2.6);
            crush(&mut b, 11, 1.5);
            let sub = thump(0.7, 90.0, 30.0, 0.045, 0.16);
            mix_at(&mut b, &sub, 0.5, 0.0);
            reverb(&mut b, 0.3, 0.6, 0.12, 0.3);
            let wheeze = bellows_wheeze(r, 0.32);
            mix_at(&mut b, &wheeze, 0.2, 0.22);
            b
        }
        BellowsGust => {
            // A pressure pop, then a huge low push of air that swells for ~45 ms and keeps
            // blowing: no crack, no pellets.
            let pop = thump(0.06, 520.0, 180.0, 0.004, 0.012);
            let snap = click(r, 0.002);
            let mut n = Rng::new(r.next_u64());
            let mut svf = Svf::default();
            let rush = render(0.8, |t, _| {
                let env = smoothstep(0.0, 0.045, t) * (-(t - 0.045).max(0.0) / 0.22).exp();
                svf.process(n.bipolar(), sweep(t, 1800.0, 260.0, 0.7), 0.7, SR)
                    .band
                    * env
            });
            let low = noise(r, 0.7, Pass::Low, 600.0, 140.0, 0.9, 0.03, 0.25);
            let sub = thump(0.5, 70.0, 30.0, 0.06, 0.12);
            let mut b = layer(&[
                (&pop, 0.9, 0.0),
                (&snap, 0.5, 0.0),
                (&rush, 1.3, 0.006),
                (&low, 0.9, 0.006),
                (&sub, 0.6, 0.004),
            ]);
            drive(&mut b, 1.4);
            b
        }
        BothBarrels => {
            let first = bellows_core(r, 1.3);
            // The second barrel only adds its crack and air: two identical body thumps 25 ms
            // apart lock in phase once they glide down to 40 Hz and swell up late.
            let mut second = bellows_core(r, 1.3);
            highpass(&mut second, 220.0);
            let crunch = noise(r, 0.25, Pass::Band, 1200.0, 500.0, 1.0, 0.0005, 0.05);
            let second_snap = click(r, 0.003);
            let mut b = layer(&[
                (&first, 1.0, 0.0),
                (&second, 0.95, 0.025),
                (&second_snap, 0.8, 0.025),
                (&crunch, 0.6, 0.0),
            ]);
            drive(&mut b, 2.6);
            crush(&mut b, 10, 2.0);
            // The deep sub boom goes on clean, after the distortion, so it doesn't swamp the
            // double crack.
            let sub = thump(0.9, 95.0, 28.0, 0.05, 0.2);
            mix_at(&mut b, &sub, 0.55, 0.0);
            reverb(&mut b, 0.35, 0.55, 0.16, 0.35);
            let wheeze = bellows_wheeze(r, 0.4);
            mix_at(&mut b, &wheeze, 0.15, 0.3);
            b
        }
        Deflect => {
            let fwip = whoosh(r, 0.16, 700.0, 4500.0, 1.4);
            let ping = metal(
                0.45,
                2794.0,
                &[
                    (1.0, 1.0, 0.14),
                    (2.76, 0.6, 0.09),
                    (5.4, 0.35, 0.05),
                    (8.1, 0.2, 0.03),
                ],
            );
            let snap = click(r, 0.002);
            let mut b = layer(&[(&fwip, 0.9, 0.0), (&snap, 0.6, 0.11), (&ping, 0.7, 0.11)]);
            echo(&mut b, 0.06, 0.25, 0.2, 5000.0, 0.2);
            b
        }
        ArcZap => {
            let (f0, f1) = (r.range(150.0, 200.0), r.range(90.0, 120.0));
            let buzz = zap(r, 0.085, f0, f1, 0.5, 0.4, 0.026);
            let spark = crackle(r, 0.08, 900.0, 0.03);
            let snap = click(r, 0.0012);
            let mut b = layer(&[(&snap, 0.7, 0.0), (&buzz, 1.0, 0.0), (&spark, 0.5, 0.0)]);
            crush(&mut b, 8, 1.0);
            b
        }
        ArcChain => {
            let buzz = zap(r, 0.08, 280.0, 620.0, 0.35, 0.3, 0.025);
            let mut chirp_osc = Osc::new(Wave::Square);
            let chirp = render(0.07, |t, _| {
                chirp_osc.next(sweep(t, 1400.0, 4200.0, 0.05)) * perc(t, 0.0005, 0.018)
            });
            let snap = click(r, 0.001);
            let mut b = layer(&[(&snap, 0.7, 0.0), (&buzz, 0.9, 0.0), (&chirp, 0.25, 0.004)]);
            crush(&mut b, 9, 1.0);
            b
        }
        WispLaunch => {
            let thoom = thump(0.35, 170.0, 48.0, 0.02, 0.09);
            let charge = fm(0.4, 120.0, 260.0, 1.41, 5.0, 1.5, 0.004, 0.16);
            let buzz = zap(r, 0.4, 70.0, 230.0, 0.15, 0.15, 0.18);
            let spark = crackle(r, 0.35, 300.0, 0.15);
            let mut b = layer(&[
                (&thoom, 1.0, 0.0),
                (&charge, 0.5, 0.0),
                (&buzz, 0.6, 0.02),
                (&spark, 0.3, 0.0),
            ]);
            drive(&mut b, 1.8);
            b
        }
        WispHum => {
            // Soft in and out so it can be retriggered every ~0.45 s along the wisp's path.
            let dur = 0.5;
            let mut n = Rng::new(r.next_u64());
            let (mut saw, mut square, mut flutter) = (
                Osc::new(Wave::Saw),
                Osc::new(Wave::Square).phase(0.3),
                Osc::new(Wave::Sine),
            );
            let mut lp = Svf::default();
            let hum = render(dur, |t, _| {
                let x = saw.next(98.0) * 0.6 + square.next(147.0) * 0.25 + n.bipolar() * 0.15;
                let am = 0.65 + 0.35 * flutter.next(23.0);
                lp.lowpass(x, 1600.0, 1.2) * am * bell(t, dur).sqrt()
            });
            let mut spark = crackle(r, dur, 120.0, 10.0);
            for (i, s) in spark.iter_mut().enumerate() {
                *s *= bell(i as f32 / SR, dur);
            }
            layer(&[(&hum, 1.0, 0.0), (&spark, 0.4, 0.0)])
        }
        WispZap => {
            let (f0, f1) = (r.range(200.0, 260.0), r.range(80.0, 110.0));
            let buzz = zap(r, 0.12, f0, f1, 0.6, 0.5, 0.035);
            let crack = noise(r, 0.05, Pass::High, 4500.0, 3000.0, 0.8, 0.0002, 0.008);
            let snap = click(r, 0.0015);
            let mut b = layer(&[(&snap, 0.8, 0.0), (&crack, 0.8, 0.0), (&buzz, 0.9, 0.0)]);
            drive(&mut b, 2.0);
            crush(&mut b, 9, 1.0);
            b
        }
        DischargeCharge => {
            // A capacitor bank filling: a pulsing whine rising for 1 s, then holding.
            let dur = 1.3;
            let (mut whine, mut over, mut pulse, mut vib) = (
                Osc::new(Wave::Sine),
                Osc::new(Wave::Triangle),
                Osc::new(Wave::Triangle),
                Osc::new(Wave::Sine),
            );
            let mut hum_osc = Osc::new(Wave::Saw);
            let mut hum_lp = Svf::default();
            let cap = render(dur, |t, _| {
                let f = sweep(t, 380.0, 3100.0, 1.0)
                    * (1.0 + 0.006 * vib.next(7.0) * smoothstep(0.9, 1.1, t));
                let x = whine.next(f) * 0.6 + over.next(f * 1.5) * 0.25;
                let gate = 0.7 + 0.3 * pulse.next(lerp(8.0, 40.0, t.min(1.0)));
                let hum = hum_lp.lowpass(hum_osc.next(sweep(t, 60.0, 90.0, 1.0)), 300.0, 0.8);
                let env = smoothstep(0.0, 0.25, t) * (1.0 - smoothstep(1.2, dur, t));
                (x * gate + hum * 0.35) * env
            });
            let mut sizzle = crackle(r, dur, 30.0, -1.5);
            for (i, s) in sizzle.iter_mut().enumerate() {
                *s *= smoothstep(0.1, 1.0, i as f32 / SR);
            }
            layer(&[(&cap, 1.0, 0.0), (&sizzle, 0.3, 0.0)])
        }
        DischargeRelease => {
            // Thunder-crack first, then the electric boom and a sizzling tail.
            let snap = click(r, 0.003);
            let crack = noise(r, 0.06, Pass::High, 2000.0, 2000.0, 0.6, 0.0001, 0.008);
            let thunder = noise(r, 0.5, Pass::Band, 3200.0, 500.0, 0.8, 0.0005, 0.06);
            let boom = thump(1.0, 130.0, 32.0, 0.04, 0.14);
            let mut shock = fm(0.6, 95.0, 60.0, 1.41, 9.0, 0.5, 0.001, 0.16);
            highpass(&mut shock, 150.0);
            let buzz = zap(r, 0.4, 240.0, 70.0, 0.6, 0.5, 0.12);
            let mut sizzle = envelope(crackle(r, 0.9, 500.0, 0.35), 0.02, 0.3);
            highpass(&mut sizzle, 2500.0);
            let mut b = layer(&[
                (&snap, 0.9, 0.0),
                (&crack, 1.4, 0.0),
                (&thunder, 0.9, 0.0),
                (&boom, 0.9, 0.0),
                (&shock, 0.6, 0.0),
                (&buzz, 0.5, 0.0),
                (&sizzle, 0.45, 0.02),
            ]);
            drive(&mut b, 2.0);
            crush(&mut b, 10, 1.5);
            reverb(&mut b, 0.5, 0.5, 0.15, 0.4);
            b
        }
        BombLaunch => {
            let bonk = tone(0.3, Wave::Triangle, 300.0, 150.0, 0.05, 0.001, 0.07);
            let body = thump(0.35, 130.0, 48.0, 0.015, 0.07);
            let puff = noise(r, 0.25, Pass::Low, 1800.0, 350.0, 0.9, 0.001, 0.05);
            let mut b = layer(&[(&bonk, 0.9, 0.0), (&body, 1.0, 0.0), (&puff, 0.7, 0.0)]);
            drive(&mut b, 2.0);
            // The drum ratchets round to the next bomb.
            for (k, at) in [0.13f32, 0.16, 0.19].into_iter().enumerate() {
                let tick = metal(
                    0.04,
                    1700.0 - k as f32 * 120.0,
                    &[(1.0, 1.0, 0.008), (2.3, 0.5, 0.005)],
                );
                let snap = click(r, 0.001);
                mix_at(&mut b, &tick, 0.25, at);
                mix_at(&mut b, &snap, 0.2, at);
            }
            b
        }
        BombBounce => {
            let base = r.range(560.0, 760.0);
            let clank = metal(
                0.18,
                base,
                &[
                    (1.0, 1.0, 0.05),
                    (2.41, 0.7, 0.035),
                    (3.93, 0.45, 0.02),
                    (5.6, 0.2, 0.012),
                ],
            );
            let thud = thump(0.15, 200.0, 85.0, 0.008, 0.025);
            let snap = click(r, 0.0015);
            let mut b = layer(&[(&thud, 1.0, 0.0), (&clank, 0.6, 0.0), (&snap, 0.5, 0.0)]);
            drive(&mut b, 1.6);
            b
        }
        BreachBlast => {
            // Tighter and drier than `Explosion`: crack, short boom, a little debris.
            let crack = noise(r, 0.04, Pass::High, 1500.0, 1500.0, 0.6, 0.0001, 0.006);
            let snap = click(r, 0.003);
            let boom = thump(0.7, 120.0, 34.0, 0.025, 0.12);
            let crunch = noise(r, 0.3, Pass::Band, 1700.0, 450.0, 1.0, 0.0005, 0.045);
            let blast = noise(r, 0.5, Pass::Low, 5000.0, 300.0, 0.8, 0.0008, 0.07);
            let debris = envelope(crackle(r, 0.5, 260.0, 0.14), 0.01, 0.14);
            let mut b = layer(&[
                (&crack, 1.3, 0.0),
                (&snap, 0.8, 0.0),
                (&boom, 1.0, 0.0),
                (&crunch, 0.8, 0.0),
                (&blast, 0.9, 0.0),
                (&debris, 0.45, 0.01),
            ]);
            drive(&mut b, 2.6);
            crush(&mut b, 9, 2.0);
            reverb(&mut b, 0.25, 0.6, 0.1, 0.2);
            b
        }
        GravewellLaunch => {
            let thunk = thump(0.5, 100.0, 38.0, 0.025, 0.1);
            let mut tube = tone(0.35, Wave::Square, 120.0, 72.0, 0.1, 0.002, 0.07);
            lowpass(&mut tube, 700.0);
            let puff = noise(r, 0.3, Pass::Low, 1200.0, 250.0, 0.9, 0.002, 0.06);
            // The canister's clamps lock as it leaves.
            let clamp = metal(
                0.12,
                1250.0,
                &[(1.0, 1.0, 0.025), (1.93, 0.6, 0.018), (3.1, 0.3, 0.01)],
            );
            let clack = metal(0.1, 900.0, &[(1.0, 1.0, 0.02), (2.6, 0.5, 0.012)]);
            let snap = click(r, 0.002);
            let mut b = layer(&[
                (&thunk, 1.0, 0.0),
                (&tube, 0.6, 0.0),
                (&puff, 0.7, 0.0),
                (&snap, 0.4, 0.07),
                (&clamp, 0.35, 0.07),
                (&clack, 0.3, 0.11),
            ]);
            drive(&mut b, 2.2);
            b
        }
        GravewellHum => {
            let dur = 1.6;
            let mut n = Rng::new(r.next_u64());
            let (mut sub, mut saw, mut warble) = (
                Osc::new(Wave::Sine),
                Osc::new(Wave::Saw),
                Osc::new(Wave::Sine),
            );
            let (mut lp, mut bp) = (Svf::default(), Svf::default());
            render(dur, |t, _| {
                let x = t / dur;
                let w = warble.next(lerp(5.0, 13.0, x));
                let f = lerp(48.0, 74.0, x * x) * (1.0 + 0.02 * w);
                let drone = sub.next(f) * 0.8
                    + lp.lowpass(saw.next(f * 2.0), lerp(300.0, 1100.0, x), 1.4) * 0.45;
                // Air being sucked inwards: band noise sliding down, swelling with the pull.
                let suck = bp.bandpass(n.bipolar(), sweep(t, 3000.0, 350.0, dur), 2.0)
                    * (0.25 + 0.5 * x)
                    * (0.7 + 0.3 * w);
                let env = smoothstep(0.0, 0.12, t)
                    * (0.55 + 0.45 * x)
                    * (1.0 - smoothstep(dur - 0.08, dur, t));
                (drone + suck) * env
            })
        }
        GravewellImplode => {
            let pull = 0.36;
            let mut n = Rng::new(r.next_u64());
            let mut svf = Svf::default();
            // A reversed swell: grows exponentially and stops dead at the implosion.
            let suck = render(pull, |t, _| {
                svf.process(n.bipolar(), sweep(t, 300.0, 3500.0, pull), 1.2, SR)
                    .band
                    * (-(pull - t) / 0.09).exp()
            });
            let mut rise = Osc::new(Wave::Sine);
            let groan = render(pull, |t, _| {
                rise.next(sweep(t, 60.0, 240.0, pull)) * (-(pull - t) / 0.12).exp()
            });
            let slam = thump(0.6, 140.0, 26.0, 0.02, 0.16);
            let crunch = noise(r, 0.25, Pass::Low, 2500.0, 300.0, 0.9, 0.0005, 0.04);
            let snap = click(r, 0.002);
            let mut b = layer(&[
                (&suck, 1.0, 0.0),
                (&groan, 0.5, 0.0),
                (&snap, 0.8, pull),
                (&slam, 1.3, pull),
                (&crunch, 0.8, pull),
            ]);
            drive(&mut b, 3.0);
            crush(&mut b, 10, 1.5);
            b
        }
        ThornThrow => {
            let air = whoosh(r, 0.18, 2200.0, 7500.0, 3.0);
            let mut osc = Osc::new(Wave::Sine);
            let whistle = render(0.16, |t, _| {
                osc.next(sweep(t, 3200.0, 5600.0, 0.12)) * bell(t, 0.16)
            });
            layer(&[(&air, 1.0, 0.0), (&whistle, 0.18, 0.01)])
        }
        ThornPlant => {
            let snap = click(r, 0.0015);
            let tunk = thump(0.08, 520.0, 230.0, 0.005, 0.012);
            let glass = metal(
                0.15,
                2650.0,
                &[(1.0, 1.0, 0.03), (2.32, 0.6, 0.02), (4.13, 0.35, 0.012)],
            );
            // Quivering twang: a plucked string with a fast wobble that dies away.
            let mut twang = pluck(r, 0.35, 330.0, 0.996);
            let mut wob = Osc::new(Wave::Sine);
            for (i, s) in twang.iter_mut().enumerate() {
                *s *= perc(i as f32 / SR, 0.002, 0.14) * (0.6 + 0.4 * wob.next(26.0));
            }
            lowpass(&mut twang, 2500.0);
            layer(&[
                (&snap, 0.7, 0.0),
                (&tunk, 0.9, 0.0),
                (&glass, 0.5, 0.0),
                (&twang, 1.0, 0.004),
            ])
        }
        ThornShatter => {
            let spray = glass_spray(r, 7, 0.12, 2800.0, 6000.0);
            let mut fizz = crackle(r, 0.35, 700.0, 0.1);
            highpass(&mut fizz, 3000.0);
            let hiss = noise(r, 0.3, Pass::High, 6000.0, 4000.0, 0.7, 0.002, 0.07);
            layer(&[(&spray, 0.9, 0.0), (&fizz, 0.5, 0.01), (&hiss, 0.35, 0.0)])
        }
        Explosion => {
            let crack = noise(r, 0.05, Pass::High, 1000.0, 1000.0, 0.6, 0.0002, 0.008);
            let boom = thump(2.0, 90.0, 28.0, 0.06, 0.42);
            let body = noise(r, 1.4, Pass::Low, 6000.0, 250.0, 0.8, 0.001, 0.22);
            let rumble = noise(r, 2.0, Pass::Low, 180.0, 80.0, 1.0, 0.01, 0.55);
            let debris = crackle(r, 1.6, 120.0, 0.4);
            let mut b = layer(&[
                (&crack, 1.0, 0.0),
                (&boom, 1.2, 0.0),
                (&body, 1.0, 0.0),
                (&rumble, 0.8, 0.0),
                (&debris, 0.5, 0.01),
            ]);
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
            layer(&[
                (&first, 0.6, 0.0),
                (&slide, 0.5, 0.03),
                (&clack, 0.7, 0.11),
                (&thud, 0.6, 0.11),
            ])
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
            let clang = metal(
                0.2,
                700.0,
                &[(1.0, 1.0, 0.05), (2.3, 0.6, 0.035), (3.7, 0.4, 0.02)],
            );
            let mut b = layer(&[
                (&body, 1.0, 0.0),
                (&crunch, 0.8, 0.0),
                (&snap, 0.6, 0.0),
                (&clang, 0.25, 0.0),
            ]);
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
                render(1.2, |t, _| {
                    (a.next(3520.0) + b.next(3528.0)) * 0.5 * perc(t, 0.001, 0.6)
                })
            };
            let air = whoosh(r, 0.25, 5000.0, 900.0, 1.0);
            let body = thump(0.2, 300.0, 120.0, 0.01, 0.05);
            let mut b = layer(&[
                (&snap, 0.9, 0.0),
                (&clang, 0.9, 0.0),
                (&ring, 0.25, 0.0),
                (&air, 0.7, 0.0),
                (&body, 0.5, 0.0),
            ]);
            for s in b.iter_mut() {
                *s = super::dsp::soft_clip(*s * 1.5);
            }
            reverb(&mut b, 0.7, 0.3, 0.25, 0.6);
            b
        }
        Ricochet => {
            // A thorn shatters and the shot re-fires: glass burst + a bright crystalline chime
            // (pitched up per bounce by the game).
            let base = r.range(1760.0, 1980.0);
            let shatter = noise(r, 0.06, Pass::High, 5500.0, 4000.0, 0.7, 0.0002, 0.012);
            let spray = glass_spray(r, 6, 0.04, 4000.0, 9000.0);
            let chime = metal(
                0.8,
                base,
                &[
                    (1.0, 1.0, 0.3),
                    (1.004, 0.6, 0.3),
                    (2.0, 0.55, 0.2),
                    (3.01, 0.35, 0.14),
                    (4.17, 0.25, 0.09),
                    (5.43, 0.15, 0.06),
                ],
            );
            let mut b = layer(&[
                (&shatter, 0.7, 0.0),
                (&spray, 0.6, 0.0),
                (&chime, 0.8, 0.006),
            ]);
            echo(&mut b, 0.07, 0.3, 0.22, 6000.0, 0.3);
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
            let crack = noise(r, 0.15, Pass::Band, 1500.0, 800.0, 1.0, 0.0005, 0.03);
            let thud = thump(0.12, 250.0, 120.0, 0.01, 0.03);
            let mut b = layer(&[
                (&a, 0.8, 0.0),
                (&b2, 0.8, 0.03),
                (&crack, 0.6, 0.0),
                (&thud, 0.6, 0.0),
            ]);
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
            let clack = metal(
                0.12,
                1900.0,
                &[(1.0, 1.0, 0.03), (1.48, 0.7, 0.025), (2.9, 0.4, 0.015)],
            );
            let snap = click(r, 0.003);
            let thud = thump(0.15, 220.0, 90.0, 0.01, 0.04);
            let grunt = growl(r, 0.12, 160.0, 120.0, [700.0, 1200.0], 0.05, 0.004, 0.04);
            layer(&[
                (&clack, 0.8, 0.0),
                (&snap, 0.8, 0.0),
                (&thud, 0.7, 0.0),
                (&grunt, 0.4, 0.0),
            ])
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
            layer(&[
                (&whoomph, 1.0, 0.0),
                (&embers, 0.5, 0.05),
                (&drone, 0.35, 0.0),
            ])
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
                bp.bandpass(n.bipolar(), 600.0 + 1400.0 * pulse, 1.5)
                    * (0.3 + pulse)
                    * smoothstep(0.0, 0.2, t)
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
            let clang = metal(
                0.5,
                520.0,
                &[(1.0, 1.0, 0.2), (2.4, 0.6, 0.12), (3.9, 0.3, 0.08)],
            );
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
            let mut b = layer(&[
                (&pop, 1.0, 0.0),
                (&crack, 0.8, 0.0),
                (&spray, 0.5, 0.01),
                (&sizzle, 0.3, 0.02),
            ]);
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
            layer(&[
                (&crunch, 0.9, 0.0),
                (&thud, 1.0, 0.0),
                (&flaps, 0.5, 0.05),
                (&squawk, 0.5, 0.02),
            ])
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
                render(dur, |t, _| {
                    o.next(sweep(t, 45.0, 60.0, cue))
                        * smoothstep(0.0, cue, t)
                        * (1.0 - smoothstep(cue, dur, t))
                })
            };
            lowpass(&mut rumble, 250.0);
            let ting = metal(
                0.5,
                2093.0,
                &[(1.0, 1.0, 0.15), (2.92, 0.5, 0.08), (5.1, 0.3, 0.05)],
            );
            let shimmer = metal(0.4, 3136.0, &[(1.0, 0.5, 0.25), (1.5, 0.3, 0.2)]);
            let mut b = layer(&[
                (&grind, 0.8, 0.0),
                (&rumble, 0.6, 0.0),
                (&ting, 0.9, cue),
                (&ting, 0.7, cue + 0.09),
                (&shimmer, 0.4, cue + 0.12),
            ]);
            drive(&mut b, 1.8);
            b
        }
        GaolerBash => {
            // A cell door slammed into you: a huge iron clang over a boom.
            let boom = thump(1.0, 85.0, 28.0, 0.04, 0.3);
            let clang = metal(
                1.1,
                165.0,
                &[
                    (1.0, 1.0, 0.45),
                    (2.31, 0.8, 0.3),
                    (3.62, 0.6, 0.22),
                    (5.7, 0.4, 0.12),
                ],
            );
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
                bp.bandpass(n.bipolar(), 250.0 + 600.0 * pulse, 2.0)
                    * (0.3 + pulse)
                    * smoothstep(0.0, 0.25, t)
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
            layer(&[
                (&air, 1.0, 0.0),
                (&links, 0.6, 0.0),
                (&scrape, 0.4, 0.05),
                (&thud, 0.6, 0.0),
            ])
        }
        GaolerRushWindup => {
            // A snort through the grille, iron feet scraping, a rumble building.
            let dur = 0.9;
            let snort = noise(r, 0.35, Pass::Low, 900.0, 300.0, 1.5, 0.02, 0.1);
            let scrape = noise(r, dur, Pass::Band, 1400.0, 700.0, 3.0, 0.2, 0.4);
            let mut rumble = {
                let mut o = Osc::new(Wave::Saw);
                render(dur, |t, _| {
                    o.next(sweep(t, 40.0, 65.0, dur)) * smoothstep(0.0, dur, t)
                })
            };
            lowpass(&mut rumble, 220.0);
            layer(&[
                (&snort, 1.0, 0.0),
                (&snort, 0.6, 0.45),
                (&scrape, 0.4, 0.1),
                (&rumble, 0.8, 0.0),
            ])
        }
        GaolerRush => {
            // Charging: iron footfalls coming faster.
            let mut b = Vec::new();
            for (k, at) in [0.0f32, 0.18, 0.33, 0.46, 0.57, 0.67].iter().enumerate() {
                let step = thump(0.25, 100.0, 35.0, 0.02, 0.07);
                let clank = metal(
                    0.15,
                    280.0 + 20.0 * k as f32,
                    &[(1.0, 1.0, 0.06), (2.7, 0.5, 0.04)],
                );
                mix_at(&mut b, &step, 1.0, *at);
                mix_at(&mut b, &clank, 0.25, *at);
            }
            let rattle = crackle(r, 0.8, 250.0, 1.0);
            mix_at(&mut b, &rattle, 0.4, 0.0);
            b
        }
        GaolerStep => {
            let thud = thump(0.4, r.range(85.0, 100.0), 35.0, 0.03, 0.1);
            let clank = metal(
                0.25,
                r.range(260.0, 320.0),
                &[(1.0, 1.0, 0.1), (2.6, 0.6, 0.07), (4.1, 0.3, 0.04)],
            );
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
            let (mut saw, mut sine, mut vib) = (
                Osc::new(Wave::Saw),
                Osc::new(Wave::Sine),
                Osc::new(Wave::Sine),
            );
            let mut bp = Svf::default();
            let hum = render(dur, |t, _| {
                let f = 110.0 * (1.0 + 0.01 * vib.next(7.0));
                let x = saw.next(f) * 0.6 + sine.next(f * 4.0) * 0.4;
                bp.bandpass(x, 700.0, 1.2)
                    * smoothstep(0.0, 0.08, t)
                    * (1.0 - smoothstep(1.22, dur, t))
            });
            let mut whine_osc = Osc::new(Wave::Sine);
            let whine = render(dur, |t, _| {
                whine_osc.next(sweep(t, 900.0, 2600.0, 1.05))
                    * smoothstep(0.1, 1.05, t)
                    * (1.0 - smoothstep(1.2, dur, t))
            });
            layer(&[(&hum, 1.0, 0.0), (&whine, 0.25, 0.0)])
        }
        HeronLock => {
            // Locked on: a hard mechanical click and a short high double beep.
            let clk = click(r, 0.002);
            let latch = metal(0.08, 3200.0, &[(1.0, 1.0, 0.02), (1.7, 0.6, 0.015)]);
            let beep = tone(0.06, Wave::Square, 2637.0, 2637.0, 0.01, 0.001, 0.05);
            let mut b = layer(&[
                (&clk, 1.0, 0.0),
                (&latch, 0.7, 0.0),
                (&beep, 0.35, 0.03),
                (&beep, 0.35, 0.11),
            ]);
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
            layer(&[
                (&crack, 0.8, 0.0),
                (&snap, 0.7, 0.0),
                (&thud, 0.8, 0.0),
                (&hiss, 0.25, 0.02),
            ])
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
            let mut b = layer(&[
                (&split, 0.8, 0.0),
                (&thud, 1.0, 0.05),
                (&creak, 0.35, 0.0),
                (&sizzle, 0.35, 0.1),
                (&pops, 0.4, 0.1),
            ]);
            drive(&mut b, 1.8);
            reverb(&mut b, 0.4, 0.6, 0.12, 0.3);
            b
        }
        GaolerDeath => {
            // The jailer falls: the door crashes down, iron collapses, the lantern hisses out.
            let boom = thump(1.2, 70.0, 24.0, 0.05, 0.35);
            let crash = metal(
                1.4,
                140.0,
                &[
                    (1.0, 1.0, 0.6),
                    (2.2, 0.8, 0.4),
                    (3.5, 0.5, 0.3),
                    (6.1, 0.3, 0.15),
                ],
            );
            let chain = crackle(r, 1.0, 300.0, 0.4);
            let hiss = noise(r, 1.6, Pass::High, 4000.0, 2000.0, 0.7, 0.2, 0.6);
            let mut b = layer(&[
                (&boom, 1.2, 0.0),
                (&crash, 0.5, 0.05),
                (&chain, 0.5, 0.1),
                (&hiss, 0.35, 0.3),
            ]);
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
            layer(&[
                (&whoomph, 1.0, 0.0),
                (&rise, 0.4, 0.05),
                (&embers, 0.6, 0.05),
            ])
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
            layer(&[
                (&swell, 0.6, 0.0),
                (&drone, 0.4, 0.0),
                (&zap, 0.6, 0.6),
                (&thud, 0.8, 0.6),
            ])
        }
        ProjectileImpact => {
            // A hot projectile hitting the level: a crack, a thud and a short sizzle.
            let crack_hz = r.range(1400.0, 1800.0);
            let crack = noise(r, 0.18, Pass::Band, crack_hz, 700.0, 1.0, 0.0005, 0.04);
            let thud = thump(0.15, 180.0, 70.0, 0.01, 0.04);
            let sizzle = noise(r, 0.25, Pass::High, 4500.0, 3200.0, 0.7, 0.004, 0.06);
            layer(&[(&crack, 1.0, 0.0), (&thud, 0.7, 0.0), (&sizzle, 0.35, 0.01)])
        }

        // ----- game -----------------------------------------------------------------------
        RankUp => {
            // A-major arpeggio up an octave, last note held with vibrato plus a third on top.
            let arp = [
                (69.0, 0.0, 0.05),
                (73.0, 0.055, 0.05),
                (76.0, 0.11, 0.05),
                (81.0, 0.165, 0.45),
            ];
            let lead = notes(&arp, Wave::Saw, 0.002, 0.15, 4500.0, 0.008);
            let square = notes(&arp, Wave::Square, 0.002, 0.15, 3000.0, 0.008);
            let third = notes(
                &[(85.0, 0.165, 0.45)],
                Wave::Square,
                0.01,
                0.15,
                3500.0,
                0.008,
            );
            let shimmer = noise(r, 0.6, Pass::High, 6000.0, 6000.0, 0.6, 0.002, 0.25);
            // A saw's fundamental is in anti-phase with a square's: invert the square so the
            // layers reinforce instead of cancelling.
            let mut b = layer(&[
                (&lead, 0.6, 0.0),
                (&square, -0.35, 0.0),
                (&third, 0.3, 0.0),
                (&shimmer, 0.15, 0.165),
            ]);
            echo(&mut b, 0.11, 0.3, 0.3, 4000.0, 0.4);
            b
        }
        RankDown => {
            let mut osc = Osc::new(Wave::Triangle);
            let mut b = render(0.45, |t, _| {
                let note = if t < 0.12 {
                    76.0
                } else {
                    72.0 - (t - 0.12) * 1.5
                };
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
            let clank = metal(
                0.8,
                160.0,
                &[
                    (1.0, 1.0, 0.25),
                    (2.33, 0.7, 0.18),
                    (3.71, 0.5, 0.12),
                    (5.9, 0.3, 0.08),
                ],
            );
            let thud = thump(0.4, 100.0, 40.0, 0.02, 0.1);
            let mut alarm = notes(
                &[(57.0, 0.15, 0.13), (57.0, 0.4, 0.13)],
                Wave::Square,
                0.003,
                0.03,
                1500.0,
                0.0,
            );
            crush(&mut alarm, 7, 2.0);
            layer(&[(&clank, 0.6, 0.0), (&thud, 1.0, 0.0), (&alarm, 0.35, 0.0)])
        }
        DoorOpen => {
            let hiss = noise(r, 0.6, Pass::Band, 4000.0, 3000.0, 0.8, 0.05, 0.25);
            let mut whir = tone(0.7, Wave::Saw, 120.0, 240.0, 0.6, 0.05, 0.4);
            lowpass(&mut whir, 900.0);
            let clunk = thump(0.3, 130.0, 50.0, 0.015, 0.07);
            let clank = metal(0.3, 400.0, &[(1.0, 1.0, 0.08), (2.5, 0.5, 0.05)]);
            layer(&[
                (&hiss, 0.5, 0.0),
                (&whir, 0.5, 0.0),
                (&clunk, 1.0, 0.65),
                (&clank, 0.3, 0.65),
            ])
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
            let horn = chord(
                &[40.0, 47.0, 52.0],
                1.0,
                Wave::Saw,
                3000.0,
                600.0,
                0.01,
                0.4,
                0.8,
            );
            let hit = thump(0.5, 120.0, 40.0, 0.02, 0.12);
            let crash = noise(r, 1.0, Pass::High, 5000.0, 5000.0, 0.6, 0.001, 0.35);
            let mut b = layer(&[(&horn, 1.0, 0.0), (&hit, 1.0, 0.0), (&crash, 0.3, 0.0)]);
            drive(&mut b, 3.0);
            b
        }
        EncounterClear => {
            let swell = chord(
                &[52.0, 56.0, 59.0, 64.0],
                1.4,
                Wave::Saw,
                800.0,
                4000.0,
                0.05,
                0.7,
                1.1,
            );
            let top = fm(
                1.0,
                midi_hz(88.0),
                midi_hz(88.0),
                3.0,
                1.5,
                0.1,
                0.002,
                0.35,
            );
            let crash = noise(r, 1.2, Pass::High, 5000.0, 5000.0, 0.6, 0.002, 0.4);
            let mut b = layer(&[(&swell, 1.0, 0.0), (&top, 0.35, 0.05), (&crash, 0.2, 0.0)]);
            reverb(&mut b, 0.6, 0.4, 0.2, 0.5);
            b
        }
        LevelComplete => {
            let run = notes(
                &[
                    (64.0, 0.0, 0.1),
                    (68.0, 0.12, 0.1),
                    (71.0, 0.24, 0.1),
                    (76.0, 0.36, 0.1),
                ],
                Wave::Square,
                0.003,
                0.06,
                4000.0,
                0.0,
            );
            let fin = chord(
                &[76.0, 80.0, 83.0, 88.0],
                1.4,
                Wave::Saw,
                1500.0,
                5000.0,
                0.01,
                0.6,
                1.0,
            );
            let crash = noise(r, 1.2, Pass::High, 5000.0, 5000.0, 0.6, 0.002, 0.45);
            let hit = thump(0.4, 120.0, 45.0, 0.02, 0.1);
            let mut b = layer(&[
                (&run, 0.6, 0.0),
                (&fin, 0.9, 0.48),
                (&crash, 0.25, 0.48),
                (&hit, 0.8, 0.48),
            ]);
            reverb(&mut b, 0.6, 0.4, 0.2, 0.6);
            b
        }
        Countdown => notes(&[(81.0, 0.0, 0.11)], Wave::Square, 0.002, 0.02, 3000.0, 0.0),
        MatchStart => {
            let beep = notes(&[(93.0, 0.0, 0.35)], Wave::Square, 0.002, 0.05, 3500.0, 0.0);
            let stab = chord(
                &[45.0, 52.0, 57.0],
                0.8,
                Wave::Saw,
                3000.0,
                700.0,
                0.005,
                0.3,
                0.6,
            );
            let crash = noise(r, 0.8, Pass::High, 5000.0, 5000.0, 0.6, 0.001, 0.3);
            let mut b = layer(&[(&beep, 0.4, 0.0), (&stab, 1.0, 0.0), (&crash, 0.25, 0.0)]);
            drive(&mut b, 2.0);
            b
        }
        MatchEnd => {
            let mut horn = chord(
                &[45.0, 52.0, 57.0, 60.0],
                1.6,
                Wave::Saw,
                2500.0,
                500.0,
                0.01,
                0.7,
                1.2,
            );
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
            let gong = metal(
                1.8,
                110.0,
                &[
                    (1.0, 1.0, 1.0),
                    (2.14, 0.7, 0.7),
                    (3.47, 0.5, 0.5),
                    (5.03, 0.3, 0.3),
                ],
            );
            let mut b = layer(&[(&horn, 0.8, 0.0), (&gong, 0.5, 0.0)]);
            reverb(&mut b, 0.7, 0.5, 0.2, 0.6);
            b
        }

        // ----- arsenal-client: the expansion arsenal (docs/audio.md "Arsenal") ---------------
        // Each new weapon keeps its own material: the Lancet is thin steel, the Thresher a
        // rougher Bellows with a ratchet, the Riveter compressed air and hammered metal, the
        // Cinderbox fizzing clay rockets, the Longwick wood and string, the Kiln fire.
        LancetFire => {
            // A needle leaving a thin barrel: a tight high "tsik", a steel ping, a puff of air.
            let snap = click(r, 0.001);
            let tsik = noise(r, 0.04, Pass::Band, 9000.0, 5000.0, 1.6, 0.0002, 0.006);
            let ping = metal(
                0.09,
                3300.0,
                &[(1.0, 1.0, 0.018), (2.41, 0.5, 0.012), (3.77, 0.3, 0.008)],
            );
            let puff = noise(r, 0.06, Pass::Low, 2400.0, 900.0, 0.8, 0.001, 0.012);
            let body = thump(0.05, 520.0, 260.0, 0.004, 0.01);
            layer(&[
                (&snap, 0.7, 0.0),
                (&tsik, 1.0, 0.0),
                (&ping, 0.35, 0.002),
                (&puff, 0.5, 0.0),
                (&body, 0.6, 0.0),
            ])
        }
        ThresherFire => {
            // Shorter and rougher than the Bellows: a chunky blast, pellets rattling in the
            // duckbill, then the drum ratcheting round one shell.
            let core = bellows_core(r, 0.95);
            let crack = noise(r, 0.05, Pass::Band, 4200.0, 2000.0, 1.1, 0.0002, 0.01);
            let rattle = envelope(crackle(r, 0.12, 900.0, 0.05), 0.001, 0.04);
            let ratchet = metal(0.05, 1800.0, &[(1.0, 1.0, 0.01), (2.3, 0.5, 0.006)]);
            let thud = thump(0.08, 260.0, 120.0, 0.006, 0.02);
            let mut b = layer(&[
                (&core, 1.0, 0.0),
                (&crack, 0.8, 0.0),
                (&rattle, 0.5, 0.004),
                (&ratchet, 0.35, 0.09),
                (&thud, 0.4, 0.09),
                (&ratchet, 0.3, 0.13),
            ]);
            drive(&mut b, 2.0);
            crush(&mut b, 10, 1.5);
            // A short clean sub under it (the shotguns pass): heavier, still quick enough to
            // repeat every 0.42 s.
            let sub = thump(0.35, 110.0, 40.0, 0.03, 0.08);
            mix_at(&mut b, &sub, 0.4, 0.0);
            b
        }
        RiveterFire => {
            // Pneumatic "pff-tak": a burst of air from the motor and the nose hammering a rivet
            // out, hard and metallic; short enough for eleven a second.
            let hiss = noise(r, 0.05, Pass::High, 3500.0, 2500.0, 0.7, 0.0005, 0.012);
            let snap = click(r, 0.0012);
            let tak = metal(
                0.06,
                1450.0,
                &[(1.0, 1.0, 0.012), (2.13, 0.7, 0.009), (3.4, 0.4, 0.006)],
            );
            let knock = thump(0.05, 300.0, 140.0, 0.004, 0.01);
            let mut b = layer(&[
                (&hiss, 0.6, 0.0),
                (&snap, 0.6, 0.006),
                (&tak, 0.7, 0.006),
                (&knock, 0.8, 0.006),
            ]);
            crush(&mut b, 10, 1.5);
            b
        }
        RivetImpact => {
            // A hot rivet striking: a bright clink and a short sizzle.
            let base = r.range(2100.0, 2600.0);
            let clink = metal(
                0.12,
                base,
                &[(1.0, 1.0, 0.025), (2.76, 0.5, 0.015), (4.1, 0.25, 0.01)],
            );
            let tick = thump(0.04, 600.0, 300.0, 0.003, 0.008);
            let mut sizzle = envelope(crackle(r, 0.25, 900.0, 0.12), 0.01, 0.1);
            highpass(&mut sizzle, 2500.0);
            layer(&[(&clink, 0.7, 0.0), (&tick, 0.6, 0.0), (&sizzle, 0.6, 0.01)])
        }
        CinderboxFire => {
            // One cinder rocket: a pop from the clay tube and a fizzing rush as it leaves.
            let snap = click(r, 0.002);
            let pop = thump(0.12, 210.0, 70.0, 0.008, 0.03);
            let rush = whoosh(r, 0.35, 900.0, 3200.0, 1.0);
            let mut fizz = envelope(crackle(r, 0.35, 600.0, 0.2), 0.005, 0.15);
            highpass(&mut fizz, 1800.0);
            let mut b = layer(&[
                (&snap, 0.6, 0.0),
                (&pop, 1.0, 0.0),
                (&rush, 0.8, 0.01),
                (&fizz, 0.4, 0.02),
            ]);
            drive(&mut b, 1.6);
            b
        }
        CinderBlast => {
            // A small rocket bursting: a crack, a short boom and a spit of cinders. Three land in
            // a row, so it is short and dry.
            let crack = noise(r, 0.04, Pass::High, 1800.0, 1800.0, 0.6, 0.0001, 0.006);
            let boom = thump(0.45, 140.0, 40.0, 0.02, 0.08);
            let body = noise(r, 0.3, Pass::Low, 4000.0, 400.0, 0.8, 0.0008, 0.05);
            let cinders = envelope(crackle(r, 0.45, 500.0, 0.12), 0.01, 0.12);
            let mut b = layer(&[
                (&crack, 1.2, 0.0),
                (&boom, 1.0, 0.0),
                (&body, 0.8, 0.0),
                (&cinders, 0.5, 0.01),
            ]);
            drive(&mut b, 2.4);
            crush(&mut b, 9, 2.0);
            reverb(&mut b, 0.2, 0.6, 0.08, 0.15);
            b
        }
        BowDraw => {
            // Wax-hardened wood creaking under load (stick-slip grains, coming faster as the
            // draw deepens) over the string's rising tension.
            let dur = 0.75;
            let mut n = Rng::new(r.next_u64());
            let mut bp = Svf::default();
            let mut grain = 0.0f32;
            let creak = render(dur, |t, _| {
                if n.chance(lerp(40.0, 140.0, t / dur) / SR) {
                    grain = n.range(0.5, 1.0);
                }
                grain *= 0.985;
                let env = smoothstep(0.0, 0.08, t) * (1.0 - smoothstep(dur - 0.08, dur, t));
                bp.bandpass(grain * n.bipolar(), lerp(700.0, 1300.0, t / dur), 6.0) * env
            });
            let mut tension = Osc::new(Wave::Saw);
            let mut lp = Svf::default();
            let hum = render(dur, |t, _| {
                let x = tension.next(sweep(t, 90.0, 140.0, dur));
                lp.lowpass(x, 600.0, 0.8)
                    * smoothstep(0.0, 0.3, t)
                    * (1.0 - smoothstep(0.6, dur, t))
            });
            layer(&[(&creak, 1.0, 0.0), (&hum, 0.12, 0.0)])
        }
        BowFullDraw => {
            // Full draw: the wick string flares (a soft whoomph) and rings a clear high note.
            let ring = metal(
                0.5,
                1760.0,
                &[(1.0, 1.0, 0.18), (2.0, 0.35, 0.12), (3.01, 0.2, 0.08)],
            );
            let flare = noise(r, 0.25, Pass::Band, 500.0, 1500.0, 0.9, 0.02, 0.08);
            let glint = fm(0.3, 2637.0, 2637.0, 3.5, 2.0, 0.2, 0.002, 0.09);
            layer(&[(&ring, 0.6, 0.0), (&flare, 0.5, 0.0), (&glint, 0.3, 0.0)])
        }
        BowRelease => {
            // The string's low twang, the snap of the limbs and the arrow hissing away.
            let mut twang = pluck(r, 0.4, 98.0, 0.994);
            for (i, s) in twang.iter_mut().enumerate() {
                *s *= perc(i as f32 / SR, 0.001, 0.12);
            }
            lowpass(&mut twang, 1800.0);
            let snap = click(r, 0.002);
            let thwack = thump(0.12, 180.0, 90.0, 0.006, 0.03);
            let hiss = whoosh(r, 0.3, 2500.0, 900.0, 1.2);
            let mut b = layer(&[
                (&snap, 0.6, 0.0),
                (&thwack, 0.8, 0.0),
                (&twang, 1.0, 0.0),
                (&hiss, 0.5, 0.015),
            ]);
            drive(&mut b, 1.4);
            b
        }
        ArrowImpact => {
            // An arrow thunking into something solid and quivering.
            let thunk = thump(0.1, r.range(240.0, 280.0), 110.0, 0.006, 0.02);
            let knock = noise(r, 0.05, Pass::Band, 1500.0, 700.0, 1.2, 0.0005, 0.01);
            let mut quiver = pluck(r, 0.25, 210.0, 0.995);
            let mut wob = Osc::new(Wave::Sine);
            for (i, s) in quiver.iter_mut().enumerate() {
                *s *= perc(i as f32 / SR, 0.002, 0.08) * (0.6 + 0.4 * wob.next(30.0));
            }
            lowpass(&mut quiver, 2000.0);
            layer(&[
                (&thunk, 1.0, 0.0),
                (&knock, 0.6, 0.0),
                (&quiver, 0.6, 0.004),
            ])
        }
        KilnRoar => {
            // The firebox roaring while it sprays: a breathy rumble with crackling embers, soft
            // at both ends so re-triggering it keeps one continuous roar.
            let dur = 0.55;
            let mut n = Rng::new(r.next_u64());
            let (mut lp, mut bp) = (Svf::default(), Svf::default());
            let mut flutter = Osc::new(Wave::Sine);
            let roar = render(dur, |t, _| {
                let x = n.bipolar();
                let low = lp.lowpass(x, 420.0, 0.7);
                let breath = bp.bandpass(x, 1100.0, 1.2) * 0.35;
                (low + breath) * (0.8 + 0.2 * flutter.next(13.0)) * bell(t, dur)
            });
            let pops = envelope(crackle(r, dur, 220.0, 2.0), 0.08, 1.0);
            layer(&[(&roar, 1.0, 0.0), (&pops, 0.35, 0.0)])
        }
        KilnIgnite => {
            // The spray catching: a whoomph of flame and a burst of crackle.
            let whoomph = noise(r, 0.4, Pass::Low, 300.0, 1600.0, 0.9, 0.03, 0.12);
            let body = thump(0.3, 90.0, 60.0, 0.03, 0.08);
            let pops = envelope(crackle(r, 0.4, 500.0, 0.2), 0.01, 0.15);
            layer(&[(&whoomph, 1.0, 0.0), (&body, 0.6, 0.0), (&pops, 0.4, 0.02)])
        }
        SkewerCharge => {
            // A wire drawn taut through the barrel: a glassy whine climbing 600 Hz → 2.4 kHz over
            // 0.8 s, then holding, with a trembling shimmer. 1.2 s; stop it on release.
            let dur = 1.2;
            let mut whine = Osc::new(Wave::Triangle);
            let mut over = Osc::new(Wave::Sine);
            let mut trem = Osc::new(Wave::Sine);
            let wire = render(dur, |t, _| {
                let f = sweep(t.min(0.8), 600.0, 2400.0, 0.8);
                let x = whine.next(f) * 0.6 + over.next(f * 2.01) * 0.25;
                let env = smoothstep(0.0, 0.2, t) * (1.0 - smoothstep(1.1, dur, t));
                let rate = lerp(6.0, 22.0, (t / 0.8).min(1.0));
                x * (0.8 + 0.2 * trem.next(rate)) * env
            });
            let shimmer = noise(r, dur, Pass::High, 6000.0, 8000.0, 0.7, 0.3, 2.0);
            layer(&[(&wire, 1.0, 0.0), (&shimmer, 0.15, 0.0)])
        }
        SkewerFire => {
            // A heavy crack and a long ringing "zing" through everything in line.
            let crack = quill_crack(r, 0.7);
            let boom = thump(0.25, 200.0, 60.0, 0.012, 0.05);
            let zing = tone(0.5, Wave::Sine, 4200.0, 1400.0, 0.45, 0.001, 0.14);
            let ring = metal(
                0.6,
                1900.0,
                &[(1.0, 1.0, 0.2), (2.76, 0.5, 0.12), (5.4, 0.3, 0.06)],
            );
            let mut b = layer(&[
                (&crack, 1.0, 0.0),
                (&boom, 0.7, 0.0),
                (&zing, 0.3, 0.005),
                (&ring, 0.25, 0.0),
            ]);
            drive(&mut b, 1.5);
            echo(&mut b, 0.09, 0.25, 0.2, 3000.0, 0.2);
            b
        }
        FlurryFire => {
            // Three needles 0.06 s apart, each a little higher.
            let mut b = Vec::new();
            for k in 0..3 {
                let tsik = noise(r, 0.04, Pass::Band, 8500.0, 4800.0, 1.6, 0.0002, 0.006);
                let ping = metal(
                    0.08,
                    3100.0 + k as f32 * 260.0,
                    &[(1.0, 1.0, 0.016), (2.41, 0.5, 0.01)],
                );
                let body = thump(0.05, 480.0, 240.0, 0.004, 0.01);
                let shot = layer(&[(&tsik, 1.0, 0.0), (&ping, 0.4, 0.002), (&body, 0.6, 0.0)]);
                mix_at(&mut b, &shot, 1.0 - k as f32 * 0.08, k as f32 * 0.06);
            }
            b
        }
        SlugFire => {
            // One heavy slug: a deep crack-boom and the barrel ringing after it.
            let core = bellows_core(r, 1.1);
            let crack = noise(r, 0.06, Pass::High, 2600.0, 2600.0, 0.6, 0.0001, 0.008);
            let ring = metal(
                0.7,
                620.0,
                &[(1.0, 1.0, 0.25), (2.31, 0.6, 0.15), (3.97, 0.3, 0.08)],
            );
            let mut b = layer(&[(&core, 1.0, 0.0), (&crack, 1.3, 0.0), (&ring, 0.25, 0.01)]);
            drive(&mut b, 2.4);
            crush(&mut b, 10, 1.5);
            reverb(&mut b, 0.3, 0.6, 0.1, 0.25);
            b
        }
        FlakFire => {
            // The shell thumped out of the barrel and its timer's rising whistle.
            let thunk = thump(0.3, 160.0, 55.0, 0.015, 0.06);
            let puff = noise(r, 0.2, Pass::Low, 2200.0, 500.0, 0.8, 0.001, 0.04);
            let mut osc = Osc::new(Wave::Sine);
            let whistle = render(0.35, |t, _| {
                osc.next(sweep(t, 1800.0, 3400.0, 0.35)) * bell(t, 0.35)
            });
            let tick = metal(0.05, 2400.0, &[(1.0, 1.0, 0.01), (2.0, 0.4, 0.008)]);
            let mut b = layer(&[
                (&thunk, 1.0, 0.0),
                (&puff, 0.6, 0.0),
                (&whistle, 0.15, 0.03),
                (&tick, 0.3, 0.02),
            ]);
            drive(&mut b, 1.8);
            b
        }
        FlakBurst => {
            // The airburst: a sharp crack in the air and a ring of pellets hissing outwards.
            let crack = noise(r, 0.04, Pass::High, 2200.0, 2200.0, 0.6, 0.0001, 0.006);
            let boom = thump(0.35, 170.0, 60.0, 0.015, 0.06);
            let spray = glass_spray(r, 10, 0.08, 1800.0, 4200.0);
            let hiss = noise(r, 0.35, Pass::High, 5000.0, 2500.0, 0.8, 0.002, 0.08);
            let mut b = layer(&[
                (&crack, 1.2, 0.0),
                (&boom, 0.9, 0.0),
                (&spray, 0.6, 0.005),
                (&hiss, 0.4, 0.0),
            ]);
            drive(&mut b, 2.0);
            reverb(&mut b, 0.35, 0.5, 0.12, 0.3);
            b
        }
        VolleyFire => {
            // Ten shots dumped at once: a ripping burst of hard ticks over a gust of air.
            let mut b = Vec::new();
            for k in 0..10 {
                let at = k as f32 * 0.012 + r.range(0.0, 0.004);
                let tick = metal(
                    0.04,
                    r.range(1600.0, 2200.0),
                    &[(1.0, 1.0, 0.008), (2.3, 0.5, 0.006)],
                );
                let knock = thump(0.04, 320.0, 150.0, 0.003, 0.008);
                mix_at(&mut b, &tick, 0.5, at);
                mix_at(&mut b, &knock, 0.7, at);
            }
            let rip = noise(r, 0.2, Pass::Band, 3000.0, 1200.0, 0.8, 0.002, 0.06);
            let boom = thump(0.25, 140.0, 50.0, 0.015, 0.05);
            mix_at(&mut b, &rip, 0.7, 0.0);
            mix_at(&mut b, &boom, 0.8, 0.0);
            drive(&mut b, 1.8);
            b
        }
        FlareLaunch => {
            // A pop and the flare's fizzing hiss climbing away.
            let snap = click(r, 0.002);
            let pop = thump(0.1, 300.0, 120.0, 0.006, 0.02);
            let hiss = noise(r, 0.6, Pass::Band, 2500.0, 5000.0, 1.0, 0.01, 0.25);
            let mut fizz = envelope(crackle(r, 0.6, 1400.0, 1.0), 0.01, 0.3);
            highpass(&mut fizz, 2000.0);
            layer(&[
                (&snap, 0.6, 0.0),
                (&pop, 1.0, 0.0),
                (&hiss, 0.6, 0.01),
                (&fizz, 0.4, 0.01),
            ])
        }
        FlareBurn => {
            // The flare landing and the ground catching: a whoomph, then a crackling burn.
            let whoomph = noise(r, 0.5, Pass::Low, 250.0, 1400.0, 0.9, 0.03, 0.15);
            let body = thump(0.4, 80.0, 50.0, 0.04, 0.1);
            let burn = envelope(crackle(r, 1.6, 260.0, 4.0), 0.05, 0.7);
            let rumble = noise(r, 1.6, Pass::Low, 380.0, 280.0, 0.7, 0.08, 0.7);
            layer(&[
                (&whoomph, 1.0, 0.0),
                (&body, 0.5, 0.0),
                (&burn, 0.5, 0.05),
                (&rumble, 0.4, 0.0),
            ])
        }
        LimpetThrow => {
            let swish = whoosh(r, 0.2, 1200.0, 3000.0, 1.1);
            let rattle = metal(0.08, 1500.0, &[(1.0, 1.0, 0.015), (2.6, 0.5, 0.01)]);
            layer(&[
                (&swish, 1.0, 0.0),
                (&rattle, 0.4, 0.0),
                (&rattle, 0.3, 0.05),
            ])
        }
        LimpetStick => {
            // The claws biting: a heavy clank and four quick ticks as they grip.
            let thud = thump(0.12, 200.0, 90.0, 0.008, 0.03);
            let clank = metal(
                0.25,
                520.0,
                &[(1.0, 1.0, 0.06), (2.43, 0.6, 0.04), (3.9, 0.3, 0.025)],
            );
            let mut b = layer(&[(&thud, 1.0, 0.0), (&clank, 0.5, 0.0)]);
            for k in 0..4 {
                let tick = metal(
                    0.04,
                    2600.0 + k as f32 * 150.0,
                    &[(1.0, 1.0, 0.008), (1.9, 0.4, 0.006)],
                );
                mix_at(&mut b, &tick, 0.35, 0.05 + k as f32 * 0.025);
            }
            b
        }
        LimpetBlink => {
            // An armed limpet's lamp: a tiny square beep.
            let mut b = tone(0.06, Wave::Square, 1760.0, 1760.0, 0.01, 0.001, 0.02);
            lowpass(&mut b, 5000.0);
            crush(&mut b, 7, 1.0);
            b
        }
        LimpetDetonate => {
            // Every live charge going off: a hard double crack and a boom.
            let crack = noise(r, 0.04, Pass::High, 1600.0, 1600.0, 0.6, 0.0001, 0.006);
            let boom = thump(0.8, 110.0, 32.0, 0.03, 0.14);
            let body = noise(r, 0.5, Pass::Low, 5000.0, 300.0, 0.8, 0.0008, 0.08);
            let debris = envelope(crackle(r, 0.6, 200.0, 0.2), 0.01, 0.2);
            let mut b = layer(&[
                (&crack, 1.2, 0.0),
                (&crack, 0.7, 0.03),
                (&boom, 1.1, 0.0),
                (&body, 0.9, 0.0),
                (&debris, 0.4, 0.02),
            ]);
            drive(&mut b, 2.5);
            crush(&mut b, 9, 2.0);
            reverb(&mut b, 0.35, 0.6, 0.12, 0.3);
            b
        }
        MortarLaunch => {
            // A deep tube "thoomp".
            let thoomp = thump(0.5, 95.0, 40.0, 0.03, 0.1);
            let mut tube = tone(0.4, Wave::Triangle, 160.0, 90.0, 0.15, 0.003, 0.08);
            lowpass(&mut tube, 600.0);
            let puff = noise(r, 0.35, Pass::Low, 900.0, 200.0, 0.9, 0.003, 0.08);
            let mut b = layer(&[(&thoomp, 1.0, 0.0), (&tube, 0.6, 0.0), (&puff, 0.7, 0.0)]);
            drive(&mut b, 2.0);
            b
        }
        MortarBlast => {
            // The heavy shell: the arsenal's biggest boom, a long rumble and falling debris.
            let crack = noise(r, 0.06, Pass::High, 900.0, 900.0, 0.6, 0.0002, 0.01);
            let boom = thump(2.0, 75.0, 24.0, 0.07, 0.5);
            let body = noise(r, 1.4, Pass::Low, 5000.0, 200.0, 0.8, 0.001, 0.25);
            let rumble = noise(r, 2.2, Pass::Low, 150.0, 70.0, 1.0, 0.01, 0.6);
            let debris = crackle(r, 2.0, 160.0, 0.6);
            let mut b = layer(&[
                (&crack, 1.0, 0.0),
                (&boom, 1.3, 0.0),
                (&body, 1.0, 0.0),
                (&rumble, 0.9, 0.0),
                (&debris, 0.5, 0.02),
            ]);
            drive(&mut b, 2.4);
            crush(&mut b, 10, 2.0);
            reverb(&mut b, 0.7, 0.6, 0.2, 0.7);
            b
        }
        TripleNock => {
            // Three arrows at once: a spread chord of twangs and a wider hiss.
            let mut b = Vec::new();
            for (k, f) in [98.0f32, 123.5, 146.8].into_iter().enumerate() {
                let mut twang = pluck(r, 0.35, f, 0.993);
                for (i, s) in twang.iter_mut().enumerate() {
                    *s *= perc(i as f32 / SR, 0.001, 0.1);
                }
                lowpass(&mut twang, 2000.0);
                mix_at(&mut b, &twang, 0.8, k as f32 * 0.012);
            }
            let snap = click(r, 0.002);
            let hiss = whoosh(r, 0.35, 3000.0, 800.0, 0.9);
            mix_at(&mut b, &snap, 0.6, 0.0);
            mix_at(&mut b, &hiss, 0.6, 0.015);
            drive(&mut b, 1.4);
            b
        }
        PinningRelease => {
            // A heavier, lower twang and the pin's chain rattling out after the arrow.
            let mut twang = pluck(r, 0.45, 82.0, 0.995);
            for (i, s) in twang.iter_mut().enumerate() {
                *s *= perc(i as f32 / SR, 0.001, 0.15);
            }
            lowpass(&mut twang, 1500.0);
            let thwack = thump(0.15, 150.0, 70.0, 0.008, 0.04);
            let mut b = layer(&[(&thwack, 0.9, 0.0), (&twang, 1.0, 0.0)]);
            for k in 0..6 {
                let link = metal(
                    0.05,
                    r.range(2200.0, 3200.0),
                    &[(1.0, 1.0, 0.01), (2.7, 0.4, 0.007)],
                );
                mix_at(
                    &mut b,
                    &link,
                    0.3,
                    0.02 + k as f32 * 0.025 + r.range(0.0, 0.01),
                );
            }
            drive(&mut b, 1.4);
            b
        }
        PinThunk => {
            // A pinning arrow driving home: a heavy thunk, then the chain going taut with a clank.
            let thunk = thump(0.18, 200.0, 70.0, 0.008, 0.04);
            let knock = noise(r, 0.06, Pass::Band, 1300.0, 600.0, 1.2, 0.0005, 0.012);
            let clank = metal(
                0.3,
                740.0,
                &[(1.0, 1.0, 0.07), (2.51, 0.6, 0.05), (4.2, 0.3, 0.03)],
            );
            let rattle = envelope(crackle(r, 0.2, 600.0, 0.1), 0.002, 0.08);
            let mut b = layer(&[
                (&thunk, 1.0, 0.0),
                (&knock, 0.6, 0.0),
                (&rattle, 0.4, 0.03),
                (&clank, 0.5, 0.06),
            ]);
            drive(&mut b, 1.6);
            b
        }
        EmberArrowBurst => {
            // The ember head bursting: a crack, a whoomph of flame and a shower of embers.
            let crack = noise(r, 0.04, Pass::High, 2000.0, 2000.0, 0.6, 0.0001, 0.006);
            let whoomph = noise(r, 0.45, Pass::Low, 250.0, 1800.0, 0.9, 0.01, 0.12);
            let boom = thump(0.35, 120.0, 50.0, 0.015, 0.07);
            let mut embers = envelope(crackle(r, 0.8, 700.0, 0.3), 0.01, 0.3);
            highpass(&mut embers, 1500.0);
            let mut b = layer(&[
                (&crack, 0.9, 0.0),
                (&whoomph, 1.0, 0.0),
                (&boom, 0.8, 0.0),
                (&embers, 0.5, 0.02),
            ]);
            drive(&mut b, 1.8);
            reverb(&mut b, 0.25, 0.6, 0.1, 0.2);
            b
        }
        BurnSizzle => {
            // Something burning: a fizzing sizzle, a guttering flame and a few pops.
            let sizzle = noise(r, 0.6, Pass::High, 4500.0, 3000.0, 0.8, 0.05, 0.25);
            let flame = gutter(r, 0.6, 900.0, 600.0, 24.0, 0.3);
            let pops = envelope(crackle(r, 0.6, 300.0, 1.0), 0.02, 0.3);
            layer(&[(&sizzle, 0.5, 0.0), (&flame, 0.7, 0.0), (&pops, 0.4, 0.0)])
        }
        // ----- potg ------------------------------------------------------------------------
        PotgStinger => {
            // A short riser, then everything at once: a sub boom, a detuned brass stab in
            // D minor (add 9) closing down, the foundry bell and a crash of sparks.
            let riser = whoosh(r, 0.32, 400.0, 5000.0, 1.4);
            let boom = thump(1.2, 110.0, 32.0, 0.09, 0.45);
            let stab = chord(
                &[50.0, 57.0, 62.0, 65.0, 69.0, 76.0],
                1.9,
                Wave::Saw,
                5200.0,
                600.0,
                0.004,
                0.8,
                1.5,
            );
            let bell = metal(
                2.2,
                146.8,
                &[
                    (1.0, 1.0, 1.4),
                    (2.0, 0.55, 0.9),
                    (2.76, 0.45, 0.7),
                    (4.07, 0.3, 0.45),
                    (5.43, 0.2, 0.3),
                ],
            );
            let crash = noise(r, 1.4, Pass::High, 7000.0, 2500.0, 0.6, 0.002, 0.45);
            let sparks = crackle(r, 1.2, 90.0, 0.35);
            let hit = 0.3;
            let mut b = layer(&[
                (&riser, 0.35, 0.0),
                (&boom, 1.0, hit),
                (&stab, 0.75, hit),
                (&bell, 0.4, hit),
                (&crash, 0.3, hit),
                (&sparks, 0.25, hit + 0.05),
            ]);
            reverb(&mut b, 0.8, 0.45, 0.25, 0.8);
            b
        }
        PotgWhoosh => {
            // Air rushing past, rising, with a soft landing thud: cut to the replay.
            let air = whoosh(r, 0.75, 250.0, 3800.0, 1.1);
            let thud = thump(0.35, 90.0, 45.0, 0.03, 0.12);
            let mut b = layer(&[(&air, 1.0, 0.0), (&thud, 0.5, 0.55)]);
            reverb(&mut b, 0.5, 0.5, 0.15, 0.3);
            b
        }
        PotgSlowmo => {
            // Time dragging: a deep tone falling an octave and a half under a low rumble.
            let fall = tone(1.5, Wave::Triangle, 196.0, 52.0, 1.1, 0.01, 0.7);
            let rumble = noise(r, 1.4, Pass::Low, 500.0, 70.0, 0.9, 0.05, 0.6);
            let boom = thump(0.9, 70.0, 30.0, 0.12, 0.35);
            let mut b = layer(&[(&fall, 0.8, 0.0), (&rumble, 0.5, 0.0), (&boom, 0.7, 0.0)]);
            reverb(&mut b, 0.7, 0.6, 0.2, 0.5);
            b
        }

        // ----- enemies: the third roster (docs/enemies.md) ---------------------------------
        BellwetherAlert => {
            // A hollow groan through the hood over the bell humming in sympathy.
            let groan = growl(r, 1.0, 70.0, 58.0, [380.0, 820.0], 0.04, 0.2, 0.45);
            let hum = metal(
                1.2,
                98.0,
                &[(1.0, 0.6, 0.9), (2.4, 0.4, 0.6), (3.1, 0.25, 0.5)],
            );
            let mut b = layer(&[(&groan, 1.0, 0.0), (&hum, 0.35, 0.05)]);
            drive(&mut b, 1.8);
            reverb(&mut b, 0.6, 0.5, 0.2, 0.5);
            b
        }
        BellwetherTollWindup => {
            // Hauling the bell back: the yoke's timbers creak, a bowed-metal hum swells and
            // climbs into a shriek, cut just before the strike (1.6 s).
            let dur = 1.6;
            let mut creak = tone(dur, Wave::Saw, 140.0, 95.0, dur, 0.2, 0.9);
            lowpass(&mut creak, 900.0);
            stutter(&mut creak, 23.0, 0.0);
            let mut bow = {
                let mut o = Osc::new(Wave::Saw);
                let mut o2 = Osc::new(Wave::Saw);
                render(dur, |t, _| {
                    let f = sweep(t, 196.0, 523.0, dur);
                    let env =
                        smoothstep(0.0, dur * 0.9, t) * (1.0 - smoothstep(dur - 0.08, dur, t));
                    (o.next(f) + 0.6 * o2.next(f * 2.73)) * env
                })
            };
            lowpass(&mut bow, 2600.0);
            let hiss = noise(r, dur, Pass::High, 2500.0, 7000.0, 0.7, 1.2, 0.3);
            let mut b = layer(&[(&creak, 0.6, 0.0), (&bow, 0.8, 0.0), (&hiss, 0.25, 0.0)]);
            fade_out(&mut b, 0.03);
            b
        }
        BellwetherToll => {
            // One deep bronze strike: a long inharmonic bell over a sub boom, the beating of its
            // partials swelling the tail. Loud: it means every Mould near it is about to come
            // at you twice as hard.
            let strike = metal(
                3.2,
                73.4,
                &[
                    (0.5, 0.9, 2.6),
                    (1.0, 1.0, 2.2),
                    (1.19, 0.7, 1.8),
                    (1.5, 0.45, 1.5),
                    (2.0, 0.5, 1.2),
                    (2.52, 0.35, 0.8),
                    (3.01, 0.25, 0.6),
                ],
            );
            let beat = metal(3.0, 74.1, &[(1.0, 0.6, 2.0), (2.0, 0.3, 1.0)]);
            let boom = thump(1.4, 70.0, 30.0, 0.05, 0.45);
            let clank = click(r, 0.004);
            let mut b = layer(&[
                (&strike, 1.0, 0.0),
                (&beat, 0.6, 0.0),
                (&boom, 0.9, 0.0),
                (&clank, 0.5, 0.0),
            ]);
            drive(&mut b, 1.4);
            reverb(&mut b, 0.8, 0.5, 0.25, 0.6);
            b
        }
        BellCrack => {
            // The bell hit mid-swing: a cracked, buzzing clank (no ring), a wooden crack.
            let mut clank = metal(
                0.6,
                233.0,
                &[
                    (1.0, 1.0, 0.18),
                    (1.37, 0.8, 0.12),
                    (2.71, 0.6, 0.08),
                    (4.4, 0.4, 0.05),
                ],
            );
            crush(&mut clank, 6, 3.0);
            let crack = noise(r, 0.25, Pass::Band, 2400.0, 700.0, 1.2, 0.0005, 0.05);
            let thud = thump(0.3, 160.0, 60.0, 0.01, 0.08);
            let mut b = layer(&[(&clank, 1.0, 0.0), (&crack, 0.8, 0.0), (&thud, 0.6, 0.0)]);
            drive(&mut b, 2.2);
            b
        }
        Frenzied => {
            // Heat flaring up inside a Mould: a rising hiss and a short snarl.
            let flare = noise(r, 0.45, Pass::Band, 900.0, 4200.0, 1.3, 0.04, 0.2);
            let snarl = growl(r, 0.35, 140.0, 260.0, [800.0, 1700.0], 0.12, 0.03, 0.15);
            let mut b = layer(&[(&flare, 1.0, 0.0), (&snarl, 0.6, 0.05)]);
            drive(&mut b, 2.0);
            b
        }
        BellwetherClapperWindup => {
            // A fist drawn back: knuckles creaking, the bell clinking as it shifts.
            let dur = 0.6;
            let creak = noise(r, dur, Pass::Band, 500.0, 1100.0, 5.0, 0.3, 0.3);
            let clink = metal(0.4, 880.0, &[(1.0, 0.6, 0.15), (2.7, 0.3, 0.08)]);
            layer(&[(&creak, 1.0, 0.0), (&clink, 0.35, 0.2)])
        }
        BellwetherClapper => {
            let air = whoosh(r, 0.35, 2000.0, 400.0, 1.2);
            let thud = thump(0.3, 130.0, 55.0, 0.01, 0.08);
            let ding = metal(0.6, 196.0, &[(1.0, 0.5, 0.3), (2.4, 0.3, 0.2)]);
            layer(&[(&air, 1.0, 0.0), (&thud, 0.8, 0.0), (&ding, 0.3, 0.02)])
        }
        SnufferAlert => {
            // Brass legs ticking, a dry chitter under the cap.
            let ticks = crackle(r, 0.5, 90.0, 0.3);
            let mut chitter = growl(r, 0.45, 300.0, 380.0, [1500.0, 2900.0], 0.2, 0.02, 0.2);
            stutter(&mut chitter, 30.0, 0.0);
            let mut b = layer(&[(&ticks, 0.8, 0.0), (&chitter, 0.8, 0.03)]);
            highpass(&mut b, 300.0);
            b
        }
        SnufferPounceWindup => {
            // Rearing back: air hissing out from under the lifted cap, rising; legs clicking.
            let dur = 0.7;
            let hiss = noise(r, dur, Pass::Band, 1200.0, 5200.0, 2.0, 0.5, 0.2);
            let clicks = crackle(r, dur, 50.0, 2.0);
            let mut b = layer(&[(&hiss, 1.0, 0.0), (&clicks, 0.6, 0.0)]);
            fade_out(&mut b, 0.02);
            b
        }
        SnufferLatch => {
            // Clamped on: a muffled brass clank, the flame inside it pressed out with a puff.
            let mut clank = metal(0.35, 520.0, &[(1.0, 1.0, 0.08), (2.2, 0.6, 0.05)]);
            lowpass(&mut clank, 2200.0);
            let thud = thump(0.3, 150.0, 60.0, 0.01, 0.07);
            let puff = noise(r, 0.35, Pass::Low, 1800.0, 300.0, 0.8, 0.01, 0.08);
            layer(&[(&clank, 0.9, 0.0), (&thud, 0.9, 0.0), (&puff, 0.7, 0.04)])
        }
        SnufferSmother => {
            // Your wick being smothered: a muffled flame guttering under a cap.
            let mut flame = gutter(r, 0.6, 700.0, 240.0, 14.0, 0.3);
            lowpass(&mut flame, 1500.0);
            let hum = tone(0.6, Wave::Sine, 110.0, 95.0, 0.6, 0.05, 0.3);
            layer(&[(&flame, 1.0, 0.0), (&hum, 0.3, 0.0)])
        }
        SnufferThrown => {
            // Punched off: the cap rings, legs skitter on the floor.
            let ring = metal(
                0.7,
                660.0,
                &[(1.0, 1.0, 0.3), (2.76, 0.5, 0.15), (5.4, 0.25, 0.08)],
            );
            let skitter = crackle(r, 0.5, 120.0, 0.3);
            let thud = thump(0.2, 180.0, 70.0, 0.01, 0.05);
            layer(&[(&ring, 0.8, 0.0), (&skitter, 0.6, 0.08), (&thud, 0.8, 0.0)])
        }
        SnufferNip => {
            // The cap's rim snapping shut: a brass snip.
            let snip = metal(0.15, 1400.0, &[(1.0, 1.0, 0.03), (1.6, 0.6, 0.02)]);
            let snap = click(r, 0.002);
            layer(&[(&snip, 0.9, 0.0), (&snap, 0.8, 0.0)])
        }
        SmelterAlert => {
            // A furnace roaring up through the grate, chains shaken.
            let dur = 1.1;
            let mut n = Rng::new(r.next_u64());
            let mut lp = Svf::default();
            let roar = render(dur, |t, _| {
                let env = smoothstep(0.0, 0.15, t) * (-(t - 0.15).max(0.0) / 0.4).exp();
                lp.lowpass(n.bipolar(), sweep(t, 250.0, 900.0, 0.3), 1.2) * env
            });
            let bellow = growl(r, 0.9, 52.0, 45.0, [300.0, 600.0], 0.05, 0.1, 0.4);
            let rattle = crackle(r, 0.7, 160.0, 0.4);
            let mut b = layer(&[(&roar, 1.0, 0.0), (&bellow, 0.6, 0.0), (&rattle, 0.4, 0.1)]);
            drive(&mut b, 2.0);
            reverb(&mut b, 0.6, 0.5, 0.2, 0.5);
            b
        }
        SmelterPourWindup => {
            // The crucible tipping (1.1 s): iron grinding, slag bubbling up to the lip.
            let dur = 1.1;
            let grind = noise(r, dur, Pass::Band, 300.0, 900.0, 5.0, 0.4, 0.4);
            let mut bubbles = crackle(r, dur, 60.0, 4.0);
            lowpass(&mut bubbles, 1200.0);
            let rumble = tone(dur, Wave::Saw, 48.0, 62.0, dur, 0.5, 0.6);
            let mut b = layer(&[
                (&grind, 0.9, 0.0),
                (&bubbles, 0.9, 0.0),
                (&rumble, 0.3, 0.0),
            ]);
            fade_out(&mut b, 0.03);
            b
        }
        SmelterPour => {
            // A heavy molten gush that hisses as it hits the floor and runs along it.
            let gush = noise(r, 1.2, Pass::Low, 1200.0, 400.0, 0.8, 0.05, 0.5);
            let hiss = noise(r, 1.4, Pass::High, 4000.0, 2500.0, 0.7, 0.1, 0.6);
            let spits = crackle(r, 1.2, 220.0, 0.6);
            let thud = thump(0.4, 90.0, 40.0, 0.02, 0.12);
            let mut b = layer(&[
                (&gush, 1.0, 0.0),
                (&hiss, 0.5, 0.1),
                (&spits, 0.5, 0.1),
                (&thud, 0.6, 0.0),
            ]);
            drive(&mut b, 1.6);
            b
        }
        SlagBurn => {
            // Standing in slag: a sharp sizzle.
            let sizzle = noise(r, 0.35, Pass::High, 5000.0, 3000.0, 0.8, 0.003, 0.12);
            let spits = crackle(r, 0.3, 300.0, 0.15);
            layer(&[(&sizzle, 1.0, 0.0), (&spits, 0.6, 0.0)])
        }
        SmelterSwingWindup => {
            // Hauling the crucible round: chains snapping taut, iron groaning.
            let dur = 0.8;
            let links = crackle(r, dur, 200.0, 1.5);
            let mut groan = tone(dur, Wave::Saw, 90.0, 70.0, dur, 0.2, 0.5);
            lowpass(&mut groan, 600.0);
            layer(&[(&links, 0.8, 0.0), (&groan, 0.6, 0.0)])
        }
        SmelterSwing => {
            let air = whoosh(r, 0.55, 250.0, 1400.0, 1.4);
            let clang = metal(
                0.8,
                110.0,
                &[(1.0, 1.0, 0.4), (2.3, 0.6, 0.25), (3.8, 0.4, 0.15)],
            );
            let links = crackle(r, 0.4, 350.0, 0.2);
            let mut b = layer(&[(&air, 1.0, 0.0), (&clang, 0.4, 0.15), (&links, 0.5, 0.0)]);
            drive(&mut b, 1.6);
            b
        }
        SmelterStep => {
            // A heavy footfall, the slag in the crucible sloshing.
            let thud = thump(0.35, 75.0, 35.0, 0.02, 0.1);
            let slosh = noise(r, 0.3, Pass::Low, 700.0, 250.0, 1.0, 0.03, 0.1);
            layer(&[(&thud, 1.0, 0.0), (&slosh, 0.4, 0.04)])
        }
        EffigyHit => {
            // A shot into packed wax and the post creaking as it rocks.
            let thunk = thump(0.2, 210.0, 90.0, 0.01, 0.05);
            let crack = noise(r, 0.12, Pass::Band, 1800.0, 900.0, 1.0, 0.0005, 0.03);
            let mut creak = tone(0.35, Wave::Saw, 180.0, 140.0, 0.35, 0.05, 0.15);
            lowpass(&mut creak, 1000.0);
            stutter(&mut creak, 30.0, 0.0);
            layer(&[(&thunk, 1.0, 0.0), (&crack, 0.6, 0.0), (&creak, 0.35, 0.06)])
        }
        MouldMelt => {
            // A Mould slumping into a puddle: a wet, sagging groan falling away, its heat
            // hissing out into the floor, the wax bubbling as it spreads.
            let dur = 0.75;
            let mut sag = tone(dur, Wave::Saw, 130.0, 42.0, dur, 0.04, 0.5);
            lowpass(&mut sag, 520.0);
            let hiss = noise(r, dur, Pass::High, 3800.0, 2200.0, 0.7, 0.08, 0.35);
            let bubbles = envelope(crackle(r, dur, 110.0, 0.9), 0.06, 0.45);
            let slosh = noise(r, 0.4, Pass::Low, 600.0, 200.0, 1.2, 0.02, 0.18);
            layer(&[
                (&sag, 0.75, 0.0),
                (&slosh, 0.6, 0.05),
                (&hiss, 0.35, 0.12),
                (&bubbles, 0.5, 0.15),
            ])
        }
        MouldRise => {
            // Rising out of a puddle: bubbling welling up, a wet groan climbing, a slap as it
            // stands.
            let dur = 0.85;
            let bubbles = envelope(crackle(r, dur, 150.0, 1.2), 0.25, 0.3);
            let mut groan = tone(dur, Wave::Saw, 48.0, 115.0, dur, 0.3, 0.3);
            lowpass(&mut groan, 640.0);
            let slap = thump(0.25, 170.0, 65.0, 0.01, 0.12);
            let slosh = noise(r, 0.3, Pass::Low, 300.0, 800.0, 1.2, 0.08, 0.12);
            layer(&[
                (&bubbles, 0.55, 0.0),
                (&groan, 0.7, 0.08),
                (&slosh, 0.45, 0.4),
                (&slap, 0.6, 0.6),
            ])
        }

        // ----- feel ---------------------------------------------------------------------------
        ShotPunch => {
            // A short kick-drum thump with a snap on top: the weight under a shot.
            let kick = thump(0.12, r.range(150.0, 175.0), 52.0, 0.012, 0.045);
            let snap = click(r, 0.004);
            let body = noise(r, 0.06, Pass::Low, 900.0, 300.0, 0.9, 0.0005, 0.02);
            let mut b = layer(&[(&kick, 1.0, 0.0), (&snap, 0.5, 0.0), (&body, 0.4, 0.0)]);
            drive(&mut b, 2.2);
            b
        }
        ParryImpact => {
            // A heavy boom with a sub drop, and a shimmer rising out of it.
            let boom = thump(0.45, 120.0, 38.0, 0.03, 0.16);
            let crack = noise(r, 0.08, Pass::Band, 2400.0, 900.0, 1.2, 0.0003, 0.02);
            let shimmer = tone(0.5, Wave::Sine, 1400.0, 4200.0, 0.35, 0.02, 0.18);
            let mut b = layer(&[
                (&boom, 1.0, 0.0),
                (&crack, 0.6, 0.0),
                (&shimmer, 0.18, 0.02),
            ]);
            drive(&mut b, 1.8);
            reverb(&mut b, 0.5, 0.4, 0.15, 0.3);
            b
        }
        FullDrawGlint => {
            // Two bright pings a fifth apart (the wick string singing) over a soft sparkle.
            let a = metal(
                0.5,
                1568.0,
                &[(1.0, 1.0, 0.18), (2.01, 0.4, 0.1), (3.0, 0.2, 0.06)],
            );
            let b2 = metal(
                0.5,
                2349.0,
                &[(1.0, 1.0, 0.2), (2.0, 0.35, 0.1), (2.98, 0.15, 0.06)],
            );
            let sparkle = noise(r, 0.2, Pass::High, 7000.0, 9000.0, 0.7, 0.002, 0.06);
            layer(&[(&a, 0.8, 0.0), (&b2, 0.8, 0.045), (&sparkle, 0.3, 0.0)])
        }
        WeakPointTick => {
            let tick = metal(0.12, 3100.0, &[(1.0, 1.0, 0.03), (2.7, 0.5, 0.02)]);
            let snap = click(r, 0.003);
            let thud = thump(0.08, 300.0, 140.0, 0.01, 0.025);
            layer(&[(&tick, 1.0, 0.0), (&snap, 0.7, 0.0), (&thud, 0.5, 0.0)])
        }
        // ----- arsenal-client: melee and knacks (docs/refit.md, docs/audio.md "Melee") -------
        // The Trimmer is shear steel (a snip, a thin ringing whistle), the Tamper a foundry
        // tool (heavy air, packed sand and iron); the knacks are glass, wax, clay and soot.
        TrimmerSlash => {
            // The shears snip as the wrist turns, then the blade whistles through the air with
            // a thin steel ring riding on it.
            let snip = metal(
                0.05,
                r.range(3600.0, 4200.0),
                &[(1.0, 1.0, 0.01), (1.48, 0.6, 0.008), (2.31, 0.4, 0.006)],
            );
            let tick = click(r, 0.001);
            let air = whoosh(r, 0.17, 1400.0, 5200.0, 2.4);
            let ring = metal(
                0.2,
                r.range(2400.0, 2700.0),
                &[(1.0, 0.6, 0.07), (2.7, 0.3, 0.04)],
            );
            let mut b = layer(&[
                (&tick, 0.5, 0.0),
                (&snip, 0.7, 0.0),
                (&air, 1.0, 0.012),
                (&ring, 0.25, 0.03),
            ]);
            highpass(&mut b, 300.0);
            b
        }
        TrimmerHit => {
            // The hook biting into wax: a crisp slice, a split, a short scrape of steel.
            let slice = noise(r, 0.09, Pass::Band, 6000.0, 2200.0, 1.4, 0.0003, 0.03);
            let split = thump(0.12, 380.0, 150.0, 0.006, 0.03);
            let snap = noise(r, 0.06, Pass::Band, 1900.0, 1100.0, 1.2, 0.0005, 0.015);
            let scrape = metal(
                0.18,
                r.range(1700.0, 2000.0),
                &[(1.0, 0.8, 0.05), (2.2, 0.5, 0.03), (3.6, 0.3, 0.02)],
            );
            let mut b = layer(&[
                (&slice, 1.0, 0.0),
                (&split, 0.9, 0.0),
                (&snap, 0.6, 0.004),
                (&scrape, 0.35, 0.01),
            ]);
            drive(&mut b, 1.5);
            crush(&mut b, 11, 1.0);
            b
        }
        TamperSwing => {
            // A heavy head moved fast: a deep rush of air and the haft creaking under it.
            let air = whoosh(r, 0.38, 180.0, 900.0, 1.3);
            let low = noise(r, 0.4, Pass::Low, 260.0, 160.0, 0.8, 0.06, 0.16);
            let mut creak = tone(0.18, Wave::Saw, 150.0, 120.0, 0.18, 0.02, 0.07);
            lowpass(&mut creak, 900.0);
            stutter(&mut creak, 40.0, 0.0);
            layer(&[(&air, 1.0, 0.0), (&low, 0.6, 0.02), (&creak, 0.3, 0.0)])
        }
        TamperHit => {
            // The rammer's foot driving home: a deep packed thud, sand crunching, the iron head
            // ringing once, grit scattering.
            let boom = thump(0.7, 95.0, 34.0, 0.03, 0.18);
            let crunch = noise(r, 0.45, Pass::Low, 3000.0, 220.0, 0.9, 0.0005, 0.09);
            let clang = metal(
                0.6,
                r.range(230.0, 260.0),
                &[
                    (1.0, 1.0, 0.25),
                    (2.62, 0.6, 0.16),
                    (4.3, 0.35, 0.09),
                    (6.7, 0.2, 0.05),
                ],
            );
            let grit = crackle(r, 0.45, 380.0, 0.14);
            let mut b = layer(&[
                (&boom, 1.3, 0.0),
                (&crunch, 0.9, 0.0),
                (&clang, 0.35, 0.002),
                (&grit, 0.35, 0.02),
            ]);
            drive(&mut b, 2.6);
            crush(&mut b, 10, 2.0);
            reverb(&mut b, 0.45, 0.5, 0.18, 0.35);
            b
        }
        Lunge => {
            // Launching off the back foot: a hard rush forward and the blade drawn along steel.
            let rush = whoosh(r, 0.32, 500.0, 2600.0, 1.6);
            let push = thump(0.15, 140.0, 70.0, 0.01, 0.05);
            let draw = metal(
                0.3,
                2100.0,
                &[(1.0, 0.7, 0.12), (1.97, 0.4, 0.08), (3.1, 0.25, 0.05)],
            );
            let mut scrape = noise(r, 0.25, Pass::Band, 4500.0, 7000.0, 2.0, 0.02, 0.08);
            highpass(&mut scrape, 2500.0);
            layer(&[
                (&push, 0.8, 0.0),
                (&rush, 1.0, 0.0),
                (&draw, 0.25, 0.03),
                (&scrape, 0.4, 0.03),
            ])
        }
        Whirl => {
            // A full turn: the air whooshing round twice, a low drone of the spin under it.
            let dur = 0.6;
            let mut n = Rng::new(r.next_u64());
            let mut svf = Svf::default();
            let spin = render(dur, |t, _| {
                // Two passes of the blade: the band sweeps up and the level pulses twice.
                let pass = (t / dur * std::f32::consts::TAU).sin().abs();
                let env = bell(t, dur).powf(0.5) * (0.35 + 0.65 * pass);
                svf.process(n.bipolar(), sweep(t, 400.0, 2200.0, dur), 1.8, SR)
                    .band
                    * env
            });
            let mut drone = tone(dur, Wave::Saw, 110.0, 160.0, dur, 0.05, 0.25);
            lowpass(&mut drone, 500.0);
            let ring = metal(0.4, 1800.0, &[(1.0, 0.5, 0.15), (2.4, 0.3, 0.08)]);
            layer(&[(&spin, 1.0, 0.0), (&drone, 0.35, 0.0), (&ring, 0.2, 0.25)])
        }
        HurlThrow => {
            // Flung: a fwoosh, then the blade / head whirring away end over end.
            let fling = whoosh(r, 0.18, 700.0, 2500.0, 1.5);
            let dur = 0.45;
            let mut n = Rng::new(r.next_u64());
            let mut svf = Svf::default();
            let whirr = render(dur, |t, _| {
                let flutter = 0.5 + 0.5 * (t * 14.0 * std::f32::consts::TAU).sin();
                let env = perc(t, 0.02, 0.18) * flutter;
                svf.process(n.bipolar(), sweep(t, 1600.0, 900.0, dur), 2.5, SR)
                    .band
                    * env
            });
            layer(&[(&fling, 1.0, 0.0), (&whirr, 0.8, 0.06)])
        }
        HurlHit => {
            // The spinning piece biting into something: a heavy chop and a ringing judder.
            let chop = thump(0.2, 260.0, 90.0, 0.008, 0.05);
            let bite = noise(r, 0.08, Pass::Band, 3500.0, 1500.0, 1.2, 0.0003, 0.025);
            let mut judder = metal(
                0.45,
                r.range(620.0, 700.0),
                &[(1.0, 1.0, 0.15), (2.76, 0.5, 0.1), (4.4, 0.3, 0.06)],
            );
            stutter(&mut judder, 24.0, 0.0);
            let mut b = layer(&[(&chop, 1.0, 0.0), (&bite, 0.7, 0.0), (&judder, 0.4, 0.01)]);
            drive(&mut b, 1.6);
            b
        }
        Upswing => {
            // From the floor up: a rising whoosh that ends in a heavy upward heave.
            let rise = whoosh(r, 0.3, 300.0, 3000.0, 1.6);
            let heave = thump(0.25, 70.0, 140.0, 0.08, 0.08);
            let launch = noise(r, 0.3, Pass::Low, 600.0, 1800.0, 0.8, 0.02, 0.1);
            layer(&[(&rise, 1.0, 0.0), (&heave, 0.9, 0.12), (&launch, 0.6, 0.14)])
        }
        KnackThrow => {
            // A knack leaving the hand: a short underhand lob and a soft clink.
            let lob = whoosh(r, 0.2, 500.0, 1500.0, 1.4);
            let clink = metal(0.1, 2600.0, &[(1.0, 0.7, 0.03), (2.4, 0.4, 0.02)]);
            layer(&[(&lob, 1.0, 0.0), (&clink, 0.3, 0.0)])
        }
        SnareSet => {
            // The trap slapped onto the floor: a wax thud, the jaws ratcheting open.
            let thud = thump(0.18, 180.0, 80.0, 0.01, 0.05);
            let wax = noise(r, 0.12, Pass::Low, 1500.0, 500.0, 0.9, 0.001, 0.04);
            let ratchet = metal(0.04, 2300.0, &[(1.0, 1.0, 0.008), (2.6, 0.5, 0.005)]);
            layer(&[
                (&thud, 1.0, 0.0),
                (&wax, 0.6, 0.0),
                (&ratchet, 0.5, 0.08),
                (&ratchet, 0.5, 0.12),
                (&ratchet, 0.6, 0.16),
            ])
        }
        SnareArmed => {
            // The wick lamp catching: two small bright ticks and a puff.
            let tick = metal(0.06, 3100.0, &[(1.0, 1.0, 0.012), (2.2, 0.4, 0.008)]);
            let puff = noise(r, 0.15, Pass::Band, 900.0, 1500.0, 1.0, 0.01, 0.05);
            layer(&[(&tick, 0.8, 0.0), (&tick, 0.6, 0.07), (&puff, 0.5, 0.05)])
        }
        SnareSpring => {
            // Sprung: the jaws snapping shut (a hard steel bite), a chain jerking taut.
            let snap = click(r, 0.0015);
            let bite = metal(
                0.3,
                r.range(950.0, 1100.0),
                &[(1.0, 1.0, 0.09), (2.31, 0.7, 0.06), (3.9, 0.4, 0.04)],
            );
            let thud = thump(0.2, 200.0, 70.0, 0.01, 0.06);
            let chain = envelope(crackle(r, 0.35, 700.0, 0.12), 0.002, 0.1);
            let mut b = layer(&[
                (&snap, 0.9, 0.0),
                (&bite, 0.8, 0.0),
                (&thud, 0.8, 0.0),
                (&chain, 0.6, 0.03),
            ]);
            drive(&mut b, 1.8);
            b
        }
        SootBurst => {
            // The pot breaking: a dull clay pop, then soot billowing out (a long soft rush that
            // stays low and muffled).
            let pop = thump(0.15, 240.0, 90.0, 0.006, 0.04);
            let clay = noise(r, 0.08, Pass::Band, 1400.0, 700.0, 1.1, 0.0005, 0.02);
            let shards = crackle(r, 0.2, 260.0, 0.06);
            let billow = noise(r, 1.4, Pass::Low, 900.0, 220.0, 0.7, 0.06, 0.5);
            let rush = whoosh(r, 0.9, 300.0, 700.0, 0.9);
            let mut b = layer(&[
                (&pop, 1.0, 0.0),
                (&clay, 0.7, 0.0),
                (&shards, 0.5, 0.005),
                (&billow, 0.9, 0.03),
                (&rush, 0.6, 0.05),
            ]);
            lowpass(&mut b, 3500.0);
            b
        }
        TallowSkin => {
            // Coated in hot tallow: a soft sizzling slide and a muffled, waxy settle.
            let sizzle = noise(r, 0.5, Pass::High, 3500.0, 1800.0, 0.8, 0.02, 0.2);
            let slide = noise(r, 0.4, Pass::Band, 1200.0, 400.0, 1.4, 0.04, 0.15);
            let settle = thump(0.3, 110.0, 70.0, 0.04, 0.1);
            let shimmer = fm(0.5, 520.0, 440.0, 1.5, 2.0, 0.3, 0.02, 0.2);
            layer(&[
                (&sizzle, 0.6, 0.0),
                (&slide, 0.8, 0.0),
                (&settle, 0.7, 0.08),
                (&shimmer, 0.2, 0.05),
            ])
        }
        Surge => {
            // A burst of speed: the wick flaring (a fwoom) and a hard rush of air.
            let fwoom = noise(r, 0.3, Pass::Low, 300.0, 1800.0, 1.0, 0.005, 0.09);
            let rush = whoosh(r, 0.3, 900.0, 4000.0, 1.3);
            let kick = thump(0.12, 160.0, 60.0, 0.008, 0.04);
            layer(&[(&kick, 0.8, 0.0), (&fwoom, 1.0, 0.0), (&rush, 0.8, 0.02)])
        }
        // ----- client-a: the rework (docs/rework.md, docs/audio.md "The rework") -------------
        // The Lamplighter is glass and brass lit by a wick (a lens singing, a searing beam);
        // the Wickline a burning cord, the Tally chalk, the Waxwork hot wax, the arrows' tricks
        // air and light, the Riposte steel.
        LamplighterFocus => {
            // The focus ring ratcheting round, faster as the lens comes true; the wick hissing
            // up under a glassy hum that climbs 320 Hz → 1.3 kHz over 0.9 s, then holds with
            // a trembling shimmer. 1.4 s; stop it on release.
            let dur = 1.4;
            let mut ticks = Vec::new();
            let (mut at, mut k) = (0.02f32, 0);
            while at < 0.95 {
                let tick = metal(
                    0.03,
                    2600.0 + k as f32 * 60.0,
                    &[(1.0, 1.0, 0.006), (2.3, 0.5, 0.004)],
                );
                mix_at(&mut ticks, &tick, 0.5 + 0.4 * at, at);
                at += lerp(0.12, 0.05, at / 0.95);
                k += 1;
            }
            let mut hum = Osc::new(Wave::Triangle);
            let mut over = Osc::new(Wave::Sine);
            let mut trem = Osc::new(Wave::Sine);
            let glass = render(dur, |t, _| {
                let f = sweep(t.min(0.9), 320.0, 1300.0, 0.9);
                let x = hum.next(f) * 0.6 + over.next(f * 3.01) * 0.2;
                let env = smoothstep(0.0, 0.25, t) * (1.0 - smoothstep(1.25, dur, t));
                let rate = lerp(5.0, 14.0, (t / 0.9).min(1.0));
                x * env * (0.85 + 0.15 * trem.next(rate))
            });
            let hiss = noise(r, dur, Pass::Band, 1800.0, 4200.0, 1.1, 0.4, 2.0);
            layer(&[(&glass, 1.0, 0.0), (&ticks, 0.5, 0.0), (&hiss, 0.25, 0.0)])
        }
        LamplighterFull => {
            // The lens comes true: one clear glassy note (C7 with a soft twelfth above it) and
            // the wick's flame flaring behind it.
            let note = metal(
                0.7,
                2093.0,
                &[(1.0, 1.0, 0.25), (3.0, 0.3, 0.12), (4.17, 0.12, 0.06)],
            );
            let flare = noise(r, 0.3, Pass::Band, 400.0, 1400.0, 0.9, 0.02, 0.1);
            let sparkle = noise(r, 0.25, Pass::High, 7500.0, 9500.0, 0.7, 0.002, 0.07);
            let mut b = layer(&[(&note, 0.8, 0.0), (&flare, 0.5, 0.0), (&sparkle, 0.25, 0.0)]);
            echo(&mut b, 0.08, 0.25, 0.2, 5000.0, 0.2);
            b
        }
        LamplighterFire => {
            // The focused beam: a hard crack, the wick's light searing down the line (a bright
            // hiss falling 7 → 1.8 kHz), a deep boom under it and the brass tube ringing on
            // into a long tail.
            let crack = quill_crack(r, 0.6);
            let sear = noise(r, 0.35, Pass::Band, 7000.0, 1800.0, 1.4, 0.0005, 0.09);
            let boom = thump(0.5, 150.0, 38.0, 0.02, 0.12);
            let ring = metal(
                0.9,
                880.0,
                &[(1.0, 1.0, 0.3), (2.37, 0.5, 0.18), (4.1, 0.25, 0.1)],
            );
            let mut b = layer(&[
                (&crack, 1.0, 0.0),
                (&boom, 0.9, 0.0),
                (&sear, 0.5, 0.004),
                (&ring, 0.18, 0.01),
            ]);
            drive(&mut b, 1.6);
            reverb(&mut b, 0.6, 0.45, 0.2, 0.25);
            b
        }
        LamplighterTell => {
            // A lens catching the light somewhere out there: a faint, high glassy shimmer
            // that swells and fades (0.6 s; re-trigger at the sniper while they focus).
            let dur = 0.6;
            let (mut a, mut b2, mut wob) = (
                Osc::new(Wave::Sine),
                Osc::new(Wave::Sine),
                Osc::new(Wave::Sine),
            );
            let shimmer = render(dur, |t, _| {
                let w = 1.0 + 0.004 * wob.next(7.0);
                (a.next(3520.0 * w) * 0.6 + b2.next(5274.0 * w) * 0.3) * bell(t, dur)
            });
            let air = noise(r, dur, Pass::High, 6000.0, 8000.0, 0.7, 0.2, 0.3);
            layer(&[(&shimmer, 1.0, 0.0), (&air, 0.3, 0.0)])
        }
        WicklineFire => {
            // The burning cord whipped out: a whip-crack, a fast falling zip and the wick
            // crackling along its length.
            let crack = noise(r, 0.05, Pass::Band, 6000.0, 2500.0, 1.2, 0.0002, 0.01);
            let mut zip = tone(0.22, Wave::Saw, 2400.0, 500.0, 0.18, 0.001, 0.07);
            lowpass(&mut zip, 3200.0);
            let burn = envelope(crackle(r, 0.35, 900.0, 0.2), 0.005, 0.12);
            let rush = whoosh(r, 0.25, 3000.0, 900.0, 1.3);
            layer(&[
                (&crack, 0.9, 0.0),
                (&zip, 0.35, 0.0),
                (&rush, 0.6, 0.005),
                (&burn, 0.5, 0.02),
            ])
        }
        WicklineZip => {
            // Yanked along the line: the cord snapping taut (a low twang and a tug), then a
            // hard rush of air.
            let mut twang = pluck(r, 0.35, 130.0, 0.993);
            for (i, s) in twang.iter_mut().enumerate() {
                *s *= perc(i as f32 / SR, 0.001, 0.1);
            }
            lowpass(&mut twang, 1600.0);
            let snap = click(r, 0.002);
            let tug = thump(0.15, 120.0, 70.0, 0.01, 0.04);
            let rush = whoosh(r, 0.4, 600.0, 3500.0, 1.2);
            layer(&[
                (&snap, 0.6, 0.0),
                (&tug, 0.8, 0.0),
                (&twang, 0.8, 0.0),
                (&rush, 0.9, 0.03),
            ])
        }
        TallyMark => {
            // A target tallied: a dry chalk stroke and a small bright tick.
            let stroke = noise(r, 0.06, Pass::Band, 3200.0, 2000.0, 1.5, 0.002, 0.018);
            let tick = metal(0.08, 2900.0, &[(1.0, 1.0, 0.02), (2.5, 0.4, 0.012)]);
            layer(&[(&stroke, 0.8, 0.0), (&tick, 0.7, 0.01)])
        }
        TallyShot => {
            // One of the tally's shots: a short, dry crack with a little weight under it.
            let pitch = r.range(0.8, 0.9);
            let crack = quill_crack(r, pitch);
            let body = thump(0.12, 260.0, 90.0, 0.006, 0.03);
            layer(&[(&crack, 1.0, 0.0), (&body, 0.6, 0.0)])
        }
        WaxworkCast => {
            // Hot wax poured into your shape: a thick, glugging pour with a sizzle on it, then
            // a hollow knock as the double sets.
            let dur = 0.45;
            let mut n = Rng::new(r.next_u64());
            let mut bp = Svf::default();
            let mut glug = Osc::new(Wave::Sine);
            let pour = render(dur, |t, _| {
                let g = 0.55 + 0.45 * glug.next(lerp(9.0, 14.0, t / dur));
                bp.bandpass(n.bipolar(), lerp(380.0, 650.0, t / dur), 2.2) * g * bell(t, dur)
            });
            let sizzle = noise(r, 0.5, Pass::High, 4000.0, 2500.0, 0.8, 0.03, 0.2);
            let set = thump(0.18, 320.0, 160.0, 0.006, 0.05);
            let knock = metal(0.15, 700.0, &[(1.0, 0.6, 0.05), (2.2, 0.3, 0.03)]);
            layer(&[
                (&pour, 1.0, 0.0),
                (&sizzle, 0.3, 0.05),
                (&set, 0.8, 0.4),
                (&knock, 0.3, 0.4),
            ])
        }
        WaxworkBurst => {
            // The double slumps (a wet sag), then bursts: a soft boom, hot wax splattering
            // and a long sizzle.
            let sag = noise(r, 0.3, Pass::Low, 700.0, 200.0, 1.0, 0.02, 0.12);
            let boom = thump(0.4, 120.0, 45.0, 0.015, 0.1);
            let splat = envelope(crackle(r, 0.4, 1400.0, 0.12), 0.002, 0.15);
            let sizzle = noise(r, 0.7, Pass::High, 3500.0, 2000.0, 0.8, 0.02, 0.3);
            let mut b = layer(&[
                (&sag, 0.7, 0.0),
                (&boom, 1.0, 0.16),
                (&splat, 0.7, 0.17),
                (&sizzle, 0.4, 0.18),
            ]);
            drive(&mut b, 1.4);
            b
        }
        Blink => {
            // Stepping to the arrow: a short sucking rush that swells to its end (a whoosh run
            // backwards), a pop where you arrive and a bright shimmer after it.
            let dur = 0.16;
            let mut n = Rng::new(r.next_u64());
            let mut svf = Svf::default();
            let suck = render(dur, |t, _| {
                let k = t / dur;
                svf.process(n.bipolar(), lerp(800.0, 4500.0, k), 1.4, SR)
                    .band
                    * k
                    * k
            });
            let pop = thump(0.12, 300.0, 120.0, 0.004, 0.03);
            let shimmer = fm(0.3, 1800.0, 2600.0, 2.0, 3.0, 0.3, 0.002, 0.1);
            layer(&[(&suck, 1.0, 0.0), (&pop, 0.9, dur), (&shimmer, 0.3, dur)])
        }
        BeaconLight => {
            // The beacon catching: a whoomph and a bright bell note that keeps on ringing.
            let whoomph = noise(r, 0.35, Pass::Low, 300.0, 1600.0, 0.9, 0.01, 0.1);
            let note = metal(
                1.0,
                1318.0,
                &[(1.0, 1.0, 0.35), (2.76, 0.4, 0.2), (5.4, 0.2, 0.1)],
            );
            let mut b = layer(&[(&whoomph, 0.8, 0.0), (&note, 0.7, 0.03)]);
            echo(&mut b, 0.12, 0.3, 0.25, 4000.0, 0.3);
            b
        }
        BeaconHum => {
            // A lit beacon: a low, slowly pulsing hum with a faint high ring on it (1.2 s,
            // soft both ends: re-trigger while it burns).
            let dur = 1.2;
            let (mut a, mut b2, mut lfo) = (
                Osc::new(Wave::Triangle),
                Osc::new(Wave::Sine),
                Osc::new(Wave::Sine),
            );
            render(dur, |t, _| {
                let pulse = 0.7 + 0.3 * lfo.next(2.5);
                (a.next(220.0) * 0.5 + b2.next(1318.0) * 0.15) * pulse * bell(t, dur)
            })
        }
        HurlReturn => {
            // The piece coming round: its whirr dipping as it turns, then rising again as it
            // heads back.
            let dur = 0.5;
            let mut n = Rng::new(r.next_u64());
            let mut svf = Svf::default();
            render(dur, |t, _| {
                let flutter = 0.5 + 0.5 * (t * 16.0 * std::f32::consts::TAU).sin();
                let turn = (2.0 * t / dur - 1.0).abs();
                let f = lerp(800.0, 1400.0, turn);
                svf.process(n.bipolar(), f, 2.5, SR).band * flutter * bell(t, dur)
            })
        }
        HurlCatch => {
            // Caught: the glove slapping round the haft and a short ring of the steel.
            let slap = noise(r, 0.06, Pass::Band, 1800.0, 900.0, 1.0, 0.0005, 0.015);
            let thud = thump(0.1, 220.0, 110.0, 0.004, 0.025);
            let ring = metal(
                0.3,
                r.range(1500.0, 1700.0),
                &[(1.0, 0.7, 0.1), (2.7, 0.3, 0.05)],
            );
            layer(&[(&slap, 1.0, 0.0), (&thud, 0.8, 0.0), (&ring, 0.4, 0.005)])
        }
        RiposteGuard => {
            // The guard raised: steel scraping along steel, a set and a held ring.
            let scrape = noise(r, 0.25, Pass::Band, 2500.0, 5000.0, 3.0, 0.01, 0.08);
            let set = thump(0.12, 200.0, 120.0, 0.005, 0.03);
            let ring = metal(0.4, 1250.0, &[(1.0, 0.6, 0.15), (2.4, 0.3, 0.08)]);
            layer(&[(&scrape, 0.8, 0.0), (&set, 0.6, 0.08), (&ring, 0.4, 0.08)])
        }
        RiposteBlock => {
            // A blow caught on the guard: a hard bright clang and a burst of sparks.
            let clang = metal(
                0.5,
                r.range(780.0, 860.0),
                &[
                    (1.0, 1.0, 0.16),
                    (2.31, 0.7, 0.1),
                    (3.9, 0.45, 0.06),
                    (5.6, 0.25, 0.04),
                ],
            );
            let hit = thump(0.15, 260.0, 100.0, 0.004, 0.04);
            let sparks = crackle(r, 0.25, 1600.0, 0.08);
            let mut b = layer(&[(&clang, 0.9, 0.0), (&hit, 0.8, 0.0), (&sparks, 0.5, 0.004)]);
            drive(&mut b, 1.7);
            b
        }
        RiposteCounter => {
            // The counter-swing: a heavy whoosh let go all at once and a ringing edge.
            let swing = whoosh(r, 0.3, 500.0, 3500.0, 1.6);
            let heave = thump(0.2, 90.0, 150.0, 0.06, 0.06);
            let ring = metal(0.5, 1900.0, &[(1.0, 0.7, 0.2), (2.7, 0.35, 0.1)]);
            let mut b = layer(&[(&swing, 1.0, 0.0), (&heave, 0.8, 0.05), (&ring, 0.3, 0.08)]);
            drive(&mut b, 1.3);
            b
        }
        // ---- client-b: the rework (docs/rework.md) ----
        KindleDrink => {
            // The stopper popped, two quick gulps, then the kindling catching in the wick: a
            // warm whoomph rising into a bright chime (the haste is on).
            let pop = tone(0.05, Wave::Sine, 900.0, 420.0, 0.03, 0.0005, 0.012);
            let gulp = |r: &mut Rng| {
                let mut g = tone(0.12, Wave::Sine, 190.0, 330.0, 0.09, 0.006, 0.04);
                let wet = noise(r, 0.1, Pass::Band, 700.0, 1100.0, 3.0, 0.004, 0.03);
                mix_at(&mut g, &wet, 0.4, 0.0);
                lowpass(&mut g, 1800.0);
                g
            };
            let (g1, g2) = (gulp(r), gulp(r));
            let catch = noise(r, 0.6, Pass::Low, 240.0, 1900.0, 1.0, 0.03, 0.18);
            let embers = envelope(crackle(r, 0.6, 140.0, 0.3), 0.02, 0.25);
            let chime = notes(
                &[(74.0, 0.0, 0.12), (81.0, 0.07, 0.2)],
                Wave::Triangle,
                0.004,
                0.22,
                5200.0,
                0.0,
            );
            let mut b = layer(&[
                (&pop, 0.7, 0.0),
                (&g1, 0.8, 0.07),
                (&g2, 0.75, 0.2),
                (&catch, 1.0, 0.32),
                (&embers, 0.5, 0.34),
                (&chime, 0.3, 0.4),
            ]);
            echo(&mut b, 0.1, 0.25, 0.2, 4000.0, 0.25);
            b
        }
        KindleTick => {
            // A heal tick: a soft warm chime (a fifth) on a breath of flame, quiet enough to
            // repeat every second without nagging.
            let chime = notes(
                &[(79.0, 0.0, 0.08), (86.0, 0.04, 0.14)],
                Wave::Triangle,
                0.003,
                0.2,
                4200.0,
                0.0,
            );
            let breath = noise(r, 0.3, Pass::Band, 500.0, 1100.0, 0.8, 0.03, 0.1);
            layer(&[(&chime, 0.7, 0.0), (&breath, 0.5, 0.0)])
        }
        HasteFade => {
            // The haste running out: the wick settling back, a falling soft gutter.
            let settle = gutter(r, 0.5, 1400.0, 500.0, 14.0, 0.2);
            let sigh = noise(r, 0.45, Pass::Low, 1200.0, 300.0, 0.8, 0.02, 0.18);
            layer(&[(&settle, 0.8, 0.0), (&sigh, 0.6, 0.0)])
        }
        ThornBat => {
            // A blow or a blast batting the thorn away: a glass crack, a woody knock and the
            // spike whipping off towards its target.
            let crack = noise(r, 0.05, Pass::High, 5000.0, 3500.0, 0.8, 0.0002, 0.012);
            let knock = thump(0.12, 520.0, 260.0, 0.004, 0.03);
            let glass = glass_spray(r, 8, 0.06, 2800.0, 6500.0);
            let whip = whoosh(r, 0.22, 1200.0, 4800.0, 2.2);
            let ring = metal(
                0.4,
                r.range(1750.0, 1950.0),
                &[(1.0, 1.0, 0.12), (2.01, 0.5, 0.08), (2.99, 0.3, 0.05)],
            );
            let mut b = layer(&[
                (&crack, 0.9, 0.0),
                (&knock, 0.7, 0.0),
                (&glass, 0.6, 0.003),
                (&whip, 0.8, 0.02),
                (&ring, 0.35, 0.01),
            ]);
            drive(&mut b, 1.4);
            b
        }
        ThornTurn => {
            // A projectile turned at the thorn: the spike shattering in a glassy ring and the
            // shot's rush bending away (a whoosh whose band sweeps up then down).
            let shatter = glass_spray(r, 10, 0.08, 2500.0, 6000.0);
            let ring = metal(
                0.6,
                r.range(1500.0, 1700.0),
                &[(1.0, 1.0, 0.2), (1.5, 0.6, 0.14), (2.76, 0.35, 0.09)],
            );
            let dur = 0.35;
            let mut n = Rng::new(r.next_u64());
            let mut svf = Svf::default();
            let bend = render(dur, |t, _| {
                let k = t / dur;
                let f = 900.0 + 2600.0 * (std::f32::consts::PI * k).sin();
                svf.process(n.bipolar(), f, 2.0, SR).band * bell(t, dur)
            });
            let mut b = layer(&[(&shatter, 0.7, 0.0), (&ring, 0.5, 0.0), (&bend, 0.9, 0.01)]);
            echo(&mut b, 0.09, 0.2, 0.2, 5000.0, 0.2);
            b
        }
        BolasThrow => {
            // Flung: the two weights whirring round their cord (a flutter at 11 Hz) away.
            let fling = whoosh(r, 0.16, 500.0, 1600.0, 1.4);
            let dur = 0.55;
            let mut n = Rng::new(r.next_u64());
            let mut svf = Svf::default();
            let whirr = render(dur, |t, _| {
                let flutter = 0.35 + 0.65 * (t * 11.0 * std::f32::consts::TAU).sin().abs();
                svf.process(n.bipolar(), sweep(t, 700.0, 420.0, dur), 2.2, SR)
                    .band
                    * flutter
                    * perc(t, 0.03, 0.25)
            });
            layer(&[(&fling, 1.0, 0.0), (&whirr, 0.9, 0.04)])
        }
        BolasBind => {
            // Wrapped: the cord whipping round twice, then the wax weights thudding together.
            let whip = whoosh(r, 0.12, 1500.0, 4500.0, 2.5);
            let whip2 = whoosh(r, 0.1, 1800.0, 5000.0, 2.5);
            let thud = thump(0.2, 170.0, 70.0, 0.01, 0.06);
            let knock = thump(0.15, 260.0, 120.0, 0.006, 0.04);
            let creak = envelope(crackle(r, 0.3, 500.0, 0.15), 0.002, 0.12);
            let mut b = layer(&[
                (&whip, 0.8, 0.0),
                (&whip2, 0.7, 0.07),
                (&thud, 1.0, 0.13),
                (&knock, 0.6, 0.16),
                (&creak, 0.5, 0.14),
            ]);
            drive(&mut b, 1.5);
            b
        }
        BolasLink => {
            // The cord snapping taut between two bound targets: a low plucked twang and a hum.
            let twang = pluck(r, 0.6, 98.0, 0.996);
            let snap = click(r, 0.002);
            let mut hum = tone(0.6, Wave::Saw, 98.0, 96.0, 0.6, 0.02, 0.3);
            lowpass(&mut hum, 600.0);
            let mut b = layer(&[(&snap, 0.6, 0.0), (&twang, 1.0, 0.0), (&hum, 0.3, 0.0)]);
            lowpass(&mut b, 3500.0);
            b
        }
        BolasShare => {
            // Damage carried along the cord: a short, low thrum.
            let thrum = pluck(r, 0.25, 130.0, 0.99);
            let tick = noise(r, 0.04, Pass::Band, 2000.0, 1500.0, 1.5, 0.0005, 0.01);
            let mut b = layer(&[(&thrum, 1.0, 0.0), (&tick, 0.4, 0.0)]);
            lowpass(&mut b, 2500.0);
            b
        }
        BulwarkRaise => {
            // A slab of wax heaved up in front of you: a deep scrape-thud, then a hiss as it
            // hardens and a few cooling ticks.
            let heave = thump(0.3, 95.0, 55.0, 0.04, 0.09);
            let scrape = noise(r, 0.25, Pass::Band, 400.0, 900.0, 1.2, 0.02, 0.08);
            let hiss = noise(r, 0.6, Pass::High, 4500.0, 2500.0, 0.8, 0.05, 0.22);
            let ticks = envelope(crackle(r, 0.6, 90.0, 0.4), 0.02, 0.3);
            let mut b = layer(&[
                (&scrape, 0.7, 0.0),
                (&heave, 1.0, 0.04),
                (&hiss, 0.45, 0.1),
                (&ticks, 0.4, 0.2),
            ]);
            drive(&mut b, 1.3);
            b
        }
        BulwarkAbsorb => {
            // A blow taken by the wax: a dull packed thunk, a crackle as it gives a little.
            let thunk = thump(0.16, 240.0, 110.0, 0.006, 0.04);
            let body = noise(r, 0.1, Pass::Low, 1600.0, 500.0, 0.9, 0.0005, 0.03);
            let crack = envelope(crackle(r, 0.2, 400.0, 0.08), 0.001, 0.06);
            let mut b = layer(&[(&thunk, 1.0, 0.0), (&body, 0.7, 0.0), (&crack, 0.5, 0.01)]);
            drive(&mut b, 1.6);
            crush(&mut b, 11, 1.0);
            b
        }
        BulwarkCrack => {
            // The slab splitting further: a sharp dry crack running away, a groan of strain.
            let crack = noise(r, 0.08, Pass::High, 3500.0, 1800.0, 0.9, 0.0002, 0.02);
            let run = envelope(crackle(r, 0.25, 900.0, 0.08), 0.001, 0.1);
            let mut groan = tone(0.3, Wave::Saw, 140.0, 90.0, 0.3, 0.01, 0.1);
            lowpass(&mut groan, 700.0);
            let mut b = layer(&[(&crack, 1.0, 0.0), (&run, 0.7, 0.01), (&groan, 0.35, 0.0)]);
            drive(&mut b, 1.8);
            b
        }
        BulwarkShatter => {
            // The slab bursting: a heavy crack, hard wax chunks raining down, a boom under it.
            let boom = thump(0.5, 120.0, 40.0, 0.02, 0.14);
            let crack = noise(r, 0.1, Pass::High, 3000.0, 1500.0, 0.8, 0.0002, 0.03);
            let chunks = crackle(r, 0.9, 260.0, 0.35);
            let mut thuds = envelope(crackle(r, 0.8, 60.0, 0.4), 0.005, 0.3);
            lowpass(&mut thuds, 900.0);
            let mut b = layer(&[
                (&boom, 1.0, 0.0),
                (&crack, 1.0, 0.0),
                (&chunks, 0.7, 0.02),
                (&thuds, 1.2, 0.04),
            ]);
            drive(&mut b, 2.0);
            reverb(&mut b, 0.35, 0.5, 0.15, 0.3);
            b
        }
        BulwarkDrop => {
            // Lowered: the wax slumping, a soft thud.
            let slump = noise(r, 0.3, Pass::Low, 900.0, 200.0, 0.9, 0.02, 0.1);
            let thud = thump(0.2, 130.0, 60.0, 0.01, 0.06);
            layer(&[(&slump, 0.8, 0.0), (&thud, 0.7, 0.05)])
        }
        WispFling => {
            // Flung off its orbit: the wisp's crackle whining up and a fwoosh away.
            let whine = fm(0.4, 500.0, 1800.0, 2.0, 3.0, 0.5, 0.01, 0.18);
            let buzz = zap(r, 0.3, 900.0, 1600.0, 0.3, 0.3, 0.12);
            let fwoosh = whoosh(r, 0.3, 700.0, 3000.0, 1.4);
            layer(&[(&fwoosh, 1.0, 0.0), (&whine, 0.45, 0.0), (&buzz, 0.4, 0.02)])
        }
        StokeIgnite => {
            // Stoked: a bellows' breath roaring into the weapon's fire, a rising crackle.
            let breath = noise(r, 0.25, Pass::Band, 300.0, 900.0, 1.0, 0.04, 0.1);
            let roar = noise(r, 0.8, Pass::Low, 200.0, 2400.0, 1.1, 0.06, 0.3);
            let embers = envelope(crackle(r, 0.8, 260.0, 0.5), 0.05, 0.35);
            let mut rumble = tone(0.8, Wave::Saw, 55.0, 75.0, 0.8, 0.05, 0.35);
            lowpass(&mut rumble, 300.0);
            let mut b = layer(&[
                (&breath, 0.7, 0.0),
                (&roar, 1.0, 0.12),
                (&embers, 0.6, 0.15),
                (&rumble, 0.5, 0.12),
            ]);
            drive(&mut b, 1.4);
            b
        }
        StokeVent => {
            // Spent: steam venting hard from the overheated weapon, then metal ticking as it
            // cools.
            let burst = noise(r, 0.12, Pass::High, 6000.0, 3500.0, 0.7, 0.002, 0.05);
            let steam = noise(r, 1.2, Pass::High, 4500.0, 2200.0, 0.9, 0.01, 0.55);
            let mut b = layer(&[(&burst, 0.8, 0.0), (&steam, 1.0, 0.02)]);
            let tick = metal(0.05, 2400.0, &[(1.0, 1.0, 0.01), (2.3, 0.4, 0.006)]);
            for (k, at) in [0.55f32, 0.8, 1.0, 1.25].into_iter().enumerate() {
                mix_at(&mut b, &tick, 0.35 - 0.05 * k as f32, at);
            }
            b
        }
        LanternSet => {
            // Set down: a brass clank on the floor, the glass rattling, the flame catching.
            let clank = metal(
                0.35,
                r.range(820.0, 900.0),
                &[(1.0, 1.0, 0.08), (2.4, 0.5, 0.05), (3.9, 0.3, 0.03)],
            );
            let thud = thump(0.12, 200.0, 90.0, 0.006, 0.035);
            let rattle = glass_spray(r, 6, 0.1, 3000.0, 5500.0);
            let catch = noise(r, 0.4, Pass::Low, 300.0, 1600.0, 1.0, 0.03, 0.12);
            layer(&[
                (&thud, 0.8, 0.0),
                (&clank, 0.5, 0.0),
                (&rattle, 0.4, 0.01),
                (&catch, 0.7, 0.12),
            ])
        }
        LanternOut => {
            // Burnt out: the flame guttering away and a glass pane ticking as it cools.
            let flame = gutter(r, 0.7, 1200.0, 400.0, 16.0, 0.3);
            let tick = metal(0.08, 3600.0, &[(1.0, 1.0, 0.015), (2.2, 0.4, 0.01)]);
            layer(&[(&flame, 1.0, 0.0), (&tick, 0.4, 0.45)])
        }
        KiteLaunch => {
            // Launched: oiled paper snapping taut and the ember heart catching with a fwoom.
            let snap = noise(r, 0.05, Pass::Band, 2500.0, 1800.0, 1.6, 0.0003, 0.012);
            let snap2 = noise(r, 0.05, Pass::Band, 2200.0, 1600.0, 1.6, 0.0003, 0.012);
            let fwoom = noise(r, 0.4, Pass::Low, 260.0, 1800.0, 1.0, 0.01, 0.12);
            let rush = whoosh(r, 0.3, 600.0, 2400.0, 1.3);
            layer(&[
                (&snap, 0.8, 0.0),
                (&snap2, 0.6, 0.05),
                (&fwoom, 1.0, 0.03),
                (&rush, 0.7, 0.05),
            ])
        }
        KiteFlutter => {
            // The kite in flight: paper fluttering (an amplitude flutter at ~22 Hz) over a soft
            // ember crackle; soft at both ends so it can be re-triggered.
            let dur = 0.7;
            let mut n = Rng::new(r.next_u64());
            let mut svf = Svf::default();
            let flutter = render(dur, |t, _| {
                let f = 0.4 + 0.6 * (t * 22.0 * std::f32::consts::TAU).sin().abs();
                svf.process(n.bipolar(), 1300.0, 1.5, SR).band * f * bell(t, dur)
            });
            let embers = envelope(crackle(r, dur, 120.0, 1.0), 0.1, 0.4);
            layer(&[(&flutter, 1.0, 0.0), (&embers, 0.4, 0.0)])
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
                assert!(
                    measured <= sfx.info().level_db + 0.5,
                    "{sfx:?} louder than its target"
                );
            }
        }
    }

    #[test]
    fn generation_is_deterministic() {
        for &sfx in &[Sfx::Explosion, Sfx::Parry, Sfx::EnemyHurt, Sfx::Footstep] {
            assert_eq!(render_sfx(sfx), render_sfx(sfx));
        }
        assert_ne!(
            render_variant(Sfx::Footstep, 0),
            render_variant(Sfx::Footstep, 1)
        );
    }

    #[test]
    fn serde_roundtrip() {
        let s = ron::to_string(&Sfx::GaolerBashWindup).expect("serialize");
        let back: Sfx = ron::from_str(&s).expect("deserialize");
        assert_eq!(back, Sfx::GaolerBashWindup);
    }
}
