# Rework: abilities with ideas, a sniper, a drinkable Kindle, relay Thorns, real shotguns

The player asked (2026-10-28):

> Don't have a healing pool, but have the ability increase speed and do healing over time.
> Thorns should be gun agnostic. Add a sniper weapon to the game. The shotguns are severely
> underpowered, so buff them. Please fix bow hit registration, it seems wonky still. The right
> clicks are all kind of the same. Please make them a bit more inspired and creative.

This file is the contract between the areas that build it. `docs/arsenal.md` stays the
reference (each area updates its own rows and sections there); change this file first if an
area needs something else, and say so in its report. Numbers below are **starting points**:
`balance` owns the final ones (PvE in `combat`, PvP in `balance.pvp`), the rules are fixed.

`PROTOCOL_VERSION` 10 → **11**.

## 1. The right clicks (abilities)

The old pool was mostly "a different shot": a burst, a slug, a fan, a piercing shot, a bigger
blast. Each class's four abilities now cover four different jobs: **move**, **control or
defend**, **set up**, and **one class-flavoured damage tool**. Eleven abilities are new, two
kept ones get a twist, the rest stay as they are.

| class | weapons | pool (menu order) |
|---|---|---|
| Precision | Quill, Lancet, **Lamplighter** | Brand, **Wickline**, **Tally**, **Waxwork** |
| Scatter | Bellows, Thresher | Gust, **Bolas**, **Bulwark**, Flak |
| Rapid | Arclash, Riveter, Kiln | Wisp *(twist)*, Discharge, **Stoke**, **Lantern** |
| Explosive | Tumbler, Cinderbox | Breach, Gravewell, Limpet, **Kite** |
| Draw | Longwick | Brand, **Blink Arrow**, Pinning Arrow, **Beacon Arrow** |
| Melee | Trimmer, Tamper | Lunge, Whirl, Hurl *(twist)*, **Riposte** |

Removed: Splinter, Skewer, Flurry, Both Barrels, Slug, Volley, Flare (and burning patches),
Mortar, Triple Nock, Ember Arrow, Upswing. Their code, tuning, wire kinds, models, sounds,
loadout text and tests go (git keeps them).

### Ability ids (wire, `Ability` discriminants)

Kept abilities keep their ids; new ones take the freed ids.

| id | ability | id | ability | id | ability |
|---|---|---|---|---|---|
| 0 | Brand | 8 | **Tally** | 16 | **Blink Arrow** |
| 1 | **Wickline** | 9 | **Waxwork** | 17 | Pinning Arrow |
| 2 | Gust | 10 | **Bulwark** | 18 | **Beacon Arrow** |
| 3 | **Bolas** | 11 | Flak | 19 | Lunge |
| 4 | Wisp | 12 | **Stoke** | 20 | Whirl |
| 5 | Discharge | 13 | **Lantern** | 21 | Hurl |
| 6 | Breach | 14 | Limpet | 22 | **Riposte** |
| 7 | Gravewell | 15 | **Kite** | | |

**Old settings must load.** Loadout presets in a player's `settings.ron` name abilities; a
removed name (e.g. `Splinter`) must not fail the whole settings file: it becomes the weapon's
default for that slot (test with an old file).

### Defaults (also the *home weapon* for `ability_power`: the first weapon whose default has it)

| weapon | default | weapon | default |
|---|---|---|---|
| Quill | Brand, Wickline | Riveter | Stoke, Lantern |
| Lancet | Tally, Waxwork | Kiln | Lantern, Discharge |
| **Lamplighter** | Wickline, Waxwork | Tumbler | Breach, Gravewell |
| Bellows | Gust, Bulwark | Cinderbox | Limpet, Kite |
| Thresher | Bolas, Flak | Longwick | Blink Arrow, Pinning Arrow |
| Arclash | Wisp, Discharge | Trimmer | Lunge, Hurl |
| | | Tamper | Whirl, Riposte |

The default loadout stays Quill, Bellows, Arclash, Tumbler (with these defaults).

### The new and changed abilities

Every one is server-authoritative like today's; impulses and teleports are server-side and the
owner's client reconciles (like the Lunge). "Hostile" follows the usual rules (Moulds; in PvP
other players, never teammates or yourself). All of them work on every carrier of their class
(`ability_power` scaling as today).

**Wickline** (Precision, move). A burning wick-thread, hitscan to 34 m (lag-compensated like any
ray). Anchored in the level: you are yanked towards the anchor, one impulse of 24 m/s along
the line (at least 4 m/s up), like a grappling zip. On a hostile: a light one (base health
below `heavy_health`, any PvP player) is yanked towards you (16 m/s × its knockback scale,
PvP half) and takes 10 dmg; a heavy one yanks *you* to it. Misses: nothing, half the cooldown.
The line is drawn for 0.35 s. 4.0 s.

**Tally** (Precision, set up). Hold ALT: for up to 1.2 s every hostile the crosshair passes
over (within 45 m, line of sight, lag-compensated, a generous 1.5° assist cone) is *tallied*,
up to 4 (PvP 3), each marked on the owner's screen. Release (or at 1.2 s): one shot at each
tallied target that is still in line of sight, 30 dmg each, aimed at the part you passed over
(weak point / head if that was it). Releasing with nothing tallied: no shot, a quarter of the
cooldown. The server keeps the tally; the owner sees it through own state (the tallied ids).
6.0 s.

**Waxwork** (Precision, defend). Leave a wax double of yourself where you stand, posed as you
were and facing your look, for 5 s (60 HP). Moulds within 25 m that could see it treat it as a
player and prefer it to you (`select_target`); PvP opponents see a still player with your look
and name. Destroyed or at the end it slumps and bursts in hot wax: r 3.5 m, 30 dmg, 1 Burn
stack. One at a time. 10 s from its end.

**Bolas** (Scatter, control). Two wax-weighted balls on a wick cord (projectile 30 m/s, gravity
8, passes nothing). The first hostile it touches is bound: Pinned (the Pinning Arrow's times:
a Mould 1.4 s, heavies 0.6 s, a player slowed). If another hostile is within 6 m of the first,
the cord snaps to it too: both are bound, and for 4 s 60 % of the damage either takes is also
dealt to the other (never chained further, never to the source itself). 5.0 s.

**Bulwark** (Scatter, defend). Hold ALT to raise a slab of hardened wax in front of you (about
2.2 × 1.8 m, 1.2 m out, turning with your aim's yaw, pitch clamped ±30°), up to 4 s. Hostile
shots and blows arriving from inside the slab's 110° front cone are absorbed (no damage, no
knockback; projectiles are destroyed; hitscan and lasers stop at it) until it has absorbed
150 damage, then it shatters. Your own shots pass through; you move at 75 % while it's up.
Cooldown 6 s from when it drops, 9 s if it shattered. Uses the shared **guard** (below).

**Wisp** *(twist)* (Rapid, set up). The wisp now orbits **you** (1.6 m out, at chest height)
for 6 s, zapping the nearest hostile within 7 m (line of sight) every 0.25 s for 9 dmg. Press
ALT again while it orbits to fling it along your aim (18 m/s): it pops on the first contact
(r 4 m, 35 dmg) or after 2 s. Cooldown 4 s, starting when it is gone.

**Stoke** (Rapid, damage). Stoke the weapon: for 4 s the primary fires at 1.6× its rate and
every hit also sets 1 Burn stack (the Kiln, which already burns: +30 % cone reach instead).
Then it is spent: overheated for 1.5 s, no primary (the other ability, the fist and the knack
still work), venting. Predicted in `step_weapons` (`WeaponState` fields). 10 s from use.

**Lantern** (Rapid, set up). Set down a lantern (tossed 10 m/s, settles on the floor; never over
a pit) that for 8 s fires the carrier's primary at the nearest hostile within 18 m in line of
sight: at half its rate and 0.6× its damage (Arclash: chaining zaps, Riveter: rivets, Kiln: an
8 m cone). Moulds may attack it (80 HP). One at a time; a new one replaces the old. 12 s.

**Kite** (Explosive, damage). Launch an ember kite (16 m/s) you steer with your crosshair while
ALT is held: it turns towards the point under your crosshair at up to 120°/s. Released, or
after 4 s, it flies straight; it explodes on contact: r 4.5 m, 75 dmg, kb 18. Your movement
and the primary are free while you steer. 5.0 s.

**Blink Arrow** (Draw, move). Armed like the other arrows; the next release looses it with the
draw. Press ALT while it flies, or within 1.5 s after it sticks, to step to it: you are moved
to the arrow (stuck in the level: just in front of the surface; in a target: just in front of
the target) to the nearest spot your body fits (searched back along the arrow's path), your
speed set to 6 m/s along the arrow's flight. The arrow's own hit is 0.8× a plain arrow's.
Cooldown 6 s, starting at the blink or when the 1.5 s pass.

**Beacon Arrow** (Draw, set up). An arrow with the draw (0.6× damage) that lights a beacon where
it sticks (in a target: it rides along) for 6 s: every hostile within 16 m of the beacon is
**revealed** to the shooter and their teammates (drawn as a silhouette through walls) and takes
×1.15 damage from every player. 8.0 s.

**Hurl** *(twist)* (Melee). The thrown blade / hammer-head now comes back: it flies out up to
14 m (or until it hits the level), then returns to the thrower's chest, hitting each hostile at
most once per leg: 1.5× on the way out, 1.0× on the way back. The Trimmer's blade passes
through targets; the Tamper's head stops at the first one and comes back from there. Caught
(it reaches you): 40 % of the cooldown back. 2.5 s.

**Riposte** (Melee, defend). Hold ALT to guard, up to 1.0 s, moving at 60 %: blows and shots
from the front 120° are blocked (no damage, no knockback; enemy parryable projectiles are
deflected like a parry, others destroyed; melee attackers are staggered). Release (or the end):
if anything was blocked, a counter-swing with the carrier's arc at 2.0× (+0.5× per further
block, at most 3.0×), and the first block heals like a parry; nothing blocked: no swing, full
cooldown. 4.0 s. Uses the shared **guard**.

**The guard** (shared by Bulwark and Riposte; lands with the groundwork): a player-side state
`{ facing, half_angle, absorb_left, until }` consulted wherever a player takes damage from a
hostile source with a direction (rays, projectiles, explosions' centre, melee, enemy attacks,
lasers): inside the cone, the damage is absorbed. Explosions whose centre is inside the cone
are absorbed whole (no splash leak). Burn ticks and world damage are not blocked.

## 2. The Lamplighter (sniper)

Weapon id **12**, class **Precision**, `ability_power` 1.0, models / sounds of its own. A
lamplighter's long pole turned rifle: a brass tube with a lens at the end, a lit wick in a
glass chamber that brightens as it focuses.

- **Primary, focus** (a new primary style, like the bow's draw but hitscan): hold FIRE to focus
  (0.9 s to full; only once off its interval); release fires one hitscan beam, exact (no
  spread), damage 35 → 130 PvE by focus (weak points ×2 as any ray); a full focus pierces the
  first target and hits one more behind it at ×0.7. A tap fires at the minimum. 0.35 s
  before the next focus. Long range (the usual ray length).
- **Scope**: while focusing, the view zooms in with the focus, to tan(fov/2) × 0.4 at full
  (eased like the bow's zoom), with a lens vignette and a focus ring at the crosshair; mouse
  sensitivity scales with the zoom. Movement is not slowed.
- **Tell**: others see a focusing Lamplighter (its player state carries the focus like the
  bow's draw: `DRAWING` + the level) as a thin glint line along its aim and a lens flare
  that grows with the focus, so a sniper can be spotted and dodged.
- **PvP**: a full-focus headshot may kill (≥ 100), like the full-draw arrow (the one-shot rule
  gets that line); a full-focus body shot does not (~70).

## 3. The Kindle Flask, drunk

Knack id 1 keeps its name and slot; the pool is gone (`KINDLE_POOL`, the flask projectile).
Using it: you drink it (instant, no throw). For 5 s, **haste**: your ground, air and slide
speeds and accelerations ×1.3 (the dash too), and you heal 8 HP every second (40 in all,
`PlayerCombat::heal`, never above the effective max). It's yours only. Allowed in PvP (the
numbers may differ there). 1 charge, 14 s.

Haste lives in `MoveState` next to the Pinned slow (`haste_time`, `haste_scale`; both apply,
multiplied), so the owning client predicts it like any movement. A `HASTE` player flag lets
everyone draw it (embers streaming off the wick, a speed trail); the heal ticks are
`SimEvent::Healed` (`HealSource::KindleFlask`).

## 4. Thorns relay any weapon

The Thorn (PvE only, as before) now relays **every** weapon, not just the Quill's shots. The
first player attack that reaches a live thorn (flying, dropping, planted or lying) shatters it
and is relayed to the nearest hostile from the thorn (the brand and `ricochet_target` rules of
today):

- **rays** (any hitscan primary or ability, pellets included): the ray ricochets as the
  Quill's does (one pellet per thorn: it shatters on the first);
- **projectiles** (bombs, rivets, rockets, arrows, kites, bolas, limpets, hurled blades...):
  the projectile turns at the thorn onto a straight line at the target, keeping its payload
  and speed;
- **melee swings and cones / novas / blasts** whose area holds the thorn: it is flung at the
  target as a ray of the attack's damage (melee: its swing damage; a cone or nova: its per-hit
  damage; an explosion: `thorn.relay_damage`).

Each relay multiplies by `thorn.damage_multiplier` like a ricochet and counts as RICOCHET for
style. The HUD / PLAY.txt / loadout text stop saying "with the Quill".

## 5. Shotguns that hit hard

The Bellows and the Thresher feel weak in play even where the harness says 132–142 DPS at 5 m.
`balance` finds out why in play conditions (moving Moulds at 5–12 m, real pellet spread, hit
assist on pellets, damage numbers / feedback), then fixes it, with these targets:

- PvE: at 0–8 m both are clearly the strongest primaries (≥ 1.35× today's DPS at 5 m), a blast
  that connects staggers a light Mould, and the pellet pattern is consistent (a fixed pattern
  with a little jitter instead of uniform random spread) so a good aim always lands most of it.
- PvP: a full Bellows blast at point blank (≤ 3 m) may kill a body (a stated exception to the
  one-shot rule); the Thresher kills in 2–3 close shots.
- Feedback: heavier blast sound, a bigger hit marker for a mostly-landed blast, more kick.

## 6. Bow hit registration

Arrows (and through them every player projectile) must hit what the shooter saw. `hitreg`
reproduces it first: a latency test (e.g. 60–150 ms RTT, `--netsim`, net loopback) of arrows at
moving and strafing Moulds and players, counting hits the client drew as hits and the server
didn't (and the reverse), then fixes the cause(s): e.g. the server's projectile starting the
input delay behind the client's stand-in, hit tests against current instead of the
shooter-seen hit shapes (lag compensation for a projectile's early flight), the sweep / pad
size for fast arrows, the stand-in drawing a hit the server will never confirm. The fix applies
to every player projectile unless there's a reason not to. Numbers before / after go in
`docs/feel.md`.

## Areas and files

Seven agents in worktrees. **Groundwork first**: `arsenal-a`'s first milestone (M1) lands the
API for everything above so the others build against it.

| area | owns |
|---|---|
| **arsenal-a** | **M1** (groundwork, quick): `shared/src/loadout.rs` (ids, pools, defaults, names, `WeaponKind::Lamplighter`, the settings compatibility), `combat.rs` `Attack` variants, `style.rs` keys, protocol (`PROTOCOL_VERSION` 11, kinds / flags for everything above, retired ones freed), tuning structs + `assets/tuning.ron` sections with the starting numbers, the **guard** hook, `resolve_intent` dispatching each new ability to its own file in `shared/src/sim/abilities/<name>.rs` (stubs), removal of the old abilities everywhere (client code too, minimally, so the workspace compiles and the tests pass). Then **M2**: the Lamplighter (focus style, pierce), Wickline, Tally, Waxwork (incl. the `select_target` hook), Blink Arrow, Beacon Arrow (reveal flag), Hurl's return, Riposte |
| **arsenal-b** | the Kindle Flask (`knacks.rs`, haste in movement), Thorn relays (`hitscan.rs`, `thorn.rs`, the projectile / melee / area relay hooks); after M1: Bolas, Bulwark, Wisp's orbit, Stoke, Lantern, Kite (their files under `sim/abilities/`) |
| **client-a** | presentation of arsenal-a's set: the Lamplighter (model, viewmodel, third person, scope zoom, vignette, focus ring, glint tell, sounds, loadout preview), Wickline, Tally (marks), Waxwork (the double), Blink Arrow (blink effect, the camera snap), Beacon Arrow (beacon, revealed silhouettes through walls), Hurl's return, Riposte (guard pose, block sparks); their loadout texts and icons |
| **client-b** | presentation of arsenal-b's set: drinking the Kindle Flask (viewmodel), haste (trail, wick embers, FOV kick), the heal ticks; relays off thorns from every weapon (tracers, projectile turns, melee bat); Bolas (cord), Bulwark (slab, cracks, shatter), the orbiting / flung Wisp, Stoke (glow ramp, overheat venting, HUD), Lantern (model, its shots), Kite (model, trail); their loadout texts and icons |
| **hitreg** | `shared/src/sim/projectiles.rs` hit tests, lag compensation for projectiles (server plumbing it needs), `client/src/ingame/combat/stand_in.rs`, the latency test(s), `docs/feel.md` section |
| **balance** | phase 1 now: the shotguns (`assets/tuning.ron` bellows / thresher and their PvP overrides; pellet pattern in their firing code; feedback numbers), the harness. Phase 2 after the others merge: every new ability, the Lamplighter, the Kindle Flask; PvP rules; `docs/balance.md`, `docs/arsenal.md` tables, README's weapon / ability tables |
| **readme** | `README.md` (the generative-AI disclaimer, screenshots), `docs/images/`, a script that regenerates the screenshots |

Shared files several areas touch (`weapons.rs` dispatch, `loadout_text.rs`, `sfx.rs`,
`arsenal.rs`, `docs/arsenal.md`, `assets/tuning.ron`): keep your changes to your own arms,
rows and sections; whoever merges later merges main into their branch and resolves.
