# Roster: weapons, enemies and hit assist

The second-generation arsenal and bestiary. It replaces the first set (Lancet / Thresher /
Riveter / Cinderbox + Prism; Husk / Spitter / Gnat / Bastion), whose line-up mapped too
closely onto other games' signature designs. Everything here is meant to be **original**: the
names, the silhouettes, the mechanics combos, the sounds.

This file is the design contract between the areas. Numbers are starting values (they live in
`assets/tuning.ron` / `assets/enemies/*.ron` and are tuned in play); names, net kinds, slots,
part names and the behaviour described here are the contract. If an implementation has to
deviate, update this file in the same commit and say why.

Design rules for anything added later:

- No revolver + coin, no shotgun with a parryable grenade, no nailgun with saw discs, no
  plain rocket launcher, no orb-throwing humanoid, no hover drone that shoots bolt bursts, no
  armoured brute whose only trick is an overhead slam. Archetypes (a precision gun, a close-range
  gun, a flyer, a heavy) are fine; their *signature* must be ours.
- Every enemy attack has a readable wind-up (pose + glow + sound) before any damage.
- Enemies are **big and readable** at 640×360: nothing smaller than ~2 m in its longest
  dimension, high-contrast glowing weak points, telegraph colours that read against fog.

## Weapons

Slots stay 1–4 (`PlayerInput::weapon` 0–3), two alt modes each (`weapon_mode` 0 = A, 1 = B;
pressing the slot key again cycles), `FIRE` = primary (automatic while held), `ALT_FIRE` = the
selected alt, `PUNCH` = Fist / parry (unchanged), `THROW` = Thorn. Weapon switching stays
instant with per-weapon cooldowns. Everything in `step_weapons` stays integer-tick and
deterministic so the owning client predicts it.

| slot | weapon | role | primary | alt A | alt B |
|---|---|---|---|---|---|
| 1 | **Quill** | precision spike-caster (hitscan) | *Quill* | *Brand* | *Splinter* |
| 2 | **Bellows** | twin-barrel blast cannon (close range) | *Blast* | *Gust* | *Both Barrels* |
| 3 | **Arclash** | arc caster (sustained, chaining) | *Arc* | *Wisp* | *Discharge* |
| 4 | **Tumbler** | bouncing-bomb launcher (explosive) | *Tumble* | *Breach* | *Gravewell* |
| — | **Fist** | punch / parry (unchanged rules) | | | |
| — | **Thorn** | thrown relay spike (ricochet object) | | | |

### 1 Quill — precision spike-caster
A slender, long-barrelled caster that fires hardened quills; a fan of spare quills stands up
along its spine like a crest and flattens as it fires. Not a revolver: no cylinder, no hammer.

- **Quill** (primary): hitscan, 24 dmg, 0.28 s interval, perfectly accurate, ricochets off
  Thorns.
- **Brand** (alt A): hitscan marking shot, 10 dmg, 2.5 s cooldown, ricochets off Thorns. What it
  hits (enemy, or a player in PvP) is **branded** for 5 s: it takes ×1.35 damage from every
  source and every player, glows with a sigil, and Thorn ricochets / Arc chains pick branded
  targets first. One live brand per player (a new brand replaces the old). Replicated as a
  flag on the target so every client draws the sigil.
- **Splinter** (alt B): hitscan, 20 dmg, 1.1 s cooldown, ricochets off Thorns. On impact it
  bursts into 6 splinters: secondary hitscan traces from the impact point inside a 40° cone, 18 m
  range, 9 dmg each. From a target the cone continues along the shot (the target hit directly is
  excluded); from a wall it follows the shot reflected about the surface normal (corner shots).

### 2 Bellows — twin-barrel blast cannon
Two short, wide barrels over an accordion bellows chamber that pumps between shots; brass and
leather.

- **Blast** (primary): 12 pellets × 8 dmg, 8° cone, 60 m, 0.8 s interval, 2 m/s knockback per
  pellet.
- **Gust** (alt A): a compressed-air blast, no pellets. 7 m, 60° cone (line of sight): every
  hostile inside takes 6 dmg and is knocked back 18 m/s along the aim (+4 m/s up); every
  **hostile projectile** inside is **deflected** — it becomes yours and flies along your aim at
  1.25× speed and 1.5× damage (no heal, no parry reward; style *DEFLECT*). Self recoil 10 m/s
  opposite the aim (gust-jump off the floor). 1.6 s cooldown.
- **Both Barrels** (alt B): both barrels at once: 24 pellets × 8 dmg in a 5° cone, self recoil
  9 m/s opposite the aim (blast-jump), 1.4 s cooldown; also puts the primary on its interval.

### 3 Arclash — arc caster
Three copper prongs around a glass capacitor full of crawling light; insulated grip, coil
windings, a ground-strap that sparks.

- **Arc** (primary): while held, a hitscan zap every 0.1 s, 30 m, 6 dmg. A zap that hits a target
  **chains** to the nearest other hostile within 7 m of it (line of sight from the target, branded
  first), up to 2 extra jumps, ×0.75 damage per jump. Zaps don't interact with Thorns. Drawn as a
  jagged beam, not a tracer.
- **Wisp** (alt A): a slow ball of lightning (12 m/s, no gravity, 2.5 s life). Every 0.2 s it zaps
  the nearest hostile within 6 m (line of sight) for 8 dmg. On world contact or expiry it pops:
  4 m shock, 25 dmg. 2.8 s cooldown. One projectile kind (`Wisp`).
- **Discharge** (alt B): hold to charge (up to 1.0 s, uses `WeaponState::charge` like the old
  hold-to-charge), release: a radial shock nova centred on you, radius 3.5–8 m, 25–80 dmg and
  12–24 m/s outward knockback scaling with charge; never hurts you. 2.0 s cooldown. Switching
  weapon or mode cancels the charge.

### 4 Tumbler — bouncing-bomb launcher
A squat drum-fed launcher with a wide round muzzle; bombs are studded iron balls with a
glowing fuse band.

- **Tumble** (primary): bomb 30 m/s, gravity 18, bounces off the world (restitution 0.6) and
  explodes on its 3rd world contact, on touching any hostile hitbox, or after 1.6 s. Explosion
  r 4.5 m, 65 dmg (falloff to 30 % at the edge), knockback 20. 0.85 s interval. Projectile kind
  `Bomb` (`state` = bounces left).
- **Breach** (alt A): a point-blank charge, no projectile: explodes at the first surface or
  hostile within 3 m along the aim (pulled back 0.2 m), or 3 m out in the air. r 4 m, 70 dmg,
  knockback 24; normal self-damage rules (35 %, full knockback) — the blast-jump and panic button.
  1.2 s cooldown.
- **Gravewell** (alt B): a heavy canister lobbed at 20 m/s, +25° above the aim, gravity 20; sticks
  where it lands (world only). For 1.5 s it **pulls** every enemy (and, in PvP, hostile players,
  at half strength) within 9 m towards its centre (up to 9 m/s, scaled by the enemy's
  `knockback_scale`), then implodes: r 5 m, 80 dmg. 3.0 s cooldown. Projectile kind `Gravewell`.

### Thorn — thrown relay spike (replaces the Prism)
A barbed glass spike. Thrown, not tossed: it flies nearly straight and **plants itself** in the
first wall, floor or ceiling it hits, quivering, a lit relay you placed on purpose.

- 3 charges, 1 recharged per 3.0 s. Thrown at 30 m/s along the aim (gravity 12), inheriting half
  the thrower's velocity. Sticks into the world (oriented along its flight) and lives 8 s once
  planted. Hitting an enemy / player: 8 dmg, and it drops to the floor where it lies as a live
  thorn.
- **Ricochet**: a Quill shot (any mode's main shot) passing within 0.6 m of a live Thorn (in
  flight, planted or lying) shatters it and re-fires from it at: another live thorn within 30 m
  (chain), else a branded hostile within 40 m, else the nearest hostile within 40 m with line of
  sight (weak point preferred). ×1.5 damage per bounce. Server-resolved; clients get the tracer
  path to draw.
- Replicated like the prism was (trajectory + a `PLANTED` flag, `owner` = thrower). The HUD
  shows thorn pips where the prism pips were.

### Unchanged
Fist / parry rules, blood-burst healing, hard damage, explosion falloff and self-damage, the
damage pipeline, freshness (per weapon / mode / action), the style ranks.

### Implementation notes (weapons, as built)
Details the contract left open, and the few places where the build adds to it:

- **Players' projectiles are not parryable** (no punchable grenades); the **Gust** turns any
  hostile projectile around, opponents' bombs / wisps included. A deflected projectile's attack
  becomes `Gust` (kill feed, freshness) and it carries `PARRIED`, so a kill with it is also
  RETURN TO SENDER. A Gust hit is always a body hit (no weak-point bonus). A deflected bomb
  explodes on its first contact.
- **Wisp** and **Gravewell** pass through targets in flight (the Wisp zaps them; the Gravewell
  only sticks to the level). A Gravewell that never lands implodes after 5 s; its implosion has
  a light 6 m/s knockback. Pulled enemies are kept staggered so their steering doesn't fight
  the pull (enemy velocity / `stagger` only; no enemy code changed).
- **Thorn**: a thorn that never lands fizzles after 3 s; one that hits a target lives 8 s from
  the hit. Replication adds `LYING` (bit 1) and `DROPPING` (bit 2) to `PLANTED` (bit 0); while
  planted, `velocity` carries its orientation (unit). Splinters, pellets and zaps don't
  ricochet; only the Quill's main shot does.
- **Discharge** deals its full charge-scaled damage everywhere inside the radius (no falloff).
- **Brands** don't multiply world / dev damage.
- **Hit assist**: rays test the plain shapes first and the padded ones only on a miss (padding
  never turns a headshot into a body hit); an explosion never pads its own owner.
- **Style**: freshness gains a `Thorn` key (16 keys) and **DEFLECT** scores 30.
- **Flags**: `DamageFlags` bit 10 `PIERCED` became `CHAINED` (Arc chain jumps), bit 15 is
  `BRANDED`. Player `aux` bits 12–15 (old live stickies) are unused.

## Enemies

All five are server-run with the existing state machine (idle → alert → chase → wind-up →
strike → recover), nav grid and encounters. Faction: the **Moulds**, things poured and set in
the foundry: grey wax-flesh, slag, brass and chain, lit from inside.

| net kind | name | role | size (capsule r × h) | health | speed |
|---|---|---|---|---|---|
| 20 | **Gnawer** | fodder melee, pack crawler | 0.6 × 2.0 | 45 | 10 |
| 21 | **Censer** | ranged lobber | 0.6 × 2.5 | 80 | 5 |
| 22 | **Harrier** | flyer, dive-bomber | 0.7 × 1.3 (3 m wingspan) | 40 | 15 |
| 23 | **Gaoler** | heavy, shielded, parryable bash | 1.3 × 3.7 | 520 | 4.5 |
| 24 | **Heron** | sniper | 0.5 × 3.4 | 60 | 7 |

Net kinds 16–19 (the old four) are retired. Definition files: `assets/enemies/<name>.ron`
(lower-case file stem = the name used by levels and `/spawn`).

### Gnawer — fodder melee (replaces Husk)
A hunched knuckle-walker: runs on its oversized fists, wax-grey hide over a caged ribcage, a
huge under-slung jaw of brass teeth. Head (with the jaw) is the weak point: big, low and
forward.

- Weak point **head** (0, 1.62, −0.38) r 0.38, ×2.
- **Bite**: 2.6 m reach, 100° arc, 14 dmg, kb 7. Wind-up 0.4 s: the jaw unhinges, the throat
  glows orange, a rising chitter.
- **Pounce** (target moving > 4 m/s, 3–9 m away): wind-up 0.55 s crouched, then a leaping arc
  (13 m/s forward + 6 m/s up) that rakes the first players it reaches, 18 dmg, kb 9; recover
  0.7 s; cooldown 3 s.

### Censer — ranged lobber (replaces Spitter)
A tall robed figure whose head is a caged brazier; it swings a smoking censer on a long chain.
Keeps 14–30 m and side-steps slowly.

- Weak point **lantern** (the brazier head, 0, 2.22, −0.1) r 0.42, ×2.
- **Volley**: wind-up 0.8 s (whirls the censer overhead, embers brighten, chain rattles), then
  lobs 3 **ember bombs** on ballistic arcs at the target's predicted position (±8° yaw spread,
  speed solved for the arc, ≤ 26 m/s, gravity 20). Each explodes on impact: r 2.5 m, 18 dmg with
  falloff. **Parryable** (a punched ember flies straight along your aim like any parried
  projectile). Cooldown 3 s. Projectile kind `EmberBomb`.
- **Censer swing** (players inside 3 m): wind-up 0.5 s, 160° arc, 12 dmg, kb 12 — shoves rushers
  away.

### Harrier — flying dive-bomber (replaces Gnat)
A ragged carrion flyer: wax-skin wings stretched over brass ribs, a hooked beak, dangling
talons, a glowing heart sac under its chest. Hovers 5–10 m up and circles 8–16 m out.

- Weak point **heart** (under the chest) r 0.35, ×2.5.
- **Dive**: wind-up 0.7 s (rears up, wings fold back, beak and heart glow red, shriek), then dives
  in a straight 3D line at the target's position (small lead) at 26 m/s for up to 0.6 s, hitting
  each player it passes within reach once: 20 dmg, kb 10. Hitting the world ends the dive early.
  Recover 0.8 s (climbs back), cooldown 2.2 s.
- **Swat**: punching a Harrier mid-dive within reach counts as a parry (heal + PARRY style) and
  knocks it out of the sky: stunned, falling, 2× punch damage.

### Gaoler — heavy, shielded (replaces Bastion)
A 3.7 m iron jailer: a riveted cell door for a shield on its left arm, a chain flail in its right
hand, a grilled helm, and a caged **heart-lantern** on its back — the unarmoured side.

- Weak points **head** (helm grille, 0, 3.32, −0.35) r 0.45 ×1.5; **lantern** (back,
  0, 2.2, 0.95) r 0.55 ×2.5.
- Armour: front ×0.15 in a 120° arc (the shield), explosions too; head and lantern bypass.
- **Shield Bash** (melee 3.2 m): wind-up 1.0 s (shield drawn back, rim glows yellow); the last
  0.35 s is the **parry window** (white-hot strobe). Parried within 4.5 m: the shield is knocked
  aside, stunned 2.5 s, 150 counter damage through the armour. Otherwise 40 dmg, kb 18 + up.
- **Chain Sweep** (players within 7 m): wind-up 0.8 s (chain glows red, spun low), a 300° sweep at
  knee height: 25 dmg, kb 14, but it only hits players whose feet are less than 1.2 m above the
  Gaoler's feet — **jump it**. Not parryable.
- **Rush** (gap closer, target 8–20 m away): wind-up 0.9 s, charges at 15 m/s for 0.8 s behind
  the shield, 30 dmg, kb 16, stops at walls; cooldown 5 s.

### Heron — sniper (new)
A stilt-legged wading-bird construct, 3.4 m tall: thin brass legs, a narrow body, a long neck and
a single lens-eye head that stands above most cover. Keeps 25–45 m, repositions after firing,
sight range 80 m.

- Weak point **head** (the lens, 0, 3.12, −0.4) r 0.35, ×2.5.
- **Needle**: wind-up 1.3 s with a visible red **aim laser** from its eye along its aim (drawn by
  every client from the replicated aim); it tracks the target until the last 0.25 s, then locks
  (dash sideways to dodge). Fires one needle at 110 m/s, 30 dmg, **parryable**. Needs line of
  sight. Cooldown 3.5 s. Projectile kind `Needle`.
- **Kick** (players inside 4 m): wind-up 0.4 s, 15 dmg, kb 16 — sends you back out.

## Hit assist (forgiving hitboxes)

Shots should feel generous; being shot should feel fair.

- **Player-owned attacks** (every weapon mode, splinters, arc chains, explosions, Fist, Thorn
  hits) test against hit shapes inflated by `combat.hit_assist`:
  `enemy_body_pad` 0.2 m, `enemy_weak_pad` 0.1 m (weak points/heads), and in PvP
  `player_body_pad` 0.12 m, `player_head_pad` 0.06 m.
- **Enemy attacks** against players use the plain hitboxes (no padding).
- Padding is applied at test time (lag-compensated history keeps the plain shapes).

## Visibility

- Enemies are modelled at the sizes above (roughly 1.2–1.4× the old ones) with bright weak
  points; enemy projectiles are bigger and brighter than before (ember bombs ~0.35 m, needles
  with a long glowing streak).
- Other players' Shrike models are drawn 10 % larger (visual only; the movement capsule is
  unchanged).

## Net kinds and protocol

| range | entries |
|---|---|
| enemies | 20 Gnawer, 21 Censer, 22 Harrier, 23 Gaoler, 24 Heron (16–19 retired) |
| player projectiles | 72 `BOMB`, 73 `GRAVEWELL`, 74 `WISP` (64–69 retired) |
| enemy projectiles | 80 `EMBER_BOMB`, 81 `NEEDLE` (70–71 retired) |
| relay | `THORN` takes over the prism's kind slot |

Flag bits: `BRANDED` is bit 15 in both `player_flags` and `enemy_flags` (weapons area). Brand
state lives in a `SimWorld` side table keyed by target and is OR-ed into the replicated flags in
`SimWorld::replicate`, so it never touches the `Enemy` struct. New enemy flags (enemies area)
use bits 6–14.

`PROTOCOL_VERSION` is bumped once for the whole roster change.

## Ownership while the roster is being built

| area | owns |
|---|---|
| weapons | `shared::sim::{weapons, hitscan, prism→thorn, combat (Attack)}`, player projectile kinds, `CombatTuning` weapon/thorn/hit-assist sections, client weapon/viewmodel/Shrike models, weapon fx, weapon sounds, HUD weapon names, style bonuses tied to weapons, weapon tests |
| enemies | `assets/enemies/*`, `shared::sim::{enemy::*, nav}`, enemy projectile kinds, `enemy_projectiles` tuning, client enemy models/fx, enemy sounds, `/spawn`, enemy tests |
| campaign maps | `client/examples/level_gen*`, campaign `assets/levels/*`, campaign parts of `shared/tests/levels.rs`, `server/tests/encounter_crucible.rs`, `docs/levels.md` campaign sections |
| arena maps | new arena levels (own generator module), arena parts of `shared/tests/levels.rs`, `docs/levels.md` arena sections |

`DESIGN.md` and `README.md` are updated once at the end, after everything is merged.
