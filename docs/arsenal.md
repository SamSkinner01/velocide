# Arsenal: loadouts, builds, weapons, abilities, balance per mode

The third-generation arsenal (`docs/expansion.md` is the contract it implements; the
second-generation weapons and their as-built notes are in `docs/roster.md`), with the refit's
melee class and knacks (`docs/refit.md`) and the rework (`docs/rework.md`: new abilities, the
Lamplighter, the drunk Kindle Flask). Thirteen weapons in six classes, twenty-three abilities,
six knacks on Q, loadouts of up to four builds, the Burn and Pinned status effects, and separate
balance for PvE and PvP. Everything a weapon decides is integer-tick and predicted by its owner;
the server stays authoritative for everything else.

> **Rework in progress** (`docs/rework.md`, `PROTOCOL_VERSION` 11). Eleven abilities retired
> (Splinter, Skewer, Flurry, Both Barrels, Slug, Volley, Flare and its burning patches, Mortar,
> Triple Nock, Ember Arrow, Upswing). The sections below that still describe them, and the PvP
> balance tables, are rewritten by the areas as they land; the ids, kinds and flags every
> area builds against are in **"The rework on the wire"** below.

## Code map

| where | what |
|---|---|
| `shared::loadout` | `WeaponKind` (13), `WeaponClass` (6), `Ability` (23), `Knack` (6), `WeaponBuild` (reading a retired ability's name from RON gives the weapon's default), `Loadout`: ids, names, pools, defaults, `sanitized()`, `Knack::pvp_allowed` (plain data) |
| `shared::sim::weapons` | `WeaponState`, `step_weapons` (predicted), `FireIntent` (with its `AbilityPhase`), firing styles, `SimWorld::resolve_intent` (every primary, the kept abilities; the rework's go to `abilities/`), `ability_tuning` |
| `shared::sim::abilities` | the rework's abilities, one file each (`wickline.rs` … `riposte.rs`), dispatched by `SimWorld::cast_ability` with an `AbilityCast`; their state between ticks (`SimWorld::abilities`), per-tick steps, replication; the shared **guard** (`guard.rs`) |
| `shared::sim::loadouts` | `SimWorld::request_loadout` / `force_loadout` / `loadout` / `spawn_loadout`: when loadouts apply; `pvp_world` / `fit_loadout`: the knack rule |
| `shared::sim::melee` | the Trimmer and Tamper swings (`in_arc`), Lunge / Whirl / Hurl / Upswing, lunges in progress (`SimWorld::melee`) |
| `shared::sim::knacks` | `Knack::charges` / `recharge_time`, `SimWorld::use_knack`, Kindle pools, snares, soot clouds (`soot_blocks`), Tallow Skin (`SimWorld::knacks`) |
| `shared::sim::{hitscan, area, projectiles}` | rays (Skewer's pierce), cones / novas / blasts (the Kiln spray), projectiles and explosions |
| `shared::sim::payload` | what the new projectiles do on contact: the Flak ring, Limpets, the Flare's patch, the special arrows, the flask / pot breaking, the snare settling |
| `shared::sim::parry` | punch parries; the Trimmer's deflect (`deflect_in_arc`) shares the projectile part |
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
| Precision | Quill, Lancet, Lamplighter | Brand, Wickline, Tally, Waxwork |
| Scatter | Bellows, Thresher | Gust, Bolas, Bulwark, Flak |
| Rapid | Arclash, Riveter, Kiln | Wisp, Discharge, Stoke, Lantern |
| Explosive | Tumbler, Cinderbox | Breach, Gravewell, Limpet, Kite |
| Draw | Longwick | Brand, Blink Arrow, Pinning Arrow, Beacon Arrow |
| Melee | Trimmer, Tamper | Lunge, Whirl, Hurl, Riposte |

Defaults (`WeaponKind::default_build`, also the home weapons for `ability_power`): Quill
Brand / Wickline, Lancet Tally / Waxwork, Lamplighter Wickline / Waxwork, Bellows Gust /
Bulwark, Thresher Bolas / Flak, Arclash Wisp / Discharge, Riveter Stoke / Lantern, Kiln
Lantern / Discharge, Tumbler Breach / Gravewell, Cinderbox Limpet / Kite, Longwick Blink
Arrow / Pinning Arrow, Trimmer Lunge / Hurl, Tamper Whirl / Riposte.

The loadout also holds the **knack** on Q (`Loadout::knack`, the Thorn by default; below).

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
  mode, fist / parry state, the knack's spent charges (if it is the same knack; a new one
  starts full) and edge detection; every weapon's cooldowns, charges and bursts start afresh
  (`WeaponState::with_loadout`; at a spawn: `WeaponState::respawned`).
* A forced loadout applies at once (first slot selected) and wins over requests, also at
  respawns, until `force_loadout(id, None)`.
* **Unarmed** (`Loadout::unarmed()`, no weapon at all; gun game's old Fist tier — no mode hands
  it out since the melee finale) exists only as a forced loadout: `force_loadout` keeps it as it
  is, while requests (clients, `/loadout`) are always sanitised to at least one weapon. Unarmed,
  `step_weapons` fires nothing but the fist (and the knack): there is no slot to select, no
  primary, no ability, no charge.
  `WeaponState::is_unarmed()` / `held() == None` say so (`build()` / `current()` fall back to
  the default build — check first); the player entity carries the `UNARMED_WEAPON_ID`
  sentinel (below).
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
| `charge` | the Discharge's charge, the Limpet's hold, a held ability (Tally, Bulwark, Riposte) while it is up: ticks held |
| `draw` | Longwick: ticks drawn; Lamplighter: ticks focused (0 = not drawing) |
| `armed` | Longwick: ability armed for the next release (0 none, 1 A, 2 B) |
| `bloom` | Lancet heat (ticks) |
| `burst` | a burst in progress: shots left, ticks to the next, which ability (`None` = the primary salvo) |
| `punch_cooldown`, `parry` | fist and parry window (unchanged) |
| `knack_used`, `knack_recharge` | knack charges spent and ticks towards the next (were `thorns_used` / `thorn_recharge`) |
| `live[4]`, `live_mode[4]` | per slot: ticks a live ability has left and which mode (rework: Wisp, Waxwork, Blink Arrow); its cooldown starts when it ends |
| `stoke[4]`, `overheat[4]` | per slot: Stoke ticks left, then the overheat (no primary) |

Selection: `PlayerInput::weapon` picks a slot; an empty slot or an out-of-range value keeps
the current weapon. Switching weapon cancels the charge, the draw, the arming and a burst;
switching ability cancels the charge and the arming.

| primary style | weapons | rule |
|---|---|---|
| automatic | Quill, Bellows, Arclash, Tumbler, Lancet, Thresher, Riveter, Kiln, Trimmer, Tamper | while `FIRE` is held, one shot (swing) per interval (stoked: `stoke.rate` × faster; overheated: none) |
| salvo | Cinderbox | a pull fires `cinderbox.rockets` rockets `salvo_gap` apart (a `burst`) |
| draw | Longwick | hold to draw (only once off its interval), release looses (`charge` = draw 0..1) |
| focus | Lamplighter | the draw as a hitscan beam: hold to focus (`lamplighter.focus_time` to full, only once off its interval), release fires (`charge` = focus 0..1) |

| ability style | abilities | rule |
|---|---|---|
| instant | most | while `ALT_FIRE` is held, once per cooldown. The Whirl is the primary's swing fired another way: it also waits for the primary to be ready, takes its swing (the primary doesn't swing that tick) and puts it on its interval |
| charge | Discharge | hold to charge once off cooldown, fires on release (`charge` 0..1) |
| hold | Limpet | a tap (released before `limpet.detonate_hold`) fires with `charge` 0 = throw, off cooldown; holding to it fires once with `charge` 1 = detonate (any time) |
| channel | Tally, Bulwark, Riposte | the press (off cooldown) is its `Use` (`charge` counts from 1), holding keeps it up for at most its `charge_time` (`tally.hold_time`, `bulwark.hold_time`, `riposte.hold_time`), letting go — or the limit, or switching weapon / ability — is its `Release` (`charge` = how long it was held); the cooldown starts at the release |
| press | Kite, Wisp | fires on the press only, once per cooldown (holding the Kite steers it) |
| live | Wisp, Waxwork, Blink Arrow | after its use the ability lives `Ability::live_time` (`WeaponState::live`): its build's abilities wait, a second press is its `Recast` if it `recasts()` (the Wisp flung, the Blink Arrow's step: cooldown `recast_cooldown`), and when it runs out its cooldown starts. The server may end or shorten it (`WeaponState::end_live` / `shorten_live`) |
| armed (any ability on a Draw weapon) | Brand, Blink Arrow, Pinning Arrow, Beacon Arrow on the Longwick | pressing `ALT_FIRE` (off cooldown) arms it, pressing again disarms; the next release fires it with the draw, and starts its cooldown (the Blink Arrow: its live time) |

`FireIntent::{Primary { weapon, charge }, Ability { weapon, ability, charge, phase }, Punch,
Knack(knack)}`: `charge` is the draw (Longwick) / focus (Lamplighter), the bloom (Lancet: the
first shot is exact, sustained fire settles at 75 % of `bloom_degrees`) or the ability charge;
`phase` (`AbilityPhase`) is `Use` (every ability's one intent, a held one's press, the bow's
armed release), `Release` (a held ability let go) or `Recast` (a live one pressed again).
Server-decided cooldown changes (a Wickline miss, a Tally with nothing tallied, a shattered
Bulwark, a caught Hurl, a Waxwork destroyed early, a Blink Arrow that stuck) are written into the
owner's `WeaponState` by the server; the owner's client re-bases on them. THROW (KNACK) fires
the loadout's knack while it has a charge; one recharges every `recharge_time` (the Thorn's
numbers are `combat.thorn`, the others' `combat.knacks.*`).

## Weapons

| id | weapon | class | primary (PvE numbers) |
|---|---|---|---|
| 0 | Quill | Precision | unchanged (`docs/roster.md`): 24 dmg hitscan, 0.28 s, ricochets off Thorns |
| 1 | Bellows | Scatter | unchanged: 12 × 8, 8°, 0.8 s |
| 2 | Arclash | Rapid | as `docs/roster.md`: 7 dmg zaps every 0.1 s (was 6, `docs/balance.md`), chaining |
| 3 | Tumbler | Explosive | as `docs/roster.md`: bouncing bombs, r 4.5 m, 72 dmg (was 65), 0.85 s |
| 4 | Lancet | Precision | hitscan needle, 14 dmg, 0.15 s; each shot heats it (+0.25 s, cap 0.6 s, drains in real time) and the spread grows to 3° at full heat — the first shot is exact, sustained fire settles at 2.25° |
| 5 | Thresher | Scatter | 7 × 7.5 dmg, 11°, 45 m, 0.42 s, automatic while held |
| 6 | Riveter | Rapid | rivet projectile 90 m/s, gravity 6, 7 dmg, 1.2° spread, 0.09 s (5 ticks: 12/s at 60 Hz; 11/s is not a whole number of ticks) |
| 7 | Cinderbox | Explosive | a pull fires 3 rockets 0.08 s apart (5 ticks), 42 m/s, 2.5° spread, r 2.6 m, 32 dmg, kb 14; 1.3 s per salvo |
| 8 | Longwick | Draw | hold to draw (0.62 s to full), release: arrow 55–135 m/s, 22–105 dmg by draw, gravity 10, 0.25 s before the next draw; a tap looses a weak, slow arrow (handling: `docs/feel.md`) |
| 9 | Kiln | Rapid | every 0.08 s (5 ticks) a 13 m / 20° cone: 4 dmg + 1 Burn stack to everything inside (line of sight, no weak points, no ricochet) |
| 10 | Trimmer | Melee | every 0.32 s a 3.0 m / 70° arc: 36 dmg to up to 3 targets, deflects parryable shots ("Melee" below) |
| 11 | Tamper | Melee | every 0.85 s a 3.4 m / 100° arc: 85 dmg to everything in it, kb 16 |

Only the Quill's shots ricochet off Thorns, whichever ability fires them (its identity): Brand,
Splinter and Flurry ricochet on the Quill and not on the Lancet; Skewer pierces instead.

## Abilities

Existing abilities keep their rules (`docs/roster.md`); on another weapon of their class they
behave the same (their damage scaled, below). New ones, as built:

| ability | as built |
|---|---|
| Skewer | hold to charge ≤ 0.8 s, release: a hitscan shot, 30–90 dmg by charge, through up to 3 targets (nearest first, before the level stops it), ×0.8 for each target passed (`PIERCED` after the first). No ricochet. 1.4 s |
| Flurry | 3 shots 0.06 s apart (4 ticks), each 1.0 × the weapon's primary damage, 1° spread; 1.4 s |
| Slug | one exact hitscan slug, 55 dmg, kb 6; 1.2 s |
| Flak | a shell (45 m/s, straight) that bursts after 14 m of flight, or 0.5 m short of a target it touches, or off a wall (the ring then sprays along the reflected flight): 10 pellet rays evenly around the flight axis, each 25° off it, 14 m, 9 dmg each. On contact the ring closes on the target (a point-blank shotgun); in the open it fans out behind cover and around corners. Not parryable. `SimEvent::Burst` / `Fx::Burst` carry the rays. 1.8 s |
| Volley | 10 of the weapon's own shots at once in a 6° cone, each 0.85 × its damage: rivets (Riveter), chaining zaps (Arclash), ember rays that each set a Burn stack (Kiln); it takes the primary's shot (waits for it to be ready, then the primary waits its interval). 2.2 s |
| Flare | a flare lobbed 10° above the aim (22 m/s, gravity 16); where it lands — on the floor, at the feet of what it hit, under where it hit a wall, or below where it burns out after 4 s — a 3 m burning patch for 4 s (no patch over a pit). 3.0 s |
| Limpet | tap: throws a charge (24 m/s, gravity 16) that sticks to the level or to the enemy / player it hits and rides along; hold 0.25 s: every charge out detonates (r 3.5 m, 40 dmg, kb 14, a direct hit on what it sticks to). Up to 3 out: a throw with 3 out detonates them instead. A charge left alone goes off after 20 s. 0.9 s between throws |
| Mortar | a shell lobbed 35° above the aim at 26 m/s, gravity 20: r 6 m, 85 dmg, kb 26; 3.0 s |
| Triple Nock | 3 arrows in a 7° fan at the current draw (≥ 30 %), each 0.6 × a plain arrow's damage; 1.4 s |
| Pinning Arrow | an arrow at the draw that pins what it hits (Pinned, below); 2.5 s |
| Ember Arrow | an arrow at the draw whose own hit is 0.9 × a plain arrow's, then it bursts — on a target, on the level or at the end of its flight: r 3 m, 36 dmg, kb 8, and 2 Burn stacks on everything in the burst; 3.0 s |
| Both Barrels | as `docs/roster.md` (24 × 8 dmg, 5°, recoil 9), but 1.8 s (was 1.4) and, like Volley, it takes the primary's shot: it waits for the blast to be ready and fires instead of it (never a blast and Both Barrels on one tick) |
| Wisp | as `docs/roster.md`, but zaps 11 dmg (was 8), pop 30 (was 25), 2.4 s (was 2.8) |
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

## Melee (refit)

`docs/refit.md` is the contract. `WeaponClass::Melee`: the **Trimmer** (id 10, hooked
wick-shears) and the **Tamper** (id 11, a foundry ramming hammer); `PrimaryStyle::Auto` (hold to
keep swinging). Code: `shared::sim::melee`.

| weapon | primary (PvE) |
|---|---|
| Trimmer | every 0.32 s a 3.0 m / 70° arc in front of the eye: 36 dmg to up to 3 targets (nearest first), kb 4; the swing deflects the parryable hostile projectiles in its arc like a punch parry (same reward) |
| Tamper | every 0.85 s a 3.4 m / 100° arc: 85 dmg to everything in it, kb 16; no deflect |

A swing tests the arc against the hit shapes the swinger saw (lag compensation, like
hitscan), with hit assist and line of sight from the eye. `melee::in_arc`: along the aim within
reach; or a (padded) shape whose surface is within reach of the eye and that the arc's cone
touches — the angle from the aim to the point of the shape's axis nearest the eye, less the
angle the shape's radius covers from there. Measuring to a target's centre (as the punch does)
missed a low Mould right at your feet while you looked ahead; this doesn't. Melee **ignores weak
points**: the arc already forgives where a ray wouldn't, and a hammer that headshots for ×2
would one-shot (the Hurl's thrown blade / hammer-head strikes the body too). All melee damage
carries `DamageFlags::MELEE`; its embers heal at
`embers.melee_weapon_heal_fraction` (0.5: between a gun's 0.4 and the fist's 1.0 — the melee
weapons hit often and several targets at once, the fist's rate made them near-immortal).

| id | ability | as built (damage = `damage_scale` × the carrier's primary damage) |
|---|---|---|
| 19 | Lunge | an impulse along the aim (16 m/s, 35 % of its vertical part kept) and at least 4.5 m/s up (`lunge.lift`: off the ground it glides some 5 m; ground friction stopped a standing lunge within 1.5 m); for 0.35 s the first hostile within 0.8 m of the body (current shapes — the body is really there — hit assist, line of sight) takes 1.5×, kb 10 (`Fx::LungeStruck`). 3.0 s |
| 20 | Whirl | in place of the next swing (it takes the primary's swing, like Both Barrels its shot): everything within 3.5 m of the body (lag-compensated, line of sight) takes 1.5×, knocked outward (kb 12, up bias 0.35). 4.0 s |
| 21 | Hurl | a spinning blade / hammer-head (30 m/s, gravity 6, r 0.25 m, 2 s), 1.5× on contact (on the body: no weak points, no headshots), kb 6. 2.5 s |
| 22 | Upswing | in place of the next swing: the carrier's arc, 1.4× and an upward launch of 14 m/s (enemies × their knockback scale); the swinger hops (vertical speed at least 5 m/s). 3.0 s |

Defaults: Trimmer `[Lunge, Hurl]`, Tamper `[Whirl, Upswing]` (the home weapons for
`ability_power`, both 1.0: the abilities follow the carrier's swing, `ability_power` scales them
like any ability). The impulses (Lunge, Upswing's hop) are server-side like Both Barrels'
recoil: the owner's client reconciles.

PvP: no headshots, so the rules are the body ones. The Trimmer (×0.84: 30) kills in 4 slashes
(0.95 s), the Tamper (×0.7: 59.5) in 2 swings (0.85 s); its Lunge, Whirl and Hurl hit 89, its
Upswing 83 (`combat_melee.rs` checks every swing and ability stays under 100, the balance
harness measures it in play: `docs/balance.md`).

## Knacks (refit)

The Q slot (`Buttons::THROW`, the KNACK control) holds the loadout's `Knack`
(`Loadout::knack`, `#[serde(default)]`: old presets load with the Thorn). `step_weapons`
spends a charge (`WeaponState::knack_used`, one back every `recharge_time`, predicted) and
fires `FireIntent::Knack(knack)`; `SimWorld::use_knack` (`shared::sim::knacks`) does the rest.

| id | knack | as built | charges / recharge |
|---|---|---|---|
| 0 | Thorn | the relay spike (`thorn.rs`, `combat.thorn`). **PvE only** | 3 / 3 s |
| 1 | Kindle Flask | lobbed 15° above the aim (16 m/s); where it breaks (on a body: at its feet) a 3.5 m pool for 4 s heals every living Shrike whose feet are inside 2.5 HP every 0.25 s (10 HP/s, 40 in all) through `PlayerCombat::heal` — never above the effective max, so hard damage still caps it; in PvP only the thrower | 1 / 14 s |
| 2 | Snare | thrown (12 m/s, passes through bodies) to the floor, lies there `STUCK` (off a wall it drops on); 0.6 s after landing it is armed (`ARMED`); the nearest hostile whose feet come within 1.8 m (and at most 2 m above) springs it: 15 dmg (`Attack::Snare`) and Pinned (the Pinning Arrow's times: a Mould rooted 1.4 s, heavies 0.6 s, a player slowed to half for 1 s). Teammates and the owner walk over it. Up to 2 out (a throw with 2 out retires the oldest), 20 s each | 2 / 10 s |
| 3 | Soot Pot | thrown (18 m/s); bursts on contact (or after 2 s) into a 5 m cloud for 7 s: `soot_blocks` — a Mould whose eye or target is inside it, or whose line of sight crosses it, can't see that player: never picked as a target (aware arena spawns included), a target hidden this way is lost at once, a Heron's laser wind-up breaks off. Players' shots pass through | 1 / 15 s |
| 4 | Tallow Skin | for 3 s: damage × 0.5 and no knockback (your own blasts still push: blast jumps work); `TALLOW` player flag. PvP: × 0.7 for 2 s (`balance.pvp` overrides: half damage for 3 s won every even duel) | 1 / 16 s |
| 5 | Surge | an impulse of 18 m/s along the look (30 % of its vertical part kept) | 2 / 6 s |

**PvP**: a knack that isn't `Knack::pvp_allowed` (the Thorn) is replaced by
`Knack::PVP_FALLBACK` (Surge) whenever a loadout applies in a PvP world (a PvP match or the
warm-up before one: `SimWorld::pvp_world`, `fit_loadout`); the stored request keeps the pick.
The old `"thorn.charges": 0` PvP override is gone. Gun game keeps the requested knack in its
forced loadouts. `/loadout ... knack:<name>` sets it (alone: keeps the weapons).


## Status effects

**Burn** (`status.rs`; `burn` tuning): `dps` 5 per stack, up to `max_stacks` 3, for
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

**Player entity `state`**: held `WeaponKind` id (bits 0–3; `UNARMED_WEAPON_ID` = 15: fists
only) | selected `Ability` id (bits 4–8) | charge / draw level (bits 9–15, 7 bits). Always
through the helpers:

```rust
pub fn pack_player_weapon(weapon: WeaponKind, ability: Ability, charge: u8) -> u16;
pub fn pack_player_unarmed() -> u16;
/// Holds a weapon (false: fists only — `player_weapon` then decodes the defaults).
pub fn player_armed(state: u16) -> bool;
/// (held weapon, selected ability, charge / draw 0..=255); unknown ids → Quill / Brand.
pub fn player_weapon(state: u16) -> (WeaponKind, Ability, u8);
```

`WeaponKind` ids stay below 15 (the sentinel).

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
| 77 | `HURL` | the Hurl's blade / hammer-head, gravity `hurl.gravity`; `state` = the carrier's `WeaponKind` id (10 Trimmer: a blade, 11 Tamper: a hammer-head); direct damage |
| 78 | `KINDLE_FLASK` | gravity `knacks.kindle_flask.gravity`; breaks into a `KINDLE_POOL` (and a `ProjectileImpact`) |
| 79 | `SNARE` | gravity `knacks.snare.gravity`, passes through bodies; `STUCK` once it lies on the floor (`position` current), `ARMED` (projectile flag bit 4) once armed; sprung or worn out: removed |
| 96 | `SOOT_POT` | gravity `knacks.soot_pot.gravity`; bursts into a `SOOT_CLOUD` (and a `ProjectileImpact`) |
| 97–103 | reserved for the arsenal | (82–95 are the enemies area's) |
| 136 | `BURNING_PATCH` | `position` = centre on the floor, `owner` = owner code, `state` = radius (dm), `aux` = burn-out tick |
| 137 | `KINDLE_POOL` | `position` = centre on the floor, `owner` = thrower, `state` = radius (dm), `aux` = tick it dries up |
| 138 | `SOOT_CLOUD` | `position` = centre, `owner` = thrower, `state` = radius (dm), `aux` = tick it clears; draw it dense enough to hide players |
| 139 | reserved | |

`Fx::Burst { shooter, attack, origin, ends }` (everyone, repeated like other important
effects): a projectile burst into rays — the Flak ring. Owner-only: `OwnState::combat.weapons`
carries the whole `WeaponState`, loadout included (47 B of own combat state per snapshot with
the default loadout), and `OwnState::movement` the slow (8 B).

### Melee and knacks on the wire (refit, `PROTOCOL_VERSION` 9)

* **Loadout**: the packed `u64` keeps its four 16-bit slots; the **knack id** rides in bit 15
  of each slot (4 bits, slot 0 holds the lowest; empty slots carry theirs too), so a loadout is
  still one integer in `OwnState` (no growth). `WeaponState::{knack_used, knack_recharge}`
  replace `thorns_used` / `thorn_recharge` (same sizes).
* **Player entity**: `aux` bits 8–11 = the knack's charges (were the thorns'); flag bit 16
  `TALLOW` = Tallow Skin is on (a waxy sheen). `state` packs the melee weapons (ids 10, 11)
  and abilities (19–22) like any other.
* **Swings** are events, not entity bits (a swing is an instant, an entity bit would need a
  counter to show two swings in a row): `SimEvent::Swing { player, attack, hits }` →
  `Fx::Swing` for every primary swing, Whirl, Upswing and the start of a Lunge — not sent to
  the swinger (it predicts its own from `step_weapons`). The Trimmer alternates sides: the
  client alternates per swing it shows. `SimEvent::LungeStruck` → `Fx::LungeStruck { player,
  position }` to everyone (the swinger can't predict where a Lunge connects). The Hurl is a
  projectile (`kind::HURL`) plus the usual `Fx::Shot` muzzle event. A Trimmer deflect is a
  `GameEvent::Parried` like a punch parry.
* **Knacks**: `SimEvent::KnackUsed { player, knack }` → `Fx::Knack` for every knack but the
  Thorn (that one keeps `Fx::ThornThrown`), not sent to the user. `SimEvent::SnareSprung` →
  `Fx::SnareSprung { position }` to everyone. Kindle pool heals are `SimEvent::Healed` with
  `HealSource::KindleFlask(pool)` → `Fx::Heal` (from the pool to the lantern).
* The replay records the local player's own swings and knacks as the same `Fx` the others
  receive (`client/src/ingame/replay.rs`).

## The rework on the wire (`PROTOCOL_VERSION` 11)

The groundwork every rework area builds against (`docs/rework.md`; arsenal-a M1). Kept
abilities kept their ids, the new ones took the retired ids.

**`Ability` ids** (`state` bits 4–8 of a player, the packed loadout, the lobby):

| id | ability | id | ability | id | ability |
|---|---|---|---|---|---|
| 0 | Brand | 8 | Tally | 16 | Blink Arrow |
| 1 | Wickline | 9 | Waxwork | 17 | Pinning Arrow |
| 2 | Gust | 10 | Bulwark | 18 | Beacon Arrow |
| 3 | Bolas | 11 | Flak | 19 | Lunge |
| 4 | Wisp | 12 | Stoke | 20 | Whirl |
| 5 | Discharge | 13 | Lantern | 21 | Hurl |
| 6 | Breach | 14 | Limpet | 22 | Riposte |
| 7 | Gravewell | 15 | Kite | | |

**`WeaponKind`**: `Lamplighter` = 12 (Precision, `lamplighter.ability_power` 1.0,
`PrimaryStyle::Focus`). `Attack` gains `Lamplighter`, `Wickline`, `Tally`, `Waxwork`, `Bolas`,
`Bulwark`, `Stoke` (the Burn a stoked hit sets), `Lantern`, `Kite`, `BlinkArrow`, `BeaconArrow`,
`Riposte`; `StyleKey` the same (41 keys). `DamageFlags::REVEALED` (bit 18): a Beacon's ×1.15.
`Blocked::Guarded`: a guard caught it.

**Entity kinds** (freed: 69 `MORTAR_SHELL`, 70 `FLARE`, 75 `EMBER_ARROW`, 136 `BURNING_PATCH`):

| kind | name | what |
|---|---|---|
| 69 | `BOLAS` | projectile, gravity `bolas.gravity`; gone once it binds |
| 70 | `KITE` | projectile, no gravity; `GUIDED` while steered (re-launched every tick it turns) |
| 74 | `WISP` | as before; `ORBITING` while it orbits its owner (`position` current, `aux` = orbit start tick) |
| 75 | `BLINK_ARROW` | arrow gravity; `STUCK` once it sticks in the level |
| 77 | `HURL` | as before; `GUIDED` + `RETURNING` on the way back |
| 97 | `BEACON_ARROW` | arrow gravity; `STUCK` where it lit (rides along in a target) |
| 98 | `LANTERN_TOSS` | a Lantern being tossed, gravity `lantern.gravity`; `state` = the carrier's weapon id |
| 136 | `WAXWORK` | the decoy: `position` = feet, `yaw` / `pitch` = the look it was cast with, `owner` = the player it copies (draw their Shrike, cosmetics, name), `state` = their weapon / ability as `pack_player_weapon`, `flags` = the pose's player flags + `waxwork_flags::SLUMPING` (bit 24), `health`, `aux` = the tick it slumps |
| 139 | `LANTERN` | a Lantern on the floor: `owner`, `state` = the carrier's weapon id, `health`, `aux` = the tick it goes out, `yaw` = where it last aimed |
| 99–103, 144–151 | reserved for the arsenal | (`kind::is_arsenal_thing`: 136–139, 144–151) |

**Flags**:

| flags | bit | meaning |
|---|---|---|
| player `HASTE` | 17 | the drunk Kindle Flask's haste (arsenal-b sets it) |
| player `REVEALED` | 18 | lit by a Beacon Arrow |
| player `GUARDING` | 19 | a guard is up (the held ability says Bulwark or Riposte); `aux` bits 12–15 = strength left, sixteenths (15 = whole) |
| player `STOKED` / `OVERHEATED` | 20 / 21 | the held weapon is stoked / overheated (from `WeaponState`) |
| player `BOUND` | 22 | bound by a Bolas cord |
| player `TALLYING` | 23 | holding a Tally (`state`'s level = how long) |
| player `DRAWING` | 14 | also the Lamplighter's focus (level in `state`, like the bow's draw) |
| enemy `REVEALED` / `BOUND` / `LURED` | 16 / 17 / 18 | lit by a beacon / bound / targeting a Waxwork |
| projectile `GUIDED` / `RETURNING` / `ORBITING` | 5 / 6 / 7 | see the kinds; `replicated_position`: `GUIDED` flies without gravity, `ORBITING` is at `position` |

**Own state**: `WeaponState` gains `live[4]`, `live_mode[4]`, `stoke[4]`, `overheat[4]`;
`PlayerCombat::tallied: [Option<NetId>; 4]` (the Tally's marks, players as `NetId::player`);
`MoveState::guard_time` / `guard_scale` (a raised guard's slow, server-set).

**Effects** (`Fx`, everyone unless said): `Wickline { player, from, to, anchored }`,
`TallyFired { shooter, origin, ends }`, `Blink { player, from, to }`, `GuardBlock { player,
ability, position, broke }`, `BolasBound { positions }`, `LanternShot { lantern, origin, ends }`,
`HurlCaught { player }` (from the `SimEvent`s of the same names). `Fx::Splinters` is gone.

**Hooks for the abilities' files** (so each area edits only its own): `SimWorld::cast_ability`
(every intent), `step_abilities` (per tick), `abilities_after_input` (every input, with the
lag-compensation history: the Tally), `abilities_after_damage` (every damage instance that went
through: the Bolas cord, the Stoke's Burn), `payload.rs` (`bolas_hit_*`, `lantern_hit_world`,
`blink_arrow_hit_*`, `beacon_arrow_hit_*`, `payload_tick` → `wisp_tick` / `kite_tick` /
`hurl_tick`), `guard_meets_projectile` (`projectiles.rs`, before a hit on a guarded player:
`riposte_meets_projectile` / `bulwark_meets_projectile`), `damage_waxwork` (`apply_damage`,
before the player pipeline), `is_revealed` / `is_bound` / `is_lured` (replication),
`replicate_waxworks` / `replicate_lanterns`.

**The guard** (`abilities/guard.rs`): `Guard { ability, weapon, slot, facing, half_angle,
absorb_left, absorb, until, blocks, absorbed, pitch_limit }`, one per player
(`SimWorld::raise_guard` / `drop_guard` / `guard`). `damage_player` asks `guard_covers`: a
hostile hit flagged `HITSCAN` / `PROJECTILE` / `EXPLOSIVE` / `MELEE` / `SLAM` (never `BURN`, the
world, floor hazards or your own blasts) travelling from inside the cone (back along
`DamageInfo::dir`, else from its source) is absorbed whole: `Blocked::Guarded`, no damage, no
knockback; `guard_absorbed` books it (`absorb_left`, `blocks`), calls `bulwark_blocked` /
`riposte_blocked` and emits `SimEvent::GuardBlock`. The guard faces the owner's current aim
(the Bulwark's pitch clamped), slows them (`MoveState::guard_*`), drops when its ability is let
go (`AbilityPhase::Release`), at `until`, or when its owner dies or stops holding it.

**The Waxwork's decoy id**: a double stands in for its owner as `HitTarget::Player(owner | 0x80)`
in Mould targeting and hit tests (`abilities/waxwork.rs`); player ids stay below 128.

## Client: melee and knacks (refit, area arsenal-client)

What you see and hear of the melee weapons and the knacks. Models in
`client/src/models/weapons/{trimmer,tamper,melee}.rs` and `client/src/models/knacks.rs`
(`docs/models.md`), presentation in `client/src/ingame/combat/{melee,knacks}.rs`, sounds in
the `arsenal-client` melee / knack block of `engine/src/audio/sfx.rs` (`docs/audio.md`).

**The weapons.** The **Trimmer** is a pair of hooked wick-shears whose pivot pin knocks out:
two straight blued blades bent at the end into an angled jaw (the hook that catches a wick),
brass finger loops closed round the knuckles, the right half carrying the snuffer box with a
wick stub smouldering in it. One blade per hand; the bind pose (the loadout preview) is the
closed pair. The **Tamper** is a foundry ramming hammer: an ash haft and a black-iron head with
the rammer's broad round foot on its face and a wedge peen behind, still hot (slag glows in its
flank vents). Edges / the head's band glow in the selected ability's colour like every gun
(Lunge cyan, Whirl amber, Hurl violet, Upswing lime); the primary colour (trails, sparks) is
the Trimmer's honed steel and the Tamper's hot iron.

**Swings** (`models::weapons::melee`: `Swing { kind, progress, left }`, `SwingKind::{Slash,
Lunge, Whirl, Hurl, Upswing}`). The blow lands on the tick the trigger is pulled, so every
move strikes early and spends the rest recovering: the Trimmer's slash reaches its blow in
0.09 s of a 0.3 s animation, alternating hands (right to left, then the left blade left to
right); the Tamper is raised over the right shoulder and slammed down the middle in 0.1 s of
0.78 s, bounces and is hauled back up (the left hand slides down the haft for the slam). In
first person each move is a keyframed track of the right hand (the left blade mirrors it) with
the forearms re-aimed at the shoulders so the arm never sweeps across the view; the left fist
still punches (the Trimmer's left blade rides in the punching fist) and throws. In third
person both arms follow the move (`melee::arms`), the Trimmer's left half in the left hand.

| move | first person / third person | trail and effect | sound |
|---|---|---|---|
| Trimmer slash | the blade whips across, edge leading; right then left | a crescent swept across the front at the blade's pace | `TrimmerSlash` (a snip, a whistle, a ring) |
| Tamper slam | raised behind the head, slammed down the middle | a crescent from over the right shoulder to the floor (its plane leans so it reads, not edge on) | `TamperSwing` |
| Lunge | point first, held through the dash | your own: speed lines at the screen's edges; others': a streak at knee / chest / head height; `Fx::LungeStruck`: a cyan burst, the hit sound | `Lunge` |
| Whirl | the blades / head swept flat round | a full ring round the body and rings rushing out to 3.5 m, sparks and dust | `Whirl` |
| Hurl | wound back past the ear, flung; the hand comes back up empty | the spinning blade / hammer-head (the `HURL` projectile: `state` = the carrier, drawn 1.25×, a violet aura, light and streak), an impact where it vanishes | `HurlThrow`, `HurlHit` |
| Upswing | from low in front up past the face | a crescent rising; a burst of dust and sparks under what it launches | `Upswing` |

**Deflects**: a Trimmer slash checks, on the frame it is predicted, for a parryable hostile
projectile drawn inside its arc and reach in front of the eye and turns it back at once with
the parry's flash, hitstop and sound (`InGame::combat_predict_deflect`, the punch's path in
`docs/feel.md`); the server's `Parried` then only confirms it. **Thrown things** follow
feel's projectile scheme: `HURL`, `KINDLE_FLASK`, `SNARE` and `SOOT_POT` have their own looks
in `entities::projectile_look` (halo, core, streak: violet, kindling gold-green, amber,
fuse orange) and local stand-ins from the hand on the press frame (`stand_in.rs`; a stand-in
snare sticks where it lands); the pieces themselves are drawn by `melee.rs` / `knacks.rs`
(a stand-in Hurl shows the weapon in hand).

Hits: a `HitConfirm` of a melee attack (not the Hurl, whose piece makes its own impact) plays
the blow where it struck: the Trimmer slices (sparks, molten wax, a bright nick, `TrimmerHit`,
a tiny hitstop), the Tamper lands (lumps, dust, a shock ring, a light, `TamperHit`, 0.06 s
hitstop and a jolt). Remote players: `Fx::Swing` starts their Shrike's swing (the client
alternates the Trimmer's hands per swing it shows), the sound and the trail drawn whole in front
of their eyes (sparks at its end when it hit); a Hurl's `Fx::Shot` is their throw. Spectating
a teammate mirrors their swings on the viewmodel; the play of the game replays the viewpoint's
own swings and knacks in first person (`melee::first_person_swing`,
`knacks::first_person_knack`).

**Knacks.** The HUD's **Q slot** (`knacks::draw_slot`, bottom right above the weapon block,
replacing the thorn pips): a 12×12 pixel icon per knack in a frame with the key, the name, a pip
per charge with the next one filling as it recharges; out of charges the icon dims and refills
from the bottom; a running Tallow Skin drains a bar over the frame. No slot when the knack has
no charges.

| knack | in the hand | in the world | you | sounds |
|---|---|---|---|---|
| Thorn | the thorn between the knuckles (as before) | as before | | `ThornThrow` … |
| Kindle Flask | the flask cupped on the fist | the `KINDLE_FLASK` tumbling with kindling sparks; breaking: glass, a burst in the heal colour; the `KINDLE_POOL`: embers rising and a glowing haze in the palette's heal colour, a turning dashed rim, a warm light. Its heals (`Fx::Heal` from the pool) are a quiet trickle of kindling from the floor into the lantern, not the full kindling chime every quarter second | a faint warm flash per heal | `KnackThrow`, `FlaskShatter`, `KindlePool` (crackle while it burns) |
| Snare | the disc held edge-on | the `SNARE` flying, then lying on the floor (`STUCK`): the wax trap with its jaws open, its wick lamp dull while it arms and pulsing once `ARMED` (in the palette's hazard colour), a dashed trigger ring for its owner; `Fx::SnareSprung`: a flash, chains whipping up, the jaws snapped shut there for a moment | | `SnareSet`, `SnareArmed`, `SnareSpring` |
| Soot Pot | the pot cupped on the fist | the `SOOT_POT` with its fuse spitting; bursting: shards, a dark puff, a scorch; the `SOOT_CLOUD`: big, dark, near-opaque billows kept topped up over the whole sphere (~60 alive at 5 m, darker at the core), thinning over its last fifth — dense enough to hide players inside and behind it | | `SootBurst` |
| Tallow Skin | | other Shrikes under `TALLOW` are paler and glossier with a slow glisten and drips of glinting wax | a waxy sheen at the screen's edges, tallow drips running down from the top, a glint travelling round | `TallowSkin` |
| Surge | | others: a streak along their look | speed lines streaming in from the screen's sides | `Surge` |

`InGame::preview_melee(MeleePreview)` / `preview_knack(KnackPreview)` stage any of it
client-side for tools; `combat_shots` shoots the previews (`melee_*`, `knack_*`) and the real
thing after `/loadout trimmer tamper knack:...` (`melee_weapon_*`, `knack_use_*`).
`COMBAT_SHOTS=refit` stops after them.

**Gun game's finale** (`ingame::mode_hud`, `menus::results`): reaching it is the banner
"FINAL TIER: TRIMMER" (the finale's weapon), others reaching it a feed line ("X IS ON THE FINAL
TIER (TRIMMER)"), the tier block shows the weapon and "A KILL TO WIN", the results name the tier
"FINAL TRIMMER". **Controls**: the Q / gamepad North action is **KNACK** (`Action::Knack`;
settings files that call it `ThrowThorn` or `ThrowPrism` still load).

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

**PvE** changes nothing: `combat` (the numbers above) is the PvE reference. The balance pass
(`docs/balance.md`: the measurements, before / after, every change and its reason) moved base
numbers, not a profile. Difficulty and player-count scaling of the Moulds are the modes area's
(`difficulty` section), not a weapon profile.

**PvP** is tuned for players killing players: 100 HP, headshots ×2 (`player_head_multiplier`)
for everything that isn't explosive, a time to kill of **0.8–1.6 s** for a skilled player at
the weapon's range, and **one-shots only from a full-draw bow headshot, a Slug headshot and a
full-charge Skewer headshot** — plus a Bellows blast to the head inside 5 m, the scattergun's
point-blank reward (from 5 m on it no longer kills). No Thorns (the knack rule: the Thorn isn't
`pvp_allowed` and applies as Surge; it used to be a `"thorn.charges": 0` override) and a brand multiplies by 1.25 instead of
1.35 (in a free-for-all a brand invites everyone, the extra was too much on top of a third
party). Times to kill and per-pull damage below are measured in play by the balance harness
(`cargo run -p shared --release --example balance_report -- pvp oneshots`).

| weapon / ability | PvP | after | time to kill (all on the body; heads faster) and why |
|---|---|---|---|
| Quill | 1.0 | 24 / 0.28 s | 5 hits: 1.13 s (3 heads: 0.57 s). The reference precision weapon |
| Lancet | damage 0.85 | 11.9 / 0.15 s | 9 hits: 1.2 s; raw it out-damaged the Quill at range despite the bloom |
| Bellows | damage 0.6 | 12 × 4.8 = 58 a blast | 2 blasts: 0.8 s; at 0.7 (67) a blast at the head one-shot at 5 m (106), now 91 at 5 m and 115 only inside it |
| Thresher | damage 0.75 | 7 × 5.6 = 39 a blast | 3 blasts: 0.83 s (PvE pellets went 7 → 7.5: the same 5.6 here) |
| Arclash | 1.0 | 7 / 0.1 s | 15 zaps: 1.4 s (PvE zaps went 6 → 7 and PvP follows: at 1.6 s the tracking beam was the slowest hitscan) |
| Riveter | 1.0 | 7 / 5 ticks | 15 rivets: 1.2 s at 5 m, 1.32 s at 15 m; projectiles at 90 m/s already cost aim |
| Kiln | damage 0.9 | 3.6 / 5 ticks + 15 dps burn | 1.75 s inside 13 m, and the 3 s burn keeps going (45) — the counterplay is breaking contact (PvE spray went 3 → 4: the same 3.6 here) |
| Tumbler | damage 0.9 | 65 a direct bomb | 2 direct: 0.98 s at 5 m; splash kills take three (PvE bombs went 65 → 72: the same 65 here) |
| Cinderbox | damage 0.75 | 3 × 24 = 72 a salvo | 2 salvos: 1.47 s at 5 m, 1.7 s at 15 m; three 32s (96) were a body one-shot in 0.16 s with any brand |
| Longwick | damage 0.5, interval 0.85 | 11–52.5 by draw, 0.53 s draw | a full-draw headshot (105) is the one-shot; 90 % draw (97) is not; body: 2 full draws, 1.30 s at 5 m, 1.38 s at 15 m with the flight (feel's 0.62 s draw, 55–135 m/s) |
| Both Barrels | damage 0.25 | 24 × 2 = 48 (96 to the head) | at 0.4 (77) a 5° cone at the head was a one-shot (154), and fired on the same tick as a blast it was a body one-shot (144) — it now takes the blast's shot; it stays the blast-jump |
| Wisp | damage 0.5 | 5.5 a zap | it aims itself: 40 dps of free damage was too much (PvE zaps went 8 → 11: about the same 5.6 as before here) |
| Discharge | damage 0.9 | 22.5–72 | no one-shot even with a hit before it |
| Breach | damage 0.85 | 59.5 | the panic button, not a kill button |
| Gravewell | damage 0.75 | 60 | players are pulled at half strength; the implosion still punishes staying |
| Skewer | damage 0.58 | 17–52 (on the Lancet) | full charge to the head (104) is the one-shot, 90 % (97) is not; it still pierces a line of players |
| Flurry | damage 0.6 | Quill: 3 × 14.4 | three Quill heads in 0.13 s (115) were a one-shot (PvE went 0.8 → 1.0 of the primary: the same 14.4 here) |
| Flak | damage 0.8 | 10 × 7.2 | a contact burst landing every pellet stays under 100 |
| Volley | damage 0.75 | Riveter / Arclash: 10 × 4.46 (0.85 of the shot × 0.75) | ten rivets at once (70, more with heads) was an instant kill up close; ten 7-dmg zaps at 0.7 put 98 on a head, now 89 |
| Limpet | damage 0.75 | 30 a charge | three charges on one player (88) is a kill only with a hit before it (PvE went 50 → 40 a charge: the same 30 here; 0.9 s between throws in both) |
| Mortar | damage 0.75, radius 0.85 | 64, r 5.1 m | a huge blast is area denial in PvE but a free kill on a corner in PvP |
| Triple Nock | damage 0.55 | 3 × 17 (a third of an arrow each) | at 0.5 three heads at a 90 % draw (144) one-shot: now 95, and a full draw's three heads (104) is the bow's one-shot; the fan is a forgiving shot, not more damage |
| Ember Arrow | damage 0.5 | 0.45 × the arrow + an 18 burst + 2 Burn stacks | at 1.0 a 90 % draw to the head with its burst and burn (124) one-shot; now 89 (93 at a full draw) |
| Tamper | damage 0.7 | 59.5 a swing | 2 swings: 0.85 s; its abilities are multiples of the swing: Lunge, Whirl, Hurl 89, Upswing 83 (at 1.0 a 1.5 × Lunge or Hurl was a body one-shot; the Hurl to a head was 180 before it struck the body only) |
| Trimmer | damage 0.84 | 30 / 0.32 s | 4 slashes: 0.95 s (no headshots: melee); its Lunge, Whirl, Hurl hit 45, Upswing 42 (PvE slashes went 30 → 36: the same 30 here) |
| everything else | 1.0 | | Gust (6 dmg, the knockback is the point), Brand (10), Splinter, Slug (55: its headshot is the allowed one-shot), Flare (burn), Pinning / Brand arrows (they ride the bow's 0.5) |

`shared/tests/combat_arsenal.rs::pvp_profile_keeps_the_one_shot_rules_and_the_time_to_kill_band`
recomputes these from the effective tuning: no body one-shot from any trigger pull or ability
use, exactly the allowed headshot one-shots, and every weapon's all-body time to kill inside
0.8–1.8 s. `pvp_one_shots_and_times_to_kill_measured_in_play` measures the same in the
simulation (every pull and every ability use on every carrier at 3, 5 and 10 m, body and head,
the bow at a full and a 90 % draw, both buttons on one tick, every primary's time to kill at
its range).

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
* Melee swings, Whirls and Upswings are lag-compensated (what the swinger saw); a Lunge's strike,
  snares and pools test the current positions, like projectiles.
* A Trimmer deflect happens at the swing only (no window like the punch's): held, it swings
  every 0.32 s, and a window would have made it a permanent shield.
* Soot hides players from Moulds only; their projectiles already flying keep flying.

## Tests

* `shared/src/loadout.rs` (unit): ids never move (melee 10, 11, 19–22; knacks 0–5), every class
  offers four abilities, the knack defaults to the Thorn and falls back to Surge in PvP, old
  presets without a knack load.
* `shared/src/sim/weapons.rs` (unit): intervals, switching, charges, Both Barrels, punch /
  knack charges (none at 0), every knack's own charges and recharge, a new knack starting
  full, empty slots, every weapon × ability fires, salvo / Flurry bursts, bow draw + arming +
  Triple Nock's minimum, Lancet bloom, Limpet tap / hold, ability homes and power, lossless
  loadout packing (every knack with every slot pattern).
* `shared/src/sim/knacks.rs` (unit): segments meeting soot spheres; every knack has charges.
* `shared/src/tuning/balance.rs` (unit): override paths, PvE = reference / PvP neutral once
  applied, the multiplier table covers everything (melee included), profiles from RON.
* `shared/tests/combat_loadouts.rs`: when requests apply (fresh / dead / warm-up / respawn),
  sanitising, forced loadouts, unarmed (forced only, kept through respawns, replicated, fists
  only, exact prediction, no default-build kill credit), bodiless players and leaving,
  `restart_level`, the replicated weapon / ability / draw, every weapon and ability firing in a
  live world, `KillInfo::weapon` for a shared ability, exact prediction with a custom loadout.
* `shared/tests/modes.rs`: gun game's melee finale (`a_kill_with_the_finale_weapon_wins_the_gun_game`,
  `the_finale_tier_is_the_melee_weapon`) and the knack kept on every tier
  (`gun_game_keeps_the_players_knack`).
* `shared/tests/combat_melee.rs`: the Trimmer's three nearest and its interval, no weak points
  (Moulds and PvP heads), reach / arc (a low target at your feet) / walls, lag compensation (hit
  as seen, a miss without), the Tamper's everything-in-the-arc and knockback, the Trimmer's
  deflect with a parry's reward (the Tamper doesn't), Lunge (impulse and lift, the first target
  only, running out, 1.5 × the carrier), Whirl (all around, outward, on either weapon), Hurl
  (blade or hammer-head on the wire, 1.5 × the carrier), Upswing (launch and hop), kill credit
  for a shared ability, the melee ember rate, `ability_power`, exact prediction with a melee
  loadout and a knack, the PvP numbers inside the one-shot rules and the time-to-kill band.
* `shared/tests/combat_knacks.rs`: the PvP rule (Surge in hand, the request kept, the warm-up
  following the next mode), Surge's impulse and charges, Tallow Skin (half damage, no
  knockback, own blasts still push, replicated, wears off), Kindle pools (the hard-damage cap,
  the teammate, PvP only the thrower, replicated, dry after 4 s), Snares (held until armed,
  springing at the arm tick: damage, Pinned, gone; armed and replicated, teammates walk over;
  two out; a PvP player slowed), the Soot Pot (a Gnawer loses the player and finds them again
  when it clears; a Heron's lock breaks, no needle), the thrown knacks' kinds and landings,
  exact prediction for every knack.
* `shared/tests/combat_arsenal.rs`: every new primary (Lancet bloom, Thresher, Riveter flight,
  Cinderbox salvo, Longwick draw, Kiln + Burn), Burn stacks / refresh / kill credit / no
  LONGSHOT, Skewer pierce and falloff, Flurry, Slug, the Flak airburst and contact burst,
  Volley, Flare patches, Limpets (stick, follow, hold, the fourth throw), Mortar, Triple Nock's
  fan, Pinning Arrow (enemy root, heavy, PvP slow at half speed, predicted), Ember Arrow, the
  bow's Brand, Burn on PvP players but not teammates, ability power, the PvP profile's rules.
* `server/tests/arsenal_net.rs`: PvP clients carry Surge for the Thorn (world and own state),
  co-op clients the Thorn with 3 charges, a reload recomputes; a loadout (with its knack) sent
  in the lobby is what the match spawns (sanitised), a change in play waits for the respawn;
  melee swings and knack uses reach the other client as `Fx::Swing` / `Fx::Knack` but not the
  user, a thrown snare replicates `STUCK`.
* `client/tests/client_combat.rs`: the client's `WeaponPredictor` stays exact over the network
  with the default loadout (Surge in PvP) and with both melee weapons and the Snare.
* `server/src/commands.rs` (unit): `/loadout` argument parsing (melee builds, `knack:<name>`
  anywhere, a knack alone keeping the weapons).
