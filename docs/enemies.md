# Enemies, navigation and arena encounters

The Moulds (`docs/roster.md`): five enemy types — Gnawer, Censer, Harrier, Gaoler, Heron —
their server-side AI, the nav grid ground enemies walk on, arena encounters (locked rooms,
waves) and co-op scaling. Rules and default numbers come from `docs/gameplay.md` and
`docs/roster.md` (the design contract); the damage pipeline, parry hook, projectiles and
replication basics are in `docs/combat.md`.

## Where things live

| module | contents |
|---|---|
| `assets/enemies/{gnawer,censer,harrier,gaoler,heron}.ron` | the definitions (also compiled in as built-ins, `EnemyRegistry::builtin`) |
| `shared::sim::enemy` (`mod.rs`) | `Enemy` (runtime state, incl. `aim` / `aim_locked`), damage side of the pipeline (weak points, armour, knockback + stagger, damage alerts), `spawn_enemy[_scaled]`, replication |
| `shared::sim::enemy::def` | `EnemyDef` format: `AttackKind::{Melee, Projectile, Slam, Dive}`, lunges / pounces, lobs, `AimLaser`, `jumpable`, `parry_strike`, `parry_reach`, `ai.reposition` |
| `shared::sim::enemy::ai` | the state machine, targeting (`is_targetable` hook), attacks (leap, dive, lob, laser), `move_enemy` (capsule collide-and-slide), push-apart, kill volumes, `frozen` handling, `SimWorld::build_nav`, `lob_velocity` |
| `shared::sim::enemy::steer` | ground path following (`NavAgent`), edge guard, flight steering, `AiWorld` (nav grid + A* buffers + stats on `SimWorld::ai`) |
| `shared::sim::nav` | `NavGrid`: build, agent classes (`NavClass` = footprint + headroom), `node_at`, `nearest_node`, `find_path` (A*), `walkable_line`, `smooth`, `ground_ok` |
| `shared::sim::encounter` | encounter states (`SimWorld::encounters`), triggers, waves, co-op scaling, doors, rally teleport, reset hooks, `kind::ENCOUNTER` replication |
| `server::combat` | routes `EncounterStarted` / `WaveStarted` / `EncounterCleared` (reliable, everyone) and `Fx::EnemySpawned` |
| `server::commands` | dev `/spawn`, `/encounter <n>`, `/encounters`, `/needle`, `/ember`, `/killall`... |
| `client::models::enemies` | the five rigs and their pure pose functions (`docs/models.md`) |
| `client::ingame::combat::enemy_fx` | names / sizes / sounds per kind, the Heron's aim laser, wax-and-ember effects |
| `client::ingame::encounter_hud` | banners + sounds for the encounter events, "WAVE 2/3 - 5 LEFT" readout |
| `client/examples/enemy_shots.rs` | headless screenshots of every attack of every type + an arena encounter |

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

## AI (`ai.rs`, server only)

```text
 Idle ──(sees a targetable player in sight range with LOS, or gets hurt)──▶ Alert (reaction_time, EnemyAlerted fx)
   ▲                                                                            │
   └──(no target)── Chase ◀──────────────────────────────────────────────────────┘
                    │  ▲
   first usable     │  └── Recover (recover s) ◀── Strike (melee / leap / dive / lobs / needle) ◀── Windup (telegraph)
   attack ──────────┘          ▲ a wall / the level stops a leap or dive early ┘   parried: Stunned ──▶ Chase
 any state ──(killed)──▶ Dying (corpse, no hitboxes, death_time) ──▶ removed
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
* **Attack choice**: definition order = priority; needs the cooldown ready, line of sight, the
  range band (melee: between body surfaces and vertically overlapping, a `jumpable` sweep not
  against someone standing well above it; projectile: muzzle to chest; dive: centre to chest,
  clear line), `min_target_speed`, and for leaps safe ground along the leap.
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
* **Chase movement**: melee enemies walk the nav grid to the target and stop at 60 % of their
  shortest non-leap melee reach. Ranged enemies (Censer, Heron) walk until they see the target
  within their band's max, then hold it: back off when closer than the min (if there is room),
  strafe sideways, flip direction when blocked, when the strafe would break line of sight, every
  ~2.5 s, and (`ai.reposition`, the Heron) after every ranged strike. Harriers circle (below).
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
  a feature); so is a swatted Harrier falling.
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
* **Links** (8 directions): step (≤ 0.45 m), continuous ramp (≤ 1.2 m per cell, midpoint on the
  slope) or one-way **drop** (≤ 5.5 m), nothing in the way at knee / chest height. Diagonals
  only where both orthogonal routes agree. **Jump pads and gaps have no links.**
* **Doors**: nodes near a door brush are impassable while it is closed.
* **Queries**: `node_at`, `nearest_node`, `walkable_line`, `find_path` (A*, octile-ish
  heuristic ×1.1, costs distance + 0.5 × |Δy| + hazard, generation-stamped buffers, `max_expand`
  budget → `Partial` path), `smooth` (string pulling), `ground_ok` (edge guard).

**Path following** (`NavAgent`, `steer::ground_velocity`): straight walk when `walkable_line`
allows it; otherwise A* (at most **4 searches per tick** for all enemies, ≤ 3000 nodes each),
string-pulled waypoints, re-planned every `repath_interval` (staggered), when the goal moved
> 1.5 m + 20 %, or when stuck (< 0.5 m in 0.5 s → re-plan + side-step). A partial path ends at
the closest reachable point and the enemy waits there (ranged enemies still shoot).

### Measurements (release build)

| level | columns | floors | walkable / large / hazard / deadly | links | build | passable per class (gnawer / censer / heron / gaoler) |
|---|---|---|---|---|---|---|
| `crucible_of_veins` | 168 × 406 | 35 397 | 31 268 / 25 672 / 620 / 1 298 | 232 719 | **31 ms** | 31 268 / 31 247 / 31 183 / 25 636 |
| `cistern_nine` | 74 × 74 | 10 900 | 10 074 / 7 896 / 64 / 96 | 74 270 | 11 ms | 10 074 / 10 074 / 10 074 / 7 896 |
| `movement_test` | 202 × 262 | 53 824 | 50 384 / 45 028 / 0 / 480 | 382 308 | 25 ms | 50 384 / 50 384 / 50 384 / 45 008 |

On the campaign 13 950 walkable nodes are reachable from the spawns / checkpoints / enemy
spawns, 1 687 columns have stacked walkable floors and 320 walkable floors lie over lava. A*
start → exit: 0.15 ms; an arena spawn → its rally point: 2 µs.

**Per-tick AI cost** (`shared/tests/enemies_perf.rs`): 4 players running and jumping around the
Furnace Cathedral (arena 5) against **55 aware enemies of all five types** (every arena-5
spawn, a copy of each gnawer and a heron beside each gaoler), whole `SimWorld::step` timed over
780 ticks: dev build **avg 0.16 ms, p99 0.29 ms, max 0.62 ms**, release avg 0.14 ms, p99 0.23
ms, max 0.41 ms — far under the 2 ms budget (measured while parallel builds loaded the machine;
at load ~30 on 16 cores the dev build measured avg 0.22–0.25 ms, p99 0.40–0.55 ms, with single
preempted ticks up to ~4 ms). ~0.7 A* searches per tick (≈155 nodes each, 14 % partial, 12 %
deferred by the budget), ~2.8 straight-walk plans per tick.
No enemy fell into the chasm or the lava pools (pouncing Gnawers did, before the leap checked
its whole airborne arc).

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
  (the laser's pitch; a diving Harrier's flight pitch), `aim_locked`. Weak points are bright
  `Core` parts; telegraphs `Glow` parts (`docs/models.md`).
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

## Dev commands

`/spawn gnawer 4`, `/spawn censer 2`, `/spawn harrier 3`, `/spawn gaoler`, `/spawn heron`
(8 m in front, facing you), `/enemies`, `/killall`, `/needle [dist]` (alias `/orb`: a needle
slowed to 22 m/s flies at you, parry practice), `/ember [dist]` (an ember bomb is lobbed onto
you), `/encounter <n>` starts arena encounter *n*, `/encounters` lists their phases, `/god`,
`/notarget` as before.

## Tests

```text
cargo test -p shared --test enemies_ai -- --nocapture    # behaviour per type in test arenas
cargo test -p shared --test enemies_nav -- --nocapture   # grid on every level, classes, connectivity, lava
cargo test -p shared --test enemies_perf -- --nocapture  # 55 enemies on the campaign, per-tick cost
cargo test -p shared --test combat_enemy --test combat_parry --test combat_replication
cargo test -p server --test encounter_crucible           # encounters end to end (real server + clients)
cargo run -p client --release --example enemy_shots -- out/   # screenshots
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
* **Nav** (`enemies_nav.rs` + unit tests): builds on every shipped level with sane counts,
  every class has room, player spawns on the grid; on the campaign every gnawer / gaoler spawn
  has a path for its own class to its arena centre and both ways to the rally point, censer
  spawns are on the grid; every path node is outside kill volumes and the route never crosses
  lava; stacked floors and bridges over lava exist and are safe; no link climbs more than a
  ramp; unit tests for classes vs headroom / footprint, walls/gaps, a ramp to a platform and the
  one-way drop.
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
arena 4 of the campaign ("ARENA LOCKED", "WAVE 1/2 - 5 LEFT", the Gaoler on the island).

## Known limitations / next steps

* **No nav links for jump pads / gaps**: walkers never use pads; pad-only ledges only hold the
  ranged enemies that spawn there.
* Drops are one-way and up to 5.5 m; enemies knocked onto a ledge they can't leave stay there.
* The grid is 1 m and the narrow clearance capsule is r 0.5: the r 0.6 Gnawer / Censer rely on
  collide-and-slide in 1 m gaps; the wide class is an erosion of the grid, not a real r 1.3
  capsule test.
* Players and enemies don't block each other (player movement must stay predictable).
* Harriers have no pathfinding: one with no line of sight flies straight at its target and
  relies on the feeler sweeps. A pillar exactly on its circle makes it turn back and forth.
* The aim laser is clipped by remote players' approximate capsules and by the camera, not by
  enemies standing in the way (the needle isn't either: enemy projectiles never hit enemies).
* Enemy projectiles already in flight when the world freezes still land.
