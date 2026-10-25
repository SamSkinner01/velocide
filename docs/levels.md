# Levels

Two shipping levels live in `assets/levels/` (the movement playground `movement_test.ron` is
documented in `docs/movement.md`):

| file | name | kind | players | brushes | lights | notes |
|---|---|---|---|---|---|---|
| `crucible_of_veins.ron` | **Crucible of Veins** | `Campaign` | 1–4 co-op | 341 | 67 | 5 locked-room arenas, 86 enemies (SP), exit |
| `cistern_nine.ron` | **Cistern Nine** | `Arena` | up to 8 | 115 | 15 | deathmatch, 10 spawns, no doors/encounters |

## Tools

```text
cargo run -p client --example level_gen               # regenerate both .ron files
cargo run -p client --example level_gen -- --check    # fail if the .ron files are stale
cargo run -p client --example level_view -- crucible_of_veins [--out DIR] [--only GROUPS]
        [--lines] [--cam x,y,z,yaw,pitch] [--doors open|closed]
cargo test -p shared --test levels [-- --nocapture]   # checks + bot traversal (prints route times)
```

- **`client/examples/level_gen.rs` is the source of truth.** The RON files are generated (and
  round-trip checked) from it; they carry a comment per brush and per section, so they are
  readable, but edit the generator. Rooms are described once (interior box, wall material,
  openings) and the generator derives walls, sills, lintels and corner overlaps, so rooms are
  always sealed; doors are derived from openings (0.6 m thick, recessed in the 1 m wall,
  overlapping jambs/lintel by 0.25 m so they seal without z-fighting). Ramps are rotated boxes
  computed from their two surface end points (`ramp_x`/`ramp_z`, low end buried 1 m); thick
  ramps fill the wedge underneath so there are no dead-end nooks under them.
- **`client/examples/level_view.rs`** renders headless PNGs (default `captures/levels/<level>/`):
  every player spawn and checkpoint (along its yaw), each encounter centre looking N/W/S/E, a
  top-down overview (fog off; roofs removed for indoor levels: flat brushes nothing visible sits
  on), per-encounter top-down + two high-corner views (arena: four corner views), plus contact
  sheets per group. The overview always has the debug overlay: encounter triggers orange,
  checkpoints green, exit white, kill volumes red, doors red (closed) / green (open), player
  spawns cyan with facing, jump pads cyan with their ballistic arc, enemies husk yellow /
  spitter magenta / gnat light blue / bastion red.

## Conventions and metrics

Y up, metres, **north = -Z**, yaw 0 faces north, yaw 90 west, -90 east, 180 south. Built
around `docs/movement.md`:

| metric | value | used for |
|---|---|---|
| walls | 1 m thick | everything; doors 0.6 m inside them |
| doorways | 5 m wide × 5 m tall | all arena doors |
| step | ≤ 0.45 m | lips/trims (0.06–0.15 m) never need a jump |
| critical-path ledge | ≤ 1.0 m (hurdle) | jump apex is 2.5 m |
| critical-path gap | ≤ 8 m (lava trench 5 m, chasm has a bridge) | a running jump clears ~11.8 m |
| ramps | 18°–27° | walkable up to 50°; slides climb them too |
| slide-only gap | 1.2 m clearance | slide capsule is 0.9 m |
| wall-jump shaft | 3.5 m wide | 2 wall jumps ≈ 6 m, 3 ≈ 9 m |
| pads | land 4–12 m higher, ≤ 11 m/s sideways | air control still steers |
| gnat spawns | 4–8 m above ground | gameplay.md hover band |
| ceilings | 6 m corridors … 36 m (Bell Loft) | room for rocket jumps |

Lava (unlit `lava` material) always kills: every exposed lava surface has a kill volume
starting at its bottom and ending 0.25–2.5 m above the surface (below any standable rim). Lava
behind grilles (furnaces, gutters) is decoration and is never reachable. Both levels have a
kill volume under the whole world.

## Crucible of Veins (campaign)

A blood foundry built like a cathedral: brick and flesh halls, bone colonnades, brass organ
pipes, molten-blood channels. Warm palette (red/orange point lights, dark red fog 22–130 m,
ambient `(0.5, 0.42, 0.4)`), music `campaign`.

```text
                                  N (-Z)
                           +---------------+
                           |  EXIT  (dais) |  Ascension Hall: exit volume, y 4
                           +-------+-------+
                                   | C6 (y 4)
                 +-----------------+-----------------+
                 |        R5 FURNACE CATHEDRAL       |  arena 5 (finale), 62 x 67 x 28
                 | gallery |    altar dais y4  | gal.|  exit door 10 behind the altar (y 4)
                 |  y 8    |   ramps   ramps   | y 8 |  galleries via pads
                 |  [pool]=|=== chasm + bridge =|=[pool]  chasm 20 m deep, lava pools in aisles
                 |         |  pad    nave   pad|     |
                 +-----------------+-----------------+
                                   | door 9 / C5 Pipe Run (3 pipes: slide under or hop)
   +--------------+      +---------+---------+
   | R3 BELL LOFT |      | R4 CRUCIBLE HEART |  arena 4: island in a lava moat, 4 bridges,
   | floor y 10   | C4   |  [NE plat] (pad)  |  NE/SE platforms y 4 via pads,
   | balcony y 16 |=====>|   moat [isle] moat|  hanging crucible
   | platform y22 |spill-|  [SE plat] (pad)  |  door 7 (W) / door 8 (N)
   | bell over pit| way  +-------------------+
   +------+-------+ (slide 22 -> 0 m)
          | C3 Vein Stair (ramp y 5 -> 10)
 +--------+-----------+
 | R2 BLEEDING        |  arena 2: mezzanine y 5 along the N wall (ramps E/W + pad),
 |   CLOISTER         |  lava trench across the middle with 2 bridges, bone colonnade;
 |  ====trench====    |  exit door 4 on the mezzanine (y 5)
 +--------+-----------+
          | C2 Sluice (5 m lava trench)
 +--------+-----------+
 | R1 RENDERING FLOOR |  arena 1: terraces y 4 along E/W walls (ramps), lava vat + perch,
 +--------+-----------+  4 pillars
          | C1 Split Gullet: W lane slide slot (1.2 m) / E lane 1 m hurdle
 +--------+-----------+
 | R0 RECEIVING NAVE  |  start: 4 player spawns facing north, grated lava channels
 +--------------------+
                                  S (+Z)
```

| area | interior (x, z) | floor / ceiling | features |
|---|---|---|---|
| R0 Receiving Nave | -12..12, 2..32 | 0 / 10 | spawns at z 25; three grated lava channels; furnace |
| C1 Split Gullet | -3..3, -22..1 | 0 / 6 | divider; W lane shutter (slide slot 1.2 m), E lane 1 m hurdle |
| R1 Rendering Floor | -20..20, -63..-23 | 0 / 12 | terraces y 4 (ramps from the south), lava vat, perch y 5.6 |
| C2 Sluice | -3..3, -90..-64 | 0 / 7 | 5 m lava trench (z -80..-75) |
| R2 Bleeding Cloister | -30..30, -141..-91 | 0 / 14 | trench z -122..-114, bridges x ±13..18, mezzanine y 5 (z < -133), pad |
| C3 Vein Stair | -26..-19, -172..-142 | 5→10 / 17 | ramp z -150..-162, bone ribs |
| R3 Bell Loft | -44..0, -215..-173 | 10 / 46 | pit (kill) under the bell, balcony y 16 (W), catwalk + ramp, platform y 22 (E), 2 pads, wall-jump fin, boilers |
| C4 Spillway | 1..65, -212..-205 | 22→0 / 29→7 | 48 m chute at 24.6° |
| R4 Crucible Heart | 66..116, -233..-183 | 0 / 18 | moat x 82..100 z -217..-199, island y 1, corner platforms y 4, 2 pads |
| C5 Pipe Run | 87..95, -262..-234 | 0 / 8 | pipes at y 1.2–1.9 (z -242, -248, -254) |
| R5 Furnace Cathedral | 60..122, -330..-263 | 0 / 28 | chasm z -299..-291, bridge x 89..93, aisle lava pools, galleries y 8, dais y 4 |
| C6 + Exit | 87..95 / 81..101, -372..-331 | 4 / 11, 20 | exit volume x 86..96, z -367..-357 |

### Encounters, doors, checkpoints

Every arena has its **checkpoint just inside the entry door** (a 6 m vestibule), before the
encounter trigger. Respawning players therefore always come back *inside* the locked room
(a checkpoint in the corridor would strand them behind the locked door), and the trigger is
deep enough that a group running in together is inside before the doors shut. Entry doors
start open and are locked by the encounter; exit doors start closed and open when it's won.

| # | arena | checkpoint spawn | lock → unlock | wave 1 | wave 2 | wave 3 | wave 4 | total |
|---|---|---|---|---|---|---|---|---|
| 1 | Rendering Floor | (0, 0, -25.5) | 1 → 2 | 4 husk | 6 husk (2 on terraces) | | | 10 |
| 2 | Bleeding Cloister | (0, 0, -93.5) | 3 → 4 | 4 husk, 2 spitter (mezzanine) | 3 spitter, 4 husk | | | 13 |
| 3 | Bell Loft | (-22.5, 10, -175.5) | 5 → 6 | 3 gnat, 3 husk | 4 gnat, 2 spitter (tiers), 2 husk | | | 14 |
| 4 | Crucible Heart | (68.5, 0, -208.5) | 7 → 8 | **bastion** (island), 4 husk | 2 spitter (platforms), 3 gnat, 3 husk | | | 13 |
| 5 | Furnace Cathedral | (91, 0, -265.5) | 9 → 10 | 6 husk, 2 spitter (dais) | 4 gnat, 2 spitter (galleries), 3 husk | **bastion**, 4 husk, 3 gnat | **bastion** (dais), 3 spitter, 3 gnat, 4 husk | 36 |

86 enemies single-player (co-op adds 50 % per extra player per gameplay.md). Wave delays
0.5–2 s. Spawns face the entrance, stand on solid floor with room for a 1.2 m-radius bastion,
are ≥ 8 m from the checkpoint and spread ≥ 1 m apart (capsule to capsule). Spitters sit on
ledges ≥ 8 m deep so co-op copies offset around them stay on the ledge.

### Routes and movement tech

Critical path (only running, jumping, walking up ramps, one jump pad and one slide; verified by
the traversal bots, ~35 s of pure movement from spawn to exit):

1. R0 → C1 east lane (hop the 1 m hurdle) → R1 vestibule.
2. R1 around the vat → exit door 2 → C2: jump the 5 m lava trench → R2 vestibule.
3. R2 over the east bridge → east ramp to the mezzanine → exit door 4 → C3 ramp → R3.
4. R3 pad (floor → upper platform) → exit door 6 → C4 spillway down to R4.
5. R4 along the north side of the moat → door 8 → C5: slide under the pipes (or hop them) → R5.
6. R5 over the chasm bridge → west dais ramp → door 10 behind the altar → C6 → exit.

Alternate routes and rewards for good movement (all tested):

- **C1 slide slot**: the west lane's shutter leaves 1.2 m — slide through without slowing down.
- **R1 perch**: a dash-jump off either terrace lands on the hood above the lava vat (12 m out,
  1.6 m up). Great vantage point; a miss is a swim in the vat.
- **R2**: west bridge, or the pad in front of the mezzanine; the 8 m centre of the trench is a
  plain running jump.
- **R3**: west pad → balcony → catwalk → ramp to the platform; or wall-jump up the 3.5 m shaft
  between the south-west fin and the west wall onto the balcony. The pit under the bell is
  14 m deep — falling from the tiers is lethal.
- **C4 spillway**: sliding down the 48 m chute reaches ~45 m/s, and the slide keeps that speed
  across R4's flat floor (no friction while sliding) — a flying entrance into the bastion fight.
- **R4**: cross the island (bridges are small ramps), NE/SE pads to the corner platforms.
- **R5**: aisle paths between the lava pools and the chasm (3 m wide), pads to the galleries,
  drop from a gallery onto the dais. Rocket/slam-bounce jumps fit everywhere (28 m ceiling).

### Par (results grading)

Bot traversal is ~35 s; fights are the bulk. Suggested `par` for the time grade:
**5:30** (S ≤ 5:30, A ≤ 7:00, B ≤ 9:00, C ≤ 12:00, else D). A first playthrough lands around
6–8 minutes. Kills: 86 total in single player.

## Cistern Nine (deathmatch arena)

An open-air coolant cistern at night: cold teal concrete and tile, violet accents, cyan
coolant, fog 30–150 m, cool key light, music `arena`. Point-symmetric (180°), 72 × 72 m inside
16 m walls; invisible player clip walls and a clip ceiling at 32 m keep everyone in.

```text
                               N (-Z)
  +------------------------------------------------------------+
  |[NW tower]==============  catwalk y 6  ==============[NE tower]|  towers y 6 (corners)
  | |  fin   [pump]  [step]   ~~ pit ~~   [step]  [pump]   fin  | |  pump houses y 3, coolant pits
  | r                                                         r |
  | a              +-------- bowl y -3 ---------+             a |
  | m    [crate]   |  []      ramp N       []   |    [lowcrate] m |  ramps W/E walls -> towers
  | p              |                            |             p |
  |  ==trench==tunnel  (pad)  [CORE y4]  (pad)  tunnel==trench== |  slide trenches + 1.2 m tunnels
  | r              |                            |             r |
  | a  [lowcrate]  |  []      ramp S       []   |     [crate]   a |
  | m              +----------------------------+             m |
  | p                                                         p |
  | |  fin   [pump]  [step]   ~~ pit ~~   [step]  [pump]   fin  | |
  |[SW tower]==============  catwalk y 6  ==============[SE tower]|
  +------------------------------------------------------------+
                               S (+Z)
```

Layers: bowl -3, ring 0, crates 1.2/2, pump houses 3, reactor core 4, towers + catwalks 6,
pillar caps 9.6.

- **Contested centre**: the reactor core (6 × 6 m, y 4) in the sunken bowl, reached only by the
  two bowl pads (from the E/W slide tunnels) — fully exposed to the towers and catwalks.
- **Exposed high ground**: four corner towers joined by the N/S catwalks (no railings).
  Ramps along the W/E walls lead up from the ring; wall-jump fins next to each tower are the
  shortcut (3.5 m shaft, two wall jumps).
- **Slide channels**: from the W/E ring a trench ramps down into a 1.2 m-high grated tunnel
  that exits into the bowl right in line with a core pad: slide → pad → core in one motion.
- **Loops, no dead ends**: ring ↔ bowl (2 ramps, 2 tunnels, drop anywhere), ring ↔ towers
  (4 ramps, 4 fins), towers ↔ catwalks, core → anywhere. A full lap is ~20 s.
- **Sightline breakers**: four bowl pillars, the core, pump houses and step crates on the N/S
  ring, crates and low walls (1.2 m, slide/crouch cover) on the E/W ring.
- **Hazards**: two 8 × 6 m coolant pits (kill) just outside the N/S bowl ramps — knockback and
  carelessness are punished.
- **Spawns** (10): four tower tops, two in the bowl, four on the ring (one per side), every
  quadrant covered, ≥ 10 m apart, three heights, all facing the centre.

Suggested match settings: frag limit 20 / 10 minutes (gameplay.md defaults) for 4–8 players.

## Tests (`shared/tests/levels.rs`)

- Every level in `assets/levels/` parses; known materials and enemy names; unique door ids;
  encounter door references exist. Shipped levels: no zero-volume brushes, < 600 brushes,
  a kill volume under the whole level, lights and music set.
- Campaign structure: 4 spawns, exit, 4–5 encounters each with a checkpoint whose spawn is
  outside the encounter trigger, lock doors start open / unlock doors start closed, every door
  used; husks-only first arena, spitters in arena 2, gnats introduced in a ≥ 25 m tall room,
  a bastion before the finale, a finale with all four types over ≥ 3 waves.
- Arena structure: ≥ 8 spawns, ≥ 10 m apart, every quadrant, 3+ heights, no doors/encounters.
- Player/checkpoint spawns (doors open and closed): ground within 0.3 m, capsule free, head room
  for a jump, not in a kill volume, and the real movement code settles there without drifting.
- Enemy spawns: inside their locked room, not in kill volumes, ≥ 8 m from the checkpoint,
  spread out, grounded enemies stand on floor all around (no ledge overhang) with a generous
  capsule (bastion r 1.2 × 3.4 m); gnats in open air 3.5–9 m above ground with 3 m above.
- Jump pads: with no input each one lands on ground ≥ 2 m higher, never in a kill volume.
- No z-fighting: no coplanar same-facing overlapping faces between differently textured
  axis-aligned brushes (including doors).
- Sealing: with every door closed, a flood fill through free space (1 m balls on a 1 m grid)
  from each compartment (start, each arena, exit hall) never leaves the level, never reaches
  another compartment's key point or the far side of the arena's own doors; with doors open
  the exit is reachable from the start. Arena: the flood never leaves the level. (Every wall,
  however thin, stops the flood; openings narrower than ~1 m may go unnoticed.)
- Traversal bots: waypoint followers driven by `shared::movement::step` (steer, auto-jump at
  hurdles/gaps/ledges or when stuck, slide on slide segments, take pads) run the full critical
  path from all four spawns and from every checkpoint to the exit (doors open), never touching a
  kill volume, entering checkpoint/encounter triggers in order; five alternate routes; a full
  arena lap through every ramp, catwalk, tunnel and pad.
- Advanced tech, scripted: the R1 perch dash-jump, the R3 and arena wall-jump shafts, and the
  spillway slide (> 40 m/s).
- Chaos: random jump/dash/slide/slam input for 40 s from every spawn and checkpoint (doors open
  and closed) never leaves the level bounds.

All of it is deterministic (same inputs, same results) and runs in well under a second.

## Level-format wishes

Things the format can't express yet (worked around or left to game code):

- **Encounter arena volume / straggler pull-in**: when an encounter locks, co-op players still
  in the corridor are locked out. An `Encounter::arena: Option<Volume>` (or "teleport players
  outside the trigger to checkpoint N on lock") would solve it.
- **`par_time` / grading data on `LevelDef`** (and maybe `display_name`, `description`) so the
  results screen doesn't hardcode them.
- **Kill-volume contract**: document whether the feet point or the capsule is tested (these
  levels assume the feet point; volumes start at the hazard bottom).
- **Per-area environment** (fog/ambient zones) — one `Environment` per level today.
- **Door motion** (slide offset + speed) instead of popping; door sounds.
- **Wedge/prism brushes** for ramps and slopes (rotated boxes work but need care at joints).
- **Per-spawn co-op spread** (`spread: f32` or "no copies") for enemies on narrow ledges.
- **Nav hints / links** for AI across jump pads. Walking enemies can't use pads, so the
  pad-only ledges (R3 tiers, R4 corner platforms, R5 galleries) only ever hold the spitters
  that spawn there; R2's mezzanine has ramps for everyone.
