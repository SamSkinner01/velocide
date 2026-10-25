# Enemies, navigation and arena encounters

Milestone 10: the four enemy types (Husk, Spitter, Gnat, Bastion), their server-side AI, the
nav grid ground enemies walk on, arena encounters (locked rooms, waves) and co-op scaling.
Rules and default numbers come from `docs/gameplay.md`; the damage pipeline, parry hook,
projectiles and replication basics are in `docs/combat.md`.

## Where things live

| module | contents |
|---|---|
| `assets/enemies/{husk,spitter,gnat,bastion}.ron` | the definitions (also compiled in as built-ins, `EnemyRegistry::builtin`) |
| `shared::sim::enemy` (`mod.rs`) | `Enemy` (runtime state), damage side of the pipeline (weak points, armour, knockback + stagger, damage alerts), `spawn_enemy[_scaled]`, replication |
| `shared::sim::enemy::def` | `EnemyDef` format (+ `AttackDef::lunge`, `min_target_speed`, `AiDef::repath_interval`) |
| `shared::sim::enemy::ai` | the state machine, targeting (`is_targetable` hook), attacks, `move_enemy` (capsule collide-and-slide), push-apart, kill volumes, `frozen` handling, `SimWorld::build_nav` |
| `shared::sim::enemy::steer` | ground path following (`NavAgent`), edge guard, flight steering, `AiWorld` (nav grid + A* buffers + stats on `SimWorld::ai`) |
| `shared::sim::nav` | `NavGrid`: build, `node_at`, `nearest_node`, `find_path` (A*), `walkable_line`, `smooth`, `ground_ok` |
| `shared::sim::encounter` | encounter states (`SimWorld::encounters`), triggers, waves, co-op scaling, doors, rally teleport, reset hooks, `kind::ENCOUNTER` replication |
| `server::combat` | routes `EncounterStarted` / `WaveStarted` / `EncounterCleared` (reliable, everyone) and `Fx::EnemySpawned` |
| `server::commands` | dev `/encounter <n>`, `/encounters` (plus the existing `/spawn`, `/killall`...) |
| `client::ingame::encounter_hud` | banners + sounds for the encounter events, "WAVE 2/3 - 5 LEFT" readout |
| `client/examples/enemy_shots.rs` | headless screenshots of every type in action + an arena encounter |

## The four enemies

Position convention: **every enemy's `position` is its feet** (bottom of the collision
capsule). The Gnat's hover centre (its model origin) is half its height (0.45 m) above it;
the client draws it there. Local offsets in the RON files are from the feet (+X right, +Y up,
−Z forward).

| | Husk (16) | Spitter (17) | Gnat (18) | Bastion (19) |
|---|---|---|---|---|
| role | fodder melee | ranged | flying | heavy armoured |
| health | 40 | 70 | 30 | 450 |
| speed / accel | 9 / 60 | 6 / 40 | 14 / 30 | 5 / 25 |
| capsule r × h | 0.45 × 1.9 | 0.5 × 1.7 | 0.45 × 0.9 | 1.1 × 3.1 |
| nav class | small | small | — (flies) | large |
| weak points | head ×2 | head ×2 | core ×2.5 (under the abdomen) | head ×2, furnace (back) ×2 |
| armour | — | — | — | front ×0.35 in a 140° arc, explosions too; head + furnace bypass; parry counters bypass |
| knockback scale | 1.3 | 1.1 | 1.6 | 0.12 |
| keeps | melee reach | 12–25 m, strafes | orbit 6–13 m, 4–8 m up | melee reach |

| attack | wind-up (telegraph) | strike | recover / cooldown | effect |
|---|---|---|---|---|
| Husk **swipe** | 0.45 s, arms up + red claws | 0.12 s | 0.45 / 0.5 s | 15 dmg, 2.2 m reach, 120° arc, kb 7 |
| Husk **lunge** (target faster than 5 m/s, 2.8–6.5 m away) | 0.5 s | 0.4 s leap at 14 m/s, rakes everyone it reaches (1.4 m) once | 0.6 / 2.5 s | 15 dmg, kb 9 |
| Spitter **spit** | 0.7 s, sac swells/glows | 0.15 s | 0.45 / 2.5 s | SpitterOrb 22 m/s, 20 dmg, leads the target 35 %, parryable |
| Gnat **bolts** | 0.4 s, abdomen curls, emitters glow | burst of 3, 0.12 s apart | 0.35 / 1.6 s | GnatBolt 45 m/s, 10 dmg each, 2.5° spread, 20 % lead, parryable |
| Bastion **slam** | 1.0 s, fists overhead; **last 0.35 s PARRYABLE** (yellow) | 0.3 s | 1.0 / 1.2 s | 45 dmg → 50 % at the edge of a 5 m shockwave (centre 1.6 m in front), kb 16 mostly up; parried → stunned 2.5 s + 150 damage |

Every attack telegraphs: the `Windup` state (replicated `state` + progress 0–255 in `aux` bits
0–7, attack index in bits 8–15), `Fx::EnemyAttack { strike: false, parryable }` at its start
(sound + glint), the `PARRYABLE` flag during a parry window, then `Fx::EnemyAttack { strike:
true }` / `EnemyFired` / `EnemyShockwave` when it lands. Damage is never dealt before the wind-up
completed (tested for every type).

## AI (`ai.rs`, server only)

```text
 Idle ──(sees a targetable player in sight range with LOS, or gets hurt)──▶ Alert (reaction_time, EnemyAlerted fx)
   ▲                                                                            │
   └──(no target)── Chase ◀──────────────────────────────────────────────────────┘
                    │  ▲
   first usable     │  └── Recover (recover s) ◀── Strike (melee / lunge / slam / burst) ◀── Windup (telegraph)
   attack ──────────┘                                                      parried: Stunned (2.5 s) ──▶ Chase
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
  **Hook for the modes**: add downed / spectating there; enemies then never target or hit them
  (strikes only consider targetable players).
* **Attack choice**: definition order = priority; needs the cooldown ready, line of sight, the
  range band (melee: between body surfaces and vertically overlapping; projectile: muzzle to
  chest), `min_target_speed`, and for lunges safe ground along the leap.
* **Chase movement**: Husk/Bastion walk the nav grid to the target and stop at 60 % of their
  swipe reach. Spitters walk until they see the target and are within 25 m, then hold 12–25 m:
  back off when closer than 12 (if there is room), strafe sideways, flip direction when blocked,
  when the strafe would break line of sight, or every ~2.5 s. Gnats circle (below).
* **Movement** (`move_enemy`): accelerate towards the desired velocity (ground: horizontal +
  gravity), capsule collide-and-slide against `MASK_MOVEMENT` with step-up (0.45 m), walkable
  hits followed like the player controller (no speed loss on ramps, no micro-hops), ground snap,
  and the capsule kept just outside the sweep skin (resting inside it made sweeps along the
  floor eat the motion). Soft separation velocity while moving plus a hard **push-apart** pass
  after everyone moved (3 relaxation passes, mass ∝ r²h so a Bastion shoves Husks aside,
  capsule sweep so nobody is pushed into a wall, never over a ledge).
* **Knockback**: `velocity += knockback × knockback_scale`; above 4 m/s (after scaling) the
  enemy **staggers** for 0.3 s (no steering, slow braking) so the hit reads; upward knockback
  launches it (no air control). Bastions (×0.12) barely move.
* **Edge guard** (`steer::guard`, `heading_off_edge`): before moving, a grounded enemy checks
  the nav column ~r + 0.35 m ahead: lava (a kill volume) or a drop deeper than allowed (paths:
  5.5 m; strafing, backing off, lunges, fallback steering: a step) makes it turn up to ±90° or
  stop; its own momentum (a lunge, braking) is stopped dead at the edge. Knockback is exempt
  (launching enemies into lava is a feature).
* **Kill volumes**: an enemy whose feet enter one (or that falls below the level) dies through
  the damage pipeline (`Attack::World`, `Owner::World`).
* **`SimWorld::frozen`** (results screen): living enemies drop their target and attack, idle and
  just stand; encounters don't trigger. The server sets it in `end_match`; game modes can use the
  same flag. A fresh world starts unfrozen.

### Flying (Gnat, `steer::fly_velocity`)

No nav grid. Horizontal: orbit the target (tangent × strafe sign) with a radial correction
towards the middle of its 6–13 m band; when the target has been out of sight for 0.75 s it
closes in instead. Vertical: hover between 4 and 8 m (centre 6 m ± a slow 0.8 m bob) above the
ground below it **or the target's feet, whichever is higher** (so it neither dives into pits
nor stays below a player on a tower); a ray up keeps it off ceilings. A sphere **feeler sweep**
(r + 0.2 m, ~0.45 s ahead, ≥ 2 m) slides the velocity along / pushes it off walls and flips
the orbit direction when something is close. Flight uses the same collide-and-slide (no
gravity), so it can't clip the level even if steering fails.

## Navigation grid (`shared::sim::nav`)

Built once per level from the collision world: by the server at startup
(`SimWorld::build_nav`, logged) and at match start (`replace_world` reuses it for the same
level), lazily on the first enemy tick otherwise (client prediction worlds never build one).

* **Columns**: 1 m cells over the XZ bounds of every movement-blocking collider (doors
  ignored). Each column gets **one node per floor**: downward rays started just above the top
  and just below the bottom of every collider overlapping the column (from a 4 m AABB
  broadphase), keeping hits whose normal is ≤ 50° from up. Stacked floors (galleries over
  aisles, bridges over lava, the mezzanine) are separate nodes.
* **Node flags**: `SMALL` walkable — an r 0.5 capsule from 0.45 m (step height) to 2.0 m above
  the floor fits (the physics test only runs when some collider AABB is near); `DEADLY` — the
  floor is inside a kill volume (lava, pit bottoms); `HAZARD` — a kill volume is within 1.25 m
  sideways (or a short drop away): walkable, +4 m path cost; `LARGE` — walkable, all 8
  neighbours linked within 0.6 m height and ≥ 3.4 m headroom (Bastions). Headroom comes from an
  upward ray only when something is overhead.
* **Links** (8 directions): to the neighbour column's floor with the smallest height change
  that is a step (≤ 0.45 m), a continuous ramp (≤ 1.2 m per cell, midpoint ray must lie on the
  slope) or a **drop** (≤ 5.5 m, one-way: enemies walk off ledges but can't climb back), with
  nothing in the way at knee / chest height above the higher floor (rays, AABB-prefiltered).
  Diagonals only where both orthogonal routes agree (no corner cutting). **Jump pads and gaps
  have no links** (no nav links yet: enemies never use pads).
* **Doors**: nodes within a door brush's AABB (+0.5 m) remember the door; while it is closed
  they are impassable (A* and straight-walk tests take the doors' open state each tick).
* **Queries**: `node_at` (highest passable floor under the feet, else the closest in the 3×3
  neighbourhood — agents brushing walls), `nearest_node` (spiral search; targets mid-air or on a
  beam), `walkable_line` (march the straight line through linked passable nodes),
  `find_path` (A*, octile-ish Euclidean heuristic ×1.1, costs: distance + 0.5 × |Δy| + hazard,
  generation-stamped buffers so no allocation per search, `max_expand` budget → `Partial` path
  to the reachable node closest to the goal), `smooth` (string pulling with `walkable_line`),
  `ground_ok` (edge guard: the highest floor at or below a step must be safe and within the
  allowed drop).

**Path following** (`NavAgent`, `steer::ground_velocity`): a straight `walkable_line` from the
enemy's node to the target's node → walk straight at the target (no A*). Otherwise A* (at most
**4 searches per tick** for all enemies; the rest keep their old path and retry next tick, ≤ 3000
nodes each), string-pulled waypoints, the target position appended when the goal node is next
to it. Re-plan every `repath_interval` (0.5–0.8 s, staggered by id), when the goal moved more
than 1.5 m + 20 % of the distance, or when stuck (< 0.5 m progress in 0.5 s → re-plan +
short side-step). Every 8 ticks a look-ahead skips a waypoint if it's directly walkable. No
path / off the grid (airborne, pushed off) → direct steering, edge-guarded. A partial path ends
at the closest reachable point and the enemy waits there (e.g. a player on a pad-only ledge;
ranged enemies still shoot).

### Measurements (dev build: our code opt-level 1, dependencies opt-level 3)

| level | columns | floors | walkable / large / hazard / deadly | links | build |
|---|---|---|---|---|---|
| `crucible_of_veins` | 168 × 406 | 35 397 | 31 268 / 25 672 / 620 / 1 298 | 232 719 | **28 ms** (release 27 ms) |
| `cistern_nine` | 74 × 74 | 10 900 | 10 074 / 7 896 / 64 / 96 | 74 270 | 9 ms |
| `movement_test` | 202 × 262 | 53 824 | 50 384 / 45 028 / 0 / 480 | 382 308 | 25 ms |
| `proving_ground` (built-in) | 122 × 122 | 14 892 | 14 312 / 13 114 / 0 / 0 | 109 478 | 6 ms (release) |

On the campaign 13 950 walkable nodes are reachable from the spawns / checkpoints / enemy
spawns (the rest are roofs and wall tops), 1 687 columns have stacked reachable floors and 320
walkable floors lie over lava (bridges). A* inside an arena: tens of µs; across the whole
campaign 0.2 ms (≈600 nodes).

**Per-tick AI cost** (`shared/tests/enemies_perf.rs`): 4 players running and jumping around
the Furnace Cathedral (arena 5) against **53 aware enemies** of all four types (every arena-5
spawn plus a copy of each husk), whole `SimWorld::step` timed over 780 ticks: **avg 0.13–0.16
ms, p99 0.23–0.35 ms, max 0.4 ms** — far under the 2 ms budget. ~0.65 A* searches per tick
(≈200 nodes each, 10 % partial, 7 % deferred by the budget), ~3.6 straight-walk plans per
tick. No enemy fell into the chasm or the lava pools.

## Arena encounters (`shared::sim::encounter`)

Campaign levels only (`LevelKind::Arena` levels have none; `/spawn` works everywhere).
`SimWorld::step` runs them after the world rules.

1. **Idle** until a living player's feet (`counts_for_encounters`: `!dead`, hook for downed /
   spectating) enter `trigger`. **Start**: `lock_doors` close; every living player outside
   `arena` is teleported to `rally` (`MoveState::spawn`, view yaw from the rally point) and gets
   a `Respawned` event — their client snaps the camera (`NetEvent::Spawned`); pending hits on
   them are dropped. `SimEvent::EncounterStarted { index, waves }`.
2. **Waves**: wave 1 `delay` s after the start; wave *n+1* `delay` s after every enemy of wave
   *n* is dead (Dying counts as dead). On spawn: `SimEvent::WaveStarted { index, wave (1-based),
   waves, enemies }`, an `EnemySpawned` per enemy (→ `Fx::EnemySpawned`: flash + `EnemySpawn`
   sound). Wave enemies are `aware` and tagged `Enemy::encounter = (index, wave)`.
3. **Cleared** when the last wave is dead: `unlock_doors` **and** the entry `lock_doors` open
   (backtracking / late joiners), `SimEvent::EncounterCleared { index }`. A cleared encounter
   never starts again until `restart_level` or a reset hook.

**Co-op scaling** (only when `rules.coop_enemy_scaling`, i.e. campaign): with *p* =
`coop_player_count()` (players in the sim; hook for spectators) **at wave spawn**, a wave of *n*
authored spawns has `round(n × (1 + 0.5 (p − 1)))` enemies (2 players: 1.5×, 4 players: 2.5×),
each with health × `(1 + 0.5 (p − 1))`. Extra copies cycle through the authored spawns and are
placed on rings around them (r_def × 2 + 0.8 … + 4.5 m, 10 angles): same height (±0.7 m) on a
walkable non-hazard nav node with safe ground, capsule free, outside kill volumes, in line of
sight of the authored point (same room), not overlapping other spawns; flyers: free air with
≥ 1 m below. If nothing fits, the copy spawns on the authored point and the push-apart spreads
them. Gnat spawn points are authored at the hover centre and converted to feet.

**Replication**: one static entity per encounter, `kind::ENCOUNTER` (2),
`NetId::encounter(i)` = `STATIC_START + 0x100000 + i`: `position` = arena centre, `flags` =
`encounter_flags::{ACTIVE 1, CLEARED 2, BETWEEN_WAVES 4}`, `state` = current wave (1-based),
`health` = **enemies left in the wave**, `aux` = waves | wave size << 16. Delta compression makes
it free unless it changes; late joiners get it too. `SimWorld::active_encounter()` gives the same
on the server.

**Reliable events** (`GameEvent`, everyone): `EncounterStarted { index, waves }`,
`WaveStarted { index, wave, waves, enemies }`, `EncounterCleared { index }`.

**Client** (`ingame/encounter_hud.rs`): "ARENA LOCKED" banner + `DoorLock`; "WAVE n/N" banner
(from wave 2) + `WaveStart`; "CLEARED" banner + `EncounterClear` + `DoorOpen`; readout under the
timer "WAVE 2/3 - 5 LEFT" / "WAVE 2/3 INCOMING" from the encounter entity. The combat
presentation plays `Fx::EnemySpawned` (red flash + smoke + `EnemySpawn`).

### Hooks for the game modes / lobby

| hook | use |
|---|---|
| `enemy::is_targetable(&SimPlayer)` | players enemies may target / hit (add downed, spectating) |
| `encounter::counts_for_encounters(&SimPlayer)` | players who trigger encounters and get rallied |
| `SimWorld::coop_player_count()` | co-op scaling player count (exclude spectators) |
| `SimWorld::reset_encounter(i)` | encounter back to idle: its enemies (and their projectiles / pending hits) removed, its doors back to their level-start state |
| `SimWorld::reset_uncleared()` | `reset_encounter` for every encounter in progress (team wiped → checkpoint) |
| `SimWorld::start_encounter(i)` | start one now (dev `/encounter`, scripted events) |
| `SimWorld::remove_enemy(id)` | remove without a death |
| `SimWorld::frozen` | results screen: AI and encounters stop |
| `SimWorld::encounters.states[i].phase` | `Idle` / `Active` / `Cleared` (results, checkpoints) |

`restart_level()` clears enemies, resets every encounter to idle and the doors to their start
state.

## Protocol additions

`kind::ENCOUNTER` + `encounter_flags` + `NetId::encounter`; `GameEvent::{EncounterStarted,
WaveStarted, EncounterCleared}`; `Fx::EnemySpawned { enemy, kind, position }` (important:
repeated once); `SimEvent::{EncounterStarted, WaveStarted, EncounterCleared}`. All additive
(new enum variants at the end); `PROTOCOL_VERSION` 4.

## Dev commands

`/encounter <n>` starts arena encounter *n* (1-based; resets it first; you are pulled to its
rally point if outside), `/encounters` lists their phases. `/spawn spitter 2`, `/spawn gnat 3`,
`/spawn bastion`, `/killall`, `/god`, `/notarget` as before.

## Tests

```text
cargo test -p shared --test enemies_nav -- --nocapture   # grid on every level (timed), connectivity, lava, multi-floor
cargo test -p shared --test enemies_ai -- --nocapture    # behaviour per type in test arenas
cargo test -p shared --test enemies_perf -- --nocapture  # 53 enemies on the campaign, per-tick cost
cargo test -p server --test encounter_crucible           # encounters end to end (real server + clients)
cargo run -p client --release --example enemy_shots -- out/   # screenshots
```

* **Nav**: builds on every shipped level with sane counts, player spawns on the grid; on the
  campaign every husk/bastion spawn has a path to its arena centre and both ways to the rally
  point, spitter spawns are on the grid; every path node is outside kill volumes and the
  string-pulled route never crosses lava; stacked floors and bridges over lava exist and are
  safe; no link climbs more than a ramp; unit tests for walls/gaps, a ramp to a platform (no
  ledge climbing) and the one-way drop.
* **AI**: each type acquires a player and telegraphs every attack (every strike exactly one
  wind-up after its wind-up event, projectiles only during strikes, no hit before a strike; gnats
  fire bursts of 3); Spitters spend > 90 % of the time in 12–25 m from 5 m and from 45 m and
  strafe; a Spitter behind a wall walks out and never spits without line of sight; Gnats hover
  5–6.5 m up for 25 s around pillars / under a roof without ever intersecting the level and
  circle; Bastion slam: parried in the window → stunned 150 ticks + exactly 150 counter damage
  through the armour and no slam damage, too early / too late → no parry and the slam lands
  only after the full 1 s; front armour 35 %, back 100 %, head/furnace ×2; husk knocked 20 m/s,
  bastion < 4 m/s; husks + bastion + spitter cross a lava trench by its bridge without anyone
  touching the lava, and a husk put in the lava dies; a husk routes around a wall (A*);
  10 husks spawned on one point separate and stay ≥ 0.8 × (r1 + r2) apart while swarming;
  dead / untargetable players are dropped at once; damage alerts an idle enemy; all types are
  deterministic; a frozen world stops all attacks.
* **Encounters** (crucible, 2 real clients): A enters arena 1 → door 1 locks, B (in the start
  hall) is pulled to the rally point and its client gets the teleport; wave 1 = 6 husks with
  60 HP (2 players), all inside the arena; clients see "wave 1/2, 6 left"; killing through the
  damage pipeline → wave 2 (9 husks) after 1.5 s → cleared: doors 1 and 2 open, events in order
  on the server and both clients, 15 spawn events; walking through the trigger again does
  nothing; `restart_level` → idle, door 1 open, door 2 closed, and it can be fought again.
  Single player: no scaling; `reset_uncleared` / `reset_encounter` put arena 2 back (enemies
  gone, doors reset) and it restarts on re-entry. Deathmatch level: no encounters, `/spawn`
  works, nav grid built at load.

Screenshots (`enemy_shots`) were checked by eye: Spitters' glowing sac wind-up and orb, Gnats
hovering and their bolt burst, the Bastion's overhead wind-up / yellow parry window / slam dust,
a mixed crowd, and arena 4 on the campaign ("ARENA LOCKED", "WAVE 1/2 - 5 LEFT", the Bastion
winding up next to husks, "WAVE 2/2", "CLEARED").

## Known limitations / next steps

* **No nav links for jump pads / gaps**: walkers never use pads; pad-only ledges (R3 tiers, R4
  corner platforms, R5 galleries) only hold the spitters that spawn there, and melee enemies
  wait at the closest reachable point when a player stands on one. A `NavLink` list (pad →
  landing) in the level format would fix it (also a level-format wish in `docs/levels.md`).
* Drops are one-way and up to 5.5 m; enemies knocked onto a ledge they can't leave stay there.
* The grid is 1 m: openings narrower than ~1 m and thin (< 0.5 m) obstacles between cell
  centres can be misjudged; collide-and-slide copes, the edge guard is column-based.
* Large-agent clearance is an erosion of the small grid (8 neighbours), not a real capsule
  test; very thin pillars near a Bastion's path can still snag it.
* Players and enemies don't block each other (unchanged: player movement must stay
  predictable), so a husk can stand inside a player.
* Gnats have no pathfinding: a gnat with no line of sight flies straight at its target and
  relies on the feeler sweeps; long detours around big walls are slow.
* Enemy projectiles already in flight when the world freezes still land.
* `PROTOCOL_VERSION` is 4 for these protocol additions (3 was the lobby).
