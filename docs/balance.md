# Balance: how the weapons are measured, and what the measurements changed

The player asked: *"please spend time balancing the weapons more, they need work."* This file
is the **balance** area of the refit (`docs/refit.md`): a harness that measures every weapon,
ability and knack in the real simulation, what it found, every number and rule that changed
and why, and the tests that keep it that way. The numbers live in `assets/tuning.ron`
(`combat`: the PvE reference; `balance.pvp`: the PvP profile); `docs/arsenal.md` has the
weapon / ability / melee / knack / "Balance profiles" tables that match them.

Two passes: **phase 1** (the ten guns and their nineteen abilities, then re-checked against
**pve**'s bigger Mould hit shapes and **feel**'s projectile hit pad and bow), **phase 2** (the
Trimmer and the Tamper, Lunge / Whirl / Hurl / Upswing, the knacks). Then the rework
(`docs/rework.md`): **the shotguns pass** (its own section at the end: why the Bellows and the
Thresher felt weak in play, the fixed pellet pattern, point blank, the stagger, the feedback).
The Bellows / Thresher rows of the tables in between are from before it.

## The harness

```text
cargo run -p shared --release --example balance_report                    # all of it (~15 s)
cargo run -p shared --release --example balance_report -- pvp oneshots    # sections: primaries
                              ttk swarm pvp oneshots abilities knacks healing shotguns
cargo run -p shared --release --example balance_report -- --tuning other.ron   # another file
```

`shared/tests/balance_support/mod.rs` (shared by the example and the tests through `#[path]`)
builds a `SimWorld` on a flat 100 m floor with one god-mode shooter carrying a build and its
targets: shipped Moulds (`EnemyRegistry::builtin`: pve's hit shapes) facing the shooter (the
Gaoler also from behind), a pack of six Gnawers, or a PvP opponent. It runs `Tuning::for_mode`
(PvE: campaign, PvP: deathmatch) through `SimWorld::step` and reads the `SimEvent::Damage`
events. It measures main as it is: nothing is assumed any more (`Assume` is an empty hook for
numbers ahead of a merge).

* **Real code paths**: `step_weapons` (intervals, bursts, salvos, draws, charges, bloom,
  cooldowns, the shared ability cooldown, knack charges), pellet and spread RNG, hit assist and
  the projectile hit pad, weak points, the Gaoler's armour, PvP head multipliers, projectile
  flight and gravity, explosion falloff and line of sight, chains, splinters, pierce, burns,
  brands, patches, Limpets sticking, melee arcs and lunges, `ability_power`, Tallow Skin, Kindle
  pools, ember healing.
* **Idealised**: the aim. Every tick the crosshair is on the aim point (body centre, weak point,
  head); for anything with gravity or a lob the low ballistic arc is solved (the harness steps a
  copy of the weapon state to know what is about to fire). Targets stand still: position,
  velocity and facing are restored every tick (no knockback drift, no AI); they take damage
  without dying unless a run uses real health, so one run is a damage-over-time curve and every
  time to kill (NORMAL ×1.0, HARD ×1.3) is read off it. The shooter is held in place too, except
  while a Lunge carries it. Moving targets, dodging and cover are not measured by these: the
  numbers are a skilled player's ceiling.
* **Play-like** (the `shotguns` section, the shotguns pass): `Motion::Strafe` moves a target
  across the line of fire (5 m/s, turning back every 0.8 s: `Play::STRAFE`) and
  `Bench::aim_lag` puts the crosshair where the aim point was 0.1 s before (a good player
  tracking it); `Motion::Free` lets Moulds run their own AI at the shooter and take the real
  knockback (`blast_push`). Every hitscan trigger pull is a `Volley` (rays fired, rays landed,
  damage, whether it staggered the Mould).
* **Ranges** (feet to feet): 5 / 15 / 30 m; melee at contact (a Mould's hit radius + 1.2 m, a
  player 2 m); "at its range": scatterguns and the Kiln 5 m, melee 2 m, the rest 15 m.
* **Policies**: a primary is held (the bow: full draws); an ability is used whenever it is ready
  (charges released full, Flurry held, the Limpet: three charges thrown, all stuck, held to
  detonate; on the bow it is armed for every draw it can ride). *Build gain* = primary + ability
  minus the primary alone over 12 s: what taking the ability adds, cost included (Both Barrels,
  Volley, Whirl and Upswing use up the primary's shot or swing). *Duels* use real 100 HP and the
  opponent's knack just before the first shot.

Sections: per trigger; sustained DPS on a Gnawer (body / head); time to kill every Mould (body /
weak point; NORMAL at 5 / 15 / 30 m, HARD at 15 m); the swarm (six NORMAL Gnawers packed 1.6 m
apart, front row at 10 m, melee at contact); PvP times to kill (body / head) and with the
default build's abilities; PvP one-shots (every pull and every ability use on every carrier at
3 / 5 / 10 m, the bow at a full and a 90 % draw); every ability (per use, on a pack, spam DPS,
build gain alone and on a pack); knacks in a PvP duel; ember healing.

## What it found

| outlier | measured | why it matters |
|---|---|---|
| **Both Barrels fired on the same tick as the primary** (a mechanic bug) | FIRE + ALT together: a blast and Both Barrels in one tick, **144** to a PvP body (Thresher 108); build gain 92 (Bellows) / 129 (Thresher) against Flak's 52 | a two-button body one-shot; the dominant Scatter pick. Volley had the same shape |
| **Kiln**: the weakest primary at everything | 48 DPS, Gaoler (back) 10.8 s, swarm 2.9 s | nobody's pick |
| **Arclash** | 60 DPS, the lowest hitscan | single target it lost to everything but the Kiln |
| **Thresher** vs Bellows | first below it at every range; with pve's bigger shapes at 8 / 10° above it to 15 m (150 vs 146) | one of the two scatterguns always dominated |
| **Limpet** | 85 spam DPS (50 every 0.4 s), 380 added on a pack | three times its siblings |
| **Triple Nock** | +105 DPS to the bow | the bow's other picks: 34, 6, 0 |
| **Wisp**, **Flurry on the Lancet**, **Ember Arrow** | 15 spam DPS; 22 vs the Skewer's 45; +6 | dead weight or near it |
| **Projectiles lost weak points** to pve's 0.35 m body pad (a mechanic bug) | the Longwick at the Smelter's hatch: a body hit (1.85 → 3.85 s), the Bellwether's bell, the Gaoler's grille; the Riveter likewise | rays test plain shapes first ("padding never turns a headshot into a body hit"); projectiles didn't |
| **Volley** with feel's projectile pad | ten rivets added 42 DPS to the Riveter, its next pick 25 | the pad lands more of the cone |
| PvP **head one-shots** outside the rules | a Bellows blast at 5 m 106, Both Barrels 154, Triple Nock at a 90 % draw 144, Ember Arrow at 90 % 124 | only full-draw arrows, the Slug and a full Skewer may |
| **Hurl scored headshots** (a bug against "melee has no weak points") | the Tamper's Hurl to a PvP head: **180** | a one-shot |
| **Whirl / Upswing on the same tick as a swing** | Tamper swing + Whirl: **132** at 2 m in one tick | the Both Barrels bug again |
| **Melee in PvE** | Trimmer 96 / Tamper 90 DPS at contact, no weak points | below the scatterguns at 5 m (132 / 142), from closer in |
| **Tallow Skin in PvP** | half damage for 3 s: every time to kill ×1.9 on average (Tumbler 1.32 → 3.02 s) | it won every even duel: a must-pick |

Looked at and kept: explosives against big Moulds (5–7 s on a Smelter / the Gaoler's back,
three times the precision weapons on weak points; they have no weak points by design and clear a
pack in 0.3–0.4 s); melee healing (below); the Lunge's lift (4.5 m/s: it carries the body some
5 m and connects across the 4.5 m gap the harness tests); the bow after feel's handling (below).

## Changes

PvE numbers are `combat` (the reference; the PvE profile stays empty). Where a PvE number moved
but PvP was right, the PvP profile holds the old PvP number.

| what | before → after | why |
|---|---|---|
| **Both Barrels, Volley, Whirl, Upswing** (`weapons.rs`, `takes_primary_shot`) | fired as well as the primary on the same tick → wait for the primary to be ready and **take its shot / swing** | the barrels / the weapon can't fire twice at once; removes the 144 and 132 two-button PvP kills. Unit test + measured (PvP test) |
| **Projectile weak points** (`hitbox.rs`, `sweep_sphere_padded`) | a padded body contact counted as the body → a padded body contact whose centre line meets a plain weak point first is that weak point | the rays' rule; restores the Longwick's and the Riveter's weak points on the Smelter, Bellwether and Gaoler. Unit test + `projectiles_score_weak_points_like_rays` |
| **Hurl** (`projectiles.rs`, `projectile_direct_damage`) | struck the hit part (heads ×2, weak points) → the body | "melee ignores weak points"; the 180 PvP one-shot. Tested in `combat_melee.rs` |
| `bellows.both_cooldown` | 1.4 → 1.8 s | gain 92 / 129 → 43 / 64 (Slug 46, Flak 63) |
| `kiln.damage`, `cone_degrees`; `burn.dps` | 3 → 4, 16° → 20°; 4 → 5 a stack | 48 → 63 DPS; swarm 2.9 → 0.75 s; Gaoler (back) 10.8 → 8.3 s |
| `arclash.zap_damage` | 6 → 7 | 60 → 70 DPS |
| `thresher.pellet_damage` | 7 → 7.5 (cone stays 11°) | 142 DPS at 5 m (the class's close-range best), 127 at 15 m (the Bellows 146): each scattergun wins somewhere |
| `tumbler.bomb_damage` | 65 → 72 | 76 → 84 DPS: the heavier explosive beats the Cinderbox single target |
| `limpet.damage`, `cooldown` | 50 → 40, 0.4 → 0.9 s | 85 → 39 spam DPS, pack gain 380 → 170 |
| `triple_nock.damage_scale` | 1.0 → 0.6 | gain 105 → 49 (Brand 40) |
| `wisp_zap_damage`, `wisp_pop_damage`, `wisp_cooldown` | 8 → 11, 25 → 30, 2.8 → 2.4 s | 15 → 27 spam DPS |
| `flurry.damage_scale`, `cooldown` | 0.8 → 1.0, 1.6 → 1.4 s | Lancet 22 → 32, Quill 38 → 54 |
| `ember_arrow.damage_scale`, `damage` | 0.75 → 0.9, 30 → 36 | gain 6 → 16, 91 on a pack (18 a burst in PvP) |
| `volley.damage_scale` | 1.0 → 0.85 | with the pad: Riveter 42 → 33 (Wisp 25) |
| `trimmer.damage`, `tamper.damage` | 30 → 36, 75 → 85 | 96 / 90 → 115 / 102 DPS at contact: worth carrying against the scatterguns' 132–142 at 5 m |
| `whirl.damage_scale`, `upswing.damage_scale` | 1.2 → 1.5, 0.8 → 1.4 | they replace a swing now: a little more than it, plus the spin / the launch |
| PvP `Bellows` | 0.7 → 0.6 (4.8 a pellet) | head blast at 5 m 106 → 91 (115 inside 5 m: the point-blank reward, a stated exception); still two body blasts, 0.8 s |
| PvP `BothBarrels` | 0.4 → 0.25 | 48 body / 96 head (Thresher 43 / 86) |
| PvP `TripleNock`, `EmberArrow` | 0.5 → 0.55, — → 0.5 | 90 % draw to the head: 144 → 96, 124 → 89 |
| PvP `Volley` | 0.7 → 0.75 (× 0.85) | ten to the head 98 → 89 |
| PvP `Trimmer`, `Tamper` | — → 0.84, 0.8 → 0.7 | the same 30 / 59.5 as before the PvE raise: 0.95 / 0.85 s; the Tamper's Lunge / Whirl / Hurl 89, Upswing 83 |
| PvP `Thresher`, `Kiln`, `Tumbler`, `Wisp`, `Flurry`, `Limpet` | 0.8 → 0.75, 1.2 → 0.9, — → 0.9, 0.7 → 0.5, 0.75 → 0.6, 0.6 → 0.75 | hold the PvP numbers where the PvE base moved |
| PvP `Arclash` | follows PvE (7 a zap) | 1.6 → 1.4 s: the tracking beam was the slowest hitscan |
| PvP overrides `knacks.tallow_skin.damage_scale`, `duration` | 0.5 for 3 s → 0.7 for 2 s | average slowdown ×1.9 → ×1.55: a strong defensive pick, not a must-pick |
| one-shot rule | + "a Bellows blast to the head inside 5 m" | with 12 pellets and heads ×2 a two-blast body kill always means ≥ 100 when every pellet lands on a head; the alternative was a three-blast (1.6 s) Bellows |

Feel's bow (0.62 s draw, 55–135 m/s, gravity 10) was re-checked, not changed: 116 DPS body /
231 head in PvE (105 / 210 before); PvP 1.30 s at 5 m, 1.38 s at 15 m with the flight; a
full-draw headshot 105 (the allowed one-shot), a 90 % draw 97 (Triple Nock 96, Ember Arrow 89).

## The numbers now

Measured on main (pve's shapes, feel's pad and bow). "Before" columns are `221b596` (old
shapes): not every difference is this pass's.

### Primaries, PvE (DPS on a Gnawer, body / head)

| weapon | 5 m before → after | 15 m before → after | 30 m after |
|---|---|---|---|
| Quill | 86 / 173 → 86 / 173 | 86 / 173 → 86 / 173 | 86 / 173 |
| Bellows | 125 / 250 → 132 / 250 | 102 / 87 → 146 / 144 | 58 / 61 |
| Arclash | 60 / 120 → **70 / 140** | 60 / 120 → **70 / 140** | 70 / 140 |
| Tumbler | 76 → **84** | 76 → **84** | 77 |
| Lancet | 94 / 188 → 94 / 188 | 94 / 188 → 94 / 188 | 106 / 181 |
| Thresher | 118 / 223 → **142 / 252** | 55 / 54 → **127 / 92** | 33 / 29 |
| Riveter | 84 / 168 → 84 / 168 | 83 / 167 → 83 / 167 | 95 / 164 |
| Cinderbox | 74 → 74 | 74 → 74 | 74 |
| Longwick | 105 / 210 → 116 / 231 | 105 / 210 → 116 / 231 | 116 / 231 |
| Kiln | 48 → **63** | — | — |
| Trimmer (contact) | 96 → **115** | | |
| Tamper (contact) | 90 → **102** | | |

### Time to kill, NORMAL, 15 m (s, body / weak point; melee at contact)

| weapon | Gnawer 45 | Censer 80 | Harrier 40 | Snuffer 70 | Heron 60 | Bellwether 220 | Smelter 420 | Gaoler front | Gaoler back |
|---|---|---|---|---|---|---|---|---|---|
| Quill | 0.28 / 0 | 0.85 / 0.28 | 0.28 / 0 | 0.28 / 0.28 | 0.57 / 0 | 2.55 / 1.13 | 1.70 / 1.70 | — / 3.97 | 5.95 / 2.27 |
| Bellows | 0 / 0 | 0 / 0.80 | 0 / 0 | 0 / 0 | 0 / 0 | 1.60 / 1.60 | 2.40 / 2.40 | — / 8.80 | 3.20 / 2.40 |
| Arclash | 0.60 / 0.30 | 1.10 / 0.50 | 0.50 / 0.20 | 0.30 / 0.30 | 0.80 / 0.30 | 3.10 / 1.50 | 2.30 / 2.30 | — / 4.90 | 7.40 / 2.90 |
| Tumbler | 0.45 | 1.30 | 0.48 | 1.27 | 0.45 | 2.98 | 5.50 | — | 6.38 |
| Lancet | 0.45 / 0.15 | 0.75 / 0.30 | 0.30 / 0.15 | 0.15 / 0.15 | 0.60 / 0.15 | 2.25 / 1.20 | 1.65 / 1.65 | — / 3.60 | 5.55 / 2.10 |
| Thresher | 0 / 0.42 | 0.42 / 1.67 | 0 / 0 | 0.42 / 0.42 | 0.42 / 0.83 | 1.67 / 2.50 | 2.92 / 2.92 | — / 12.9 | 2.92 / 2.92 |
| Riveter | 0.63 / 0.38 | 1.05 / 0.57 | 0.57 / 0.32 | 0.38 / 0.38 | 0.80 / 0.38 | 2.72 / 1.38 | 2.05 / 2.05 | — / 4.22 | 6.30 / 2.55 |
| Cinderbox | 0.38 | 0.47 | 0.40 | 0.45 | 0.38 | 2.98 | 5.57 | — | 6.87 |
| Longwick | 0.70 | 0.70 / 0.72 | 0.72 | 0.70 | 0.70 | 2.43 / 1.57 | 1.57 | — / 3.30 | 4.17 / 1.57 |
| Kiln | — | — | — | 1.08 | — | — | 6.67 | — | — (5 m: 8.25) |
| Trimmer | 0.32 | 0.63 | — | 0.32 | 0.32 | 1.90 | 3.48 | — | 4.43 |
| Tamper | 0 | 0 | — | 0 | 0 | 1.70 | 3.40 | — | 5.10 |

"—": over 20 s or out of reach. The Gaoler's front is armoured (×0.15, explosions too): flank
it, parry its bash, or shoot its grille. Melee can't reach a hovering Harrier (it comes down to
dive); a melee build's Hurl kills it in one throw. The Kiln at 15 m reaches only the Moulds
whose hit shapes stick out (the Snuffer, the Smelter's crucible).

### Swarm: six NORMAL Gnawers, front row at 10 m (melee at contact; s until the last falls)

| weapon | primary | + ability A | + ability B |
|---|---|---|---|
| Quill | 3.12 | 3.12 (Brand) | 1.98 (Splinter) |
| Bellows | 1.60 | 1.60 (Gust) | 1.60 (Both Barrels) |
| Arclash | 1.80 | 1.40 (Wisp) | 1.80 (Discharge) |
| Tumbler | 0.27 | 0.27 (Breach) | 0.40 (Gravewell) |
| Lancet | 3.45 | 2.55 (Skewer) | 2.55 (Flurry) |
| Thresher | 2.08 | 1.25 (Slug) | 1.25 (Flak) |
| Riveter | 3.68 | 2.93 (Volley) | 2.77 (Wisp) |
| Cinderbox | 0.38 | 0.38 (Limpet) | 0.32 (Mortar) |
| Longwick | 5.02 | 3.28 (Triple Nock) | 5.02 (Pinning Arrow) |
| Kiln | 0.75 | 0.83 (Flare) | 0.75 (Discharge) |
| Trimmer | 0.95 | 0.95 (Lunge) | 0.95 (Hurl) |
| Tamper | 0 (one swing) | 0 (Whirl) | 0 (Upswing) |

Before: the Kiln 2.92, the Arclash 2.00, the Tumbler 1.20 (one bomb clears it since 72: a
breakpoint that moves with health), the Trimmer 0.95, the Tamper one swing.

### Abilities (build gain DPS: one Gnawer / six)

| class | ability (carrier) | before | after |
|---|---|---|---|
| Precision | Brand (Quill / Lancet) | 35 / 37 | 35 / 37 |
| | Splinter (Quill) | 18, pack 68 | 18, pack 75 |
| | Skewer | 45, pack 81 | 45, pack 81 |
| | Flurry (Lancet / Quill) | 22 / 38 | **32 / 54** |
| Scatter | Gust | 4, pack 24 | 4, pack 24 |
| | Both Barrels (Bellows / Thresher) | **92 / 129** | **43 / 64** |
| | Slug | 46 | 46 |
| | Flak | 52 | 63 |
| Rapid | Wisp (Arclash) | 15, pack 24 | **28, pack 32** |
| | Discharge (Arclash, 4 m) | 27, pack 160 | 27, pack 160 |
| | Volley (Riveter / Arclash) | 34 / 31 | 33 / 28 (pack 63) |
| | Flare (Arclash / Kiln) | 9 / −1, pack 59 / 18 | 13 / 0, pack 80 / −7 |
| Explosive | Breach (Tumbler, 4 m) | 54, pack 249 | 56, pack 269 |
| | Gravewell | 27, pack 133 | 27, pack 146 |
| | Limpet | **85, pack 380** | **39, pack 170** |
| | Mortar | 25, pack 131 | 25–28, pack 130–140 |
| Draw | Brand | 34 | 40 |
| | Triple Nock | **105** | **49** |
| | Pinning Arrow | 0 (the root is the point) | 0 |
| | Ember Arrow | 6, pack 58 | **16, pack 91** |
| Melee | Lunge (Trimmer / Tamper; a 4.5 m gap) | 18 / 38 | 21 / 43 |
| | Hurl (Trimmer / Tamper; 10 m) | 19 / 47 | 23 / 53 |
| | Whirl (Tamper / Trimmer) | 23 / 9, pack 135 / 54 | 11 / 5, pack 64 / 54 |
| | Upswing (Tamper / Trimmer) | 20 / 8, pack 120 / 48 | 11 / 5, pack 68 / 65 |

Whirl and Upswing now take the swing's place: their damage over it is small on one target;
the spin (everything around you, knocked away) and the launch are what they are for. On every
weapon the best single-target pick adds at most 1.5 × the second best.

### PvP (100 HP)

| weapon | per pull, body / head | TTK body at its range | + ability A / B (default build) |
|---|---|---|---|
| Quill | 24 / 48 | 1.13 (15 m) | 0.85 (Brand) / 0.85 (Splinter) |
| Bellows | 58 / 91 (115 at 3 m) | 0.80 (5 m) | 0.80 / 0.80 |
| Arclash | 7 / 14 | 1.40 (15 m) | 1.20 (Wisp) / 1.40 |
| Tumbler | 63 / 63 | 1.32 (15 m) | 1.32 / 1.32 |
| Lancet | 12 / 24 | 1.20 (15 m) | 0.80 (Skewer) / 0.90 (Flurry) |
| Thresher | 39 / 28 | 0.83 (5 m) | 0.42 (Slug) / 0.42 (Flak) |
| Riveter | 7 / 14 | 1.32 (15 m) | 0.82 (Volley) / 1.15 (Wisp) |
| Cinderbox | 69 / 69 | 1.70 (15 m) | 1.70 / 0.53 (Mortar) |
| Longwick | 52.5 / 105 (90 %: 48.6 / 97) | 1.38 (15 m) | 2.88 (Triple Nock) / 1.38 |
| Kiln | 17 (with its burn) | 1.75 (5 m) | 1.83 (Flare) / 1.00 (Discharge) |
| Trimmer | 30 | 0.95 (2 m) | 0.32 (Lunge) / 0.32 (Hurl) |
| Tamper | 59.5 | 0.85 (2 m) | 0.85 (Whirl) / 0.85 (Upswing) |

One-shots before → after (5 m, head): Bellows 106 → 91, Both Barrels 154 → 96 (Thresher 138 →
86), Triple Nock at 90 % 144 → 96, Ember Arrow at 90 % 124 → 89, Volley 98 → 89, the Tamper's
Hurl 180 → 89, a blast + Both Barrels on one tick 144 → impossible, a swing + Whirl 132 →
impossible. Allowed and kept: a full-draw arrow 105 (Triple Nock's three 104), the Slug 110, a
full Skewer 104.

**Combos.** An ability is a second action on its own cooldown, so the one-shot rules hold per
pull and per use, not per pair: the default builds' fastest kills are Thresher + Slug or Flak
0.42 s at 5 m, the Trimmer's Lunge or Hurl then two slashes 0.32 s, Cinderbox + Mortar 0.53 s.

**The Trimmer as gun game's finale.** Four slashes (0.95 s) from 3 m, no headshots, a forgiving
70° arc; the Lunge closes some 5 m and strikes for 45, the Hurl reaches 45 from range. A kill
needs closing the distance on someone holding a gun (0.8–1.4 s at their range): achievable,
not trivial. Left as arsenal built it in PvP.

### Knacks

PvP duel (real 100 HP; the opponent's knack just before the first shot; s to the kill):

| attacker | no knack | Tallow Skin (PvP: 0.7, 2 s) | Kindle Flask |
|---|---|---|---|
| Quill | 1.13 | 1.42 | 1.13 |
| Bellows | 0.80 | 1.60 | 0.80 |
| Arclash | 1.40 | 2.00 | 1.60 |
| Tumbler | 1.32 | 2.17 | 1.32 |
| Lancet | 1.20 | 1.80 | 1.35 |
| Thresher | 0.83 | 1.25 | 0.83 |
| Riveter | 1.32 | 1.82 | 1.48 |
| Cinderbox | 1.70 | 2.92 | 1.70 |
| Longwick | 1.38 | 2.13 | 2.13 |
| Kiln | 1.75 | 2.25 | 2.00 |
| Trimmer | 0.95 | 1.27 | 0.95 |
| Tamper | 0.85 | 1.70 | 0.85 |

At half damage for 3 s Tallow Skin made every kill ×1.9 on average (Tumbler 3.02 s, Longwick
3.30); now ×1.55. Two-hit weapons still need a third hit through it (Bellows, Tamper, the
Cinderbox's salvos): that is what the knack is for. The Kindle Flask (40 HP over 4 s, the
thrower only in PvP) matters against the steady, slower killers (the bow's second draw, the
Kiln, the Arclash) and between fights. Surge (two charges of mobility), the Snare (15 + a slow)
and the Soot Pot (cover that hides players; in PvE it blinds Moulds) act outside a standing
duel. In PvE the knacks keep their numbers: Tallow Skin's half damage for 3 s is for a Gaoler's
bash or a Smelter's heave, the flask heals the whole team in co-op. The Thorn without a Quill is
8 damage a throw: it stays the default for the Quill's ricochets; other builds should pick.

### Healing from embers (PvE)

HP per second the shooter draws in, held 40 HP down, holding the primary 10 s:

| weapon | one Gnawer | six |
|---|---|---|
| guns at their range (5–15 m) | 0 | 0 |
| Trimmer (contact) | 58 | 173 |
| Tamper (contact) | 51 | 306 |

Embers are drawn by the closest Shrike within 3.5 m of the hit: only melee (and guns used up
close) heal from their own hits. `embers.melee_weapon_heal_fraction` 0.5 against a gun's 0.4:
the rate on one target is a close-range scattergun's (0.4 × ~130), and on a pack it is capped by
what the pack is worth (the Tamper's six-Gnawer rate is against endless targets: a real pack
dies in one swing) and by hard damage (35 % of every hit taken can't be healed until it
recovers). Kept at 0.5.

## Locked in by tests

* `shared/tests/combat_balance.rs` (PvE): every primary sustains **60–150 DPS** on a Gnawer
  body at its range (the scatterguns above it: the shotguns pass below has their tests); every
  Mould dies to every primary at NORMAL within **9 s** (200+ HP) /
  **1.5 s** (the rest), best of body and weak point, melee at contact (the Hurl for a hovering
  Harrier), the Gaoler from behind; on every weapon the best ability adds at most **1.5 ×** the
  second best (single target), none adds more than **300 DPS** on a pack, every damage ability
  adds at least **10 DPS** somewhere (Gust and Pinning Arrow are utility); arrows score weak
  points like Quill rays on every Mould; the shipped `balance` section is
  `BalanceTuning::default()`.
* `shared/tests/combat_arsenal.rs`: the analytic PvP test; `pvp_one_shots_and_times_to_kill_
  measured_in_play` (every pull and ability use on every carrier, melee included, at 3 / 5 /
  10 m, body and head — a full Bellows blast to the body kills at 3 m and only there, the
  shotguns pass's exception — the bow at a full and a 90 % draw; both buttons on one tick for Both
  Barrels, Volley, Whirl and Upswing; every primary's all-body time to kill at its range inside
  0.8–1.8 s); `pvp_knacks_are_choices_not_a_must_pick` (Tallow Skin at most ×1.6 on average, the
  flask changes some duel).
* `shared/tests/combat_melee.rs` (arsenal's, numbers now read from the tuning): the swings, the
  abilities, the Hurl striking the body, the melee PvP limits. Unit tests:
  `weapons::both_barrels_also_holds_the_primary`,
  `hitbox::a_padded_body_never_steals_a_projectiles_weak_point`.

## Known limits, left for play-testing

* **Flare on the Kiln** adds nothing where the Kiln already reaches (its cone keeps everything at
  the Burn cap); it is for what the cone can't reach. On the Arclash / Riveter: +13, pack +80.
* **PvP combos** (above): mostly Scatter. Cutting them would cost the Slug its headshot one-shot
  or the Flak its contact burst.
* **Breakpoints** (one bomb or one swing clearing a pack, two vs three hits through Tallow Skin)
  move with difficulty, player count and grind waves; the harness reports NORMAL / HARD solo.
* Stationary targets and perfect aim favour projectiles and lobs; against moving Moulds hitscan
  gains. Not measured: thorn ricochets, knockback and crowd control (Gust, Gravewell's pull,
  Pinning Arrow, Whirl, Upswing's launch, the Tamper's knockback), parries and the Trimmer's
  deflects, Surge / Snare / Soot Pot.
* `docs/roster.md`, `docs/gameplay.md` and `docs/expansion.md` still quote some old starting
  values; `docs/arsenal.md` and `assets/tuning.ron` are the reference.

## The shotguns pass (the rework, `docs/rework.md` §5)

The player (2026-10-28): *"The shotguns are severely underpowered, so buff them."* The harness
disagreed: on paper the Bellows and the Thresher were already the top primaries at 5 m (132 /
142 DPS on a Gnawer's body; the Longwick 116, the Trimmer 115 at contact). The report's
`shotguns` section (`Play`, `Motion`, `Volley`, `blast_push` in the harness) measures what a
standing target and a perfect aim hid.

### Why they felt weak

| finding | measured before | why it matters |
|---|---|---|
| **Every blast was a dice roll**: each pellet flew uniformly at random over the cone's disc (half of them in its outer 30 %) | a Mould juking at 5 m/s, the crosshair 0.1 s behind it: the Thresher's worst blast on a Censer at 8 m landed 4 of 7 pellets (30 of 52.5), on a Heron at 12 m 2 of 7 (15); the Bellows' worst on a Heron at 12 m 6 of 12 (48 of 96), on a Censer 7 | the breakpoints you feel became luck: a juking Censer at 12 m took one or two Bellows blasts, two or three of the Thresher's, a Snuffer up to three of the Thresher's |
| **Nothing reacted to a blast**: a stagger needs one hit's knockback above 4 m/s (`ai::STAGGER_KNOCKBACK`, after `knockback_scale`), and a pellet pushed 2.0 (Bellows) / 1.5 (Thresher) × at most 1.5 | 0 % of the blasts staggered anything. A Gnawer running in, all 12 pellets at 8 m: shoved 3.1 m back without losing its footing, running at you again 0.40 s later (the Thresher's 7: 0.1 m, 0.13 s) | a blast that lands everything looked like it did nothing |
| **On a moving Mould the "top DPS" tied the bow's body shots** | juking at 8 m, aimed at the body: Bellows 127 (Gnawer) / 120 (Censer) / 114 (Heron), Thresher 145 / 125 / 113, Longwick 118, Lancet 94. At the head the precision weapons keep their ×2 / ×2.5 while pellets spill off it: Longwick 236 / 236 / 295 against the Bellows' 161 / 129 / 138 | the close-range weapon lost to an arrow at the head from the range it should own |
| **The blast didn't read** | one body marker for twelve pellets, the size of a Quill needle's (the confirm merges a tick's hits on a target, so the number did show the total); the view punch of a bow release (kick 1.0, squared); a breathy −11 LU "whumpf" | nothing said "that hit hard" |
| Not the cause | hit assist (pellets test the same padded shapes as every ray: 0.35 m body / 0.15 m weak point on a Mould), damage falloff (there is none), the damage number (the blast's total) | |

### Changes

| what | before → after | why |
|---|---|---|
| **Fixed pellet pattern** (`shared/src/sim/scatter.rs`: `pellet_pattern`, `pellet_dirs`) | uniformly random over the cone → a centre pellet, an inner ring at half the cone (Bellows 5, Thresher 3) and the outer ring on its edge (6 / 4), each pellet moved at most 0.5° / 0.6° (`pattern_jitter_degrees`) | a good aim lands the same pellets every time. The jitter is seeded by the input's `seq` alone, so the client's tracers (`weapon_feel::hitscan_dirs`) are exactly the server's pellets |
| **Stagger** (`scatter::blast_stagger`; `stagger_fraction`, `stagger_time`) | never → a blast landing at least 4 of the Bellows' 12 / 3 of the Thresher's 8 pellets on a light Mould (base health below `pinning_arrow.heavy_health`, 300) staggers it 0.35 s / 0.2 s | "a blast that connects staggers a light Mould"; the Smelter and the Gaoler shrug it off |
| `bellows.pellet_knockback`, `thresher.pellet_knockback` | 2.0 → 1.3, 1.5 → 0.7 | a staggered Mould stops steering and brakes slowly: at 2.0 a standing Gnawer would fly ~10 m, out of the shotgun's range. A charging one is now stopped and shoved 2 m back |
| **Point blank** (`hitscan::PointBlank` on the pellets' `ShotSpec`; `point_blank_range` / `_end` / `_multiplier`) | — → the Bellows' pellets hitting within 3 m deal ×1.25, easing back to ×1 at 4 m (the Thresher: ×1) | the jam-it-in-their-face reward in PvE; in PvP it carries the one-shot exception (below) without touching 5 m |
| `bellows.pellet_damage` | 8 → 11 (96 → 132 a blast) | ×1.375 a blast; 5 m DPS 132 → 199 (the 5 m Gnawer's pellets land at 3.9 m: a little point blank) |
| `thresher.pellets`, `pellet_damage`, `cone_degrees` | 7 × 7.5 → 8 × 10 (52.5 → 80 a blast), 11° → 10° | 5 m DPS 142 → 213; eight make a rounder pattern (1 + 3 + 4), the tighter cone keeps its outer ring on a Gnawer to 12 m |
| PvP `Bellows` | damage 0.6 → 0.52 (5.72 a pellet); overrides `"bellows.point_blank_multiplier": 1.6`, `"bellows.pellet_knockback": 2.0` | 69 a blast: still two at 5 m (0.8 s) and three through Tallow Skin (at 0.55, 73, two broke through), 80 to the head at 5 m (was 91); a full blast inside 3 m 110 (11 of its 12 pellets still kill) |
| PvP `Thresher` | damage 0.75 → 0.49 (4.9 a pellet); override `"thresher.pellet_knockback": 1.5` | 39 a blast as before: three on the body (0.83 s), two on the head up close (74 a blast at 3 m; at 5 m 49, was 28). Thresher + Flak stays 0.42 s (a blast and a contact ring stay under 100) |
| **one-shot rule** | + a full Bellows blast to the body at point blank (≤ 3 m) | the contract's stated exception; from 5 m on no Bellows blast kills, body or head |
| hit marker (client: `hud::blast_marker_scale`, `CombatHud::hit_marker_scaled`) | always ×1 → ×1 up to half a blast's pellets on one target, growing to ×1.7 (longer, thicker strokes, a harder pop) for all of them | a mostly-landed blast reads as one |
| view punch / shake (client: `AttackStyle::punch`; `CameraRig::punch` takes 0..2) | Bellows 1.0 / 0.22 → 2.0 / 0.3 (~0.7° → 1.4°), Thresher 0.81 / 0.18 → 1.3 / 0.22, the Thresher's viewmodel kick 0.9 → 1.0 | more kick |
| blast sounds (`engine/src/audio/sfx.rs`, `docs/audio.md`) | `BellowsBlast` −11 → −10 LU: a deeper core (weight 1.15), the pellets crunching out, a clean 90→30 Hz sub under the distortion. `ThresherFire` −12 → −11: core 0.8 → 0.95, a short clean sub | heavier, the Thresher still quick enough for every 0.42 s |

### The numbers, before → after

`cargo run -p shared --release --example balance_report -- primaries shotguns pvp oneshots`.

**Primaries on a Gnawer** (standing, the crosshair on it; DPS body / head). The next best
primary: the Longwick 116 / 231, the Trimmer 115 at contact.

| weapon | 5 m | 15 m | 30 m |
|---|---|---|---|
| Bellows | 132 / 250 → **199 / 352** (×1.51) | 146 / 144 → 210 / 196 | 58 / 61 → 94 / 87 |
| Thresher | 142 / 252 → **213 / 384** (×1.50) | 127 / 92 → 197 / 178 | 33 / 29 → 94 / 74 |

**Juking Moulds** (5 m/s, turning every 0.8 s, the crosshair 0.1 s behind; DPS body / head;
endless). Body to body, both scatterguns out-damage every other primary on all three Moulds to
12 m; on the Gnawer (what the test checks) at the head too. At a small weak point 8 m away (the
Censer's lantern, the Heron's ×2.5 head) a bow that lands every arrow on it still deals more
than a blast aimed there: the precision weapons' reward, not the scatterguns' job.

| weapon | Gnawer 5 m | Gnawer 8 m | Censer 8 m | Heron 8 m | Gnawer 12 m | Censer 12 m | Heron 12 m |
|---|---|---|---|---|---|---|---|
| Bellows before | 120 / 194 | 127 / 161 | 120 / 129 | 114 / 138 | 122 / 131 | 104 / 106 | 93 / 103 |
| Bellows after | **168 / 284** | **180 / 236** | **165 / 210** | **153 / 233** | 170 / 188 | 153 / 164 | 147 / 153 |
| Thresher before | 138 / 205 | 145 / 165 | 125 / 132 | 113 / 141 | 127 / 115 | 103 / 99 | 96 / 99 |
| Thresher after | **216 / 331** | **230 / 296** | **200 / 224** | **174 / 251** | 209 / 180 | 156 / 140 | 149 / 158 |
| Longwick | 118 / 236 | 118 / 236 | 118 / 236 | 118 / 295 | 118 / 223 | 118 / 210 | 118 / 33 |
| Lancet | 94 / 182 | 94 / 175 | 94 / 189 | 94 / 210 | 94 / 170 | 94 / 173 | 94 / 171 |
| Quill | 87 / 174 | 87 / 174 | 87 / 174 | 87 / 218 | 87 / 174 | 87 / 174 | 87 / 218 |

**Blast by blast** on the same juking Moulds, aimed at the body: the worst blast (pellets
landed, damage), blasts to kill one at NORMAL (most / mean over 12 starts), the blasts that
staggered it.

| Mould, range | Bellows before → after | Thresher before → after |
|---|---|---|
| Gnawer, 12 m | 9 of 12, 80; 1 / 1.0 → 11 of 12, 132; 1 / 1.0 | 4 of 7, 30; 2 / 1.2 → 6 of 8, 70; 1 / 1.0 |
| Censer, 8 m | 12 of 12, 96; 1 / 1.0 → 12 of 12, 132; 1 / 1.0 | 4 of 7, 30; 2 / 2.0 → 8 of 8, 80; 1 / 1.0 |
| Censer, 12 m | 7 of 12, 56; 2 / 1.1 → 11 of 12, 121; 1 / 1.0 | 4 of 7, 30; 3 / 2.4 → 6 of 8, 60; 2 / 2.0 |
| Snuffer, 12 m | 11 of 12, 100; 1 / 1.0 → 11 of 12, 170; 1 / 1.0 | 3 of 7, 22.5; 3 / 2.1 → 6 of 8, 75; 1 / 1.0 |
| Heron, 8 m | 10 of 12, 80; 1 / 1.0 → 11 of 12, 121; 1 / 1.0 | 4 of 7, 30; 2 / 2.0 → 6 of 8, 60; 1 / 1.0 |
| Heron, 12 m | 6 of 12, 48; 2 / 1.1 → 10 of 12, 110; 1 / 1.0 | 2 of 7, 15; 3 / 2.1 → 5 of 8, 50; 2 / 1.5 |
| Bellwether, 8 m | 12 of 12, 96; 3 / 3.0 → 12 of 12, 132; 2 / 2.0 | 7 of 7, 52.5; 5 / 5.0 → 8 of 8, 80; 3 / 3.0 |
| staggered | 0 % → 100 % of the blasts, every light Mould | 0 % → 100 % |

**One blast at a Mould running in** (its own AI, endless; at 8 m): pushed back / staggered /
running at you again.

| Mould | Bellows before → after | Thresher before → after |
|---|---|---|
| Gnawer | 3.1 m, —, 0.40 s → 2.0 m, 0.32 s, 0.45 s | 0.1 m, —, 0.13 s → 0.0 m, 0.17 s, 0.25 s |
| Snuffer | 4.9 m, —, 0.58 s → 4.2 m, 0.32 s, 0.58 s | 0.5 m, —, 0.58 s → 0.1 m, 0.17 s, 0.58 s |
| Smelter, Gaoler | not moved, not staggered (heavy) | not moved, not staggered |

**Other PvE tables.** Time to kill at 15 m, NORMAL (body / weak point): the Bellows' Bellwether
1.60 → 0.80, Smelter 2.40 → 1.60, Gaoler from behind 3.20 / 2.40 → 1.60 / 1.60; the
Thresher's Bellwether 1.67 / 2.50 → 0.83 / 0.83, Smelter 2.92 → 1.67, Gaoler from behind 2.92 →
2.08 / 1.67. The swarm (six Gnawers, 10 m): Bellows 1.60 → 1.60 (three blasts: the 8° pattern
covers one or two of a pack 1.6 m apart), Thresher 2.08 → 1.25. Build gains: Flak 63, Slug 46, Gust 4 as before; Both
Barrels (on its way out) 43 / 64 → 23 / 48 (it takes the primary's now stronger shot).

**PvP** (100 HP; per blast body / head, time to kill holding the trigger):

| | before | after |
|---|---|---|
| Bellows at 3 / 5 / 10 m | 58 / **115**, 58 / 91, 48 / 24 | **110** / **220**, 69 / 80, 46 / 34 |
| Bellows time to kill, 5 m | 0.80 (two blasts) | 0.80 |
| Thresher at 3 / 5 / 10 m | 39 / 79, 39 / 28, 28 / 11 | 39 / 74, 39 / 49, 20 / 15 |
| Thresher time to kill body / head, 5 m; 15 m | 0.83 / 1.25; 5.00 / 5.00 | 0.83 / 0.83; 2.50 / 3.75 |
| through Tallow Skin (duel, 5 m) | Bellows 1.60, Thresher 1.25 | 1.60, 1.25 (at 0.55 a Bellows blast broke through in two: 0.52 keeps the third) |
| with their abilities (5 m) | Thresher + Slug / Flak 0.42, Bellows 0.80 | the same |

The tight pattern costs the Thresher at 10 m in PvP (its outer ring misses a player's body: 28
→ 20 a blast) and helps it at 15 m (the centre stays on: 5.0 → 2.5 s); its job is up close.

### Tests

* `shared/tests/combat_balance.rs`: `scatterguns_are_the_strongest_primaries_up_close` (≥ 1.35 ×
  132 / 142 at 5 m; on a Gnawer at 5 and 8 m, standing and juking, body and head, above every
  other primary aimed the same way, melee at contact), `a_connecting_blast_staggers_a_light_mould`
  (one blast from 8 m: every light Mould staggered, the Smelter and the Gaoler not),
  `blasts_land_most_of_their_pellets_every_time` (juking Gnawer, Censer, Heron: every blast at
  least ¾ of its pellets at 8 m, ½ at 12 m); `every_primary_sustains_damage_inside_the_band` lets
  the scatterguns past 150.
* `shared/tests/combat_arsenal.rs`: the analytic PvP test (a Bellows blast × its point blank
  kills; the Thresher three body blasts, not two, two head blasts, not one) and the in-play one
  (a full Bellows blast to the body kills at 3 m and only there); the Thresher's pellet count read
  from the tuning.
* `shared/tests/combat_weapons.rs`: `bellows_blast_and_both_barrels` reads its numbers from the
  tuning, `bellows_point_blank_hits_harder` (every pellet at 2 m × the multiplier).
* Unit: `scatter::tests` (the pattern's rings, the jitter bounded and seeded by `seq`),
  `hud::tests::a_mostly_landed_blast_draws_a_bigger_marker`, the camera's punch test (a blast
  punches about twice a full kick, under the cap).

### Left open

* **PvP tracking** isn't modelled: a constant 0.1 s lag against a player strafing at 8–16 m/s
  makes every hitscan miss most of the time (the Quill landed 28 % at 8 m/s), the Bellows' cone
  forgiving it most (88 % inside 8 m). Real players lead; play-test it.
* The stagger has no animation of its own: it shows as the Mould losing its footing (no
  steering, the blast's push carrying it). A flinch pose would be the client's.
* Point blank in PvE is the Bellows' only (×1.25 inside 3 m); the Thresher has the fields at ×1.
* Phase 2 (after the other areas merge): the Scatter pool's new abilities (Bolas, Bulwark) and
  every other new ability, the Lamplighter, the drinkable Kindle Flask, the README tables; the
  Both Barrels / Slug rows above go with them.
