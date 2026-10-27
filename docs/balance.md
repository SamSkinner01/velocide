# Balance: how the weapons are measured, and what the measurements changed

The player asked: *"please spend time balancing the weapons more, they need work."* This file
is the **balance** area of the refit (`docs/refit.md`): a harness that measures every weapon
and ability in the real simulation, what it found, every number that changed and why, and
the tests that keep it that way. The numbers themselves live in `assets/tuning.ron`
(`combat`: the PvE reference; `balance.pvp`: the PvP profile); `docs/arsenal.md` has the
weapon / ability / "Balance profiles" tables that match them.

## The harness

```text
cargo run -p shared --release --example balance_report                    # all of it (~1 min)
cargo run -p shared --release --example balance_report -- pvp oneshots    # sections: primaries
                                                         ttk swarm pvp oneshots abilities
cargo run -p shared --release --example balance_report -- --refit         # with the refit's
                                    bigger Mould hit shapes / hit assist and projectile pad assumed
cargo run -p shared --release --example balance_report -- --tuning other.ron   # another file
```

`shared/tests/balance_support/mod.rs` (shared by the example and the tests through `#[path]`)
builds a `SimWorld` on a flat 100 m floor with one god-mode shooter carrying a build and its
targets: shipped Moulds (`EnemyRegistry::builtin`, so **pve**'s new shapes flow in when they
merge) facing the shooter (the Gaoler also from behind), a pack of six Gnawers, or a PvP
opponent. It runs `Tuning::for_mode` (PvE: campaign, PvP: deathmatch) through `SimWorld::step`
and reads the `SimEvent::Damage` events. What is real and what is idealised:

* **Real code paths**: `step_weapons` (intervals, bursts, salvos, draws, charges, bloom,
  cooldowns, the shared ability cooldown), pellet and spread RNG, hit assist, weak points and
  their multipliers, the Gaoler's armour, PvP head multipliers, projectile flight, gravity and
  travel time, explosion falloff and line of sight, chains, splinters, pierce, burns, brands,
  patches, Limpets sticking, `ability_power`.
* **Idealised**: the aim. Every tick the crosshair is on the aim point (body centre, weak
  point, head); for anything with gravity or a lob the low ballistic arc is solved (the
  harness steps a copy of the weapon state to know what is about to fire). Targets stand
  still: position, velocity and facing are restored every tick (no knockback drift, no AI), and
  they take damage without dying, so one run is a damage-over-time curve and every time to
  kill (NORMAL ×1.0, HARD ×1.3 health) is read off it. Moving targets, dodging, cover and
  thorn ricochets are not measured; the numbers are a skilled player's ceiling.
* **Policies**: a primary is held (the bow: full draws, released when full); an ability is used
  whenever it is ready (charges released when full, Flurry held, the Limpet: three charges
  thrown, all stuck, held to detonate; on the bow the ability is armed for every draw it can
  ride). *Build gain* = primary + ability minus the primary alone over 12 s: what taking the
  ability adds, cost included (Both Barrels and Volley use up the primary's shot).

Tables: per trigger, sustained DPS (Gnawer body / head at 5 / 15 / 30 m), time to kill every
Mould (body / weak point, NORMAL at 5 / 15 / 30 m, HARD at 15 m), the swarm (six NORMAL
Gnawers packed 1.6 m apart, front row at 10 m), PvP times to kill (body / head at 5 / 15 /
30 m, and with the default build's abilities), PvP one-shots (every pull and every ability use
on every carrier at 3 / 5 / 10 m, body / head, the bow at a full and a 90 % draw), and per
ability: per use, per use on the pack, spam DPS, build gain alone and on the pack.

## What it found

| outlier | measured | why it matters |
|---|---|---|
| **Both Barrels fired on the same tick as the primary** (a mechanic bug, not a number) | FIRE + ALT together: a Bellows blast and Both Barrels in one tick, **144** to a PvP body (Thresher: 108) | a two-button body one-shot; in PvE the build gain was 92 (Bellows) / 129 (Thresher) DPS against Flak's 52 and the Slug's 46. Volley had the same shape |
| **Kiln**: the weakest primary at everything | 48 DPS (spray 36 + burn 12), 0 past 13 m, the Gaoler (back) in 10.8 s, the six-Gnawer swarm in 2.9 s (its 16° cone held two of them) | nobody's pick: weaker than every other weapon single-target and slower than the Bellows / Thresher / Arclash at the crowds it is for |
| **Arclash** | 60 DPS, the lowest hitscan; Gaoler (back) 8.6 s | single target it lost to everything but the Kiln |
| **Thresher**: strictly worse than the Bellows | 118 / 55 / 13 DPS at 5 / 15 / 30 m against the Bellows' 125 / 102 / 23 | the auto-scattergun lost at every range in its own class |
| **Limpet** | 85 spam DPS (50 every 0.4 s), 146 a use, 652 on the pack (gain 380) | three times its siblings (Breach 54, Gravewell 27, Mortar 28) |
| **Triple Nock** | +105 DPS to the bow (three full arrows every other draw) | the bow's other picks add 34 (Brand), 6 (Ember Arrow), 0 (Pinning Arrow) |
| **Wisp** | 15 spam DPS, 40 a use | the weakest damage ability of any class |
| **Flurry on its home weapon** | the Lancet's Flurry added 22 DPS against its Skewer's 45 | the home weapon's own burst was its worst pick |
| **Ember Arrow** | +6 DPS to the bow | its burst barely paid for the weaker arrow |
| PvP: **Bellows head one-shot at 5 m** | a blast at the head: 106 at 5 m, 134 at 3 m | the rules allow only bow / Slug / Skewer headshots |
| PvP: **Both Barrels head one-shot** | 154 (Thresher 138) at 3–5 m | as above |
| PvP: **Triple Nock / Ember Arrow at a 90 % draw** | three heads 144; arrow + burst + burn 124 | "a 90 % draw is not a one-shot" held only for the plain arrow |
| PvP: **Volley** (after the Arclash change) | ten 7-dmg zaps to the head: 98 | too thin a margin under 100 |

Explosives against big Moulds (Smelter 5.2–5.6 s, Gaoler from behind 6.7–7.2 s, three times
the precision weapons on weak points) were looked at and mostly kept: they have no weak points
by design and clear packs far faster than anything else (the swarm in 0.3–0.45 s). The Tumbler
got a modest raise (below) because the Cinderbox beat it at everything the harness measures.

## Changes

PvE numbers are `combat` (the reference; the PvE profile stays empty). Where a PvE number moved
but PvP was already right, the PvP profile was adjusted to hold the PvP number.

| what | before → after | why |
|---|---|---|
| **Both Barrels / Volley mechanic** (`weapons.rs`, `step_build`) | they fired as well as the primary on the same tick → they wait for the primary to be ready and **take its shot** (the primary doesn't fire that tick, then waits its interval) | "both barrels at once" can't also fire the barrels; removes the 144 two-button PvP body one-shot. Unit-tested (`both_barrels_also_holds_the_primary`) and measured (PvP test) |
| `bellows.both_cooldown` | 1.4 → 1.8 s | with the fix alone the gain was still 57 / 85; now 35 (Bellows) / 58 (Thresher), with Slug 46 and Flak 52 |
| `kiln.damage`, `kiln.cone_degrees` | 3 → 4, 16° → 20° | 48 → 63 DPS single target; the cone now holds a packed row: swarm 2.9 → 0.75 s; Smelter 8.8 → 6.7 s, Gaoler (back) 10.8 → 8.3 s |
| `burn.dps` | 4 → 5 a stack | the Kiln's identity (15/s at the cap), and the Flare / Ember Arrow with it |
| `arclash.zap_damage` | 6 → 7 | 60 → 70 DPS, Gaoler (back) 8.6 → 7.4 s, swarm 2.0 → 1.8 s |
| `thresher.pellet_damage`, `thresher.cone_degrees` | 7 → 8, 11° → 10° | 134 DPS at 5 m (the class's close-range best) and 74 at 15 m; the Bellows keeps the burst and the reach (125 / 102) |
| `tumbler.bomb_damage` | 65 → 72 | 76 → 84 DPS: the heavier explosive beats the Cinderbox's 74 single target; one bomb now clears the packed swarm |
| `limpet.damage`, `limpet.cooldown` | 50 → 40, 0.4 → 0.9 s | 85 → 39 spam DPS, pack gain 380 → 174 (Breach 249, Gravewell 133, Mortar 113–131) |
| `triple_nock.damage_scale` | 1.0 → 0.6 | gain 105 → 42 (Brand 34): three arrows a close-range shotgun, a loss at range |
| `arclash.wisp_zap_damage`, `wisp_pop_damage`, `wisp_cooldown` | 8 → 11, 25 → 30, 2.8 → 2.4 s | 15 → 23 spam DPS (Discharge 27, Volley 32) |
| `flurry.damage_scale`, `flurry.cooldown` | 0.8 → 1.0, 1.6 → 1.4 s | Lancet 22 → 32, Quill 38 → 54; on both precision weapons the Brand, Skewer and Flurry builds now total 121–140 DPS single target (Splinter, the crowd pick: 104–110) |
| `ember_arrow.damage_scale`, `ember_arrow.damage` | 0.75 → 0.9, 30 → 35 | gain 6 → 13 single, 75 on the pack: the area arrow |
| PvP `Bellows` | damage 0.7 → 0.6 (4.8 a pellet) | head blast at 5 m 106 → 91 (115 inside 5 m: the point-blank reward, now a stated exception); still two body blasts, 0.8 s |
| PvP `BothBarrels` | 0.4 → 0.25 | 48 body / 96 head (Thresher 43 / 86) |
| PvP `TripleNock` | 0.5 → 0.55 (× 0.6 = a third of an arrow each) | three heads at 90 %: 95; a full draw's 104 is the bow's allowed one-shot |
| PvP `EmberArrow` | 1.0 → 0.5 | 90 % draw to the head with burst + burn 124 → 88 (92 at a full draw) |
| PvP `Volley` | 0.7 → 0.65 | ten zaps / rivets to the head 98 → 91 |
| PvP `Thresher`, `Kiln`, `Tumbler`, `Wisp`, `Flurry`, `Limpet` | 0.8 → 0.7, 1.2 → 0.9, — → 0.9, 0.7 → 0.5, 0.75 → 0.6, 0.6 → 0.75 | hold the PvP numbers where the PvE base moved (5.6 a pellet, 3.6 a spray, 65 a bomb, ~5.5 a zap, 14.4 a Quill Flurry shot, 30 a charge) |
| PvP `Arclash` | 1.0 (no entry), now 7 a zap | 1.6 → 1.4 s: the tracking beam was the slowest hitscan; still in the band |
| one-shot rule | + "a Bellows blast to the head inside 5 m" | with 12 pellets and heads ×2, a two-blast body kill (≥ 50 a blast) always means ≥ 100 when every pellet lands on a head; the alternative was a three-blast (1.6 s) Bellows |

## Before and after

All measured with `cargo run -p shared --release --example balance_report` (the "before" with
the tuning and code of `221b596`).

### Primaries, PvE (DPS on a Gnawer, body / head)

| weapon | 5 m before | 5 m after | 15 m before | 15 m after | 30 m before | 30 m after |
|---|---|---|---|---|---|---|
| Quill | 86 / 173 | 86 / 173 | 86 / 173 | 86 / 173 | 86 / 173 | 86 / 173 |
| Bellows | 125 / 250 | 125 / 250 | 102 / 87 | 102 / 87 | 23 / 32 | 23 / 32 |
| Arclash | 60 / 120 | **70 / 140** | 60 / 120 | **70 / 140** | 60 / 120 | **70 / 140** |
| Tumbler | 76 | **84** | 76 | **84** | 69 | **77** |
| Lancet | 94 / 188 | 94 / 188 | 94 / 188 | 94 / 188 | 98 / 134 | 98 / 134 |
| Thresher | 118 / 223 | **134 / 262** | 55 / 54 | **74 / 74** | 13 / 8 | **19 / 13** |
| Riveter | 84 / 168 | 84 / 168 | 83 / 167 | 83 / 167 | 82 / 164 | 82 / 164 |
| Cinderbox | 74 | 74 | 74 | 74 | 74 | 74 |
| Longwick | 105 / 210 | 105 / 210 | 105 / 210 | 105 / 210 | 105 / 210 | 105 / 210 |
| Kiln | 48 | **63** | — | — | — | — |

### Time to kill, NORMAL, 15 m (after; s, body / weak point)

Shotguns and the Kiln are at 5 m in the bands below; the full tables (5 / 15 / 30 m, HARD) are
in the report.

| weapon | Gnawer 45 | Censer 80 | Harrier 40 | Snuffer 70 | Heron 60 | Bellwether 220 | Smelter 420 | Gaoler front 520 | Gaoler back 520 |
|---|---|---|---|---|---|---|---|---|---|
| Quill | 0.28 / 0 | 0.85 / 0.28 | 0.28 / 0.28 | 0.28 / 0.28 | 0.57 / 0 | 2.55 / 1.13 | 1.70 / 1.70 | — / 3.97 | 5.95 / 2.27 |
| Bellows | 0 / 0 | 0 / 0.80 | 0 / 0 | 0 / 0 | 0 / 0.80 | 1.60 / 2.40 | 2.40 / 2.40 | — / 11.2 | 3.20 / 3.20 |
| Arclash | 0.60 / 0.30 | 1.10 / 0.50 | 0.50 / 0.50 | 0.30 / 0.30 | 0.80 / 0.30 | 3.10 / 1.50 | 2.30 / 2.30 | — / 4.90 | 7.40 / 2.90 |
| Tumbler | 0.47 | 1.32 | 0.52 | 1.30 | 0.47 | 3.00 | 5.53 | — | 6.38 |
| Lancet | 0.45 / 0.15 | 0.75 / 0.30 | 0.30 / 0.30 | 0.15 / 0.15 | 0.60 / 0.15 | 2.25 / 1.05 | 1.65 / 1.65 | — / 3.60 | 5.55 / 2.10 |
| Thresher | 0 / 0.83 | 0.83 / 1.67 | 0.42 / 0.42 | 0.83 / 0.83 | 0.42 / 0.83 | 1.67 / 2.50 | 2.50 / 2.50 | — / 12.1 | 2.50 / 2.92 |
| Riveter | 0.65 / 0.40 | 1.07 / 0.57 | 0.57 / 0.57 | 0.38 / 0.38 | 0.82 / 0.40 | 2.73 / 1.40 | 2.05 / 2.05 | — / 4.23 | 6.30 / 2.55 |
| Cinderbox | 0.40 | 0.48 | 0.42 | 0.47 | 0.40 | 3.00 | 5.57 | — | 6.88 |
| Longwick | 0.85 | 0.85 | 0.87 | 0.85 | 0.85 | 2.85 / 1.87 | 1.85 | — / 3.85 | 4.85 / 1.85 |
| Kiln (5 m) | 0.75 | 1.25 | 0.67 | 1.08 | 1.00 | 3.50 | 6.67 | — | 8.25 |

Before, at the same ranges: the Arclash 0.70 / 1.30 / … / 8.60 / 3.40, the Thresher 0.42 / 1.25
/ … / 3.33 / 3.75, the Tumbler's Gaoler (back) 7.23, the Kiln (5 m) 0.92 / 1.67 / 0.83 / 1.50 /
1.25 / 4.58 / 8.75 / — / 10.83. "—" is over 20 s: the Gaoler's front is armoured (×0.15,
explosions too) — flank it, parry its bash, or shoot its head.

### Swarm: six NORMAL Gnawers, front row at 10 m (s until the last falls)

| weapon | primary before → after | + ability A | + ability B |
|---|---|---|---|
| Quill | 3.12 → 3.12 | 3.12 (Brand) | 2.20 (Splinter) |
| Bellows | 2.40 → 2.40 | 2.40 (Gust) | 1.40 → 1.60 (Both Barrels) |
| Arclash | 2.00 → **1.80** | 1.60 → 1.30 (Wisp) | 1.80 (Discharge) |
| Tumbler | 1.20 → **0.30** | 0.30 (Breach) | 0.45 (Gravewell) |
| Lancet | 3.45 → 3.45 | 2.55 (Skewer) | 2.55 (Flurry) |
| Thresher | 2.50 → 2.50 | 1.67 → 1.25 (Slug) | 2.02 → 1.67 (Flak) |
| Riveter | 3.68 → 3.68 | 2.80 → 2.93 (Volley) | 3.20 → 3.00 (Wisp) |
| Cinderbox | 0.40 → 0.40 | 0.40 (Limpet) | 0.33 (Mortar) |
| Longwick | 5.83 → 5.83 | 3.83 (Triple Nock) | 5.83 (Pinning Arrow) |
| Kiln | 2.92 → **0.75** | 2.83 → 0.83 (Flare) | 2.92 → 0.75 (Discharge) |

The Tumbler's 0.30 is one bomb: 72 reaches the 45 HP of the pack's far corners (65 did not). A
breakpoint, so it moves with health (grind waves add 3 % a wave).

### Abilities, PvE (build gain, DPS: one Gnawer / six)

| class | ability (carrier) | before | after |
|---|---|---|---|
| Precision | Brand (Quill / Lancet) | 35 / 37 | 35 / 37 |
| | Splinter (Quill) | 18, pack 68 | 18, pack 68 |
| | Skewer (Lancet or Quill) | 45, pack 81 | 45, pack 81 |
| | Flurry (Lancet / Quill) | 22 / 38 | **32 / 54** |
| Scatter | Gust | 4, pack 24 | 4, pack 24 |
| | Both Barrels (Bellows / Thresher) | **92 / 129** | **35 / 58** |
| | Slug | 46 | 46 |
| | Flak | 52.5 | 52.5 |
| Rapid | Wisp (Arclash) | 15, pack 24 | **23, pack 36** |
| | Discharge (Arclash, at 4 m) | 27, pack 160 | 27, pack 160 |
| | Volley (Riveter / Arclash) | 34 / 31 | 32 / 32 (pack 73) |
| | Flare (Arclash / Kiln) | 9 / −1, pack 59 / 18 | 11 / −1, pack 75 / −4 |
| Explosive | Breach (Tumbler, at 4 m) | 54, pack 249 | 54, pack 249 |
| | Gravewell (Tumbler) | 27, pack 133 | 27, pack 133 |
| | Limpet | **85, pack 380** | **39, pack 174** |
| | Mortar (Cinderbox) | 25, pack 131 | 25, pack 131 |
| Draw | Brand | 34 | 34 |
| | Triple Nock | **105** | **42** |
| | Pinning Arrow | 0 (the root is the point) | 0 |
| | Ember Arrow | 6, pack 58 | **13, pack 75** |

The pack-gain "before" column is the old tuning run with the fixed mechanic (Both Barrels
before the fix: 92 / 129 single).

### PvP (100 HP)

| weapon | per pull at 5 m, body / head | TTK body at its range before → after |
|---|---|---|
| Quill | 24 / 48 | 1.13 (15 m) |
| Bellows | 67 / **106** → 58 / 91 (115 at 3 m) | 0.80 (5 m) |
| Arclash | 6 / 12 → 7 / 14 | 1.60 → **1.40** (15 m) |
| Tumbler | 63 / 63 | 1.32 (15 m), 0.98 at 5 m |
| Lancet | 12 / 24 | 1.20 (15 m) |
| Thresher | 39 / 28 → 39 / 39 | 0.83 (5 m) |
| Riveter | 7 / 14 | 1.32 (15 m) |
| Cinderbox | 69 / 69 | 1.70 (15 m), 1.47 at 5 m |
| Longwick | 52.5 / **105** (90 % draw: 48 / 96) | 1.60 (15 m) |
| Kiln | 15 / 15 → 17 / 17 (one spray + its burn) | 1.75 (5 m) |

One-shots measured before → after (5 m, head): Both Barrels 154 → 96 (Thresher 138 → 86),
Triple Nock at 90 % 144 → 95, Ember Arrow at 90 % 124 → 88, Volley 98 → 91, a blast + Both
Barrels on one tick 144 → impossible. Allowed and kept: a full-draw arrow 105 (Triple Nock's
three 104, Pinning / Brand 105), the Slug 110, a full Skewer 104.

**Combos.** An ability is a second action on its own cooldown, so the one-shot rules hold per
pull and per use, not per pair. Used whenever ready, the default builds' fastest body kills at
their range are Thresher + Flak 0.08 s (a blast and a contact burst at 5 m, unchanged by this
pass), Thresher + Slug 0.42 s, Cinderbox + Mortar 0.55 s, Lancet + Skewer 0.8 s; the report's
PvP section lists all of them. Left for play-testing (below).

## The refit assumptions (`--refit`)

**pve** is making Mould hit shapes bigger (hit assist 0.35 / 0.15 m) and **feel** adds a 0.2 m
projectile hit pad. `--refit` approximates both (Mould shapes ×1.25, pads 0.35 / 0.15, player
projectile radii +0.2 m) until they merge; after that the plain report measures the real thing.
What moves: the scatterguns at range (Thresher 15 m 74 → 123 DPS, Bellows 102 → 137), the
Quill's Splinter on packs; explosive direct hits lose 3–6 % (the projectile now touches the
target further out, and the blast falls off from there: Tumbler 84 → 81, Cinderbox 74 → 70).
The PvE bands and the ability ratios of the tests hold under the assumptions too.

## Locked in by tests

* `shared/tests/combat_balance.rs` (PvE, harness): every primary sustains **60–150 DPS** on a
  Gnawer body at its range (scatterguns and the Kiln 5 m, the rest 15 m); every Mould dies to
  every primary at NORMAL within **9 s** (200+ HP) / **1.5 s** (the rest), best of body and weak
  point, the Gaoler from behind; on every weapon the best ability adds at most **1.5 ×** the
  second best (single target), no ability adds more than **300 DPS** on the pack, every damage
  ability adds at least **10 DPS** somewhere (Gust and Pinning Arrow are utility); the shipped
  `balance` section equals `BalanceTuning::default()`.
* `shared/tests/combat_arsenal.rs`: the analytic PvP test (unchanged rules, new numbers) and
  `pvp_one_shots_and_times_to_kill_measured_in_play` (every pull and ability use on every
  carrier at 3 / 5 / 10 m, body and head, the bow at a full and a 90 % draw; both buttons on
  one tick; every primary's all-body time to kill at its range inside 0.8–1.8 s).
* `shared/src/sim/weapons.rs` unit test: Both Barrels / Volley take the primary's shot.

The Kiln, the dominant abilities (Both Barrels, the Limpet, Triple Nock), the PvP head
one-shots and the same-tick stack each fail one of these on the old numbers; the milder ones
(Arclash, Thresher, Wisp, Flurry, Ember Arrow) were judgement calls from the tables.

## Known limits, left for play-testing

* **Flare on the Kiln** adds nothing where the Kiln already reaches: the Kiln keeps everything
  in its cone at the Burn cap. Its value there is zoning what the 13 m cone can't reach; on the
  Arclash and Riveter it adds 11 single / 75 pack DPS.
* **PvP combos** (above): primary + an instant ability can kill a full-health player at close
  range in a fraction of a second, mostly Scatter. Cutting them would cost the Slug its
  headshot one-shot or the Flak its contact burst; worth watching in play.
* **Breakpoints** (one bomb clearing a pack, two vs three salvos on a Censer) shift with
  difficulty, player count and grind waves; the harness reports NORMAL / HARD solo.
* Stationary targets and perfect aim favour projectiles and lobs (the Tumbler at 30 m, the
  Riveter's travel); against moving Moulds hitscan gains.
* Not measured: thorn ricochets (×1.5 a bounce, auto-aimed weak points: the Quill's PvE edge),
  knockback and crowd control value (Gust, Gravewell's pull, Pinning Arrow, Discharge), the
  Bellwether's toll interrupt, parries.
* The melee weapons, their abilities and the knacks (arsenal's milestone 2) are not in the
  tables or the bands yet (`balance_support::weapons()` leaves Melee out until their behaviour
  lands).
* `docs/roster.md`, `docs/gameplay.md` and `docs/expansion.md` still quote some of the old
  starting values (Tumbler 65, Arclash 6, Thresher 7 × 7, Kiln 3 / 16°); `docs/arsenal.md` and
  `assets/tuning.ron` are the reference.
