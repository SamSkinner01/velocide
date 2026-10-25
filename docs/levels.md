# Levels

The shipping levels live in `assets/levels/` (the movement playground `movement_test.ron` is
documented in `docs/movement.md`). All of them are set in the Foundry (`docs/premise.md`): molten
pools are slag or bronze, walls are brick, stone, wax and brass — no blood or flesh.

| file | name | kind | players | brushes | lights | notes |
|---|---|---|---|---|---|---|
| `crucible_of_veins.ron` | **Crucible of Veins** | `Campaign` | 1–4 co-op | 341 | 67 | the first campaign level: 5 locked-room arenas, 86 enemies (SP), exit |
| `spire_of_wicks.ron` | **Spire of Wicks** | `Campaign` | 1–4 co-op | 365 | 57 | the second: a vertical climb, 4 locked-room arenas, 66 enemies (SP), exit |
| `cistern_nine.ron` | **Cistern Nine** | `Arena` | up to 8 | 115 | 15 | deathmatch, 10 spawns, no doors/encounters |

Levels are scanned from `assets/levels/` by the menus and the lobby (`shared::lobby::scan_levels`),
so a new file shows up in the single-player list and the host's level picker on its own; new
players still start on Crucible of Veins (`Settings::last_level`).

## Tools

```text
cargo run -p client --example level_gen               # regenerate every .ron file
cargo run -p client --example level_gen -- --check    # fail if the .ron files are stale
cargo run -p client --example level_view -- crucible_of_veins [--out DIR] [--only GROUPS]
        [--lines] [--cam x,y,z,yaw,pitch] [--doors open|closed]
cargo test -p shared --test levels [-- --nocapture]   # checks + bot traversal (prints route times)
```

- **`client/examples/level_gen.rs` is the source of truth** (with one module per newer level in
  `client/examples/level_gen/`, e.g. `level_gen/spire.rs`, declared with
  `#[path = "level_gen/spire.rs"] mod spire;` because an example's root resolves `mod x;` next
  to itself). The RON files are generated (and
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
  sheets per group. A campaign encounter's top-down view also drops every brush that lies
  entirely above its arena, so rooms stacked over it (the Spire's Belfry and Lantern over the
  Casting Floor) don't hide it. The overview always has the debug overlay: encounter triggers
  orange, checkpoints green, exit white, kill volumes red, doors red (closed) / green (open),
  player spawns cyan with facing, jump pads cyan with their ballistic arc, enemies gnawer
  yellow / censer magenta / harrier light blue / gaoler red / heron white.

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
| pads | land 4–12 m higher, ≤ 11 m/s sideways | air control still steers; under an overhang, start ≥ 5 m short of it and arc ≥ 4 m over the landing so a player steering at it clears the edge |
| harrier spawns | hover centre 4–11 m above ground, 3 m clear above | roster: hovers 5–10 m |
| censer spawns | ledges ≥ 7–8 m deep (floor 3 m around) | co-op copies stay on the ledge |
| gaoler spawns | level floor 4 m around (1.3 × 3.7 m) | room to circle it |
| heron spawns | ≥ 20 m from the rally point, in sight of it; on walkable ledges | roster: keeps 25–45 m |
| ceilings | 6 m corridors … 42 m (the Hoist) | room for rocket jumps |

Lava (unlit `lava` material) always kills: every exposed lava surface has a kill volume
starting at its bottom and ending 0.25–2.5 m above the surface (below any standable rim).
Contract (`LevelDef::kill_volumes`): a player dies when their *feet position* is inside. Lava
behind grilles (furnaces, gutters) is decoration and is never reachable. Every level has a
kill volume under the whole world. (The `lava` material is the Foundry's molten slag or bronze.)

## Crucible of Veins (campaign)

A slag foundry built like a cathedral, cut through ore veins: brick halls, wax colonnades and
wax weeping down the walls in sheets, brass organ pipes, molten slag channels. Warm palette
(orange/ember point lights, smoky ember fog 22–130 m, ambient `(0.5, 0.42, 0.4)`), music
`campaign`. It is the first campaign level: every enemy type is introduced here in turn.

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
                 |  [pool]=|=== chasm + bridge =|=[pool]  chasm 20 m deep, slag pools in aisles
                 |         |  pad    nave   pad|     |
                 +-----------------+-----------------+
                                   | door 9 / C5 Pipe Run (3 pipes: slide under or hop)
   +--------------+      +---------+---------+
   | R3 BELL LOFT |      | R4 CRUCIBLE HEART |  arena 4: island in a slag moat, 4 bridges,
   | floor y 10   | C4   |  [NE plat] (pad)  |  NE/SE platforms y 4 via pads,
   | balcony y 16 |=====>|   moat [isle] moat|  hanging crucible
   | platform y22 |spill-|  [SE plat] (pad)  |  door 7 (W) / door 8 (N)
   | bell over pit| way  +-------------------+
   +------+-------+ (slide 22 -> 0 m)
          | C3 Vein Stair (ramp y 5 -> 10)
 +--------+-----------+
 | R2 WEEPING         |  arena 2: mezzanine y 5 along the N wall (ramps E/W + pad),
 |   CLOISTER         |  slag trench across the middle with 2 bridges, wax colonnade;
 |  ====trench====    |  exit door 4 on the mezzanine (y 5)
 +--------+-----------+
          | C2 Sluice (5 m slag trench)
 +--------+-----------+
 | R1 SMELTING FLOOR  |  arena 1: terraces y 4 along E/W walls (ramps), slag vat + perch,
 +--------+-----------+  4 pillars
          | C1 Split Flue: W lane slide slot (1.2 m) / E lane 1 m hurdle
 +--------+-----------+
 | R0 RECEIVING NAVE  |  start: 4 player spawns facing north, grated slag channels
 +--------------------+
                                  S (+Z)
```

| area | interior (x, z) | floor / ceiling | features |
|---|---|---|---|
| R0 Receiving Nave | -12..12, 2..32 | 0 / 10 | spawns at z 25; three grated slag channels; furnace |
| C1 Split Flue | -3..3, -22..1 | 0 / 6 | divider; W lane shutter (slide slot 1.2 m), E lane 1 m hurdle |
| R1 Smelting Floor | -20..20, -63..-23 | 0 / 12 | terraces y 4 (ramps from the south), slag vat, perch y 5.6 |
| C2 Sluice | -3..3, -90..-64 | 0 / 7 | 5 m slag trench (z -80..-75) |
| R2 Weeping Cloister | -30..30, -141..-91 | 0 / 14 | trench z -122..-114, bridges x ±13..18, mezzanine y 5 (z < -133, 8 m deep), pad, wax drapes |
| C3 Vein Stair | -26..-19, -172..-142 | 5→10 / 17 | ramp z -150..-162 through ore-rust rock, brass ribs |
| R3 Bell Loft | -44..0, -215..-173 | 10 / 46 | pit (kill) under the bell, balcony y 16 (W), catwalk + ramp, platform y 22 (E), 2 pads, wall-jump fin, boilers |
| C4 Spillway | 1..65, -212..-205 | 22→0 / 29→7 | 48 m chute at 24.6° |
| R4 Crucible Heart | 66..116, -233..-183 | 0 / 18 | slag moat x 82..100 z -217..-199, island y 1 (12 × 12), corner platforms y 4, 2 pads |
| C5 Pipe Run | 87..95, -262..-234 | 0 / 8 | pipes at y 1.2–1.9 (z -242, -248, -254) |
| R5 Furnace Cathedral | 60..122, -330..-263 | 0 / 28 | chasm z -299..-291, bridge x 89..93, aisle slag pools, galleries y 8, dais y 4 (30 × 16) |
| C6 + Exit | 87..95 / 81..101, -372..-331 | 4 / 11, 20 | exit volume x 86..96, z -367..-357 |

### Encounters, doors, checkpoints

Every arena has its **checkpoint just inside the entry door** (a 6 m vestibule), before the
encounter trigger. Respawning players therefore always come back *inside* the locked room
(a checkpoint in the corridor would strand them behind the locked door), and the trigger is
deep enough that a group running in together is inside before the doors shut. Entry doors
start open and are locked by the encounter; exit doors start closed and open when it's won.

Each encounter also sets **`arena`** = the room's whole interior (wall bottom/pit floor to
ceiling, galleries and vestibule included, never the corridors) and **`rally`** = that same
checkpoint spawn: when the doors lock, living players outside `arena` are pulled to `rally`, so
a co-op teammate lagging in the corridor is never locked out. The generator derives both from
the room description (`Gen::encounter(note, &room, …)`).

| # | arena | checkpoint spawn | lock → unlock | wave 1 | wave 2 | wave 3 | wave 4 | total |
|---|---|---|---|---|---|---|---|---|
| 1 | Smelting Floor | (0, 0, -25.5) | 1 → 2 | 4 gnawer | 6 gnawer (2 on terraces) | | | 10 |
| 2 | Weeping Cloister | (0, 0, -93.5) | 3 → 4 | 4 gnawer, 2 censer (mezzanine) | **2 heron** (mezzanine ends), 2 censer (mezzanine), 4 gnawer | | | 14 |
| 3 | Bell Loft | (-22.5, 10, -175.5) | 5 → 6 | 4 harrier, 2 gnawer | 4 harrier, 2 censer (tiers), 2 gnawer | | | 14 |
| 4 | Crucible Heart | (68.5, 0, -208.5) | 7 → 8 | **gaoler** (island), 4 gnawer | 2 censer (platforms), 3 harrier, 3 gnawer | | | 13 |
| 5 | Furnace Cathedral | (91, 0, -265.5) | 9 → 10 | 6 gnawer, 2 censer (dais) | 2 heron (dais), 3 harrier, 2 censer (galleries), 2 gnawer | gaoler (nave), 4 gnawer, 3 harrier | gaoler (dais), heron (dais), 2 censer (galleries), 3 harrier, 3 gnawer | 35 |

86 enemies single-player (co-op adds 50 % per extra player per gameplay.md). Wave delays
0.5–2 s. The roster comes in one type at a time: gnawers (arena 1), censers (arena 2, wave 1),
herons (arena 2, wave 2: on the ends of the mezzanine, 45 m down the cloister from the
entrance, which they can walk down from by the ramps), harriers (the 36 m Bell Loft, hovering
6–7 m over the floor and the tiers), the gaoler (arena 4, alone with fodder on the 12 × 12 m
island — circle it, but mind the moat), then everything in the finale (gaolers on the nave
floor and the 30 × 16 m dais, herons on the dais with the whole nave in view). Spawns face
the entrance, stand on solid floor with room for their capsule (gaoler 1.3 × 3.7 m, heron
0.5 × 3.4 m), are ≥ 8 m from the checkpoint and spread ≥ 1 m apart (capsule to capsule).
Censers sit in the middle of ledges ≥ 8 m deep so co-op copies offset around them stay on the
ledge.

### Routes and movement tech

Critical path (only running, jumping, walking up ramps, one jump pad and one slide; verified by
the traversal bots, ~35 s of pure movement from spawn to exit):

1. R0 → C1 east lane (hop the 1 m hurdle) → R1 vestibule.
2. R1 around the vat → exit door 2 → C2: jump the 5 m slag trench → R2 vestibule.
3. R2 over the east bridge → east ramp to the mezzanine → exit door 4 → C3 ramp → R3.
4. R3 pad (floor → upper platform) → exit door 6 → C4 spillway down to R4.
5. R4 along the north side of the moat → door 8 → C5: slide under the pipes (or hop them) → R5.
6. R5 over the chasm bridge → west dais ramp → door 10 behind the altar → C6 → exit.

Alternate routes and rewards for good movement (all tested):

- **C1 slide slot**: the west lane's shutter leaves 1.2 m — slide through without slowing down.
- **R1 perch**: a dash-jump off either terrace lands on the hood above the slag vat (12 m out,
  1.6 m up). Great vantage point; a miss is a swim in the vat.
- **R2**: west bridge, or the pad in front of the mezzanine; the 8 m centre of the trench is a
  plain running jump.
- **R3**: west pad → balcony → catwalk → ramp to the platform; or wall-jump up the 3.5 m shaft
  between the south-west fin and the west wall onto the balcony. The pit under the bell is
  14 m deep — falling from the tiers is lethal.
- **C4 spillway**: sliding down the 48 m chute reaches ~45 m/s, and the slide keeps that speed
  across R4's flat floor (no friction while sliding) — a flying entrance into the gaoler fight.
- **R4**: cross the island (bridges are small ramps), NE/SE pads to the corner platforms.
- **R5**: aisle paths between the slag pools and the chasm (3 m wide), pads to the galleries,
  drop from a gallery onto the dais. Rocket/slam-bounce jumps fit everywhere (28 m ceiling).

### Par (results grading)

Bot traversal is ~35 s; fights are the bulk. `par_time: 330.0` (**5:30**) is set in the level
file; suggested time grades: S ≤ 5:30, A ≤ 7:00, B ≤ 9:00, C ≤ 12:00, else D. A first
playthrough lands around 6–8 minutes. Kills: 86 total in single player. (Cistern Nine has
`par_time: 0.0` — no par in deathmatch.)

## Spire of Wicks (campaign)

The second campaign level, generated by `client/examples/level_gen/spire.rs`: a **vertical**
climb through a lost-wax bell foundry built inside a hollow spire. Bells are cast here in
wax-and-clay moulds; thousands of candles burn on every ledge. The rooms spiral up around the
**Bell Shaft**, the spire's hollow core: at its foot (y −11) a pool of molten bronze, at its
top the **great bell** hanging in the Belfry. You start at the bottom, see the shaft through a
glazed window in the first arena, climb the Hoist, cross the Rood, take the Wick Stair up and
fight the finale around the shaft's open mouth, 48 m above the bronze — then step out into the
Lantern at the top of the spire.

Palette (contrasting Crucible's red/orange): cold blue-grey stone, ivory and yellowed wax,
bronze and brass, verdigris copper, bell-frame timber; pale candle-gold and orange wick lights,
molten-bronze glow only at the shaft and the Rood's chasm, cold moonlight through the belfry
arches; indigo fog 34–170 m, sky `(0.07, 0.08, 0.16)` (it shows through the open arches),
ambient `(0.5, 0.5, 0.58)`, cool moonlight from above. Music `campaign`.

```text
Plan (N up). A room in [brackets] lies under another one.

 x: -64             -30 -24            0             16 24
  +-----------------------------------------------------+     z -70
  | A3 THE ROOD (arena 3)     y 26 | wax run  | chasm |  far floor y 14,       |
  | near gallery (checkpoint 3)   |  26 -> 14 |~bronze~  rood loft y 24 (herons)|
  +--[door 5]--------------------------------------------[door 6]--+          z -39
     | C3 Rope   |  C4 WICK STAIR  y 36 <---- ramp ----- y 24       |
     | Walk y 26 +--[door 7]-------------------------------------- +          z -26
  +--[door 4]------+ +----------------------------------------+
  | A2 THE HOIST   | | A4 THE BELFRY (arena 4)  y 36          |
  | (arena 2)      | |  galleries y 44 along E + S,           |
  | 42 m tall:     | |  ramps along N + W walls               |
  | ramps up the   | |        +----------+                    |
  | walls y 0->26, | |        |  shaft   |  great bell above  |
  | cage pad line  | |        |  mouth   |  (rim y 46)        |
  |                | |  [A1 CASTING FLOOR y 0, z 11..41]      |
  +--[door 3]------+ +---------------[door 8]-----------------+          z 24
        <- C2 (y 0)    [A1]   | C6 -> the LANTERN (exit, y 44) |
                              +--------------------------------+          z 49
                                 | C1 Fallen Bell (z 42..59)
                          +-------------------+
                          | R0 WAX CRYPT      |  start (y 0)
                          +-------------------+                          z 82
                                  S (+Z)

Elevation: crypt / casting floor 0 -> Hoist spiral 0..26 -> Rope Walk 26 -> Rood 26 (near) /
14 (far) / 24 (loft) -> Wick Stair 24..36 -> Belfry 36 (galleries 44) -> Lantern 44.
Bell Shaft: bronze pool -11, open into the Belfry floor at 36, great bell 45.4..57.
```

| area | interior (x, z) | floor / ceiling | features |
|---|---|---|---|
| R0 Wax Crypt | -11..11, 60..82 | 0 / 10 | spawns z 74 facing north; wax effigies on biers; the cracked bell, its seam glowing behind a grille |
| C1 Fallen Bell | -4..4, 42..59 | 0 / 7 | a bell lies across the W lane propped on its lip (1.2 m slide gap under it); its broken crown is a 1 m hurdle in the E lane |
| A1 Casting Floor | -24..24, 11..41 | 0 / 18 | casting pit x -6..6 z 22..30 (molten bronze 2.2 m down), two bell moulds (4 × 6 × 5.5 m), balconies y 7 along W/E walls (8 m deep, ramps from the south), wall-jump chimney (N wall / mould rack), glazed window into the Bell Shaft |
| Bell Shaft | -9..9, -9..9 | pool -11 / open at 36 | molten bronze pool, the bell rope, verdigris pipes; kill volume from the pool up to y 28 |
| C2 Wick Gate | -29..-25, 14..20 | 0 / 6 | 4 m airlock between doors 2 and 3 |
| A2 The Hoist | -60..-30, -14..22 | 0 / 42 | one turn of ramps around the walls (W, S, E, N; landings y 6.5, 13, 19.5, 26); pad line: floor pad → cage A (y 9) → cage B (y 17.5) → NW landing (y 26) |
| C3 Rope Walk | -60..-54, -38..-15 | 26 / 33 | bridge corridor |
| A3 The Rood | -64..16, -70..-39 | 26 / 14 / 24 / 44 | near gallery y 26 (x < -52); the wax run 26 → 14 (26 m at 24.8°); chasm x -23..-9 with molten bronze 12.6 m down, bridges z -67..-63 and -47..-43; far floor y 14; rood loft y 24 (W strip 6 m deep over the chasm edge, S strip 8 m deep) with a ramp up the E wall; exit from the loft |
| C4 Wick Stair | -26..18, -37..-26 | 24 → 36 / 46 | one 28 m ramp at 23°, candle shelves on both walls |
| A4 The Belfry | -24..24, -24..24 | 36 / 62 | the shaft's mouth x -9..9 z -9..9 (a 48 m drop, kill 8 m down), the great bell over it (rim y 46, 1.5 m wide), galleries y 44 along E and S (8 m deep), ramps along N and W: floor, ramps and galleries form one loop around the shaft; 2 pads; 7 arches open to the sky (player-clipped) |
| C6 + Lantern | -3..3 / -10..10, 25..49 | 44 / 51, 64 | the great candle and its flame, exit dais x -6..6 z 34..41, 3 arches to the sky |

### Encounters

Same rules as Crucible (checkpoint just inside the entry door, `arena` = the room, `rally` = the
checkpoint spawn, entry doors lock, exit doors open on the win).

| # | arena | checkpoint spawn | lock → unlock | wave 1 | wave 2 | wave 3 | total |
|---|---|---|---|---|---|---|---|
| 1 | Casting Floor | (0, 0, 38) | 1 → 2 | 5 gnawer | 2 censer (balconies), 5 gnawer | | 12 |
| 2 | The Hoist | (-33, 0, 17) | 3 → 4 | 3 harrier (in the shaft), 3 gnawer | 4 harrier (over the ramps), 4 gnawer (2 on landings) | | 14 |
| 3 | The Rood | (-57, 26, -42) | 5 → 6 | 2 heron (rood loft), 4 gnawer (far floor) | **gaoler** (far floor), heron (loft), 2 censer (loft), 2 harrier, 2 gnawer (flanking up the wax run) | | 14 |
| 4 | The Belfry | (-21, 36, -21.5) | 7 → 8 | gaoler, 4 gnawer | 2 heron, 2 censer (galleries), 3 harrier, 3 gnawer | gaoler, heron, 2 censer (galleries), 3 harrier, 4 gnawer | 26 |

66 enemies single-player. Each arena is a different problem:

1. **Casting Floor** — a swarm around a molten pit, then censers lobbing from balconies 7 m up
   (take a ramp or the chimney, or fight from behind the moulds).
2. **The Hoist** — a 42 m shaft: harriers dive at you while you climb a spiral of ledges with no
   railings (knocked off = back to the floor, not death). The pad line through the hanging
   cages is the fast way up.
3. **The Rood** — herons on the rood loft across a chasm, 50 m from the entrance and in sight of
   it: get down the wax run and over a bridge (or slide-jump the chasm), climb to the loft.
   Wave 2 lands a gaoler on the far floor (25 × 31 m) between you and the loft, censers on the
   loft and gnawers coming up the wax run behind you.
4. **The Belfry** — the finale around the shaft's open mouth: a ring of floor 15 m wide to
   circle the gaolers (their chain sweep and rush can throw you into the shaft), herons and
   censers on the galleries with sightlines across the hole, harriers over the ring.

Spawn rules as in Crucible (tested): harriers' hover centres 6–8 m over the floor or ramp
below, censers in the middle of the 8 m galleries/balconies, gaolers with level floor 4 m
around, herons ≥ 20 m from and in sight of the rally point, on ledges they can walk down from.
Every walker spawn has a walkable path to its arena centre and to and from the rally point on
the nav grid (gaolers and herons included).

### Routes and movement tech

Critical path (running, jumping, ramps; no pads or slides needed; verified by the traversal
bots: **~32 s** of pure movement from every spawn, splits 2.2 / 5.1 / 16.2 / 26.8 / 31.7 s):

1. R0 → C1 east lane (hop the bell's crown) → A1 vestibule.
2. A1 west of the moulds → exit door 2 (W wall) → C2 → Hoist vestibule.
3. Hoist: across the floor to the foot of ramp 1, one turn of ramps (W wall south, S wall
   east, E wall north, N wall west) to the NW landing → exit door 4 → C3 → Rood near gallery.
4. Rood: down the wax run, over the south bridge, across the far floor, up the loft ramp along
   the E wall → exit door 6 → C4: the Wick Stair → Belfry vestibule.
5. Belfry: west ramp up to the south gallery → exit door 8 → C6 → the Lantern.

Alternate routes (all tested by bots):

- **Fallen Bell slide**: slide under the bell in C1's west lane (1.2 m) without breaking stride.
- **Casting Floor balconies** by their ramps (or the chimney, below).
- **Hoist pad line**: floor pad → hanging cage A → cage B → the NW landing, the whole spiral in
  ~5 s instead of ~11 s. The pads arc 4.5 m over each landing, so steering at the next cage
  doesn't bonk its underside.
- **Rood north bridge**.
- **Belfry pads and the gallery loop**: pads from the floor to the east and south galleries
  (5 m short of the overhang, high arcs), round the galleries and down the west ramp.

Tech (scripted with exact inputs, tested):

- **Casting Floor chimney**: wall-jump the 3.5 m gap between the north wall and the mould rack
  up onto the west balcony (7 m).
- **Slam bounce onto a bell mould**: jump, slam, bounce off the floor (~6.3 m) and land on top of
  a 5.5 m mould — a vantage point over the pit.
- **The wax run** (the long slide ramp): slide from the near gallery down the 26 m run to
  ~36 m/s and slide-jump off the lip: you clear the 14 m chasm and land on the far floor
  ~25 m out, past the bridges — straight at the herons' loft.
- **Wick Stair slide-climb**: slides climb ramps at slide speed, so sliding up the 28 m ramp
  takes 1.45 s against 2.18 s running.
- **The great bell's rim**: a dash-jump from the east gallery lands on the bell's 1.5 m rim
  over the shaft — the best view of the finale, and a 48 m drop if you overshoot.

### Par (results grading)

Bot traversal is ~32 s; the four fights are the bulk (fewer enemies than Crucible, but more
herons and three gaolers). `par_time: 300.0` (**5:00**); suggested grades: S ≤ 5:00, A ≤ 6:30,
B ≤ 8:30, C ≤ 11:00, else D. A first playthrough lands around 6–9 minutes. Kills: 66 total in
single player.

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
Every `Campaign` level listed in `CAMPAIGNS` (Crucible of Veins, Spire of Wicks) gets every
campaign check below; its routes are registered in `campaign_route` / `alternate_routes`
(a campaign level without them fails), its scripted tech in a test of its own.

- Campaign structure: 4 spawns, exit, 4–5 encounters each with a checkpoint whose spawn is
  outside the encounter trigger, lock doors start open / unlock doors start closed, every door
  used. Introduction curve: the first wave is gnawers only; harriers first appear in an arena
  ≥ 25 m tall; the first gaoler comes before the finale, in an arena ≥ 30 × 30 m; every type
  appears before the finale; the finale has all five types over ≥ 3 waves. Crucible (the first
  level) also keeps its whole first arena to gnawers and introduces censers in arena 2.
- Encounter arenas: every campaign encounter has `arena` and `rally`; the trigger and all its
  enemy spawns lie inside `arena`; `rally` is its checkpoint spawn and a valid standing spot
  (doors open and closed) inside `arena`; the arena excludes the far side of its own doors,
  other rooms' checkpoints, the player spawns and the exit; arenas don't overlap (in 3D, so
  stacked rooms are fine). Par time set for every campaign level (3–15 min), 0 for the arena.
- Arena structure: ≥ 8 spawns, ≥ 10 m apart, every quadrant, 3+ heights, no doors/encounters.
- Player/checkpoint spawns (doors open and closed): ground within 0.3 m, capsule free, head room
  for a jump, not in a kill volume, and the real movement code settles there without drifting.
- Enemy spawns: inside their locked room, not in kill volumes, ≥ 8 m from the checkpoint,
  spread out, grounded enemies stand on floor all around (no ledge overhang) with a generous
  capsule (gaoler r 1.45 × 4 m, heron 0.6 × 3.6, censer 0.7 × 2.7, gnawer 0.7 × 2.2);
  harriers in open air 4–11 m above ground with 3 m above; censers with floor at their height
  3 m out in every direction (deep ledges); gaolers with level floor 4 m out in every direction
  (room to circle them); herons ≥ 20 m from their rally point with a clear line of sight to it.
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
  path of every campaign level from all four spawns and from every checkpoint to the exit
  (doors open), never touching a kill volume, entering checkpoint/encounter triggers in order;
  five alternate routes per campaign level; a full arena lap through every ramp, catwalk,
  tunnel and pad.
- Advanced tech, scripted: Crucible's R1 perch dash-jump, R3 wall-jump shaft and spillway slide
  (> 40 m/s), the arena's wall-jump shafts; the Spire's Casting Floor chimney, slam bounce onto
  a bell mould, wax-run slide-jump over the Rood's chasm (> 33 m/s), Wick Stair slide-climb
  (< 80 % of the running time) and dash-jump onto the great bell's rim.
- Chaos: random jump/dash/slide/slam input for 40 s from every spawn and checkpoint (doors open
  and closed) never leaves the level bounds.

All of it is deterministic (same inputs, same results) and runs in well under a second.

## Level-format wishes

Already added (and used by these levels): `Encounter::arena` / `Encounter::rally` (co-op
lock-out fix), `LevelDef::par_time`, and the feet-position kill-volume contract. Still missing
(worked around or left to game code):

- **`display_name` / `description`** on `LevelDef` for the lobby.
- **Per-area environment** (fog/ambient zones) — one `Environment` per level today (the Spire
  would like warmer fog at its molten foot than at its moonlit top).
- **Door motion** (slide offset + speed) instead of popping; door sounds.
- **Wedge/prism brushes** for ramps and slopes (rotated boxes work but need care at joints).
- **Cylinders / round brushes** for bells, moulds and pillars (the Spire's bells are stepped
  boxes).
- **Per-spawn co-op spread** (`spread: f32` or "no copies") for enemies on narrow ledges.
- **Nav hints / links** for AI across jump pads. Walking enemies can't use pads, so the
  pad-only ledges (Crucible's R3 tiers, R4 corner platforms, R5 galleries; the Spire's hoist
  cages) only ever hold the censers that spawn there; herons always stand on ledges with ramps
  (R2's mezzanine, R5's dais, the Spire's rood loft and galleries) so they can walk down.
