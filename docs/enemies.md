# Enemies, navigation and arena encounters

The Moulds (`docs/roster.md`, `docs/expansion.md`): nine enemy types — Gnawer, Censer,
Harrier, Gaoler, Heron, and the third roster's Bellwether, Snuffer, Smelter and the Effigy
training dummy — their server-side AI (frenzies, latches, slag lines, elites, difficulty
scaling), the nav grid ground enemies walk on, arena encounters (locked rooms, waves) and
co-op scaling. Rules and default numbers come from `docs/gameplay.md` and
`docs/roster.md` (the design contract); the damage pipeline, parry hook, projectiles and
replication basics are in `docs/combat.md`.

## Where things live

| module | contents |
|---|---|
| `assets/enemies/{gnawer,censer,harrier,gaoler,heron,bellwether,snuffer,smelter,effigy}.ron` | the definitions (also compiled in as built-ins, `EnemyRegistry::builtin`) |
| `shared::sim::enemy` (`mod.rs`) | `Enemy` (runtime state, incl. `aim` / `aim_locked`, elite, frenzy, latch, pour line), damage side of the pipeline (weak points, armour, interrupts, knockback + stagger, damage alerts, dummies), `spawn_enemy[_scaled]`, `spawn_enemy_with` / `SpawnOptions`, `make_elite`, `fury`, replication |
| `shared::sim::enemy::def` | `EnemyDef` format: `AttackKind::{Melee, Projectile, Slam, Dive, Toll, Pour}`, lunges / pounces, lobs, `AimLaser`, `jumpable`, `parry_strike`, `parry_reach`, `ai.reposition`, `Toll` / `Frenzy`, `Pour`, `Latch`, `Interrupt`, `Dummy`, `EliteDef` |
| `shared::sim::enemy::special` | the toll (frenzy), the latch (cling, smother, slow, punched / dashed off), pours laying their line, interrupts, training dummies |
| `shared::sim::enemy::hazard` | slag lines (`SlagLine`, `SimWorld::enemy_hazards`, `trace_floor`, burning, `kind::SLAG_LINE`) |
| `shared::sim::enemy::ai` | the state machine, targeting (`is_targetable` hook), attacks (leap, dive, lob, laser), getting there (nav leaps and pad rides, prowling, vantage points, stranded bookkeeping, melting and re-forming, `melt_enemies`), `move_enemy` (capsule collide-and-slide), push-apart, kill volumes, `frozen` handling, `SimWorld::build_nav`, `lob_velocity` |
| `shared::sim::enemy::steer` | ground path following (`NavAgent`: routes with jumps, `Reach`), edge guard, flight steering (with detours), the re-form and vantage searches (`reform_site`, `vantage_site`), `AiWorld` (nav grid + A* buffers + stats on `SimWorld::ai`) |
| `shared::sim::nav` | `NavGrid`: build, agent classes (`NavClass` = footprint + headroom + leap limits + pads), jump links (`NavJump`: leaps, pad rides), the leap arc (`leap_point`, `leap_velocity`), `node_at`, `nearest_node`, `find_path` (A*), `search` (Dijkstra, forward or over the reverse links), `walkable_line`, `smooth_route`, `ground_ok` |
| `shared::sim::encounter` | encounter states (`SimWorld::encounters`), triggers, waves, co-op scaling, doors, rally teleport, reset hooks, `kind::ENCOUNTER` replication |
| `server::combat` | routes `EncounterStarted` / `WaveStarted` / `EncounterCleared` (reliable, everyone) and `Fx::EnemySpawned` |
| `server::commands` | dev `/spawn`, `/encounter <n>`, `/encounters`, `/needle`, `/ember`, `/killall`... |
| `client::models::enemies` | the nine rigs and their pure pose functions (`docs/models.md`), the getting-there states (`Crouch`, `Airborne`, `Melt`, `Reform`, `body_transform`), the slag strip, the wax puddle (`puddle.rs`) |
| `client::ingame::combat::enemy_fx` | names / sizes / sounds per kind, the Heron's aim laser, wax-and-ember effects, the third roster's effects (toll, frenzy, elites, latch view, slag lines), leap dust, melting and rising (`melt_burst`, `rise_burst`, `draw_puddle`) |
| `client::ingame::encounter_hud` | banners + sounds for the encounter events, "WAVE 2/3 - 5 LEFT" readout |
| `client/examples/enemy_shots.rs` | headless screenshots of every attack of every type, the nine-Mould lineup, its hit shapes drawn over it, elites, an arena encounter, spectating a teammate, leaps and melting (`--getting-there`) |

## The five Moulds

Position convention: **every enemy's `position` is its feet** (bottom of the collision
capsule). The Harrier's hover centre (its model origin) is half its capsule height (0.65 m)
above it; the client draws it there. Local offsets in the RON files are from the feet (+X
right, +Y up, −Z forward). Melee ranges are between body surfaces; projectile / dive ranges
between centres.

| | Gnawer (20) | Censer (21) | Harrier (22) | Gaoler (23) | Heron (24) |
|---|---|---|---|---|---|
| role | fodder melee, pack crawler | ranged lobber | flying dive-bomber | heavy, shielded | sniper |
| health | 45 | 80 | 40 | 520 | 60 |
| speed / accel | 10 / 60 | 5 / 40 | 15 / 32 | 4.5 / 25 | 7 / 35 |
| capsule r × h | 0.6 × 2.0 | 0.6 × 2.5 | 0.7 × 1.3 (3 m wingspan) | 1.3 × 3.7 | 0.5 × 3.4 |
| drawn (`model_scale`) | ×1.25 (2.5 m) | ×1.2 (3.25 m) | ×1.3 (3.9 m wingspan) | ×1.12 (4.2 m) | ×1.2 (4 m) |
| nav class | narrow, 2 m | narrow, 2.6 m headroom | — (flies) | wide, 3.8 m headroom | narrow, 3.5 m headroom |
| weak points | head (jaw) ×2 | lantern (brazier head) ×2 | heart (under the chest) ×2.5 | head (grille) ×1.5, lantern (back) ×2.5 | head (lens) ×2.5 |
| armour | — | — | — | front ×0.15 in a 120° arc, explosions too; head + lantern bypass; parry counters bypass | — |
| knockback scale | 1.25 | 1.0 | 1.5 | 0.12 | 0.9 |
| keeps | melee reach | 14–30 m, side-steps | circles 8–16 m out, 5–10 m up | melee reach | 25–45 m, repositions after every shot |

| attack | wind-up (telegraph) | strike | recover / cooldown | effect |
|---|---|---|---|---|
| Gnawer **bite** | 0.4 s: the jaw unhinges, throat and gums glow orange, rising chitter | 0.12 s | 0.4 / 0.5 s | 14 dmg, 2.6 m reach, 100° arc, kb 7 |
| Gnawer **pounce** (target faster than 4 m/s, 3–9 m away) | 0.55 s crouched, coil growl | 0.7 s: a leap 13 m/s forward + 6 m/s up, led on the target's velocity, then a run; rakes the first players it reaches (1.4 m) once | 0.7 / 3 s | 18 dmg, kb 9; never leaps where the arc would land in lava / off a ledge |
| Censer **swing** (players inside ~3 m) | 0.5 s: the censer drawn back, chain rattle | 0.15 s | 0.5 / 1.2 s | 12 dmg, 160° arc, kb 12 (flat): shoves rushers away |
| Censer **volley** | 0.8 s: the censer whirled overhead, embers brighten, chain rattles | three ember bombs 0.12 s apart | 0.5 / 3 s | `EmberBomb` lobbed on the least-speed arc onto the predicted feet (≤ 26 m/s, gravity 20, lead 0.7), fanned 0 / −8° / +8°; bursts on impact: r 2.5, 18 dmg with falloff, kb 9; parryable |
| Harrier **dive** | 0.7 s: rears up, wings fold back, beak and heart glow, shriek | up to 0.6 s: a straight 3D line at 26 m/s at the target's chest + 0.15 s lead; rakes every player it passes (0.6 m) once; any contact with the level ends it | 0.8 s climbing back / 2.2 s | 20 dmg, kb 10; **the whole dive is parryable** (the Swat) |
| Gaoler **bash** | 1.0 s: the door drawn back, rim glows yellow; **last 0.35 s PARRYABLE** (white-hot strobe) | 0.2 s | 0.9 / 1.2 s | 40 dmg, reach 2.4 m (~4.1 m from its centre), 110° arc, kb 18 + up; parried within 4.5 m of its centre → stunned 2.5 s + 150 damage through the armour |
| Gaoler **sweep** (players within ~7 m) | 0.8 s: crouches, the chain glows red and is spun low | 0.25 s | 0.8 / 3.5 s | 25 dmg, 300° arc, kb 14, **only hits players whose feet are < 1.2 m above the Gaoler's feet** (jump it); not parryable |
| Gaoler **rush** (target 8–20 m away) | 0.9 s: hunkers behind the door, snort | 0.8 s at 15 m/s (led), stops at walls | 0.9 / 5 s | 30 dmg, kb 16 |
| Heron **kick** (players inside ~4 m) | 0.4 s: a leg drawn back | 0.15 s | 0.45 / 1.2 s | 15 dmg, kb 16 |
| Heron **needle** | 1.3 s: the **aim laser** tracks the target (120°/s) and **locks for the last 0.25 s**; the lens ring burns, then strobes white-hot | one needle | 0.4 / 3.5 s | `Needle` at 110 m/s along the locked aim, 30 dmg, parryable; needs line of sight |

Every attack telegraphs: the `Windup` state (replicated `state` + progress 0–255 in `aux`
bits 0–7, attack index in bits 8–15), `Fx::EnemyAttack { strike: false, parryable }` at its
start (sound + glint), the `PARRYABLE` flag during a parry window (the Gaoler's bash window,
the Harrier's whole dive), `AIMING` / `AIM_LOCKED` for the laser, then
`Fx::EnemyAttack { strike: true }` / `EnemyFired` when it lands. Damage is never dealt before the
wind-up completed (tested for every attack of every type).

**The Swat.** A punch while a Harrier is diving (its `parry_strike` window, the punch within
3 m + its radius of its centre — rewound to the puncher's view time like every melee parry)
is a parry: heal + PARRY style, the dive's (parry-delayed) damage is cancelled, the Harrier is
stunned for 2 s and knocked out of the air (stunned flyers don't hover: it falls) and takes 50
(2× the fist) on top of the punch itself.

**The aim laser.** While a laser attack winds up the enemy replicates `AIMING` (bit 6) and,
once locked, `AIM_LOCKED` (bit 7); its `yaw` and `pitch` are the exact aim (no yaw
quantisation while aiming), the body turns with it. Every client draws the laser from the
attack's muzzle (the definition's `projectile.muzzle` at the replicated yaw: the lens) along
`look_dir(yaw, pitch)`, stopped by the level, by player bodies and just before the camera; it
is red while tracking and white-hot and pulsing once locked (plus a click, `HeronLock`). The
needle flies exactly along the drawn laser (test: < 0.002 rad, < 2e-4 rad through the wire).

## The third roster (expansion)

Four more Moulds (`docs/expansion.md`, area **enemies**): a support that makes the others
dangerous, a skitterer that grabs you, a heavy that floods the floor, and a training dummy for
the warm-up lobby. Same conventions as above (feet origin, −Z forward, melee ranges between
body surfaces).

| | Bellwether (25) | Snuffer (26) | Smelter (27) | Effigy (28) |
|---|---|---|---|---|
| role | support: frenzies the others | skitterer: latches onto you | slow heavy: slag lines on the floor | training dummy (warm-up only) |
| look | a gaunt 3 m Mould stooped over long knuckling arms, a hooded head jutting low, a timber belfry yoke on its back carrying a big bronze bell above and behind its head | 2.2 m from tail to cap on six spidery brass legs, a segmented wax body under brass shell plates; the head is a candle snuffer: a long brass handle of a neck ending in a bell-shaped cap | a 3.4 m bow-legged hulk of slag and wax under an iron shoulder yoke, a squat iron head with a furnace-mouth grate, a huge iron crucible slung in front of its belly on chains | a wax figure bound to a timber post on an iron tripod, arms lashed along a crossbar, a target sigil on its chest, a wick-knot head |
| health | 220 | 70 | 420 | 1000 (never below 1) |
| speed / accel | 5.5 / 30 | 12.5 / 70 | 3.6 / 20 | 0 |
| capsule r × h | 0.7 × 3.1 | 0.7 × 1.4 | 1.1 × 3.4 | 0.5 × 2.2 |
| drawn (`model_scale`) | ×1.2 (3.9 m) | ×1.3 (2.9 m tail to cap) | ×1.12 (3.8 m) | ×1.25 (2.8 m) |
| nav class | narrow, 3.2 m headroom | narrow, none | wide, 3.5 m headroom | — (never moves) |
| weak points | **bell** (0, 3.17, 0.36) r 0.72 ×2 (Core) | **wick** under the cap (0, 0.82, −1.32) r 0.42 ×2.5 (Head) | **hatch** on the crucible (0, 1.62, −1.45) r 0.5 ×2.5 (Core); head (0, 3.4, −0.37) r 0.45 ×1.5 | head (0, 2.46, −0.05) r 0.37 ×2; sigil (0, 1.69, −0.35) r 0.27 ×1.5 |
| knockback scale | 0.5 | 1.3 | 0.15 | 0 |
| keeps | 14–26 m, side-steps (`preferred_range`) | melee | melee (pours on the way in) | its post |

| attack | wind-up (telegraph) | strike | recover / cooldown | effect |
|---|---|---|---|---|
| Bellwether **toll** (`AttackKind::Toll`; a target within 40 m and at least one ally within 18 m; **no line of sight needed**) | **1.6 s**: it rears up and hauls the bell back on its yoke; the bell's lip burns orange to white-hot and strobes for the last quarter; heat shimmers off it; a creaking, swelling bowed-metal hum (`BellwetherTollWindup`) | 0.3 s: the bell swings through: one deep bronze toll (`BellwetherToll`), rings racing out to 18 m | 0.8 / 6 s | every other Mould within 18 m that can be frenzied is **frenzied** for 5 s (below). Shooting the **bell** mid-wind-up for 30 damage (after its ×2: one Quill shot) **cracks** the swing: no toll, stunned 2 s, the toll goes on cooldown |
| Bellwether **clapper** (players inside ~3 m) | 0.6 s: a fist drawn back, the body twisting away | 0.15 s | 0.6 / 1.4 s | 16 dmg, 160° arc, kb 14: shoves rushers off |
| Snuffer **latch** (a leap from up to ~9 m centre to centre) | 0.7 s: it rears back, the cap tipped up off its blazing wick (the weak point bared), legs bunched, a quiver and a rising hiss | up to 0.6 s: a leap 14 m/s + 5 m/s up, led on the target; the first player it reaches (1.4 m) is **latched** | 0.6 / 3 s (from when it lets go) | 6 dmg on impact, then it clings (below). A dash through the leap (i-frames), spawn protection or a Snuffer already on that player: it misses |
| Snuffer **nip** (close; what the others do while one is latched on you) | 0.45 s: the cap lifts, the neck draws back | 0.12 s | 0.4 / 0.6 s | 9 dmg, kb 5 |
| Smelter **pour** (`AttackKind::Pour`; target 5–24 m away, feet within 1.5 m of its own level) | **1.1 s**: it plants its feet and tips the crucible over its lip, the slag brimming white-hot, drips slopping over; the **slag line is laid at once and drawn on the floor as a red telegraph strip** along where it will run | 0.6 s: the slag floods the line (`SmelterPour`) | 0.9 / 4.5 s | the line burns for 5 s: 22 dmg/s (every 0.25 s) to anyone whose feet are on it (below) |
| Smelter **heave** (close) | 0.8 s: it twists away, hauling the crucible round on its chains (chains rattle taut) | 0.2 s | 0.8 / 1.5 s | 28 dmg, 140° arc, kb 16 + up |

### Frenzy (the toll)

A frenzied Mould (`Enemy::frenzy_until`, replicated `enemy_flags::FRENZIED`) acts with the
toll's `Frenzy` multipliers on top of the difficulty scale: movement ×1.3, attack cooldowns and
recoveries ÷1.4, damage ×1.25, projectile speed ×1.15. **Wind-ups keep their length**, so every
attack stays exactly as readable. A new toll refreshes it. Bellwethers (anything with a toll),
training dummies and corpses are never frenzied (two ringers would keep each other frenzied
forever). Client: a flare and `Frenzied` when it takes, then blue-white flame tongues and
sparks licking up the body, a pulsing light, its telegraph parts flaring (`palette().frenzy`;
blue-white in the standard palette: the toll stokes them hotter than orange).

### The latch (Snuffer)

`AttackDef::latch` turns a lunge into a grab: on the first player it reaches (and may hold:
not on dash i-frames, not spawn-protected, not already carrying a Snuffer) it deals the impact
and enters **`AiState::Latched` (8)**: replicated `LATCHED`, `owner` = the victim, progress over
`max_time` (6 s). Every tick it is placed on the victim's chest (0.5 m below the eye, just in
front, facing them; no collision of its own) and it:

- **smothers the wick**: 16 dmg/s in 0.25 s ticks (`Attack::Enemy { kind: 26, attack: 0 }`),
  all of it **hard damage** (the part the pipeline doesn't already make hard is added);
- **slows the victim** to 72 % of their speeds (a free run of 16 m in a second becomes
  11.5 m: tested) through the movement state's predicted slow (`MoveState::slow_time` /
  `slow_scale`, the arsenal's Pinned slow), refreshed every tick for 0.25 s, so it eases off
  right after it lets go; a longer, stronger slow (a Pinning Arrow) wins; never a root;
- lets go when **punched** by the victim (the `Punched` event of that tick: thrown off along the
  punch at 11 m/s + up, stunned 1.6 s with its cap knocked open, 30 counter damage on top of the
  punch itself), when they **dash** (shaken off backwards, stunned 1 s), when they die, go down
  or become untargetable, after 6 s, when the world freezes, or when a **Pinning Arrow** pins
  it (it stays rooted where it was hit, its victim free).

Other players can shoot it off their friend (it dies on them like anywhere else). Only one
Snuffer latches onto a player at a time; the others `nip`.

**Prediction.** The latch state is replicated on the Snuffer (the client knows it is on you:
`LATCHED` + `owner`). The slow lives in the victim's `MoveState` (sent in their own state), so
their client predicts it like the Pinned slow: only the moments it takes hold and lets go are
corrected (one round trip each).

Client: a clank and a puff on latching; for the victim the Snuffer is drawn clinging to their
chest from the camera (its replicated position trails the predicted view): the cap clamped
along the bottom of the view, legs up the sides, soot closing in from below and a muffled
guttering (`SnufferSmother`) every 0.55 s; everyone else sees it clamped onto that player,
squeezing soot and sparks out from under the cap. Thrown off: a ring and skittering legs.

### Slag lines (Smelter)

`hazard.rs`: a pour lays its line **when the wind-up starts**: from just in front of the
Smelter (`pour.start` beyond its surface) along the floor towards the target's feet + 0.4 s of
their velocity, `overshoot` 4 m past them, at most 20 m: `trace_floor` samples the floor every
0.75 m and stops at walls (knee height), ledges, and steps of more than 0.8 m. A path shorter
than 3 m means no pour (the Smelter walks on and tries again shortly). The line is a replicated
entity (`kind::SLAG_LINE` 140) from that moment, **ignites on the strike tick** (the end of the
wind-up) and burns for `lifetime` 5 s: every 0.25 s each targetable player whose feet are on
the strip (half its 1.8 m width + half their radius, horizontally, and within 0.5 m above the
floor there) takes 5.5 (`Owner::Enemy(smelter)`, `Attack::Enemy { kind: 27, attack: 1 }`, kill
feed SMELTER [POUR]). Jump over it, step off it during the wind-up, or dash through on i-frames.
A pour cancelled before its strike (killed, stunned, frozen) removes its unlit line; a lit line
burns on after its Smelter dies. Enemies never take slag damage. Clients trace the same floor
path from the replicated start and end, draw the telegraph as a translucent strip with bright
rims in `palette().telegraph`, then a molten strip (`fx_slag_strip`, `palette().hazard` in the
colour-blind modes) with embers, smoke, a few lights and scorch marks left behind; the last
second (`COOLING`) darkens and smokes.

### The Effigy (training dummy)

`EnemyDef::dummy`: no AI at all (never targets, never alerted, no attacks, never frenzied or
elite), never moves (knockback 0, a mass the push-apart treats as immovable), can't die: every
hit is reported in full (hit markers, damage numbers) but its health stops at 1; each hit makes
it flinch (`Recover` for 0.35 s: it rocks on its post, the rope binds flash) with an
`EffigyHit` thunk; `regen_delay` 3 s after the last hit it heals at 400/s back to full. Ember
bursts are shed from the health it actually lost. Never placed in encounters or waves
(`EnemyRegistry::wave_names()` leaves it out); the **lobby** area places it in the hub.

### Elites

Any Mould can be an elite (`Enemy::elite`, replicated `enemy_flags::ELITE`). `EnemyDef::elite`
(defaults; the Smelter overrides health ×2.0, speed ×1.2): health and max health ×2.5 (on top of
the spawn's health scale), attack cooldowns / recoveries ÷1.25 (wind-ups unchanged), movement
×1.1, knockback ×0.6, its death's heat (the big ember burst) ×2. Client: the wax runs hot
(warm body tint, hotter glows), a translucent searing shell 7 % larger, a crown of searing
embers circling over its head and a light (`palette().elite`). Max health still fits the
replicated 16 bits (a 3-player elite Gaoler is ~3900).

### Difficulty: `SimWorld::enemy_scale` and `SimWorld::fury`

`SimWorld::fury(&Enemy) -> Fury` is what every enemy acts with right now: `enemy_scale`
(set by **modes** from difficulty × player count) × its frenzy (while frenzied) × its elite
bonus. It is applied in every attack path:

| | where |
|---|---|
| damage | melee arcs and lunges (`melee_hits`), slams, dives, every projectile (and so its explosion), the latch's impact and smothering, slag lines (fixed when the line is laid) |
| aggression | `end_strike` (cooldown + recover and the recovery state) and a latch letting go; **wind-ups are never scaled** |
| projectile speed | straight shots and the Heron's needle (and the aim laser's lead); lobs fly a flatter, faster arc at `least speed × scale` (the low solution, `lob_low_arc`), still landing on the predicted feet (tested at ×1.4) |
| movement speed | frenzy and elites only (walking, the flyers' steering; not leaps or dives) |

### Spawn API (for the game modes)

```rust
/// shared::sim::{SpawnOptions, SimWorld}
pub struct SpawnOptions { pub health_scale: f32, pub elite: bool, pub aware: bool }
impl SimWorld {
    /// Spawns `name` (an `assets/enemies` stem) with a health multiplier, as an elite or not,
    /// aware or not (wave spawns should be aware). `None`: unknown name / world full.
    pub fn spawn_enemy_with(&mut self, name: &str, position: Vec3, yaw: f32,
                            options: SpawnOptions) -> Option<NetId>;
    /// Turns a living enemy into an elite (not dummies). Idempotent.
    pub fn make_elite(&mut self, id: NetId) -> bool;
    /// The multipliers an enemy acts with (enemy_scale × frenzy × elite).
    pub fn fury(&self, e: &Enemy) -> Fury;
}
impl EnemyRegistry {
    /// Every type that may appear in encounters / waves (all but the Effigy).
    pub fn wave_names(&self) -> impl Iterator<Item = &str>;
}
```

`spawn_enemy` / `spawn_enemy_scaled` are unchanged (they call `spawn_enemy_with`). The game
modes' `SimWorld::spawn_pve_enemy` (encounters and grind, `difficulty.rs`) spawns through it
with the difficulty's health and elite roll; dev `/spawn` uses the same difficulty health. A
boss-grade elite is an elite with a larger `health_scale` (grind: `boss_health` ×4, then the
elite bonus). Flyers are placed by their feet
(the encounter code converts hover-centre spawn points; grind should too).

## Sizes and hit shapes (the refit)

Players found the Moulds hard to hit (`docs/refit.md`, area **pve**): the small, quick and
thin ones (Gnawer, Harrier, Snuffer, the Heron's stilts) most of all. So every Mould is now
**drawn bigger than it is built** and its **hit shapes cover more than the drawn model**.

* **Drawn size**: `EnemyDef::model_scale` (rows "drawn" above; `draw_scale()` keeps a file's
  value in 0.25..4). The client scales the model about its origin — the feet, the Harrier's
  hover centre — in `ModelLibrary::draw_enemy` / `enemy_attack_point`, so every caller (the
  game, elites' shells, the replay, `models_view`) draws the same size; `EnemyKind::scale()` /
  `height()` follow the definition. Around ×1.25, per type as it reads best: the small and
  quick ones most (Harrier and Snuffer ×1.3), the heavies least (Gaoler and Smelter ×1.12:
  at 3.7 m they read already, and a 4.6 m Gaoler would crowd every room it fights in).
* **The movement capsule stays** (`radius`, `height`), so the nav grid, the nav classes'
  headroom, doors, ledges and every level test are unchanged. The cost: a Mould can poke
  through a low ceiling. Measured on every shipped level (passable nodes of each class whose
  headroom is below the drawn height): 0–0.3 % almost everywhere, at most 2.7 % (the Heron on
  The Drowned Chandlery; 1.5 % the Bellwether there), so it is rare.
* **Hit shapes are split from the movement capsule** and sized to the scaled model
  (`EnemyDef::hitbox(target, feet, yaw)` builds them; `Enemy::hitbox` and the client's debug
  views use it): the upright body capsule has its own radius (`hit_radius`, `None` = the
  capsule's) and top (`body_top`); `body_shapes` add capsules for what one upright capsule
  misses; the weak points sit on the scaled models. All `HitPart::Body` but the weak points
  (armour, damage numbers and the parry rules are unchanged). Offsets from the feet, +X right,
  −Z forward:

  | | body capsule r, top | extra body shapes | weak points |
  |---|---|---|---|
  | Gnawer | 0.78, 2.3 | a bar across the fists (r 0.3) | head (0, 1.95, −0.65) r 0.52 |
  | Censer | 0.74, 2.35 | — | lantern (0, 2.68, −0.12) r 0.5 |
  | Harrier | 0.7, (1.3) | neck and beak (r 0.26), both wings in their V (r 0.32, to ±1.8 m) | heart (0, 0.26, −0.2) r 0.44 |
  | Gaoler | 1.4, 3.45 | the cell door (x −0.7, z −1.05, r 0.7) | head (0, 3.72, −0.35) r 0.5, lantern (0, 2.46, 1.06) r 0.6 |
  | Heron | 0.6, 3.1 | the long back (r 0.42) | head (0, 3.74, −0.48) r 0.42 |
  | Bellwether | 0.8, 2.65 | the low hooded head (r 0.4), both forearms to the floor (r 0.2) | bell (0, 3.17, 0.36) r 0.72 |
  | Snuffer | 0.75, (1.4) | the back and tail (r 0.45), the cap above the wick (r 0.44), two bars across the legs (r 0.3) | wick (0, 0.82, −1.32) r 0.42 |
  | Smelter | 1.2, 3.15 | the crucible (r 0.85), the shoulders (r 0.5) | hatch (0, 1.62, −1.45) r 0.5, head (0, 3.4, −0.37) r 0.45 |
  | Effigy | 0.5, 2.2 | the crossbar arms (r 0.15) | head (0, 2.46, −0.05) r 0.37, sigil (0, 1.69, −0.35) r 0.27 |

  Weak points stay in front of the body shapes that could hide them (the Snuffer's cap is
  above its wick, the Smelter's hatch sphere stands proud of the crucible, the Gaoler's
  lantern of its back), so a plain hit on them is still a weak-point hit.
* **More hit assist**: `combat.hit_assist.enemy_body_pad` 0.2 → **0.35**, `enemy_weak_pad`
  0.1 → **0.15** (player-owned attacks only; enemy attacks test the plain player shapes, PvP
  pads unchanged). Together: the plain shapes hold 60–94 % of each drawn model's vertices, the
  padded ones 92–100 % (the model test prints them); the hit area reaches past the model on
  every side.
* **What moved with the models**: eye heights (sight lines start at the drawn eyes), the
  Censer's volley muzzle (3.66 m, the censer whirled over its head) and the Heron's lens
  (3.74 m: the needle and the aim laser leave the drawn lens).
* **What didn't**: attack reaches and arcs. Melee ranges are measured from the movement
  capsule's surface as before; the bigger bodies only make the reach read right (a Gnawer's
  jaw now ends about where its bite lands). Knockback, mass and the push-apart use the
  movement capsule. The latched Snuffer is drawn at its old size in its victim's view (net of
  the scale), so it doesn't cover the crosshair; everyone else sees it at full size.
* **Client**: tracer ends use the hit radius (`enemy_fx::enemy_size`), the colour-vision parry
  icons sit over the drawn height.
* **Tests**: `builtin_roster_matches_the_contract` (the new weak points; every Mould drawn at
  least ×1.1, its hit radius at least the capsule's), `client::models` `enemy_models_match_
  their_definitions` (drawn top within 0.45 m of the hit shapes' top, weak points inside the
  drawn model, ≥ 55 % of the model inside the plain shapes and ≥ 90 % inside the padded ones),
  `combat_weapons::hit_assist_pads_player_attacks_only` (reads the pad from the tuning). Shots:
  `enemy_shots` `hit_shapes_1..3` (three Moulds each, white the plain shapes, orange the weak
  points, cyan as padded) and `hit_shapes_behind`.

## AI (`ai.rs`, server only)

```text
 Idle ──(sees a targetable player in sight range with LOS, or gets hurt)──▶ Alert (reaction_time, EnemyAlerted fx)
   ▲                                                                            │
   └──(no target)── Chase ◀──────────────────────────────────────────────────────┘
                    │  ▲
   first usable     │  └── Recover (recover s) ◀── Strike (melee / leap / dive / lobs / needle) ◀── Windup (telegraph)
   attack ──────────┘          ▲ a wall / the level stops a leap or dive early ┘   parried: Stunned ──▶ Chase
                               └── Latched (a latching leap reached someone) ──(punched / dashed: Stunned; let go: Recover)
 any state ──(killed)──▶ Dying (corpse, no hitboxes, death_time) ──▶ removed
 Windup ──(its interrupt weak point took enough damage)──▶ Stunned (the attack on cooldown)
 Chase ──(on a jump's takeoff)──▶ Leap (crouch, then airborne) ──(lands)──▶ Chase
 Chase ──(stranded for MeltDef::after)──▶ Melt (slumps into a puddle) ──▶ Reform (rises near
       its target) ──▶ Chase
```

* **Targeting** (every `retarget_interval`, or at once when the target became invalid): a player
  who dealt ≥ `aggro_switch_fraction` of max health in the last 2 s; else the current target
  unless a visible one is closer than `target_stickiness ×` its distance; else the nearest
  visible (sight range + line of sight from the eye). **Aware** enemies (all encounter spawns)
  fall back to the nearest targetable player even unseen and never lose interest; others drop
  the target after `lose_target_time` without sight. Hurting an enemy that has no target alerts
  it onto the attacker. Targets that die, disconnect or become untargetable are dropped the
  same tick (`remove_player` also clears targets/aggro).
* **`is_targetable(player)`** (`shared::sim::enemy::is_targetable`): `!dead && !notarget`.
  **Hook for the modes**: add downed / spectating there; enemies then never target or hit them.
* **Attack choice**: definition order = priority; needs the cooldown ready, line of sight (not
  a toll), the range band (melee: between body surfaces and vertically overlapping, a
  `jumpable` sweep not against someone standing well above it; projectile: muzzle to chest;
  dive: centre to chest, clear line; toll: centre to chest and enough allies in reach; pour:
  centre to feet on its own level), `min_target_speed`, for leaps safe ground along the leap,
  for latches nobody else latched onto the target, and for pours room for a line.
* **Strikes.** Melee: arc + reach (+ `jumpable`). Lunges lead the target (aim at where it will
  be when the leap reaches it); a pounce (`lunge_up`) only leaps where its whole airborne arc
  lands on safe ground (led direction, else straight, else it runs instead); a lunge whose last
  move covered < 35 % of its speed hit a wall and ends (the rush stopping at walls). Dives fly
  a straight 3D line (flight physics, no gravity) and end on any level contact, braking hard;
  recover climbs back with the flight steering. Lobs: `lob_velocity` solves the least-speed
  arc (45° + half the elevation) onto the target's feet + horizontal lead × flight time
  (iterated), capped at `speed`; bursts fan evenly across `spread`. Lasers: `track_aim` turns
  `Enemy::aim` towards the target (chest + optional lead) at `track_rate`, locks for the last
  `lock` seconds; the projectile flies along `aim`.
* **Chase movement**: melee enemies walk the nav grid to the target (leaping and riding pads
  on the way: "Leaps, pads, melting" below) and stop at 60 % of their shortest non-leap melee
  reach; with the target unreachable they **prowl** the end of their partial path. Ranged
  enemies (Censer, Heron, Bellwether) walk until they see the target within their band's max
  (to a **vantage point** when walking at it doesn't bring it into sight), then hold it: back
  off when closer than the min (if there is room), strafe sideways, flip direction when
  blocked, when the strafe would break line of sight, every ~2.5 s, and (`ai.reposition`, the
  Heron) after every ranged strike. Harriers circle (below).
* **Movement** (`move_enemy`): accelerate towards the desired velocity (ground: horizontal +
  gravity), capsule collide-and-slide against `MASK_MOVEMENT` with step-up (0.45 m), walkable
  hits followed like the player controller, ground snap, the capsule kept just outside the sweep
  skin. Depenetration ignores implausibly deep contacts that a slightly shrunk capsule doesn't
  confirm (the contact query reported a 4.7 m wrong-way penetration for a Gaoler resting exactly
  on a face; it fell through the floor). Soft separation while moving plus a hard
  **push-apart** pass after everyone moved (3 passes, mass ∝ r²h so a Gaoler shoves Gnawers
  aside, capsule sweep so nobody is pushed into a wall, never over a ledge).
* **Knockback**: `velocity += knockback × knockback_scale`; above 4 m/s (after scaling) the
  enemy **staggers** for 0.3 s. Gaolers (×0.12) barely move.
* **Edge guard** (`steer::guard`, `heading_off_edge`): before moving, a grounded enemy checks
  the nav column ~r + 0.35 m ahead: lava or a drop deeper than allowed (paths: 5.5 m;
  strafing, backing off, leaps, fallback steering: a step) makes it turn up to ±90° or stop; its
  own momentum is stopped dead at the edge. Knockback is exempt (launching enemies into lava is
  a feature); so is a swatted Harrier falling, and so is the last 1.5 m onto a jump's takeoff
  (the node itself is the guard: it sits right at the edge).
* **Kill volumes**: an enemy whose feet enter one (or that falls below the level) dies through
  the damage pipeline (`Attack::World`, `Owner::World`).
* **`SimWorld::frozen`** (results screen): living enemies drop their target and attack, idle and
  just stand (Harriers hover in place); encounters don't trigger.

### Flying (Harrier, `steer::fly_velocity`)

No nav grid. Horizontal: orbit the target (tangent × strafe sign) with a radial correction
towards the middle of its 8–16 m band; when the target has been out of sight for 0.75 s it
closes in instead. Vertical: hover between 5 and 10 m (centre 7.5 m ± a slow 0.8 m bob) above
the ground below it **or the target's feet, whichever is higher**; a ray up keeps it off
ceilings. A sphere **feeler sweep** (r + 0.2 m, ~0.45 s ahead, ≥ 2 m) slides the velocity
along / pushes it off walls; the orbit turns round when a wall is close **or straight ahead**
(a pillar on its circle used to hold it in a stalemate) — ceilings and floors only deflect it
(flipping under a roof made flyers dither in place). Flight uses the same collide-and-slide (no
gravity), so it can't clip the level. While stunned (swatted) or dead it falls like a ground
body.

**Detours** (`steer::find_detour`): once the target has been out of sight for 0.75 s (or the
orbit flips at a wall a second time within 1.7 s: dithering), the Harrier looks for a point
that sees the target's chest and that a sphere sweep reaches from where it is — straight up
first (3 / 6 / 9 m, no higher than its hover band allows: over a low wall), then 7 m out to
either side of the line to the target at 60° / 100° / 140°, its orbit's side first, at its
height or 3 m up (round a pillar) — and flies to it (≥ 0.6 × its speed, the feelers still
active) until it is there, the target is back in sight, or 2 s pass; then it circles on from
there. At most one search per Harrier every 0.5 s (≤ 15 ray + sweep pairs). Without a point
it closes in as before. A Harrier hovering behind a 20 m pillar from its target is never blind
for more than 55 ticks (one detour; without detours: up to 626 ticks, 10.4 s, nosing into the
pillar).

### Leaps, pads, melting (getting there)

Players found the Moulds' movement lacking ("especially when they get stuck or don't cross
water"): the grid had no links across gaps or for pads, drops were one-way, deep water is a
kill volume, so a player on a floe, across a flooded break or on a pad-only gallery was out of
reach, Moulds piled up at the water's edge or under a ledge, and one knocked onto a ledge it
couldn't leave stayed there (stalling an arena or a grind wave). Now:

| | leaps across / up / down | crouch | pads | melts after | re-forms (m from target) |
|---|---|---|---|---|---|
| Gnawer | 8 / 3.5 / 8 m | 0.3 s | yes | 6 s | 8–18 |
| Snuffer | 8 / 4 / 8 m | 0.25 s | yes | 6 s | 8–16 |
| Censer | 4.5 / 2 / 6 m | 0.45 s | yes | 6 s | 14–28 |
| Heron | 7 / 2.5 / 7 m | 0.4 s | yes | 6 s | 20–40 |
| Bellwether | 5 / 2 / 6 m | 0.45 s | yes | 6 s | 12–24 |
| Gaoler, Smelter | — | — | yes | 6 s | 8–16, 8–18 |
| Harrier | flies (detours, above) | | — | never | |

(`EnemyDef::{leap, pads, melt}`; `leap: None` = never leaps.) The pack crawler that already
pounces 8 m leaps furthest; the spidery Snuffer climbs highest; the robed Censer and the
Bellwether with a bell on its back only hop; the Heron's stilts bound far but not high; the
heavies don't leap at all — a 3.7 m Gaoler or a Smelter with a full crucible in the air would
read wrong, and their wide footprint rarely has landings — but they ride pads and re-form.

* **Leaps** (`AiState::Leap`): on its jump's takeoff the Mould faces the landing and
  **crouches** (`LeapDef::crouch`; progress replicated, no telegraph glow: it isn't an attack),
  then leaves the ground with `nav::leap_velocity` from where it actually stands to the landing
  (the arc the nav build swept) and flies it ballistically (`move_enemy`, gravity). If a wall or
  a lip steals more than half its horizontal speed, the flight is re-aimed at the landing with
  the time left (≤ 1.5× the planned speed), so a grazed lip doesn't drop it back. Other Moulds
  never shove a leaper off its arc (the push-apart skips it). A landing somebody stands on (or
  is flying to) holds the crouch up to 0.75 s, then it picks another way. A stagger during the
  crouch calls it off; knockback in the air is not corrected (knock it into the water). Landed
  on the planned spot it follows its route on; anywhere else it re-plans.
* **Pad rides**: it walks onto the floor under the pad and is launched there with the pad's
  velocity (set, as for players), no crouch. Moulds only ride a pad on purpose (a route through
  its link): walking over a pad plate doesn't throw them (players' pads fire on any overlap).
* **Prowling**: a melee Mould at the end of a partial path (its target unreachable) paces side
  to side across the line to its target at 45 % speed, facing it, turning at walls, ledges and
  lava, every ~1.5 s and 2.5 m from where its path ended, instead of standing still.
* **Vantage points** (`steer::vantage_site`): a ranged Mould whose target is out of sight (for
  1.5 s while walking at it) or unreachable searches forward from its node (Dijkstra over its
  links, ≤ 40 m of path, ≤ 3000 nodes) for nodes whose eye is within its band of the target's
  chest, scores them by walking cost + distance from the band's middle, tests the sight lines
  of the best 24 (≥ 1.5 m apart) and walks to the first that sees the target (given up after 4
  s; one search a second per Mould). A Censer under a 5 m plateau backs off ~9 m and lobs.
* **Stranded** bookkeeping, every tick of `Chase`: a Mould **in a fight** — ranged: seeing its
  target within its band; melee: within 1.5 m of its reach, about level — isn't stranded
  (neither is one winding up, striking or recovering). Otherwise it counts: a target that is
  `Unreachable` counts every tick (prowling included); `Full` / `Far` count only while it isn't
  getting anywhere (it moved < 1 m in the last 2 s window), and reset when it is; `OffGrid`
  (standing on no node of its class: a prop, a beam) counts **three times** as fast while it
  isn't moving. At `MeltDef::after` (6 s; 2 s off the grid) it **melts**.
* **Melting** (`AiState::Melt`, `sink` 0.6 s): it slumps and sinks into a wax puddle where it
  stands — no attacks, its pour line (if any) dropped, knockback ignored (a puddle doesn't
  fly). It stays **hittable**: its hit shapes sink with its model (below), so a player who sees
  it going can still finish it; killed, it dies as usual. Then it is moved to its re-form site,
  facing its target, and **re-forms** (`AiState::Reform`, `rise` 0.8 s) — rising out of a new
  puddle, equally hittable — and chases on.
* **The re-form site** (`steer::reform_site`), searched once at the start of the melt (one
  search a tick for all enemies; the next tick otherwise): back from the target's node over
  the **reverse links** (doors as they are now — so never in a room the target can't be reached
  from), ≤ 6000 nodes and 2.5 × `far` + 20 m of path, the nodes of its class from which the
  target is reachable, never deadly or at a hazard edge, never with the feet in a kill volume,
  at least 3 m from the target, not on another Mould (or another's re-form site), inside its
  encounter's arena while the target fights there; scored by how far outside `near`..`far`
  they are (×2) plus 5 % of the path cost; the first of the best 24 that is **out of the
  target's sight** (eye to the Mould's eye) wins, else the best one. **No site** (an isolated
  target: a pillar nothing reaches) → no melt; it prowls on and tries again in 3 s. A target
  standing where this class can't (a floe too small for a Gaoler) while the Mould is already
  within `far` → no melt either: re-forming wouldn't bring it closer, it waits at the edge.
* **Wading: no** (decided from the level data). The only water is deep water in The Drowned
  Chandlery, and every pool there is a pit with sheer sides: the beds lie 2.4 m (the C1 sluice)
  to 6.4 m (the Great Wick's well) under the surface and the surface 0.5–1 m under the floors
  round it (`docs/levels.md`). A wading Mould would have to drop 2.9–7 m into the pit and climb
  that far back out, and the water is over the head of every Mould but the four tallest in the
  shallowest pool (the sluice, which a leap crosses anyway). So wading would have changed
  nothing but put tall Moulds at the bottom of pits; leaps carry Moulds over the water (onto
  floes, across the breaks) and re-forming covers the rest. The level format stays as it is;
  water still quenches a Mould knocked into it (all kill volumes kill enemies).
* `/melt` (dev): every ground Mould melts on its next chasing tick, stranded or not
  (`SimWorld::melt_enemies`).

**Hit shapes while melting** (`Enemy::hitbox`): at melt depth *d* (`enemy::melt_depth`: the
first three quarters of `Melt` sink it, the last three quarters of `Reform` raise it) every
shape is sagged to `1 − 0.35 d` of its height about the feet and sunk `0.75 d` of the drawn
height into the floor (`MELT_SAG`, `MELT_SINK`); the client draws the model with the same
numbers, so what shows above the floor is what can be shot, down to the puddle (lag
compensation records them like any hitbox).

**Client** (`entities.rs`, `enemy_fx.rs`, `models::enemies`): `Leap` without `AIRBORNE` is
drawn as `EnemyState::Crouch` (the Gnawer's pounce coil, the Snuffer's bunched legs, the others
squashed down and out), with `AIRBORNE` as `Airborne` (the pounce / latch strike or a
mid-stride, stretched at takeoff and tilted along the flight); grit and dust at takeoff and
landing (`Jump` / `Land`, `HeavyLand` for the heavies, pitched by size). `Melt` / `Reform` use
the kind's death slump at the melt depth, sagged and sunk into the floor (`body_transform`), over
the **wax puddle** model (`ModelId::WaxPuddle`, `fx_wax_puddle.gltf`: lumpy lobes of the
Mould's wax with its molten heart glowing in its heat colour; it spreads as the body sinks and
drains away at the end of the slump, wells up first where it re-forms and is drawn back into the
body as it rises — zero at both ends of the move, so the teleport never shows); molten wax
slopping out (setting into wax splats) and steam as it melts (`MouldMelt`: a wet sag, a hiss,
bubbling), wax bubbling up as it re-forms (`MouldRise`: bubbling, a rising groan, a slap) and
a splash ring when it stands. Replays and spectating draw the same entities.

## Navigation grid (`shared::sim::nav`)

Built once per level from the collision world: by the server at startup
(`SimWorld::build_nav`, logged) and at match start, lazily on the first enemy tick otherwise
(client prediction worlds never build one).

* **Columns**: 1 m cells over the XZ bounds of every movement-blocking collider (doors
  ignored). Each column gets **one node per floor** (downward rays from above / below every
  collider overlapping the column, normal ≤ 50° from up). Stacked floors are separate nodes.
* **Node flags**: `SMALL` walkable — an r 0.5 capsule from 0.45 m to 2.0 m above the floor fits
  (the narrow footprint; agents up to r 0.7 use it, collide-and-slide copes with the rest);
  `DEADLY` — inside a kill volume; `HAZARD` — a kill volume within 1.25 m (walkable, +4 m path
  cost); `LARGE` — the wide footprint: walkable and all 8 neighbours linked within 0.6 m height
  with ≥ 3.3 m of shoulder headroom. Every node also stores its **headroom** (an upward ray when
  something is overhead, else 6 m).
* **Agent classes** (`NavClass { wide, headroom_dm }`, `NavClass::for_size(r, h)`): a footprint
  plus the headroom it needs (`h + 0.1`, none for agents up to 2.1 m). `passable` checks both,
  so A*, straight-walk tests and string pulling all respect them: the Censer (2.6 m) walks under
  a 3 m lintel, the 3.4 m Heron goes round it (tested), the Gaoler needs the wide footprint and
  3.8 m.
* **Walking links** (8 directions): step (≤ 0.45 m), continuous ramp (≤ 1.2 m per cell,
  midpoint on the slope) or one-way **drop** (≤ 5.5 m), nothing in the way at knee / chest
  height. Diagonals only where both orthogonal routes agree.
* **Jump links** (`NavJump`, one-way, built last; see below): **leaps** across gaps — over
  lava and deep water too — and up / down ledges, and **pad rides**.
* **Doors**: nodes near a door brush are impassable while it is closed; so is a jump whose arc
  passes a door brush.
* **Queries**: `node_at`, `nearest_node`, `walkable_line` (walking links only), `find_path`
  (A* over walking links and the jumps the class may take, octile-ish heuristic ×1.1, costs
  distance + 0.5 × |Δy| + hazard, a leap + `LEAP_COST` 5, a pad ride + `PAD_COST` 3,
  generation-stamped buffers, `max_expand` budget → `Partial` path; `NavScratch::
  last_budget_hit` tells a budget stop from an unreachable goal), `search` (Dijkstra forward
  or over the **reverse links**, `for_each_predecessor`: every node it settles can reach the
  start), `smooth_route` (string pulling that never skips across a jump: its takeoff and
  landing stay waypoints, the landing carrying the jump), `ground_ok` (edge guard).

**Jump links.** *Leaps* start from **edge nodes** (walkable, not deadly, some walking link
missing): rays every 22.5° within ~25° of a missing link, marched in 0.5 m steps from 1 m to
`LEAP_MAX_DISTANCE` 8 m; the first floor past a gap (a column with no floor at our level), or
above / below the step and drop limits (up to `LEAP_MAX_UP` 4 m, down to `LEAP_MAX_DOWN` 8
m), that isn't already a walking neighbour is a candidate (≤ 3 per ray). A candidate becomes a
leap when

* its **landing is safe**: walkable, not deadly, and 0.75 m further along the leap there is
  floor at most 1 m lower that isn't deadly, or a wall to bump into — never a drop, lava or
  water just past it (so a 1 m island in lava gets none; a 2.8 m floe does);
* the **arc is clear**: the leap flies a fixed parabola (`leap_point`: through both ends, its
  apex `leap_apex` = the higher end + 0.5 m + 15 % of the distance; its shape doesn't depend on
  gravity, so the nav build sweeps exactly what the agent flies), checked first against the
  grid's own columns (a floor inside the capsule's span or a measured headroom that ends in it
  fails) and then by **capsule sweeps** along it in ~2 m chords (sag < 5 cm), riding 0.25 m
  above the feet: the tall clearance capsule (r 0.48 × 3.5 m: `jump_clear::TALL`, Censer, Heron,
  Bellwether) first, and when that doesn't fit the narrow one (r 0.48 × 2 m: `NARROW`, Gnawer,
  Snuffer); heavies don't leap (`WIDE` only on pad rides);
* the feet **never pass through a kill volume** anywhere along the arc;
* climbs that hit the lip from the wall's foot are retried with a **run-up** from up to two
  cells further back (a 3 m ledge is taken from 1–2 m out, so the arc clears it).

*Pad rides*: from the walkable floor under each pad's trigger, the launch (the pad *sets* the
velocity, as `movement::step` does for players) is flown at 60 Hz with collide-and-slide for
each clearance capsule (r 0.6 × 2 m, r 0.6 × 3.5 m, r 1.3 × 3.8 m, gravity
`MovementTuning::default().gravity`) until it lands on a floor that class can stand on; one
link per landing, carrying the capsules that got there. Duplicates (takeoff, landing) merge.
The links are stored sorted by takeoff with a reverse index by landing.

**Agent classes** (`NavClass`) carry their leap limits (`with_leaps(distance, up, down)` from
`EnemyDef::leap`) and whether they ride pads (`EnemyDef::pads`); `can_jump` checks a link's
clearance bit and those limits, A* checks the landing's passability (headroom, the wide
footprint) and its door like any node.

**Path following** (`NavAgent`, `steer::ground_velocity`): straight walk when `walkable_line`
allows it; otherwise A* (at most **4 searches per tick** for all enemies, ≤ 3000 nodes each),
string-pulled waypoints, re-planned every `repath_interval` (staggered), when the goal moved
> 1.5 m + 20 %, or when stuck (< 0.5 m in 0.5 s → re-plan + side-step). Each plan records its
**`Reach`**: `Full` (a path or a straight walk), `Far` (the search hit its budget: progress
decides), `Unreachable` (the open set ran dry: the partial path ends as close as it gets),
`OffGrid` (the agent stands on no node of its class). A waypoint reached by a jump is never
passed by proximity: the agent walks onto the jump's takeoff node (slowing down within 1.5 m,
trusted by the edge guard) and within 0.3 m on the ground hands the jump to the AI
(`NavAgent::jump_ready`); after landing on the planned spot it follows the route on from the
landing (`NavAgent::landed`), anywhere else it re-plans. At the end of a partial path melee
enemies prowl and ranged ones look for a vantage point (below).

### Measurements (release build)

| level | columns | floors | walkable / large / hazard / deadly | links | build | passable per class (gnawer / censer / heron / gaoler) |
|---|---|---|---|---|---|---|
| `crucible_of_veins` | 168 × 406 | 35 397 | 31 268 / 25 672 / 620 / 1 298 | 232 719 | **31 ms** | 31 268 / 31 247 / 31 183 / 25 636 |
| `cistern_nine` | 74 × 74 | 10 900 | 10 074 / 7 896 / 64 / 96 | 74 270 | 11 ms | 10 074 / 10 074 / 10 074 / 7 896 |
| `movement_test` | 202 × 262 | 53 824 | 50 384 / 45 028 / 0 / 480 | 382 308 | 25 ms | 50 384 / 50 384 / 50 384 / 45 008 |

On the campaign 13 950 walkable nodes are reachable from the spawns / checkpoints / enemy
spawns (walking links), 1 687 columns have stacked walkable floors and 320 walkable floors lie
over lava. A* start → exit: 0.15 ms; an arena spawn → its rally point: 2 µs.

**Jump links and build time** (release, `enemies_nav -- --nocapture`, measured on a 16-core
machine loaded by parallel builds, load ~14–20; "before" = the grid without jump links, same
machine at the start of the work):

| level | build before → after | of which jump links | leaps (over lava / water) | pad rides | arc chord sweeps |
|---|---|---|---|---|---|
| `crucible_of_veins` | 33.0 → 57.2 ms | 30.7 ms | 7 306 (1 304) | 7 | 41 868 |
| `spire_of_wicks` | 28.0 → 60.3 ms | 38.1 ms | 4 495 (343) | 5 | 32 189 |
| `drowned_chandlery` | 18.0 → 38.6 ms | 24.3 ms | 4 674 (1 008) | 5 | 25 852 |
| `cistern_nine` | 16.8 → 37.5 ms | 23.2 ms | 6 642 (264) | 2 | 36 561 |
| `knell_yard` | 15.0 → 38.5 ms | 25.9 ms | 4 010 (616) | 12 | 26 320 |
| `ingot_run` | 14.7 → 37.8 ms | 26.0 ms | 6 093 (204) | 2 | 36 656 |
| `furnace_ring` | 44.1 → 96.7 ms | 61.0 ms | 6 088 (56) | 4 | 47 283 |
| `tallow_works` | 17.8 → 35.7 ms | 21.7 ms | 2 948 (307) | 6 | 20 874 |
| `wickmarket` | 22.4 → 55.8 ms | 38.9 ms | 5 053 (48) | 4 | 34 289 |
| `vestry` | 5.9 → 15.7 ms | 10.9 ms | 2 833 (0) | 3 | 15 604 |
| `movement_test` | 29.4 → 60.1 ms | 37.0 ms | 15 750 (92) | 1 | 70 760 |

The grid is built once per level (at server start, reused across matches on the same level,
`server::lobby`); the Furnace Ring's octagonal tiers (many turned boxes) make its sweeps the
dearest. Cuts that got it here: rays only within ~25° of a missing link, at most 2 candidates
per ray, the tall capsule swept first (in the open it clears for both), ~3 m chords, and the
grid's own columns rejecting most blocked arcs before any physics.

**Reachability** (`enemies_nav reachability_per_class_on_every_level`): how much of the floor
players can reach (approximated by walking + every leap + every pad from the player spawns and
checkpoints, doors open; players do more) each ground Mould reaches from there, counted over
the part of it the Mould fits on, walking only (before) → with its leaps and pads (after):

| level | player floor (nodes) | Gnawer | Snuffer | Censer | Heron | Bellwether | Gaoler | Smelter |
|---|---|---|---|---|---|---|---|---|
| Crucible of Veins | 14 017 | 86 → 100 % | 86 → 100 % | 86 → 100 % | 86 → 100 % | 86 → 100 % | 87 → 100 % | 87 → 100 % |
| Spire of Wicks | 8 256 | 98 → 100 % | 98 → 100 % | 98 → 100 % | 98 → 100 % | 98 → 100 % | 96 → 97 % | 96 → 97 % |
| The Drowned Chandlery | 6 249 | 93 → 100 % | 93 → 100 % | 94 → 99 % | 94 → 99 % | 94 → 99 % | 96 → 98 % | 96 → 98 % |
| Cistern Nine | 5 292 | 95 → 100 % | 95 → 100 % | 95 → 99 % | 95 → 99 % | 95 → 99 % | 98 → 98 % | 98 → 98 % |
| Knell Yard | 2 600 | 93 → 100 % | 93 → 100 % | 93 → 100 % | 93 → 100 % | 93 → 100 % | 84 → 84 % | 84 → 84 % |
| Ingot Run | 4 028 | 92 → 98 % | 92 → 100 % | 92 → 98 % | 92 → 98 % | 92 → 98 % | 93 → 94 % | 93 → 94 % |
| Furnace Ring | 7 052 | 97 → 100 % | 97 → 100 % | 97 → 98 % | 97 → 100 % | 97 → 98 % | 49 → 49 % | 49 → 49 % |
| Tallow Works | 3 764 | 93 → 100 % | 93 → 100 % | 93 → 99 % | 93 → 100 % | 93 → 99 % | 96 → 98 % | 96 → 98 % |
| Wickmarket | 5 583 | 91 → 95 % | 91 → 100 % | 91 → 93 % | 90 → 94 % | 90 → 93 % | 87 → 93 % | 87 → 93 % |
| The Vestry | 2 353 | 70 → 100 % | 70 → 100 % | 70 → 99 % | 70 → 100 % | 70 → 100 % | 82 → 82 % | 82 → 82 % |

What is still out of reach is out of reach for good reasons: the narrow classes miss only
floors beyond their own limits (the Censer's 2 m climbs, Wickmarket's awnings), the heavies
everything only a leap reaches (half of the Furnace Ring's tiers for the wide footprint from the
spawns) — that is what re-forming is for.

**Per-tick AI cost** (`shared/tests/enemies_perf.rs`): 4 players running and jumping around the
Furnace Cathedral (arena 5) against **67 aware enemies of all eight fighting types** (every
arena-5 spawn, a copy of each gnawer, a snuffer beside every other one, a bellwether behind each
censer, a heron and a smelter beside each gaoler), whole `SimWorld::step` timed over 780 ticks:
dev build (this branch, parallel builds loading the machine) **avg 0.21 ms, p99 0.37 ms, max
1.2 ms**. Before the third roster, with 55 enemies of five types: dev build **avg 0.16 ms, p99 0.29 ms, max 0.62 ms**, release avg 0.14 ms, p99 0.23
ms, max 0.41 ms — far under the 2 ms budget (measured while parallel builds loaded the machine;
at load ~30 on 16 cores the dev build measured avg 0.22–0.25 ms, p99 0.40–0.55 ms, with single
preempted ticks up to ~4 ms). ~0.7 A* searches per tick (≈155 nodes each, 14 % partial, 12 %
deferred by the budget), ~2.8 straight-walk plans per tick.
No enemy fell into the chasm or the lava pools (pouncing Gnawers did, before the leap checked
its whole airborne arc).

With leaps, prowling, vantage points, detours and melting (release, the same machine and
load as the build times above): the campaign crowd **avg 0.227 → 0.190 ms, p99 0.409 → 0.313
ms, max 0.575 → 0.424 ms** (noise dominates: no jumps fall in that arena; 63 vantage searches,
5 detours in 15 s), and a new scenario, **the Great Wick finale** (`the_great_wick_finale_
with_players_on_the_floes`: all three waves of the Chandlery's encounter 4 plus a copy of every
Mould, 54 enemies, two players out on wax floes in the flooded well, one on a gallery, one
running the floor ring): **avg 0.226 ms, p99 0.383 ms, max 0.518 ms** with 106 leaps (onto and
between the floes), 37 Harrier detours and 5 heavies that chose not to melt (their targets on
floes they can't stand on) in 15 s; nobody drowned (a Gnawer did, mid-leap, before leapers were
exempt from the push-apart and waited for a free landing). All far under the 2 ms budget.

## Arena encounters (`shared::sim::encounter`)

Campaign levels only (`LevelKind::Arena` levels have none; `/spawn` works everywhere).
`SimWorld::step` runs them after the world rules.

1. **Idle** until a living player's feet (`counts_for_encounters`) enter `trigger`. **Start**:
   `lock_doors` close; every living player outside `arena` is teleported to `rally` and gets a
   `Respawned` event. `SimEvent::EncounterStarted { index, waves }`.
2. **Waves**: wave 1 `delay` s after the start; wave *n+1* `delay` s after every enemy of wave
   *n* is dead. On spawn: `SimEvent::WaveStarted { index, wave, waves, enemies }`, an
   `EnemySpawned` per enemy. Wave enemies are `aware` and tagged `Enemy::encounter`.
3. **Cleared** when the last wave is dead: `unlock_doors` and the entry `lock_doors` open,
   `SimEvent::EncounterCleared { index }`.

**Co-op scaling** (campaign): with *p* = `coop_player_count()` at wave spawn, a wave of *n*
authored spawns has `round(n × (1 + 0.5 (p − 1)))` enemies, each with health ×
`(1 + 0.5 (p − 1))`. Extra copies are placed on rings around the authored spawns (walkable
non-hazard node for the type's class, capsule free, line of sight to the authored point; flyers:
free air with ≥ 1 m below). Harrier spawn points are authored at the hover centre and converted
to feet.

**Replication**: one static entity per encounter, `kind::ENCOUNTER` (2): `position` = arena
centre, `flags` = `encounter_flags::{ACTIVE, CLEARED, BETWEEN_WAVES}`, `state` = current wave,
`health` = enemies left in the wave, `aux` = waves | wave size << 16.

**Client** (`ingame/encounter_hud.rs`): "ARENA LOCKED", "WAVE n/N", "CLEARED" banners and the
"WAVE 2/3 - 5 LEFT" readout; `Fx::EnemySpawned` plays a flash + smoke + `EnemySpawn`.

### Hooks for the game modes / lobby

| hook | use |
|---|---|
| `enemy::is_targetable(&SimPlayer)` | players enemies may target / hit |
| `encounter::counts_for_encounters(&SimPlayer)` | players who trigger encounters and get rallied |
| `SimWorld::coop_player_count()` | co-op scaling player count |
| `SimWorld::reset_encounter(i)` / `reset_uncleared()` / `start_encounter(i)` | checkpoints, scripted events, dev |
| `SimWorld::remove_enemy(id)` | remove without a death |
| `SimWorld::frozen` | results screen: AI and encounters stop |

## Client presentation (`client::ingame::combat::{entities, enemy_fx, fx}`)

The client reads the same enemy definitions as the server (built-ins), so names, attack names,
sizes and the laser muzzle come from the data.

* **Models**: `ModelLibrary::draw_enemy` with an `EnemyAnim` from the replicated state: AI
  state + progress, attack index (which telegraph / strike to pose), speed / strafe, aim pitch
  (the laser's pitch; a diving Harrier's flight pitch), `aim_locked`; drawn `model_scale`
  bigger than built ("Sizes and hit shapes"). Weak points are bright `Core` parts; telegraphs
  `Glow` parts (`docs/models.md`).
* **Hits** crack the wax: pale wax droplets and molten orange droplets fly off, sparks, a white
  hurt flash, `EnemyHurt` (wax snap + sizzle). **Kills** release the heat as one big ember
  burst (a flash and a short point light, a fountain of embers, lumps of wax, smoke,
  `EnemyDeath` / `GaolerDeath` + `EmberRelease`); the corpse slumps, splits and melts (death
  poses) while it smoulders (smoke wisps and rising embers until it is removed).
* **Telegraph extras**: Censer volley — embers flung off the whirling censer; Harrier dive — a
  hot streak behind it; Gaoler bash — dust and a shock ring in front of the door on the strike,
  rush — dust thrown up, sweep — a burning 300° arc of sparks at knee height and floor dust,
  footsteps with dust; Heron — the laser (above), a lens glare, a dot where it lands.
* **Projectiles**: ember bombs (a ~0.35 m slag ball with glowing seams) trail smoke, falling
  embers and a hot glow, light the scene and burst into embers (`Fx::Explosion` with an enemy
  attack plays `ember_bomb_burst` + `EmberBurst` instead of a fireball); needles draw a long
  glowing streak (up to 7 m, never back past where they were first seen) and shed sparks.
* **Sounds** (`docs/audio.md`): an alert bark per type; a wind-up cue per attack; strike / lob /
  needle sounds; `EmberFlight` when a bomb appears; `HeronLock` when the laser locks;
  `HarrierSwat` when a Harrier is knocked out of the air.

## Protocol

`PROTOCOL_VERSION` 6 (the whole roster change): enemy kinds 20–24 (16–19 retired), enemy
projectiles `EMBER_BOMB` 80 and `NEEDLE` 81 (70–71 retired), `enemy_flags::{AIMING 1 << 6,
AIM_LOCKED 1 << 7}` (bits 6–14 are the enemies area's; bit 15 is `BRANDED`), exact yaw + pitch
on aiming enemies.

The expansion (protocol 8, `docs/expansion.md`): enemy kinds `BELLWETHER` 25, `SNUFFER` 26,
`SMELTER` 27, `EFFIGY` 28 (29–31 free); `enemy_flags::{FRENZIED 1 << 8, ELITE 1 << 9, LATCHED
1 << 10}` (bits 11–12 free; 13–14 are the arsenal's `BURNING` / `PINNED`); `AiState::Latched` =
8 (`owner` = the victim); hazard `SLAG_LINE` 140 (`position` = start, `velocity` = end − start,
`state` = width in cm, `aux` = ignition tick, `flags` = `slag_flags::{LIVE, COOLING}`; 141–143
free); enemy projectile kinds 82–95 are unused (no new enemy projectiles).

Getting there (**`PROTOCOL_VERSION` 10**): three new `AiState` values in the enemy entity's
`state`, nothing else on the wire (no new flags, no new fields, the player's own state
untouched):

| state | value | `aux` progress (bits 0–7) | flags | what the client draws |
|---|---|---|---|---|
| `Leap` | 9 | through the crouch (`LeapDef::crouch`), then through the planned flight | `AIRBORNE` clear while crouching, set in the air (as for any airborne enemy) | the crouch, then the flight tilted along `velocity` |
| `Melt` | 10 | through the slump (`MeltDef::sink`) | — | sinking into its puddle (`melt_depth`) |
| `Reform` | 11 | through the rise (`MeltDef::rise`) | — | rising out of a new puddle |

The move from the melt to the re-form site happens between a `Melt` and a `Reform` snapshot;
the puddle is drawn at zero size at both ends of it, so whether the client snaps it (moved
more than 3 m a tick: `client::net::interpolation::TELEPORT_SPEED`) or interpolates a short
move, nothing visible slides. Older clients refuse to connect (protocol 9 ≠ 10).

## Dev commands

`/spawn gnawer 4`, `/spawn censer 2`, `/spawn harrier 3`, `/spawn gaoler`, `/spawn heron`,
`/spawn bellwether`, `/spawn snuffer 2`, `/spawn smelter`, `/spawn effigy` (8 m in front,
facing you; add `elite` for elites: `/spawn gaoler 1 elite`), `/enemies`, `/killall`, `/needle [dist]` (alias `/orb`: a needle
slowed to 22 m/s flies at you, parry practice), `/ember [dist]` (an ember bomb is lobbed onto
you), `/encounter <n>` starts arena encounter *n*, `/encounters` lists their phases, `/melt`
(every ground Mould melts now and re-forms near its target), `/god`, `/notarget` as before.

## Tests

```text
cargo test -p shared --test enemies_ai -- --nocapture    # behaviour per type in test arenas
cargo test -p shared --test enemies_expansion -- --nocapture   # toll, latch, pour, Effigy, elites, scaling
cargo test -p shared --test enemies_nav -- --nocapture   # grid on every level, classes, connectivity, lava, leaps, reachability
cargo test -p shared --test enemies_perf -- --nocapture  # 67 enemies on the campaign, the Great Wick finale: per-tick cost
cargo test -p shared --test combat_enemy --test combat_parry --test combat_replication
cargo test -p server --test encounter_crucible           # encounters end to end (real server + clients)
cargo run -p client --release --example enemy_shots -- out/   # screenshots (--third: only the third roster, --spectate: only spectating, --getting-there: leaps and melting)
cargo run -p client --release --example models_view -- --only gaoler   # model pose sheets
```

* **AI** (`enemies_ai.rs`): every attack of every type (bite, pounce, swing, volley, dive,
  bash, sweep, rush, kick, needle) is used, lands, and is telegraphed — every strike comes
  exactly one wind-up of the same attack after its wind-up event, projectiles only during
  strikes, no damage of an attack before its first strike; the pounce leaves the ground
  (13 m/s + 6 m/s up) and rakes a strafing player once; Censers spend > 90 % of the time in
  13–31 m from 6 m and from 45 m, side-step and volley; the straight ember bomb lands < 1.2 m from
  a standing player at 15 / 22 / 29 m (0.7 m measured), the side ones ±8° to either side, a
  walking player is caught inside the blast (2.3 m); a punched ember bomb flies straight with
  doubled damage, a bomb 1.5 m away does 12.5 (falloff); Harriers hover 6.1–8.2 m up around
  pillars / under a roof without touching the level and circle (61 rad in 25 s); a dive is a
  straight 3D line (0.000 m off), ends at the floor (34 ticks < 36), hits once and climbs back
  > 1.5 m; punching a hovering Harrier does nothing, swatting a diving one is a parry (heal,
  exactly 50 counter damage, stunned, falls to the floor, the dive never lands, takes off
  again); Gaoler bash parried in the window → stunned 150 ticks + exactly 150 through the
  armour and no bash, too early / too late → no parry and the bash (40, knocked back and up)
  lands only after the full 1 s; front armour 15 %, sides / back 100 %, helm ×1.5, lantern
  ×2.5, explosions armoured; the sweep hits a grounded player (25) but a player who jumped
  0.3 s before it (feet 2.4 m up) takes nothing; the rush hits in the open and a knee-high wall
  ends it early (37 ticks) with no hit; Herons spend > 85 % of the time in 24–46 m from 10 m
  and 60 m and move > 2 m between shots; the laser is on for the whole wind-up, locks exactly 15
  ticks before the needle, doesn't move once locked, tracked a walking player before, and the
  needle (110 m/s) flies along the replicated laser; a 2.5 m sidestep at the lock makes the
  needle miss, standing still it hits; a parried needle kills its Heron (RETURN TO SENDER);
  every type rests on the floor wherever it spawns (the depenetration regression); Gnawers are
  knocked back > 20 m/s, the Gaoler < 4; gnawers / gaoler cross a lava trench by its bridge
  without anyone touching the lava, a gnawer put in the lava dies; a 3.4 m walker routes round a
  3 m tunnel that a Gnawer takes; a Gnawer routes around a wall (A*); 10 Gnawers spawned on one
  point separate and stay ≥ 0.75 × (r1 + r2) apart while swarming (0.99 measured); dead /
  untargetable players are dropped at once; damage alerts an idle enemy; all types are
  deterministic; a frozen world stops all attacks (Harriers hover in place).
* **The third roster** (`enemies_expansion.rs`, plus the generic scenarios above: clapper,
  latch, heave and pour are telegraphed and land like every other attack, every type rests on
  the floor, all types are deterministic, a frozen world stops them): the toll winds up for
  exactly 96 ticks, frenzies the ally in reach and not the one out of reach, not itself and not
  another Bellwether, and the frenzy lasts 300 ticks; a lone Bellwether never tolls and keeps
  its distance; 15 damage on the bell (×2) mid-wind-up cancels the toll (stunned, nobody
  frenzied), two 10s add up, body hits and 20 on the bell don't; a Snuffer latches on (LATCHED,
  `owner` = victim, on the chest), smothers 16/s all of it hard damage, follows its victim,
  slows a run to 72 % of a free one through the predicted movement slow (gone right after a
  dash shakes it off), is thrown off by a punch (stunned, flung along the punch, ≥ 30 damage,
  no more smothering), shaken off by a dash, pinned off by a Pinning Arrow, lets go of the dead
  and after 6 s,
  and a second one never latches the same player but nips; a pour lays its line with the
  wind-up (66 ticks before ignition, aimed through the target and past it, replicated as a
  telegraph with its width), nothing burns before ignition, then 22/s to the player on it and
  nothing to one beside it or in the air over it, gone after 5 s; a wall cuts the line short; a
  Smelter killed mid-wind-up leaves no line; the Effigy never targets, attacks or moves,
  reports hits in full, flinches, can't die (stops at 1) and heals to full 3 s after the last
  hit, can't be an elite or frenzied, isn't in `wave_names`; elites: `spawn_enemy_with` health
  1.5 × 2.5, `ELITE` replicated, `make_elite`, bites every 74 ticks instead of 85 with the same
  24-tick wind-up, twice the heat; `enemy_scale` damage ×2 (bite 28), aggression ×2 (85 → 58
  ticks between bites, same wind-up), projectile speed ×1.5 (needle 165 m/s) and lobs at ×1.4
  (18.5 → 25.9 m/s, still landing on the target); frenzy × difficulty (bite 35 = 14 × 2 × 1.25).
* **Nav** (`enemies_nav.rs` + unit tests): builds on every shipped level with sane counts,
  every class has room, player spawns on the grid; on the campaign every gnawer / gaoler spawn
  has a path for its own class to its arena centre and both ways to the rally point, censer
  spawns are on the grid; every path node is outside kill volumes and the route never crosses
  lava; stacked floors and bridges over lava exist and are safe; no walking link climbs more
  than a ramp; on every shipped level no jump lands on (or takes off from) a deadly node or in a
  kill volume, and no leap's arc passes through one; the reachability per class (above). Unit
  tests: classes vs headroom / footprint, walls/gaps, a ramp to a platform and the one-way drop;
  the leap arc passes both ends and its apex and `leap_velocity` flies exactly it at any
  gravity; leaps across a 6 m water strip within the class limits (8 m yes, 4.5 m no, walking
  no; nothing on the arc or the path in the water); walking round a bridge beats a leap that
  saves little, far from the bridge the leap wins; no leap under a low block, and under a 4 m
  overhang the narrow class leaps where the tall one can't; a wide class never leaps; no leap
  onto an island with lava right past it; a 3 m ledge climbed with a run-up (3.5 m climber yes,
  2 m no), nobody onto a 7.5 m one but a leaper comes down it; pad rides land where the pad
  throws (on the platform, the wide capsule too) and only riders use them; every successor
  link has its reverse among the predecessors.
* **Getting there** (`enemies_ai.rs`): a Gnawer crosses the Drowned Chandlery's flooded break
  (C2 Spool Walk) in 1.95 s with a leap and bites; a Gnawer and a Snuffer leap onto the player's
  floe in the Wax Press (1.9 / 2.8 s) while a Gaoler waits at the water's edge (< 9 m) without
  melting; a leaping Gnawer punched back over the water mid-air falls in and is quenched (no
  guiding a knocked leap); a Gaoler rides a pad up onto a 6 m ledge; a Gaoler stranded on a 7 m block prowls
  the edge (16.5 m paced), melts after 6.6 s (0.6 s alert + 6 s), the slump takes 36 ticks, it
  rises on the floor 8.5 m from the player (in its 8–16 m band) and attacks; a Gnawer flung by
  a blast onto a 10 m block re-forms on the floor 6.4 s after landing and reaches the player;
  six Moulds stranded on pillars all re-form, none in the lava, the water (or within 1 m of
  either), the closed room or on another; a Censer under a 5 m plateau backs off to a vantage
  point and lobs (never melting); a Harrier behind a 20 m pillar is never blind for more than
  55 ticks (626 without detours). Every leap in them: the Mould never in a kill volume.
  `client::models` `getting_there_states_squash_sink_and_rise`: every Mould crouches, stretches,
  sinks below the floor at full melt and stands again, the crouch without telegraph glow, the
  puddle zero at both ends.
* **Combat** (`combat_enemy.rs`, `combat_parry.rs`, `combat_replication.rs`): the Gnawer chases
  and bites (14, `GNAWER [BITE]`, 24-tick telegraph) and replicates its telegraph; line of
  sight; targeting and aggro; co-op scaling; punches knock Gnawers back; enemies don't hurt each
  other; lag-compensated hit on a running Gnawer; projectile parries with a slowed needle at 0
  and 200 ms of view lag; the Harrier swat at 1 and 12 ticks of view lag; ember bombs and
  needles replicate by trajectory (< 2 cm); the aim laser survives the wire.
* **Encounters** (`server/tests/encounter_crucible.rs`, campaign maps area): crucible arena 1
  with gnawers end to end.

Screenshots (`enemy_shots`, `models_view`) were checked by eye: every pose sheet and close-up,
the Gnawer pack, its open glowing jaw and pounce, the Censers whirling and the ember bombs in
flight and bursting, the Harriers hovering / rearing (heart white-hot) / diving and three of
them swatted by one punch, the Gaoler's door rim strobing in its parry window, the bash, the
red chain sweep, the rush behind the door, the Heron's red laser at you, white-hot once locked,
and passing beside you after a dash, a mixed crowd, its ember bursts and melting corpses, and
arena 4 of the campaign ("ARENA LOCKED", "WAVE 1/2 - 5 LEFT", the Gaoler on the island). The
refit: the lineup at its new sizes with the hit shapes over it from the front and behind (every
weak point on its part: jaws, braziers, the heart, helm and lantern, the lens, the bell, the
wick, hatch and head, the Effigy's head and sigil; the padded shapes past every silhouette),
the latched Snuffer at its old size in the victim's view, and spectating (`docs/style_modes.md`).
Getting there (`--getting-there`): two Gnawers in the air arms up over the edge of the 3 m
block, then landed on it; on the Drowned Chandlery a Gnawer crouched at the water's edge (the
hazard kerb in front of it), in the air over the water towards the floe, landed on it; the
lineup of seven ground Moulds as `/melt` takes them (one already flat), the Smelter and the
Gaoler sunk to their waists in dark wax puddles with orange hearts, the puddles alone, a Censer's
lantern coming up out of one, and all of them standing again inside splash rings (the Smelter
already pouring at the player).

## Known limitations / next steps

* The Bellwether tolls without line of sight (it only needs a target and an ally in reach), on
  purpose: it is the one you have to hunt down. Its bell hitbox doesn't follow the swing (the
  weak point is fixed on its yoke; the model keeps the bell near it).
* Slag lines are straight (they follow the floor's height, not corners) and stop at the first
  step they can't flow over; they don't pour down ledges.
* A Snuffer latched onto a player rides through walls with them (no collision while latched;
  it depenetrates when it lets go).

* Leaps are linked from edge nodes along 16 ray directions only: a landing reachable only at
  an odd angle from a long edge may be missed (re-forming covers it). The tall clearance
  capsule is r 0.48 × 3.5 m; the r 0.7 Snuffer / Bellwether rely on the guided flight for the
  last 0.2 m.
* Moulds don't ride a pad by walking over it (only on purpose, on a route through its link),
  unlike players; and they don't wade (above).
* Re-forming prefers a spot out of the target's sight; in a wide open arena there is none in
  the band and they rise in plain view. A target nothing can reach (a pillar top) makes melee
  Moulds prowl below indefinitely (ranged ones still shoot).
* The leap links cost 10–60 ms of nav build per level (once per level; above).
* The grid is 1 m and the narrow clearance capsule is r 0.5: the r 0.6 Gnawer / Censer rely on
  collide-and-slide in 1 m gaps; the wide class is an erosion of the grid, not a real r 1.3
  capsule test.
* Players and enemies don't block each other (player movement must stay predictable).
* Harriers have no pathfinding: with no line of sight they look for a detour point (straight
  up, or 7 m to the side) and otherwise fly straight at the target relying on the feeler sweeps;
  a maze of walls can still hold one.
* The aim laser is clipped by remote players' approximate capsules and by the camera, not by
  enemies standing in the way (the needle isn't either: enemy projectiles never hit enemies).
* Enemy projectiles already in flight when the world freezes still land.
