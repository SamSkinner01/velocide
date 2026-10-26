# Arsenal: loadouts, builds, weapons, abilities, balance per mode

The third-generation arsenal (`docs/expansion.md` is the contract it implements; the
second-generation weapons and their as-built notes are in `docs/roster.md`). Ten weapons in
five classes, nineteen abilities, loadouts of up to four builds, the Burn and Pinned status
effects, and separate balance for PvE and PvP. Everything a weapon decides is integer-tick and
predicted by its owner; the server stays authoritative for everything else.

## Code map

| where | what |
|---|---|
| `shared::loadout` | `WeaponKind` (10), `WeaponClass` (5), `Ability` (19), `WeaponBuild`, `Loadout`: ids, names, pools, defaults, `sanitized()` (plain data) |
| `shared::sim::weapons` | `WeaponState`, `step_weapons` (predicted), `FireIntent`, firing styles, `SimWorld::resolve_intent` (what every primary and ability does), `ability_tuning` |
| `shared::sim::loadouts` | `SimWorld::request_loadout` / `force_loadout` / `loadout` / `spawn_loadout`: when loadouts apply |
| `shared::sim::{hitscan, area, projectiles}` | rays (Skewer's pierce), cones / novas / blasts (the Kiln spray), projectiles and explosions |
| `shared::sim::payload` | what the new projectiles do on contact: the Flak ring, Limpets, the Flare's patch, the special arrows |
| `shared::sim::status` | Burn, Pinned, burning patches |
| `shared::sim::combat` | `Attack` (a variant per primary and ability), `KillInfo::weapon`, `SimWorld::attack_weapon`, `DamageFlags::{BURN, PIERCED}` |
| `shared::sim::style` | `StyleKey` for every primary and ability |
| `shared::movement` | `MoveState::slow_time` / `slow_scale`: a Pinned player's slow, predicted like the rest of movement |
| `shared::tuning` (`combat`) + `shared::tuning::balance` | the numbers; `BalanceTuning` profiles, `Tuning::for_mode` |
| `shared::protocol` | `pack_player_weapon` / `player_weapon`, projectile and hazard kinds, flag bits, `Fx::Burst` |
| `server` | `ClientMessage::SetLoadout`, effective tuning per mode, `/loadout`, `SimEvent::Burst` → `Fx::Burst` |

## Loadouts and builds

A **build** is a base weapon (fixed numbers, fixed primary) plus two abilities from its class's
pool (`PlayerInput::weapon_mode` 0 / 1 = ability A / B; pressing the slot key again cycles). A
**loadout** is up to four builds on the slot keys 1–4 (`PlayerInput::weapon` = slot index):
at least one weapon, no weapon twice, empty slots skipped. `Loadout::sanitized` fixes anything
else (invalid builds → the weapon's defaults, repeats dropped, nothing → the default loadout:
Quill, Bellows, Arclash, Tumbler with their second-generation abilities).

| class | weapons | ability pool (menu order) |
|---|---|---|
| Precision | Quill, Lancet | Brand, Splinter, Skewer, Flurry |
| Scatter | Bellows, Thresher | Gust, Both Barrels, Slug, Flak |
| Rapid | Arclash, Riveter, Kiln | Wisp, Discharge, Volley, Flare |
| Explosive | Tumbler, Cinderbox | Breach, Gravewell, Limpet, Mortar |
| Draw | Longwick | Brand, Triple Nock, Pinning Arrow, Ember Arrow |

### When a loadout applies

```rust
impl SimWorld {
    /// Stores the player's chosen loadout (sanitised); applies it per the rules.
    pub fn request_loadout(&mut self, id: PlayerId, loadout: Loadout);
    /// Forces a loadout now (gun game tier changes): replaces the weapons immediately,
    /// selects the first slot, keeps nothing of the old weapons' state. `None` lifts the
    /// override (the requested loadout applies at the next spawn).
    pub fn force_loadout(&mut self, id: PlayerId, loadout: Option<Loadout>);
    /// The loadout the player is carrying right now (no body: the one they'd spawn with).
    pub fn loadout(&self, id: PlayerId) -> Loadout;
    /// What the next (re)spawn hands out: forced, else requested, else the default.
    pub fn spawn_loadout(&self, id: PlayerId) -> Loadout;
}
```

* A request applies **at once** while the player is dead (co-op downed included), while the
  world is a warm-up (`SimWorld::warmup`, set by the lobby's warm-up world), or when the body is
  fresh and hasn't processed an input yet (a player joining mid-match sends `SetLoadout` right
  after connecting, a moment after their body spawned). Otherwise it waits for the next
  (re)spawn: `place_at_spawn` (deathmatch respawns, campaign checkpoint respawns, team wipes),
  `restart_level` and `add_player` all hand out `spawn_loadout`.
* Applying keeps the selected slot when it still holds a weapon (else the first weapon), the
  mode, fist / parry / thorn state and edge detection; every weapon's cooldowns, charges and
  bursts start afresh (`WeaponState::with_loadout`; at a spawn: `WeaponState::respawned`).
* A forced loadout applies at once (first slot selected) and wins over requests, also at
  respawns, until `force_loadout(id, None)`.
* Requests live in `SimWorld::loadout_requests`; the server carries them from one world to the
  next (`replace_world`), so a loadout chosen in the lobby is there when the match spawns
  everyone. Forced loadouts (`SimWorld::forced_loadouts`) belong to one world. Both are
  forgotten when the player leaves (`remove_player`).

Server: `ClientMessage::SetLoadout` (any phase) → `request_loadout` (the world sanitises),
rate-limited per client (token bucket: burst 8, 2 per second; extra messages are dropped).
Dev: `/loadout longwick:pinning,ember kiln quill` (slot by slot, `-` = empty slot, abilities by
name prefix, missing ones = the weapon's defaults) sets the loadout **now** (and as the request);
`/loadout` alone shows it. Usable in the lobby too.

## The weapon state machine (`step_weapons`)

`WeaponState` (all integers; `OwnState::combat.weapons`, compared bit-exactly by the client):

| field | meaning |
|---|---|
| `loadout` | the builds carried (only the server changes it; packed into one `u64` on the wire) |
| `slot`, `mode` | selected slot (always one holding a weapon) and ability (0 / 1) |
| `prev_buttons` | edge detection |
| `primary_cooldown[4]`, `alt_cooldown[4]` | per slot; a build's two abilities share one |
| `charge` | hold-to-charge abilities (Discharge, Skewer) and the Limpet's hold: ticks held |
| `draw` | Longwick: ticks drawn (0 = not drawing) |
| `armed` | Longwick: ability armed for the next release (0 none, 1 A, 2 B) |
| `bloom` | Lancet heat (ticks) |
| `burst` | a burst in progress: shots left, ticks to the next, which ability (`None` = the primary salvo) |
| `punch_cooldown`, `parry`, `thorns_used`, `thorn_recharge` | fist and Thorn (unchanged) |

Selection: `PlayerInput::weapon` picks a slot; an empty slot or an out-of-range value keeps
the current weapon. Switching weapon cancels the charge, the draw, the arming and a burst;
switching ability cancels the charge and the arming.

| primary style | weapons | rule |
|---|---|---|
| automatic | Quill, Bellows, Arclash, Tumbler, Lancet, Thresher, Riveter, Kiln | while `FIRE` is held, one shot per interval |
| salvo | Cinderbox | a pull fires `cinderbox.rockets` rockets `salvo_gap` apart (a `burst`) |
| draw | Longwick | hold to draw (only once off its interval), release looses (`charge` = draw 0..1) |

| ability style | abilities | rule |
|---|---|---|
| instant | most | while `ALT_FIRE` is held, once per cooldown (Both Barrels and Volley also put the primary on its interval) |
| charge | Discharge, Skewer | hold to charge once off cooldown, fires on release (`charge` 0..1) |
| burst | Flurry | `flurry.shots` shots `flurry.gap` apart |
| hold | Limpet | a tap (released before `limpet.detonate_hold`) fires with `charge` 0 = throw, off cooldown; holding to it fires once with `charge` 1 = detonate (any time) |
| armed (any ability on a Draw weapon) | Brand, Triple Nock, Pinning Arrow, Ember Arrow on the Longwick | pressing `ALT_FIRE` (off cooldown) arms it, pressing again disarms; the next release fires it with the draw, and starts its cooldown. Triple Nock below `min_draw` looses a plain arrow and stays armed |

`FireIntent::{Primary { weapon, charge }, Ability { weapon, ability, charge }, Punch,
ThrowThorn}`: `charge` is the draw (Longwick), the bloom (Lancet: the first shot is exact,
sustained fire settles at 75 % of `bloom_degrees`) or the ability charge. THROW does nothing
while `thorn.charges` is 0 (the PvP profile).

## Weapons

| id | weapon | class | primary (PvE numbers) |
|---|---|---|---|
| 0 | Quill | Precision | unchanged (`docs/roster.md`): 24 dmg hitscan, 0.28 s, ricochets off Thorns |
| 1 | Bellows | Scatter | unchanged: 12 × 8, 8°, 0.8 s |
| 2 | Arclash | Rapid | unchanged: 6 dmg zaps every 0.1 s, chaining |
| 3 | Tumbler | Explosive | unchanged: bouncing bombs, r 4.5 m, 65 dmg, 0.85 s |
| 4 | Lancet | Precision | hitscan needle, 14 dmg, 0.15 s; each shot heats it (+0.25 s, cap 0.6 s, drains in real time) and the spread grows to 3° at full heat — the first shot is exact, sustained fire settles at 2.25° |
| 5 | Thresher | Scatter | 7 × 7 dmg, 11°, 45 m, 0.42 s, automatic while held |
| 6 | Riveter | Rapid | rivet projectile 90 m/s, gravity 6, 7 dmg, 1.2° spread, 0.09 s (5 ticks: 12/s at 60 Hz; 11/s is not a whole number of ticks) |
| 7 | Cinderbox | Explosive | a pull fires 3 rockets 0.08 s apart (5 ticks), 42 m/s, 2.5° spread, r 2.6 m, 32 dmg, kb 14; 1.3 s per salvo |
| 8 | Longwick | Draw | hold to draw (0.75 s to full), release: arrow 45–120 m/s, 22–105 dmg by draw, gravity 14, 0.25 s before the next draw; a tap looses a weak, slow arrow |
| 9 | Kiln | Rapid | every 0.08 s (5 ticks) a 13 m / 16° cone: 3 dmg + 1 Burn stack to everything inside (line of sight, no weak points, no ricochet) |

Only the Quill's shots ricochet off Thorns, whichever ability fires them (its identity): Brand,
Splinter and Flurry ricochet on the Quill and not on the Lancet; Skewer pierces instead.

## Abilities

Existing abilities keep their rules (`docs/roster.md`); on another weapon of their class they
behave the same (their damage scaled, below). New ones, as built:

| ability | as built |
|---|---|
| Skewer | hold to charge ≤ 0.8 s, release: a hitscan shot, 30–90 dmg by charge, through up to 3 targets (nearest first, before the level stops it), ×0.8 for each target passed (`PIERCED` after the first). No ricochet. 1.4 s |
| Flurry | 3 shots 0.06 s apart (4 ticks), each 0.8 × the weapon's primary damage, 1° spread; 1.6 s |
| Slug | one exact hitscan slug, 55 dmg, kb 6; 1.2 s |
| Flak | a shell (45 m/s, straight) that bursts after 14 m of flight, or 0.5 m short of a target it touches, or off a wall (the ring then sprays along the reflected flight): 10 pellet rays evenly around the flight axis, each 25° off it, 14 m, 9 dmg each. On contact the ring closes on the target (a point-blank shotgun); in the open it fans out behind cover and around corners. Not parryable. `SimEvent::Burst` / `Fx::Burst` carry the rays. 1.8 s |
| Volley | 10 of the weapon's own shots at once in a 6° cone: rivets (Riveter), chaining zaps (Arclash), ember rays that each set a Burn stack (Kiln); the primary waits its interval too. 2.2 s |
| Flare | a flare lobbed 10° above the aim (22 m/s, gravity 16); where it lands — on the floor, at the feet of what it hit, under where it hit a wall, or below where it burns out after 4 s — a 3 m burning patch for 4 s (no patch over a pit). 3.0 s |
| Limpet | tap: throws a charge (24 m/s, gravity 16) that sticks to the level or to the enemy / player it hits and rides along; hold 0.25 s: every charge out detonates (r 3.5 m, 50 dmg, kb 14, a direct hit on what it sticks to). Up to 3 out: a throw with 3 out detonates them instead. A charge left alone goes off after 20 s. 0.4 s between throws |
| Mortar | a shell lobbed 35° above the aim at 26 m/s, gravity 20: r 6 m, 85 dmg, kb 26; 3.0 s |
| Triple Nock | 3 arrows in a 7° fan at the current draw (≥ 30 %), each a plain arrow's damage; 1.4 s |
| Pinning Arrow | an arrow at the draw that pins what it hits (Pinned, below); 2.5 s |
| Ember Arrow | an arrow at the draw whose own hit is 0.75 × a plain arrow's, then it bursts — on a target, on the level or at the end of its flight: r 3 m, 30 dmg, kb 8, and 2 Burn stacks on everything in the burst; 3.0 s |
| Brand (on the Longwick) | a branding arrow at the draw: what it hits is branded exactly like a Brand shot (`quill.brand_*`) |

### Abilities on other weapons

Every ability's numbers are written for its **home weapon** (the first weapon whose default
build has it: Brand / Splinter → Quill, Skewer / Flurry → Lancet, Gust / Both Barrels →
Bellows, Slug / Flak → Thresher, Wisp / Discharge → Arclash, Volley → Riveter, Flare → Kiln,
Breach / Gravewell → Tumbler, Limpet / Mortar → Cinderbox, the arrows → Longwick). Each weapon
has an `ability_power`; an ability fired from a weapon whose power is below its home weapon's
deals its damage (every damage number of it, through the balance table's damage column, and
the Flare's burn rate) × `carrier power / home power` — **never more than listed**
(`weapons::ability_tuning`). Abilities made of the weapon's own shots (Flurry, Volley) take
the carrier's primary damage instead and are not scaled again.

| weapon | `ability_power` | why |
|---|---|---|
| Quill, Bellows, Arclash, Tumbler, Longwick | 1.0 | the slower, committed weapons: their abilities hit as listed |
| Cinderbox | 0.95 | its salvo already spreads damage wide |
| Thresher, Riveter | 0.9 | automatic weapons with high sustained damage |
| Lancet, Kiln | 0.85 | the fastest trigger and the one that also burns |

The rule keeps the strongest ability from simply riding the fastest gun (Splinter or Brand on
the Lancet, Discharge on the Kiln hit a little softer), while an ability on its home weapon or
on a weightier one is exactly what its numbers say — so the PvP limits below hold on every
carrier.

## Status effects

**Burn** (`status.rs`; `burn` tuning): `dps` 4 per stack, up to `max_stacks` 3, for
`duration` 3 s — every new stack refreshes it, the latest source owns it (kill credit, its
attack: Kiln / Flare / Ember Arrow / Volley). Damage is dealt every `tick_interval` 0.25 s
through the damage pipeline with **only** `DamageFlags::BURN` (never HITSCAN or PROJECTILE,
so a burn death is never a LONGSHOT), on the body. Applies to enemies and, in PvP, to players
(the usual hostility rules: never teammates, never yourself). Dying puts it out. Brands
multiply it like any damage.

**Pinned** (`pinning_arrow` tuning): an enemy is rooted where it stood for 1.4 s (an enemy
whose base health is at least `heavy_health` 300: 0.6 s) — it keeps turning and attacking,
only its horizontal position is held (also against knockback); a PvP player is slowed to 50 %
of their speeds (ground, air, dash, slide) for 1.0 s and their horizontal velocity halved at
the hit. The player's slow lives in their `MoveState` (`slow_time`, `slow_scale`), so the
owning client predicts it like any other movement after the one correction at the hit.

**Burning patches** (Flare): a disc on the ground (`flare.patch_radius` 3 m, `patch_time` 4 s);
every hostile whose feet are inside (up to 2 m above it) gets `flare.burn_rate` 2 Burn stacks
per second. Patches live in `SimWorld::fire_patches` and replicate as `BURNING_PATCH`.

Replication: `BURNING` / `PINNED` are OR-ed into the target's entity flags by
`SimWorld::replicate` (statuses live in `SimWorld::statuses`, not on the target).

## Attacks, kills, style

`Attack` has a variant per primary (`Lancet`, `Thresher`, `Riveter`, `Cinderbox`, `Longwick`,
`Kiln`; the old four keep `Quill`, `Blast`, `Arc`, `Tumble`) and per ability (`Skewer` …
`EmberArrow`), appended after `Dev`. `Attack::primary(weapon)`, `Attack::ability(ability)`,
`primary_of()`, `as_ability()`; `name()` is the kill-feed name. An ability is the same attack on
every weapon, so `Attack::weapon()` is exact for primaries and for abilities only one weapon can
carry (the bow's arrows) and `None` for shared ones; **`KillInfo::weapon`** (filled by
`SimWorld::attack_weapon` from the killer's loadout, the weapon in hand first) says which
weapon fired the killing blow. Burn and patch damage carry the attack that set them.

Style freshness has a key per primary and per ability (`StyleKey::primary`,
`StyleKey::ability`, `StyleKey::build_keys(&build)` for the HUD's three pips), plus fist, slam,
parry, thorn: 33 keys. The first sixteen keep their old order. On the wire `StyleView` sends a
bitmask of the keys below full freshness and only their values (4 B when everything is fresh).

## Replication and wire ids

**Player entity `state`**: held `WeaponKind` id (bits 0–3) | selected `Ability` id (bits 4–8) |
charge / draw level (bits 9–15, 7 bits). Always through the helpers:

```rust
pub fn pack_player_weapon(weapon: WeaponKind, ability: Ability, charge: u8) -> u16;
/// (held weapon, selected ability, charge / draw 0..=255); unknown ids → Quill / Brand.
pub fn player_weapon(state: u16) -> (WeaponKind, Ability, u8);
```

The level is the bow's draw while `DRAWING`, else the Discharge / Skewer charge (or the
Limpet's hold) while `CHARGING` (0 otherwise; 0 and 255 survive exactly).

| flags | bit | meaning |
|---|---|---|
| player `BURNING` | 12 | burning (PvP) |
| player `PINNED` | 13 | slowed by a Pinning Arrow (PvP) |
| player `DRAWING` | 14 | drawing the Longwick |
| enemy `BURNING` | 13 | burning |
| enemy `PINNED` | 14 | rooted by a Pinning Arrow |

| kind | name | notes (trajectory-replicated like every projectile; `owner` = player) |
|---|---|---|
| 64 | `ARROW` | plain arrow (Triple Nock's too), gravity `longwick.arrow_gravity` |
| 65 | `RIVET` | gravity `riveter.rivet_gravity` |
| 66 | `CINDER_ROCKET` | straight, explodes |
| 67 | `FLAK_SHELL` | straight; its burst is `Fx::Burst` |
| 68 | `LIMPET` | gravity `limpet.gravity`; `STUCK` once attached (`position` current; it follows what it sticks to) |
| 69 | `MORTAR_SHELL` | gravity `mortar.gravity`, explodes |
| 70 | `FLARE` | gravity `flare.gravity`; its landing is a `ProjectileImpact` + a `BURNING_PATCH` |
| 71 | `PIN_ARROW` | arrow gravity |
| 75 | `EMBER_ARROW` | arrow gravity, bursts (an `Explosion` with `attack` `EmberArrow`) |
| 76 | `BRAND_ARROW` | arrow gravity |
| 77–79 | reserved | |
| 136 | `BURNING_PATCH` | `position` = centre on the floor, `owner` = owner code, `state` = radius (dm), `aux` = burn-out tick |
| 137–139 | reserved | |

`Fx::Burst { shooter, attack, origin, ends }` (everyone, repeated like other important
effects): a projectile burst into rays — the Flak ring. Owner-only: `OwnState::combat.weapons`
carries the whole `WeaponState`, loadout included (47 B of own combat state per snapshot with
the default loadout), and `OwnState::movement` the slow (8 B).

## Tuning and balance per mode

`assets/tuning.ron` `combat` holds every weapon's and ability's base numbers: the old four
weapons' sections (plus `ability_power`, and `bellows.both_pellet_damage` so Both Barrels no
longer borrows the primary's pellet damage), then `lancet`, `thresher`, `riveter`, `cinderbox`,
`longwick`, `kiln`, one section per new ability (`skewer` … `ember_arrow`) and `burn`.

The `balance` section has two profiles; `pve` applies to campaign and grind (and warm-ups
before them), `pvp` to deathmatch and gun game. Each scales `combat` per weapon (primary) and
per ability with `BalanceScale` multipliers (1.0 when left out): `damage`, `interval` (fire
interval / the bow's draw time / cooldown), `radius` (splash, cone, area), `knockback`, `speed`
(projectiles); then `overrides` set single numbers by field path inside `combat`
(`"thorn.charges": 0`; integer fields take the rounded value; a path that names no number is
skipped with a warning). Which fields each multiplier scales is the table in
`CombatTuning::scale_weapon` / `scale_ability` (a test checks every weapon and ability is in
it). Self-damage is profile-wide (`"self_damage_fraction"` override).

`Tuning::for_mode(mode)` returns the **effective tuning**: `combat` with the mode's profile
applied and sanitised, and a neutral `balance` (applying it again changes nothing). The server
keeps the file's tuning as `base_tuning`, runs the world with the effective one and sends that
to clients (`ServerMessage::Tuning`), so prediction always matches: recomputed whenever a new
world starts (`replace_world`) and on every (hot) reload (`set_tuning`). Whoever changes the
running world's mode (the lobby's warm-up) calls `Server::update_effective_tuning`.

### Balance profiles

**PvE** changes nothing: today's campaign feel (and the numbers above) is the reference, and
nothing in play gave a reason to move a co-op number yet. Difficulty and player-count scaling
of the Moulds are the modes area's (`difficulty` section), not a weapon profile.

**PvP** is tuned for players killing players: 100 HP, headshots ×2 (`player_head_multiplier`)
for everything that isn't explosive, a time to kill of **0.8–1.6 s** for a skilled player at
the weapon's range, and **one-shots only from a full-draw bow headshot, a Slug headshot and a
full-charge Skewer headshot**. The Thorn is off (`thorn.charges` 0: THROW does nothing; hiding
the pips is the HUD's) and a brand multiplies by 1.25 instead of 1.35 (in a free-for-all a
brand invites everyone, the extra was too much on top of a third party).

| weapon / ability | PvP | after | time to kill (all on the body; heads faster) and why |
|---|---|---|---|
| Quill | 1.0 | 24 / 0.28 s | 5 hits: 1.12 s (3 heads: 0.56 s). The reference precision weapon |
| Lancet | damage 0.85 | 11.9 / 0.15 s | 9 hits: 1.2 s; raw it out-damaged the Quill at range despite the bloom |
| Bellows | damage 0.7 | 12 × 5.6 = 67 a blast | 2 blasts: 0.8 s; at 8 dmg a point-blank blast with a few head pellets one-shot |
| Thresher | damage 0.8 | 7 × 5.6 = 39 a blast | 3 blasts: 0.84 s; 2 blasts with head pellets killed in 0.42 s |
| Arclash | 1.0 | 6 / 0.1 s | 17 zaps: 1.6 s (chains help in fights of three) |
| Riveter | 1.0 | 7 / 5 ticks | 15 rivets: 1.17 s; projectiles at 90 m/s already cost aim |
| Kiln | damage 1.2 | 3.6 / 5 ticks + 12 dps burn | ~1.8 s inside 13 m, and the 3 s burn keeps going (36) — the counterplay is breaking contact, so the spray itself had to threaten |
| Tumbler | 1.0 | 65 a direct bomb | 2 direct: 0.85 s; splash kills take three |
| Cinderbox | damage 0.75 | 3 × 24 = 72 a salvo | 2 salvos: 1.3 s; three 32s (96) were a body one-shot in 0.16 s with any brand |
| Longwick | damage 0.5, interval 0.85 | 11–52.5 by draw, 0.64 s draw | a full-draw headshot (105) is the one-shot; 90 % draw (48) is not; body: 2 full draws, 1.49 s |
| Both Barrels | damage 0.4 | 24 × 3.2 = 77 | 192 point blank was a one-shot; it stays the blast-jump |
| Wisp | damage 0.7 | 5.6 a zap | it aims itself: 40 dps of free damage was too much |
| Discharge | damage 0.9 | 22.5–72 | no one-shot even with a hit before it |
| Breach | damage 0.85 | 59.5 | the panic button, not a kill button |
| Gravewell | damage 0.75 | 60 | players are pulled at half strength; the implosion still punishes staying |
| Skewer | damage 0.58 | 17–52 (on the Lancet) | full charge to the head (104) is the one-shot, 90 % (97) is not; it still pierces a line of players |
| Flurry | damage 0.75 | Quill: 3 × 14.4 | three Quill heads in 0.13 s (115) were a one-shot |
| Flak | damage 0.8 | 10 × 7.2 | a contact burst landing every pellet stays under 100 |
| Volley | damage 0.7 | Riveter: 10 × 4.9 | ten rivets at once (70, more with heads) was an instant kill up close |
| Limpet | damage 0.6 | 30 a charge | three charges on one player (90) is a kill only with a hit before it |
| Mortar | damage 0.75, radius 0.85 | 64, r 5.1 m | a huge blast is area denial in PvE but a free kill on a corner in PvP |
| Triple Nock | damage 0.5 | 3 × 26 | three full arrows (315 PvE, 158 at the bow's PvP damage) |
| everything else | 1.0 | | Gust (6 dmg, the knockback is the point), Brand (10), Splinter, Slug (55: its headshot is the allowed one-shot), Flare (burn), Pinning / Ember / Brand arrows (they ride the bow's 0.5) |

`shared/tests/combat_arsenal.rs::pvp_profile_keeps_the_one_shot_rules_and_the_time_to_kill_band`
recomputes these from the effective tuning: no body one-shot from any trigger pull or ability
use, exactly the allowed headshot one-shots, and every weapon's all-body time to kill inside
0.8–1.8 s.

## As-built notes

* The Flak ring, Limpet blasts, patches and burns are server-side effects of server-simulated
  projectiles: they test the current hitboxes (no lag compensation, like every projectile).
* Explosions lose damage with distance from their centre to the target's (padded) shape, so a
  charge stuck on a target deals ~98 % (it sits its own radius off the surface), like a bomb's
  direct hit.
* A Gust turns in-flight player projectiles around as before (flak shells, flares, limpets in
  flight, arrows): they keep their contact rules and become the deflector's, attack `Gust`.
* Stuck Limpets follow their carrier; when it dies they stay where it was (like the Gravewell).
* Kill-feed weapon, potg titles: `KillInfo::weapon` / `combat::weapon_attack_label` (potg).

## Tests

* `shared/src/sim/weapons.rs` (unit): intervals, switching, charges, Both Barrels, punch /
  thorns (none at 0 charges), empty slots, every weapon × ability fires, salvo / Flurry bursts,
  bow draw + arming + Triple Nock's minimum, Lancet bloom, Limpet tap / hold, ability homes and
  power, lossless loadout packing.
* `shared/src/tuning/balance.rs` (unit): override paths, PvE = reference / PvP no Thorns /
  idempotent, the multiplier table covers everything, profiles from RON.
* `shared/tests/combat_loadouts.rs`: when requests apply (fresh / dead / warm-up / respawn),
  sanitising, forced loadouts, bodiless players and leaving, `restart_level`, the replicated
  weapon / ability / draw, every weapon and ability firing in a live world, `KillInfo::weapon`
  for a shared ability, exact prediction with a custom loadout.
* `shared/tests/combat_arsenal.rs`: every new primary (Lancet bloom, Thresher, Riveter flight,
  Cinderbox salvo, Longwick draw, Kiln + Burn), Burn stacks / refresh / kill credit / no
  LONGSHOT, Skewer pierce and falloff, Flurry, Slug, the Flak airburst and contact burst,
  Volley, Flare patches, Limpets (stick, follow, hold, the fourth throw), Mortar, Triple Nock's
  fan, Pinning Arrow (enemy root, heavy, PvP slow at half speed, predicted), Ember Arrow, the
  bow's Brand, Burn on PvP players but not teammates, ability power, the PvP profile's rules.
* `server/tests/arsenal_net.rs`: PvP clients get 0 thorn charges, co-op clients 3, a reload
  recomputes; a loadout sent in the lobby is what the match spawns (sanitised), a change in
  play waits for the respawn.
* `server/src/commands.rs` (unit): `/loadout` argument parsing.
