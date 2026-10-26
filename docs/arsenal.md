# Arsenal: loadouts, builds, weapons, abilities, balance per mode

The third-generation arsenal (`docs/expansion.md` is the contract it implements; the
second-generation weapons and their as-built notes are in `docs/roster.md`). Ten weapons in
five classes, nineteen abilities, loadouts of up to four builds, and separate balance for PvE
and PvP. Everything a weapon decides is integer-tick and predicted by its owner; the server
stays authoritative for everything else.

> **Status.** Milestone 1 (architecture) is in: loadouts and builds, the API below, the
> weapon state machine for every weapon and ability, replication, wire ids, attacks, style
> keys, the balance framework and the PvP profile without Thorns. Some new mechanics still
> fire **placeholders** (marked *placeholder* below); milestone 2 brings them, Burn / Pinned,
> burning patches, the PvP tuning and its reasoning.

## Code map

| where | what |
|---|---|
| `shared::loadout` | `WeaponKind` (10), `WeaponClass` (5), `Ability` (19), `WeaponBuild`, `Loadout`: ids, names, pools, defaults, `sanitized()` (plain data) |
| `shared::sim::weapons` | `WeaponState`, `step_weapons` (predicted), `FireIntent`, firing styles, `SimWorld::resolve_intent` (what every primary and ability does), `ability_tuning` |
| `shared::sim::loadouts` | `SimWorld::request_loadout` / `force_loadout` / `loadout` / `spawn_loadout`: when loadouts apply |
| `shared::sim::{hitscan, area, projectiles}` | rays, cones / novas / blasts (the Kiln spray), projectiles and explosions |
| `shared::sim::combat` | `Attack` (a variant per primary and ability), `KillInfo::weapon`, `SimWorld::attack_weapon` |
| `shared::sim::style` | `StyleKey` for every primary and ability |
| `shared::tuning` (`combat`) + `shared::tuning::balance` | the numbers; `BalanceTuning` profiles, `Tuning::for_mode` |
| `shared::protocol::snapshot` | `pack_player_weapon` / `player_weapon`, projectile and hazard kinds, flag bits |
| `server` | `ClientMessage::SetLoadout`, effective tuning per mode, `/loadout` |

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
| `charge` | hold-to-charge abilities (Discharge, Skewer): ticks held |
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
| press | Limpet | once per press |
| armed (any ability on a Draw weapon) | Brand, Triple Nock, Pinning Arrow, Ember Arrow on the Longwick | pressing `ALT_FIRE` (off cooldown) arms it, pressing again disarms; the next release fires it with the draw, and starts its cooldown. Triple Nock below `min_draw` looses a plain arrow and stays armed |

`FireIntent::{Primary { weapon, charge }, Ability { weapon, ability, charge }, Punch,
ThrowThorn}`: `charge` is the draw (Longwick), the bloom (Lancet: the first shot is exact,
sustained fire settles at 75 % of `bloom_degrees`) or the ability charge. THROW does nothing
while `thorn.charges` is 0 (the PvP profile).

## Weapons

| id | weapon | class | primary (as built) |
|---|---|---|---|
| 0 | Quill | Precision | unchanged (`docs/roster.md`) |
| 1 | Bellows | Scatter | unchanged |
| 2 | Arclash | Rapid | unchanged |
| 3 | Tumbler | Explosive | unchanged |
| 4 | Lancet | Precision | hitscan needle, 14 dmg, 0.15 s, bloom up to 3° (no Thorn ricochet: that stays the Quill's) |
| 5 | Thresher | Scatter | 7 × 7 dmg, 11°, 45 m, 0.42 s, automatic |
| 6 | Riveter | Rapid | rivet projectile 90 m/s, gravity 6, 7 dmg, 0.09 s (5 ticks: 12/s at 60 Hz; 11/s is not a whole number of ticks), 1.2° spread |
| 7 | Cinderbox | Explosive | salvo of 3 rockets 0.08 s apart, 42 m/s, 2.5° spread, r 2.6 m, 32 dmg, kb 14, 1.3 s |
| 8 | Longwick | Draw | arrow 45–120 m/s, 22–105 dmg by draw (0.75 s), gravity 14, 0.25 s recovery after a release |
| 9 | Kiln | Rapid | cone 13 m / 16°, every 0.08 s 3 dmg to everything inside (line of sight, no weak points, no ricochet); Burn *placeholder* (no Burn yet) |

Only the Quill's shots ricochet off Thorns, whichever ability fires them (its identity): Brand,
Splinter, Skewer and Flurry ricochet on the Quill and not on the Lancet.

## Abilities

Existing abilities keep their rules (`docs/roster.md`); on another weapon of their class they
behave the same. New ones, as built so far:

| ability | as built |
|---|---|
| Skewer | charge ≤ 0.8 s, release: one hitscan shot 30–90 dmg; *placeholder*: no piercing yet |
| Flurry | 3 shots 0.06 s apart (4 ticks), each 0.8 × the weapon's primary damage, 1° spread, 1.6 s |
| Slug | hitscan slug 55 dmg, kb 6, exact, 1.2 s |
| Flak | shell 45 m/s; *placeholder*: a direct-hit shell (9 dmg) that fizzles at 14 m — no airburst ring yet |
| Volley | 10 of the weapon's primary at once in a 6° cone (rivets / chaining zaps / sprays as rays), 2.2 s |
| Flare | lobbed flare (+10°, 22 m/s, gravity 16); *placeholder*: no burning patch yet |
| Limpet | thrown charge (24 m/s, gravity 16), r 3.5 m 50 dmg; *placeholder*: explodes on contact, no sticking / remote detonation yet |
| Mortar | shell lobbed +35° at 26 m/s, gravity 20, r 6 m, 85 dmg, kb 26, 3.0 s |
| Triple Nock | 3 arrows in a 7° fan at the current draw (≥ 30 %), 1.4 s |
| Pinning Arrow | a pinning arrow at the draw; *placeholder*: no Pinned status yet |
| Ember Arrow | an arrow that bursts on impact, r 3 m, 30 dmg; *placeholder*: no Burn yet, and the burst replaces the arrow's own hit |
| Brand (on the Longwick) | a branding arrow at the draw; *placeholder*: hits as Brand but doesn't brand yet |

### Abilities on other weapons

Every ability's numbers are written for its **home weapon** (the first weapon whose default
build has it: Brand/Splinter → Quill, Skewer/Flurry → Lancet, Gust/Both Barrels → Bellows,
Slug/Flak → Thresher, Wisp/Discharge → Arclash, Volley → Riveter, Flare → Kiln, Breach/Gravewell
→ Tumbler, Limpet/Mortar → Cinderbox, the arrows → Longwick). Each weapon has an
`ability_power`; an ability fired from another weapon deals its damage × `carrier power / home
power` (`weapons::ability_tuning`, through the balance table's damage column). All powers are
1.0 for now (milestone 2 tunes them). Abilities that are made of the weapon's own shot (Flurry,
Volley) use the carrying weapon's primary damage directly.

## Attacks, kills, style

`Attack` has a variant per primary (`Lancet`, `Thresher`, `Riveter`, `Cinderbox`, `Longwick`,
`Kiln`; the old four keep `Quill`, `Blast`, `Arc`, `Tumble`) and per ability (`Skewer` …
`EmberArrow`), appended after `Dev`. `Attack::primary(weapon)`, `Attack::ability(ability)`,
`primary_of()`, `as_ability()`; `name()` is the kill-feed name. An ability is the same attack on
every weapon, so `Attack::weapon()` is exact for primaries and for abilities only one weapon can
carry (the bow's arrows) and `None` for shared ones; **`KillInfo::weapon`** (filled by
`SimWorld::attack_weapon` from the killer's loadout, the weapon in hand first) says which
weapon fired the killing blow — use it for "[LONGWICK]" and gun-game tiers.

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

The level is the bow's draw while `DRAWING`, else the Discharge / Skewer charge while
`CHARGING` (0 otherwise; 0 and 255 survive exactly).

| flags | bit | meaning |
|---|---|---|
| player `BURNING` | 12 | Burn status (PvP) — milestone 2 |
| player `PINNED` | 13 | Pinned (slowed, PvP) — milestone 2 |
| player `DRAWING` | 14 | drawing the Longwick |
| enemy `BURNING` | 13 | Burn status — milestone 2 |
| enemy `PINNED` | 14 | rooted by a Pinning Arrow — milestone 2 |

| kind | name | notes (trajectory-replicated like every projectile; `owner` = player) |
|---|---|---|
| 64 | `ARROW` | plain arrow (Triple Nock's too), gravity `longwick.arrow_gravity` |
| 65 | `RIVET` | gravity `riveter.rivet_gravity` |
| 66 | `CINDER_ROCKET` | straight, explodes |
| 67 | `FLAK_SHELL` | straight |
| 68 | `LIMPET` | gravity `limpet.gravity`; `STUCK` once attached (milestone 2) |
| 69 | `MORTAR_SHELL` | gravity `mortar.gravity`, explodes |
| 70 | `FLARE` | gravity `flare.gravity` |
| 71 | `PIN_ARROW` | arrow gravity |
| 75 | `EMBER_ARROW` | arrow gravity, bursts |
| 76 | `BRAND_ARROW` | arrow gravity |
| 77–79 | reserved | |
| 136 | `BURNING_PATCH` | hazard: centre, `state` = radius (dm), `aux` = burn-out tick (milestone 2) |
| 137–139 | reserved | |

Owner-only: `OwnState::combat.weapons` carries the whole `WeaponState`, loadout included
(47 B of own combat state per snapshot with the default loadout, was 32).

## Tuning and balance per mode

`assets/tuning.ron` `combat` holds every weapon's and ability's base numbers: the old four
weapons' sections (plus `ability_power`, and `bellows.both_pellet_damage` so Both Barrels no
longer borrows the primary's pellet damage), then `lancet`, `thresher`, `riveter`, `cinderbox`,
`longwick`, `kiln`, one section per new ability (`skewer` … `ember_arrow`) and `burn`.

The `balance` section has two profiles; `pve` applies to campaign and grind (and warm-ups
before them), `pvp` to deathmatch and gun game:

```ron
balance: (
    pve: (weapons: {}, abilities: {}, overrides: {}),
    pvp: (
        weapons: { Longwick: (damage: 0.9) },             // per weapon (primary)
        abilities: { Mortar: (radius: 0.8, interval: 1.2) }, // per ability
        overrides: { "thorn.charges": 0.0 },             // absolute, by path inside `combat`
    ),
),
```

Multipliers (`BalanceScale`, 1.0 when left out): `damage`, `interval` (fire interval / the
bow's draw time / cooldown), `radius` (splash, cone, area), `knockback`, `speed` (projectiles).
Which fields each one scales is the table in `CombatTuning::scale_weapon` /
`scale_ability` (a test checks every weapon and ability is in it). Overrides set single numbers
after the multipliers (integer fields take the rounded value; a path that names no number is
skipped with a warning). Self-damage is profile-wide (`"self_damage_fraction"` override).

`Tuning::for_mode(mode)` returns the **effective tuning**: `combat` with the mode's profile
applied and sanitised, and a neutral `balance` (applying it again changes nothing). The server
keeps the file's tuning as `base_tuning`, runs the world with the effective one and sends that
to clients (`ServerMessage::Tuning`), so prediction always matches: recomputed whenever a new
world starts (`replace_world`) and on every (hot) reload (`set_tuning`). Whoever changes the
running world's mode (the lobby's warm-up) calls `Server::update_effective_tuning`.

Profiles as built: **PvE** changes nothing (the campaign feel is the reference). **PvP** has no
Thorns (`thorn.charges` 0: THROW does nothing; hiding the pips and the "Q" hint is the HUD's,
arsenal-client); its per-weapon tuning comes with milestone 2.

## Tests

* `shared/src/sim/weapons.rs` (unit): intervals, switching, charges, Both Barrels, punch /
  thorns (none at 0 charges), empty slots, every weapon × ability fires, salvo / Flurry bursts,
  bow draw + arming + Triple Nock's minimum, Lancet bloom, ability homes and power, lossless
  loadout packing.
* `shared/src/tuning/balance.rs` (unit): override paths, PvE = reference / PvP no Thorns /
  idempotent, the multiplier table covers everything, profiles from RON.
* `shared/tests/combat_loadouts.rs`: when requests apply (fresh / dead / warm-up / respawn),
  sanitising, forced loadouts, bodiless players and leaving, `restart_level`, the replicated
  weapon / ability / draw, every weapon and ability firing in a live world, `KillInfo::weapon`
  for a shared ability, exact prediction with a custom loadout.
* `server/tests/arsenal_net.rs`: PvP clients get 0 thorn charges, co-op clients 3, a reload
  recomputes; a loadout sent in the lobby is what the match spawns (sanitised), a change in
  play waits for the respawn.
* `server/src/commands.rs` (unit): `/loadout` argument parsing.
