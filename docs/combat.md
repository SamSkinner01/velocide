# VELOCIDE combat simulation

Server-authoritative combat: weapons with alt fires, lag-compensated hitscan with hit assist,
projectiles and explosions, area attacks, parry with network tolerance, thorn ricochets,
brands, ember bursts (healing), hard damage, the enemy definitions (the Moulds,
`docs/enemies.md`) and how all of it reaches clients. The arsenal itself (names, silhouettes,
behaviour, net kinds) is specified in `docs/roster.md`; the fiction in `docs/premise.md`.
Rules and default numbers come from `docs/gameplay.md`; every number is in `CombatTuning`
(`assets/tuning.ron`, section `combat`) or in `assets/enemies/*.ron`. Networking basics:
`docs/network.md`.

The last section is the **client presentation contract**: what the client must draw and play
for every entity kind and event (the spec for phase 2, client side).

## Where things live

| module | contents |
|---|---|
| `shared::sim` (`mod.rs`) | `SimWorld` (players, enemies, projectiles, thorns, brands, ember bursts, pending hits, rules), the tick pipeline, `SimEvent`, `ShotEvent`, `neutral_input`, replication, hitboxes |
| `shared::sim::combat` | `Owner`, `Attack`, `DamageFlags`, `DamageInfo`, `DamageEvent`, `Blocked`, `KillInfo`, `PlayerCombat`, `CombatRules`, **the damage pipeline** (`SimWorld::apply_damage`), death/respawn rule, vitals |
| `shared::sim::weapons` | `Weapon`, `WeaponState`, `FireIntent`, **`step_weapons`** (predictable), `SimWorld::resolve_intent` (what each mode does) |
| `shared::sim::hitscan` | `ShotSpec`, `ShotView`, `Impact`, lag-compensated traces, hit assist (`hit_pads`), thorn ricochets, Splinter bursts, Arclash chains |
| `shared::sim::area` | Bellows Gust (cone + deflection), Arclash Discharge (nova), Tumbler Breach |
| `shared::sim::projectiles` | `ProjectileKind`, `Projectile`, `Explosive`, projectile step, bomb bounces, Gravewell pull, Wisp zaps, explosions, `replicated_position` (client helper) |
| `shared::sim::parry` | punch, parry checks, deflection (`redirect_projectile`, shared with the Gust), auto-aim, parry-delayed damage, melee parry hook |
| `shared::sim::thorn` / `brand` / `embers` | thrown thorns (plant, drop, lie), brands (side table), ember bursts (close-range healing) |
| `shared::sim::hitbox` | `HitShape`, `Hitbox`, `HitPads` (hit assist: padded rays, sweeps, distances) |
| `shared::sim::trail` | short per-tick position history (rewinding non-hitbox things: projectiles, thorns, enemies) |
| `shared::sim::enemy` | `EnemyDef` (RON format) + `EnemyRegistry`, `Enemy`, `AiState`, damage/death; `ai.rs`: state machine, targeting, attacks, movement |
| `shared::protocol` | entity kinds and flag bits (`snapshot.rs`), `GameEvent` combat variants, `OwnState::combat`, `SnapshotMessage::fx`, `DamageReport`, `HitReport`, `Fx`/`FxEvent`, `QPos` (`combat.rs`) |
| `server::combat` | routing `SimEvent`s to reliable `GameEvent`s per recipient and batched `Fx` (`FxLog`) |
| `server::commands` | dev chat commands |
| `assets/enemies/*.ron` | the five enemies (also compiled in as the built-in definitions; `docs/enemies.md`) |

## Tick pipeline (`SimWorld::step`)

1. **Inputs → movement → weapons**, per player, per input (one input = one tick):
   movement step (dead players use `neutral_input`: no axes/buttons, view kept), then
   `step_weapons(&mut combat.weapons, input, alive, tuning, tick_rate)` → `FireIntent`s, each
   resolved immediately (`resolve_intent`): hitscan rewound to the input's view time, area
   attacks (Gust, Discharge, Breach), projectile spawns, punch + first parry check, thorn
   throws. A `MoveEvents::slam_landed` triggers the slam shockwave.
2. **Projectiles** move/collide/bounce/explode (stuck Gravewells pull, Wisps zap);
   parry-delayed impacts that ran out apply. Then **open parry windows** are re-checked
   against the moved projectiles.
3. **Enemies**: targeting, AI, attacks, movement; then due **pending enemy hits** apply.
4. **Thorns** (fly, plant, hit and drop, lie, expire), **brands** (expire, drop with their
   target), **ember bursts** (healing), **vitals** (hard damage recovery, spawn protection,
   respawn timers).
5. **World rules**: below the kill floor → put back at a spawn immediately (safety net);
   inside a kill volume → `kill_player(…, Attack::World)` (dies, respawns by the rule).
6. Returns every `SimEvent` of the tick.

The server records `SimWorld::hitboxes` (living players: body capsule + head sphere; living
enemies: body capsule + weak point spheres) into `LagCompHistory` after every tick, so enemies
are lag-compensated exactly like players. Projectiles, thorns and enemies also keep a
`Trail` of their end-of-tick positions (same moment) for parry/ricochet rewinding. The history
keeps the plain shapes; hit assist pads them at test time.

## Data model

### `PlayerCombat` (per player; `OwnState::combat`, 47 B in a snapshot)

| field | meaning |
|---|---|
| `damage` | health lost (`health = max_health − damage`, so `Default` = full health) |
| `hard` | hard damage (`effective_max = max_health − hard`; always `≤ damage`) |
| `hard_delay` | ticks until hard damage recovers (`hard_damage_recover_delay`) |
| `dead`, `respawn_in` | death state and ticks to respawn (default rule) |
| `protection` | ticks of spawn protection left |
| `god` | dev `/god` |
| `weapons` | `WeaponState` (below) |
| `last_hurt_tick`, `last_attacker`, `last_attack` | last damage taken (death cam, "killed by") |

### `WeaponState` (predictable: integers only)

`loadout` (the builds carried, `docs/arsenal.md`), `slot` (0–3, a slot holding a weapon),
`mode` (0 = ability A, 1 = ability B), `prev_buttons` (own edge detection),
`primary_cooldown[4]`, `alt_cooldown[4]` (ticks, per slot: switching mid-combo is free; the two
abilities of a build share its alt cooldown), `charge` (Discharge / Skewer ticks held), `draw`
(Longwick), `armed` (the bow's armed ability), `bloom` (Lancet), `burst` (salvo / Flurry in
progress), `punch_cooldown`, `parry` (window ticks left), `knack_used` + `knack_recharge` (the Q slot's knack, `docs/arsenal.md`; were the thorns').

**Input mapping.** `PlayerInput::weapon` = slot of the loadout (empty slots and other values
keep the current weapon); `weapon_mode` 0/1 (others → 0); switching weapon or mode is instant
and cancels a charge. With the default loadout (Quill, Bellows, Arclash, Tumbler): `FIRE` held =
automatic primary at the weapon's interval (the Arclash zaps every 0.1 s). `ALT_FIRE` held =
ability at its cooldown, except Discharge (hold to charge, fires on **release**). Both Barrels
also puts the primary on its interval. `PUNCH` press = fist + parry window. `THROW` press =
thorn (if a charge is left; 1 per `thorn.recharge_time`; none in PvP). The other weapons'
firing styles (salvo, draw, bursts, arming): `docs/arsenal.md`.

`step_weapons` is pure and deterministic. The owning client should run it on every predicted
input (exactly like `player_move_step`) for instant muzzle flashes/sounds/recoil, keep the
predicted `WeaponState` per input, and compare it with `OwnState::combat.weapons` for
`last_input_seq` (mismatch → reset to the server's and replay). Nothing server-side changes
the weapon state except a new loadout (respawn, warm-up, gun game), so the prediction is exact
(tests `server_weapon_state_is_exactly_the_predicted_one`,
`prediction_stays_exact_with_a_custom_loadout`).

### Rules (`SimWorld::rules: CombatRules`)

`friendly_fire` (PvP) and `coop_enemy_scaling`. `SimWorld::new` derives them from the level
kind (`Arena` → PvP, friendly fire on; `Campaign` → co-op, off, enemy health scaled); the
server then applies the lobby's match settings (`SimWorld::apply_match_settings`, docs/lobby.md):
deathmatch → PvP, co-op → co-op rules with the lobby's FRIENDLY FIRE toggle. Servers without a
lobby use the level kind's mode, so the result is the same as before. Dev `/ff on|off` toggles
until the next match world. Dev commands and `spawn_enemies` do nothing while the server is in
its lobby.

## Damage pipeline (`SimWorld::apply_damage(t, target, DamageInfo) -> DamageEvent`)

Everything that hurts goes through this one function.

```rust
pub struct DamageInfo {
    amount: f32,          // base damage (multipliers are applied by the pipeline)
    source: Owner,        // World | Player(id) | Enemy(net id)
    attack: Attack,       // Quill, Brand, Splinter, Blast, Gust, BothBarrels, Arc, Wisp,
                          // Discharge, Tumble, Breach, Gravewell, Thorn, Fist, Slam,
                          // Enemy{kind, attack}, World, Dev, then the expansion's
                          // primaries and abilities (docs/arsenal.md)
    point: Vec3, dir: Vec3,   // where, and the direction the damage travels
    part: HitPart,        // Body | Head | Weak(i)
    knockback: Vec3,      // velocity change
    flags: DamageFlags,   // HITSCAN PROJECTILE EXPLOSIVE MELEE PARRIED RICOCHET SLAM HEADSHOT WEAK_POINT
                          // SELF CHAINED RETURNED ENVIRONMENT ARMOURED DIRECT BRANDED
                          // BURN (status damage over time) PIERCED (Skewer past a target)
    ricochets: u8,        // thorn bounces before this hit
    projectile: Option<NetId>,
}
```

**Brands** first: a branded target (`SimWorld::brands`, see below) takes
`quill.brand_multiplier` (×1.35) of `amount` from every source except the world / dev
commands, flag `BRANDED`. A kill drops the brand.

**Players**, in order: missing/dead → `Blocked::NoTarget` (shots pass through corpses).
Self: explosive → `× self_damage_fraction` (0.35) and flag `SELF`; non-explosive self damage
→ blocked. Another player while `!friendly_fire` → `Blocked::FriendlyFire`. Head hit
(non-explosive) → `× player_head_multiplier` (2), `HEADSHOT`. Then `god` → `God`, spawn
protection → `SpawnProtection`, dash i-frames (`MoveState::is_invulnerable`) →
`Invulnerable` (kill volumes ignore the last two). **Knockback** is applied when the hit
counts *or* is explosive (blast jumps through i-frames, teammates' explosions in co-op).
Counted hits: `damage += amount`, `hard += amount × hard_damage_fraction` (capped at
`damage`), `hard_delay` reset, last attacker recorded; `damage ≥ max_health` → killed. In PvP
a counted hit sheds an ember burst the victim can't draw back in.

**Enemies**: enemy-sourced damage → `Blocked::SameTeam`. Weak point `Weak(i)` (non-explosive)
→ `× weak_points[i].multiplier`, flags `WEAK_POINT` (+ `HEADSHOT` for `kind: Head`). Armour:
damage arriving from the front arc (`-dir` within `front_arc/2` of the facing) `×
front_multiplier` unless the weak point is in `bypass` (explosions too if `explosions`),
flag `ARMOURED`. Knockback `× knockback_scale` into the enemy's velocity (airborne if it has
an upward component). Aggro bookkeeping. Ember burst (`heal_fraction` of damage dealt,
`melee_heal_fraction` for punches/parry counters). `health ≤ 0` → killed. (Hurt-flash and
death effects on the client: molten wax and sparks for the Moulds, sparks and ash for Shrikes.)

Emits `SimEvent::Damage(DamageEvent { target, info, dealt, hard, health_after, killed,
blocked, tick })` (also for blocked hits, not for `NoTarget`) and, on a kill,
`SimEvent::Killed(KillInfo)`:

```rust
pub struct KillInfo {
    victim: HitTarget, victim_kind: u16, killer: Owner, attack: Attack,
    flags: DamageFlags,           // of the killing blow
    part: HitPart, position: Vec3, ricochets: u8,
    killer_airborne: bool,        // AIRKILL
    victim_airborne: bool,        // AIRSHOT
    parried_projectile: bool,     // RETURN TO SENDER (flags also carry RETURNED when it hit its original owner)
    killer_health: f32, killer_effective_max: f32,   // CLOSE CALL
    overkill: f32,                // how violently it breaks apart
    distance: f32,                // killer to victim (m; 0 without a body): kill feed, LONGSHOT
    tick: u32,                    // MULTIKILL timing
}
```

Style mapping (for the style agent): HEADSHOT = `flags.HEADSHOT`; AIRKILL =
`killer_airborne`; AIRSHOT = `victim_airborne`; MULTIKILL = kills by the same killer within
0.6 s of `tick`; PARRY = `SimEvent::Parried`; RETURN TO SENDER = `parried_projectile`;
RICOCHET = `ricochets`; EXPLODED = `flags.EXPLOSIVE`; SLAM KILL = `attack == Slam`;
FISTED = `attack == Fist`; CLOSE CALL = `killer_health < 30`; FRAG = `victim_kind == PLAYER`;
DEFLECT = `SimEvent::Deflected` (a Gust turned a projectile around). Freshness keys off
`attack` (weapon + mode; a Gust-deflected projectile's attack becomes `Gust`). The server exposes every event via
`Server::drain_sim_events()`.

### Death and respawn (game modes)

`SimWorld::on_player_died` and `SimWorld::respawn_player` are the entry points the damage
pipeline and `tick_vitals` call; both delegate to the game-mode rule in `sim/modes.rs`
(`mode_player_died` / `mode_respawn_player`): solo campaign death cam then the last
checkpoint, co-op *downed* (revivable, `player_flags::DOWNED`), deathmatch respawn after
`respawn_delay` at the farthest spawn with spawn protection. See `docs/style_modes.md`.
While dead or downed, the body is simulated with `neutral_input` (falls/slides naturally)
and fires nothing.

## Weapons

| weapon / mode | implementation |
|---|---|
| **Quill** primary | hitscan from the eye along the aim, 24, every 0.28 s, ricochets off thorns |
| Brand (alt A) | hitscan 10, ricochets; whatever the last leg hits (and survives) is branded: `SimWorld::brand_target` (one brand per player, newest per target, 5 s), `SimEvent::Branded`; 2.5 s |
| Splinter (alt B) | hitscan 20, ricochets; at the last leg's `Impact` 6 secondary traces (`splinter_burst`): 40° cone, 18 m, 9 each, deterministic spread (`SpreadRng` of tick/player/seq); from a target along the shot with that target excluded, from the level along the shot reflected about the surface normal; same (rewound) hitboxes; `SimEvent::Splinters`; 1.1 s |
| **Bellows** primary (Blast) | 12 pellets in a fixed pattern on an 8° cone (`shared::sim::scatter`: a centre pellet, 5 at half the cone, 6 on its edge, ±0.5° jitter seeded by the input), 11 each (×1.25 within 3 m), knockback 1.3 m/s each along the ray, 60 m, no ricochet; a blast landing 4+ pellets on a light Mould staggers it 0.35 s; 0.8 s (`docs/balance.md`, the shotguns pass) |
| Gust (alt A) | `area::gust`: every hostile hitbox (rewound, padded) in a 7 m / 60° cone with line of sight: 6 (always a body hit), knockback `aim × 18 + 4ŷ`; every hostile projectile (enemies'; opponents' in PvP; not stuck, not pending on someone else) in the cone now or where the shooter saw it is turned around with `redirect_projectile` (owner → shooter, along the aim — no auto-aim, ×1.25 speed, ×1.5 damage, `parried`, attack → `Gust`) — no heal or parry reward, `SimEvent::Deflected` (style DEFLECT); recoil `−aim × 10` on the shooter; 1.6 s |
| Both Barrels (alt B) | 24 pellets in a 5° cone, 8 each; recoil `−aim × 9`; puts the primary on its interval too; 1.4 s |
| **Arclash** primary (Arc) | while held, a zap every 0.1 s: hitscan 30 m, 6, no ricochet; a hit chains (`arc_chain`) to the nearest other hostile within 7 m of the padded shape (line of sight from the previous target; branded targets first), ≤ 2 jumps, ×0.75 per jump, flags `HITSCAN \| CHAINED`, `SimEvent::ArcChain` |
| Wisp (alt A) | projectile 12 m/s from eye + 0.6 m, no gravity, 2.5 s, passes through targets; every 0.2 s zaps the nearest hostile within 6 m (padded, line of sight) for 8 (`SimEvent::WispZap`); pops on the level or at the end: r 4, 25, kb 8; 2.8 s |
| Discharge (alt B) | hold to charge (`WeaponState::charge`, 1.0 s max), release: `area::discharge` around the shooter's body centre: radius 3.5–8, damage 25–80, outward knockback 12–24 by charge to every hostile (padded distance, line of sight to the body or head), never the shooter; `SimEvent::Discharge`; 2.0 s |
| **Tumbler** primary (Tumble) | bomb 30 m/s, gravity 18; bounces off the level (×0.6, `ProjectileBounced`), explodes on its 3rd contact, on a hostile (direct) or after 1.6 s: r 4.5, 65 → 30 % at the edge, kb 20; 0.85 s |
| Breach (alt A) | `area::breach`: ray 3 m along the aim against the level and the (padded) hostiles; explosion at the first hit pulled back 0.2 m (else 3 m out): r 4, 70, kb 24, normal self damage (35 %, full knockback); no projectile; 1.2 s |
| Gravewell (alt B) | canister lobbed at 20 m/s, aim rotated 25° up (never below the aim), gravity 20, passes through targets; sticks to the level (`STUCK`, `SimEvent::GravewellStuck`); for 1.5 s each tick every hostile enemy within 9 m gets its velocity towards the centre raised to `9 × knockback_scale` m/s (easing off within 2 m, kept staggered), hostile players (PvP) half that; then implodes: r 5, 80, kb 6; 3.0 s |
| **Fist** (`PUNCH`) | lag-compensated melee (hit assist): nearest hittable target along the aim within 2.6 m, or whose nearest (padded) surface is within 2.6 m and inside the 50° cone, with line of sight; 25, `MELEE`, knockback 10 m/s (flattened aim + up); 0.4 s; opens the parry window |
| **Thorn** (`THROW`) | thrown from eye + 0.5 m along the aim at 30 m/s (+ half the thrower's velocity), gravity 12; plants in the first surface (`Planted`, oriented along its flight, `SimEvent::ThornPlanted`) and lives 8 s; hitting a hostile (padded sweep): 8 (`Attack::Thorn`, `PROJECTILE`), then it drops and lies on the floor (`Lying`, 8 s); never landing: gone after 3 s; 3 charges, 1 per 3 s |

## Hit assist

Player-owned attacks test against shapes inflated by `combat.hit_assist`
(`SimWorld::hit_pads(attacker, target)`): enemy bodies +0.35 m, enemy weak points / heads
+0.15 m (the Moulds' hit shapes themselves already cover their drawn models, `docs/enemies.md`
"Sizes and hit shapes"); in PvP player bodies +0.12 m, heads +0.06 m. Enemy (and world-owned) attacks get
`HitPads::NONE`. Applied at test time to the (rewound) plain shapes:

* rays (every hitscan, splinters, ricochet legs, Breach, the fist's aim ray):
  `Hitbox::ray_hit_padded` — the plain shapes first, the padded ones only on a miss, so a
  plain head hit is never turned into a body hit;
* projectile and thorn sweeps: `sweep_sphere_padded`;
* explosions, the Discharge, Wisp zaps, Arc chain range, the fist's cone and the Gust:
  `distance_to_point_padded` (an explosion never pads its owner).

## Hitscan and lag compensation

1. `shot_view`: hitboxes rewound through `HitboxHistory` to `input.view_time_ticks()` (the
   tick the client was rendering, `view_tick + view_frac/256`), clamped by the history to
   `[newest − net.lag_comp_max_ms, newest]`; without a history (or before it has frames)
   current hitboxes. Thorns are rewound to the same (clamped) time through their trails.
2. Ray vs level (`MASK_SHOTS`: world + doors) gives the far limit; ray vs every rewound
   hitbox the shooter `can_hit` (alive and hostile: not itself, not co-op teammates, not
   corpses; hit assist) — the nearest one is hit.
3. The first thorn within `ricochet_radius` of the ray before that limit (if the shot can
   ricochet) takes the shot instead: it shatters and the shot re-fires from it (see
   Ricochet).
4. A hit → `apply_damage` with `HITSCAN` (+ `RICOCHET`), part from the shape, knockback
   along the ray. The last leg's `Impact` (point, direction, surface normal or target) feeds
   Brand, Splinter and Arc.

**Rewind budget.** The server has to rewind ≈ one-way latency × 2 + interpolation delay +
input buffer: ~205 ms at 100 ms RTT, ~305 ms at 200 ms RTT (measured). The networking
default `net.lag_comp_max_ms = 350` therefore fully compensates players up to ~245 ms RTT;
raise it further for higher pings, at the cost of "shot behind cover" moments for their
victims.

## Projectiles

Swept spheres, simulated on the server only. Per tick: exact ballistic step
(`p += v·dt − ½g·dt²ŷ`, `v −= g·dt·ŷ`), sphere cast against the level (`MASK_SHOTS`) and a
swept-sphere test against the **current** hitboxes of everything the owner `can_hit` (owners
and co-op teammates are never hit; enemy projectiles never hit enemies); the earliest contact
wins.

| kind (net kind) | gravity | radius | life | on target | on level | parryable |
|---|---|---|---|---|---|---|
| Bomb (72) | 18 | 0.2 | 1.6 s fuse (explodes) | explode (direct) | bounce ×2 (`state` = bounces left), explode on the 3rd contact | no |
| Gravewell (73) | 20 | 0.25 | 5 s in flight, then 1.5 s stuck (implodes) | passes through | stick, pull, implode | no |
| Wisp (74) | 0 | 0.3 | 2.5 s (pops) | passes through, zaps every 0.2 s | pop | no |
| EmberBomb (80) | `enemy_projectiles.ember_gravity` (20) | 0.3 | 6 s | explode (r `ember_explosion_radius` 2.5, the attack's damage, kb `ember_knockback` 9) | explode | yes |
| Needle (81) | `needle_gravity` (0) | 0.12 | 2 s | direct damage (attack's) | vanish (`ProjectileImpact`) | yes |

Kinds 64–69 (the first arsenal's rivet, sawblade, rocket, flak, sticky, mortar) are retired.
Players' projectiles are not parryable (no punchable grenades); the Bellows Gust turns any
hostile projectile around, the players' included (PvP).

Enemy projectiles: `SimWorld::spawn_enemy_projectile(t, kind, enemy, attack, pos, vel,
damage)` (used by `AttackKind::Projectile` attacks; speed/damage/count/spread/lead/muzzle/lob
come from the enemy definition; for explosive kinds `damage` is the explosion's). Enemy
projectiles never hit enemies, and an ember bomb's explosion is blocked on enemies
(`Blocked::SameTeam`, no knockback).

**Parry-delayed impacts.** A parryable projectile touching a *player* freezes there
(`PENDING`) for `parry_delay_ticks(victim)` and only then applies (direct damage or
explosion) — unless a parry deflects it first (next section).

### Explosions (`SimWorld::explode`)

For every living player and enemy hitbox: distance from the centre to the nearest shape
surface (padded for player-owned explosions, except the owner) `d ≤ radius` (or the
direct-hit target); line of sight from the centre to the body
centre or head (blocked → unaffected, unless touching); `frac = 1 − 0.7 · d/radius`; damage
`× frac`; knockback `normalize(dir_from_centre + 0.3ŷ) × knockback × frac` (full for the
owner: blast jumps; teammates in co-op get knockback only). Flags `EXPLOSIVE` (+ `DIRECT`,
`PARRIED`, `RETURNED`).

## Parry

* `PUNCH` opens the window (`parry_window` = 12 ticks) and checks immediately; the window is
  re-checked every tick after projectiles moved (projectiles arriving during it count).
* A projectile is parried when **either** its position *rewound to the puncher's view time*
  (`trail.at(now − view_lag)`, view lag = server tick − the puncher's input view time,
  clamped to `lag_comp_max_ms`) **or** its current position (for a projectile pending on the
  puncher: the impact point backed off 0.6 m along its incoming direction) is within
  `parry_range` (3 m) of the eye inside `parry_cone_degrees` (70°, i.e. ±35°), or within
  `parry_close_range` (1.2 m) anywhere in front (never from behind).
* Eligible: parryable kinds (enemy projectiles) not stuck; others' (enemies'; opponents' in
  PvP); own and co-op teammates' only for `owner_can_parry` kinds (none at the moment). A
  projectile you just deflected can't be re-parried by you.
* **Deflection**: owner → the puncher (`original_owner` kept), `parried`, no gravity, position
  eye + 1 m along the new direction = aim, auto-aimed at the nearest hostile (head/weak point
  if visible) within `parry_auto_aim_degrees` (15°) / 80 m; speed `× 1.5`, damage `× 2`
  (explosion too), lifetime reset, pending impact cancelled. `SimWorld::redirect_projectile`
  does the work (the Gust calls it with ×1.25 / ×1.5 and no auto-aim).
* **Network tolerance**: parryable hits on a player are delayed by
  `parry_delay_ticks = ceil(net.parry_tolerance_ms + view lag of the victim)` (view lag
  clamped to `lag_comp_max_ms`; set `combat.parry_compensate_view_lag = false` to use the
  tolerance alone, as `docs/gameplay.md` literally says). Rationale: a client renders remote
  things `view lag` in the past, so it *sees* the orb reach it that much after the server
  registered the impact, and its punch arrives later still; delaying by the victim's own lag
  means a parry made on what they saw counts, while the attacker's projectile only lands
  that much later (it's the victim's tolerance window, bounded by `lag_comp_max_ms + 80 ms`).
  Measured: 200 ms of view lag → parried 167 ms after the server-side impact; without the
  view-lag term the same punch is too late.
* **Reward**: `parry_hard_damage_clear` (50 %) of hard damage cleared, healed to the effective
  max, `SimEvent::Parried(ParryEvent { player, target, position, healed, late_ticks, tick })`.
* **Telegraphed melee hook**: an attack with `parry_window: Some(s)` publishes
  `Enemy::parry_window = { start, end = strike tick, seq }` (the Gaoler's bash); one with
  `parry_strike` publishes the whole strike instead (`end` clipped when it ends early: the
  Harrier's dive, the Swat). Entity flag `PARRYABLE` while `start ≤ tick ≤ end`. A punch whose
  view time — or the current tick — falls in it, with the enemy (rewound through its trail)
  within the attack's `parry_reach` (default `melee_parry_range`) + its radius, stuns it for the
  attack's `parry_stun`, deals `parry_damage` (`Fist`, `MELEE | PARRIED`) and cancels the
  strike's pending damage; a parried flyer is knocked out of the air (stunned flyers fall).
  Strikes of parryable attacks queue their damage as `PendingHit`s keyed
  `ParryKey::Melee(enemy, seq)` with the same delay, so lagging players can parry too.

## Ricochet

From a shattered thorn the shot re-fires at: the nearest other live thorn within
`chain_range` (30 m) in line of sight (chain), else the nearest **branded** hittable target
within `target_range` (40 m), else the nearest hittable target in range — aiming at its first
visible weak point / head, else its body. No target → continues in the original direction.
Each bounce multiplies damage by 1.5 (`24 → 36 → 54`, ×2 on a head). Bounce legs use current
hitboxes/thorns (they auto-aim at current positions). Quill shots ricochet (primary, Brand,
Splinter — the main shot, not the splinters); Bellows pellets, Arclash zaps and Wisp zaps
don't. In PvP the targets are opposing players. Events: `SimEvent::Ricochet { shooter,
attack, points }` (thorn positions…, final end) and `ThornShattered`.

## Brands

`SimWorld::brands: BTreeMap<HitTarget, Brand { owner, expire_tick }>` — a side table, so
neither `SimPlayer` nor `Enemy` carries brand state. `brand_target(owner, target, point)`
drops the owner's previous brand and brands `target` for `quill.brand_duration` (one live brand
per player; a target holds the newest brand). `step_brands` drops expired brands and brands
on dead / removed targets; a kill drops it at once. `SimWorld::replicate` ORs the `BRANDED`
flag (bit 15 of `player_flags` / `enemy_flags`) into the target's entity. Ricochets and Arc
chains prefer branded targets; the damage pipeline multiplies damage to them.

## Healing, hard damage, i-frames

* Ember bursts (*kindling*, `docs/premise.md`): see `embers.rs` docs. A damage instance dealt
  *by a player* to an enemy (or another player) sheds (or merges into) a burst of embers at the
  hit point; each tick the closest living player within the radius who is below their effective
  max draws it in (the PvP victim can't draw back their own). Enemy death → its heat released
  as a big burst (25 HP, r 4.5, 1 s). Tuning section `combat.embers` (old files: `blood`).
* Hard damage (*smother*): 35 % of each hit smothers the wick, capping healing at
  `max − hard`; it rekindles at 25/s after 1.5 s without damage. Parries clear half.
* Dash i-frames: `MoveState::is_invulnerable()` blocks damage (`Blocked::Invulnerable`;
  explosive knockback still applies).
* Slam shockwave: on landing, everything hostile within `movement.slam_shockwave_radius`
  (6 m, |dy| ≤ 3 m) takes `slam_damage × s` and knockback `slam_knockback × s` (mostly up),
  `s = 0.3 + 0.7 × power`.

## Enemies

### Definition format (`assets/enemies/<name>.ron` → `EnemyDef`)

Every field has a default. Local offsets: +X right, +Y up, −Z forward, from the feet.

| field | meaning |
|---|---|
| `name`, `net_kind` | display name; replicated kind (16..64, unique: Gnawer 20, Censer 21, Harrier 22, Gaoler 23, Heron 24; 16–19 retired) |
| `model` | client model name |
| `model_scale` | drawn size of the model (the client scales it about its origin); presentation only (default 1) |
| `health` | base health (co-op: `× (1 + 0.5 (players − 1))`, hook `SimWorld::enemy_health_scale`) |
| `speed`, `acceleration`, `turn_rate` | m/s, m/s², deg/s |
| `radius`, `height`, `body_top` | movement capsule; top of the body hitbox (`None` = height) so a head can sit above it |
| `hit_radius`, `body_shapes` | radius of the body hitbox (`None` = the capsule's: hit shapes are sized to the drawn model, the capsule to the level); extra body capsules `[(a, b, radius)]` (local offsets) for wings, shields, crucibles... (`EnemyDef::hitbox` builds them all) |
| `eye_height`, `step_height` | sight origin; climbable ledge |
| `locomotion` | `Ground` or `Flying(min_height, max_height)` (hovers, no gravity) |
| `knockback_scale` | light enemies > 1 |
| `weak_points` | `[(name, kind: Head/Core, offset, radius, multiplier)]` → `HitPart::Weak(i)` |
| `armour` | `Some((front_multiplier, front_arc, bypass: ["head"], explosions))` |
| `attacks` | `[AttackDef]`, in priority order (below) |
| `ai` | `sight_range, reaction_time, lose_target_time, preferred_range: Option<(min,max)>, strafe, target_stickiness, aggro_switch_fraction, retarget_interval, repath_interval, reposition` |
| `drops` | `(heat, heat_radius)`: heal pool and radius of the death burst (old files: `blood_pool`, `blood_radius`) |
| `colors` | `body` (wax), `accent`, `glow` (the heat: weak points, hit sprays, death bursts), `telegraph`, `parry` (sRGB) |
| `death_time` | corpse duration (Dying state) |
| `leap` | `Some((distance, up, down, crouch))`: leaps on its nav path across gaps / water and up and down ledges within those limits, after a `crouch` (s); `None` = never (default) |
| `pads` | rides jump pads on its nav path (default true) |
| `melt` | `(after, sink, rise, near, far)`: stranded for `after` s (0 = never) it melts (`sink` s) and re-forms (`rise` s) `near`..`far` m from its target (`docs/enemies.md` "Leaps, pads, melting") |

`AttackDef`: `name`, `kind` (`Melee` / `Projectile` / `Slam` / `Dive`), `range`, `min_range`,
`arc` (melee cone), `damage`, `knockback`, `knockback_up` (upward bias), `windup`, `strike`,
`recover`, `cooldown`, `windup_move_scale`, `track_target`, `parry_window: Option<f32>` /
`parry_strike` (+ `parry_stun`, `parry_damage`, `parry_reach`), `shockwave_radius` (Slam),
`projectile: Option<(kind, speed, count, interval, spread, muzzle, lead, lob)>`, `lunge` (leap /
dive speed), `lunge_up` (pounce), `dive_lead`, `min_target_speed`, `jumpable: Option<f32>`
(low sweep: only targets whose feet are below that height above the attacker's), `laser:
Option<(lock, track_rate, lead)>` (aim laser).

The server loads `assets/enemies/*.ron` over the compiled-in built-ins
(`EnemyRegistry::load_default`; `ServerConfig::enemies_dir` overrides); invalid files are
logged and skipped. The five Moulds are pure data on top of these: Gnawer = `Melee` bite +
leaping `Melee` pounce (`lunge` + `lunge_up` + `min_target_speed`); Censer = `Melee` swing +
lobbed `Projectile` burst of 3 `EmberBomb` + `preferred_range` / `strafe`; Harrier =
`Flying(5, 10)` + `Dive` with `parry_strike`; Gaoler = `armour` + parryable `Melee` bash +
`jumpable` 300° sweep + `lunge` rush; Heron = `Melee` kick + `Projectile` `Needle` with a
`laser` + `reposition`. Navigation, the AI per type and arena encounters are documented in
`docs/enemies.md`.

### AI (`ai.rs`)

`Idle` → (a target is visible) `Alert` for `reaction_time` → `Chase` (nav grid; ranged
enemies hold their band and may strafe; stop at 60 % of melee range) → first usable attack
(cooldown ready, in range, line of sight) → `Windup` (telegraph; turns towards the target if
`track_target`, or tracks its aim laser; creeps at `windup_move_scale`) → `Strike` (melee arc /
leap / dive / slam radius damage, or the projectile burst; ≥ `strike` s, a leap or dive ends
early on the level) → `Recover` → `Chase`. `Stunned` (parry) → `Chase`. `Dying` (corpse, no
hitboxes) → removed after `death_time`. Getting there: `Leap` (a jump on the nav path: a
crouch, then airborne) → `Chase`; stranded → `Melt` → (moved) `Reform` → `Chase`. Details per
type: `docs/enemies.md`.

Targeting (every `retarget_interval`): a player who dealt ≥ `aggro_switch_fraction` of max
health in the last 2 s; else keep the current target unless another visible one is closer
than `target_stickiness ×` its distance; else the nearest visible (sight range + line of
sight from the eye). Lost for `lose_target_time` → Idle. `/notarget` players are ignored.

Movement: accelerate towards the desired velocity (ground: horizontal + gravity), capsule
collide-and-slide against `MASK_MOVEMENT` (4 iterations, step-up), ground snap within
`step_height`, depenetration, soft separation between enemies. Players and enemies don't
block each other (player movement must stay predictable).

## Replication

### Entities (all delta-compressed and quantised as in `docs/network.md`)

| kind | owner | position / velocity | yaw | flags | state | health | aux |
|---|---|---|---|---|---|---|---|
| `PLAYER` 0 | player id | feet / velocity | view | `LOW_PROFILE 1, INVULNERABLE 2, DEAD 4, PARRY 8, CHARGING 16, PROTECTED 32, DOWNED 64, …, BURNING 1<<12, PINNED 1<<13, DRAWING 1<<14, BRANDED 1<<15` | held weapon, selected ability, charge / draw: `protocol::player_weapon(state)` (`docs/arsenal.md`) | health (ceil) | hard (bits 0–7) \| knack charges (8–11) \| wall-cling yaw (16–31) |
| enemies 16..64 | target player or `OWNER_NONE` | feet / velocity (¼ m/s steps) | facing (~0.7° steps; exact while `AIMING`), pitch = the laser's aim pitch while `AIMING` | `ALERT 1, PARRYABLE 2, STUNNED 4, DYING 8, AIRBORNE 16, ARMOURED 32, AIMING 64, AIM_LOCKED 128` (6–12 enemies area), `BURNING 1<<13`, `PINNED 1<<14`, `BRANDED 1<<15` | `AiState` (Idle 0, Alert 1, Chase 2, Windup 3, Strike 4, Recover 5, Stunned 6, Dying 7) | health (ceil) | progress through the timed state 0–255 (0–7) \| attack index (8–15) \| max health (16–31) |
| projectiles 64..128 | shooter id, `OWNER_ENEMY` (0xFFFE), `OWNER_NONE` | **at launch tick** | – | `PARRIED 1, STUCK 2, PENDING 4, ENEMY 8` | bounces left | – | launch tick |
| `EMBER_BURST` 128 | – | burst centre | – | `BIG 1` | radius (dm) | pool left (ceil) | spawn tick |
| `THORN` 160 | thrower | **at launch tick**; while `PLANTED`: where it sits / its orientation (unit) | – | `PLANTED 1, LYING 2, DROPPING 4` | – | – | launch tick |

**Trajectory replication** (projectiles, thorns): `pos(t) = p0 + v0·Δ − ½·g·Δ²·ŷ`,
`Δ = (t − aux) / tick_rate`, g = the kind's gravity (`ProjectileKind::gravity`,
`thorn.gravity`), 0 when `PARRIED`; `STUCK`/`PENDING`/`PLANTED` → `position` as is.
`shared::sim::projectiles::replicated_position(&entity, time_ticks, tick_rate,
&tuning.combat)` implements it. The entity changes only when the trajectory does (bounce,
parry, stick), so flying projectiles cost nothing per snapshot; the reproduction error is
< 2 cm (test).

### Owner-only state

`OwnState { movement, combat: PlayerCombat }` every snapshot (unquantised, 32 B for combat).

### Reliable events (`ServerMessage::Game(GameEvent)`)

| event | recipients | payload |
|---|---|---|
| `Respawned { player, position, yaw }` | everyone | (existing) |
| `Damaged(DamageReport)` | the victim only | amount, hard, health after, source, attack, point, direction, flags, blocked (i-frames / protection / god / friendly fire), killed |
| `HitConfirm(HitReport)` | the attacker only, **merged per target per tick** (10 pellets = 1) | target, target kind, attack, damage, hits, flags (union), point, killed |
| `Killed(KillInfo)` | everyone | see the damage pipeline |
| `Parried(ParryEvent)` | everyone | player, target (projectile id + kind / melee enemy), position, healed, late ticks |
| `HighlightMark(Highlight)`, `PlayOfTheGame(Highlight)` | everyone (PvP) | play of the game: player, start / end tick, score, title (docs/potg.md) |

### Cosmetic events (`SnapshotMessage::fx: Vec<FxEvent>`, unreliable)

Batched into each client's next snapshot; important ones (`Explosion`, `Ricochet`,
`Splinters`, `ArcChain`, `Branded`, `Deflected`, `Discharge`, `SlamShockwave`,
`EnemyShockwave`, `EnemyAttack`, `EnemySpawned`) are repeated in the following snapshot as
well, so clients **must de-duplicate by `seq`** (global, +1 per event). `tick` = when it
happened (play it when the render clock reaches it). At most 256 per snapshot (important first,
then newest). Positions are `QPos` (1/64 m varints).

| `Fx` | recipients |
|---|---|
| `Shot { shooter, attack, origin, ends }` | everyone but the shooter |
| `Ricochet { shooter (owner code), attack, points }` | everyone |
| `Explosion { position, radius_dm, attack, owner }` | everyone (the owner plays it on arrival: it draws its own projectiles at the present) |
| `Punch { player, hit }` | everyone but the puncher |
| `Heal { player, amount_tenths, from }` | everyone |
| `SlamShockwave { player, position, power, radius_dm }` | everyone |
| `ThornThrown { player }` | everyone but the thrower |
| `ThornPlanted { position }` | everyone |
| `ThornExpired { id }` | everyone |
| `Splinters { shooter, origin, ends }` | everyone (the shooter can't predict them) |
| `ArcChain { shooter, points }` | everyone |
| `WispZap { from, to }` | everyone |
| `Branded { position }` | everyone |
| `Deflected { player, position }` | everyone |
| `Discharge { player, position, radius_dm }` | everyone but the player (predicted) |
| `ProjectileBounce { kind, position }` | everyone |
| `ProjectileImpact { kind, position, normal }` | everyone |
| `EnemyAttack { enemy, attack, strike, parryable }` | everyone |
| `EnemyFired { enemy, attack, muzzle }` | everyone |
| `EnemyShockwave { enemy, position, radius_dm }` | everyone |
| `EnemyAlert { enemy }` | everyone |

## Client presentation contract

Implemented in `client/src/ingame/combat/` (see `docs/client.md` → "Combat presentation"):
`weapon_feel.rs` (local weapon prediction), `fx.rs` (tracers, lights, `Fx` playback),
`entities.rs` (enemies, projectiles, healing bursts, thorns, brand sigils), `hud.rs` (combat HUD), `mod.rs`
(`CombatClient`, the `GameEvent` reactions and the per-frame glue). `NetClient` exposes
`own_combat()` (latest `OwnState::combat` + its input seq) and `drain_fx()` (de-duplicated
`FxEvent`s). Verified with `cargo run -p client --release --example combat_shots` (headless
screenshots of every case below).

Prediction duties:

* **Weapons**: `WeaponPredictor` runs `step_weapons` on each predicted input (right after
  `NetClient::tick`) and plays muzzle flash, weapon sound (`QuillFire`, `BrandFire`,
  `SplinterFire`, `BellowsBlast`, `BellowsGust`, `BothBarrels`, `ArcZap`, `WispLaunch`,
  `DischargeRelease`, `BombLaunch`, `BreachBlast`, `GravewellLaunch`), viewmodel kick and local
  tracers (client ray against the level and the displayed capsules; sparks + ash on bodies,
  sparks + scorch decal on the level; jagged arcs for Arclash zaps) immediately for every
  `FireIntent`; the Gust puff / rings and the Discharge nova are predicted too;
  `DischargeCharge` whine + capacitor glow + crosshair gauge (with the nova radius) while
  `charge > 0`; the Tumbler drum turns a chamber per shot; `Punch` anim + sound;
  `ThornThrow`; `WeaponSwitch` / `ModeSwitch` on selection changes. On every newer
  `OwnState` the prediction is re-based on `combat.weapons` and the unacked inputs are replayed
  without effects (`client_combat` test: 0 mismatches over 587 inputs at 100 ms RTT, every
  weapon and mode incl. a charged Discharge). Recoil and knockback stay server-side (movement
  reconciliation). A button change between ticks plays the next tick's shot on that frame
  (`WeaponPredictor::lookahead`, `docs/feel.md`); the tick then finds it played.
* **Death**: `NetClient::tick` predicts (and sends) `sim::neutral_input` for inputs up to
  the predicted respawn (`OwnCombat::dead_at`: `seq ≤ own.seq + respawn_in`), so the corpse
  doesn't mispredict; the death screen shows "YOU DIED", the killer line from the `Killed`
  event and "RESPAWNING IN N.N".
* **Projectiles/thorns**: other players' / enemies' projectiles and *all* thorns are drawn
  at `replicated_position(e, render_tick, …)` (consistent with lag compensation, parry and
  ricochet validation). **Your own projectiles** are taken from the newest snapshot and drawn
  at the present (`server_tick_estimate`, at most 6 ticks past the newest snapshot), starting
  at the drawn muzzle and blending onto the true path over 0.15 s; before the server reports
  one, a local stand-in flies it from the muzzle on the shot's frame and hands over to it
  without a pop or a double (`stand_in.rs`, `docs/feel.md`); your own explosions
  (`Fx::Explosion::owner`) play on arrival to match (an own Breach's bang was already played
  by the prediction).

HUD (from `OwnState::combat` + the predicted `WeaponState`): health `max − damage` with
the hard-damage segment (existing health block via `HudInfo`); thorn pips with recharge
progress; primary / alt cooldown bars under the crosshair; the Discharge gauge beside it
with the nova radius; a pulsing BRANDED warning while someone branded you; hit markers (body white, head/weak point red,
armour grey, kill: big red, expanding) + damage numbers (pellets merged); damage direction
chevrons around the crosshair; pulsing low-health vignette below 40 %; `PROTECTED` while
spawn protected; death screen.

| thing | render / play |
|---|---|
| **PLAYER** entity | drawn 10 % larger than life (visual only); body/stance from `LOW_PROFILE`; weapon model and ability from `protocol::player_weapon(state)` (the expansion's weapons borrow their class's model for now); `CHARGING` with Discharge → Arclash capacitor glow + crackling arcs (charge level from `player_weapon`); `BRANDED` → turning brand sigil above the head; `PARRY` → punch pose; `INVULNERABLE` → dash afterimage; `PROTECTED` → spawn shimmer; `DEAD` → corpse/ragdoll, no name tag; name tag health bar from `health` (+ hard from `aux`) for teammates |
| **enemy** entity (16..64) | model/colours from the `EnemyDef` with that `net_kind` (same RON files); animation by `state` and attack index (Idle, Alert: the type's alert bark, Chase: walk/run by speed, Windup: the attack's telegraph pose + `colors.telegraph` glint driven by `aux & 0xFF` progress + the attack's wind-up sound, Strike: the strike + its sound, Recover, Stunned: dazed / swatted, Dying: slump, split and melt); `PARRYABLE` → flash `colors.parry` (yellow); `AIMING` → the aim laser from the muzzle along yaw / pitch (red), `AIM_LOCKED` → white-hot + `HeronLock`; `BRANDED` → turning brand sigil (a diamond and a point-down triangle, camera-facing) above it; hits → wax and molten droplets + sparks + `EnemyHurt`; death → one big ember burst + smouldering corpse; health bar from `health` / `aux >> 16`; `AIRBORNE` → flail (details: `docs/enemies.md`) |
| **projectiles** | bomb: tumbling studded ball, fuse sparks and a flickering fuse band (`BombBounce` on `ProjectileBounce`); gravewell: canister with a green light, `STUCK` → claws down, vortex of motes drawn in from the pull radius, `GravewellHum` once; wisp: spinning ball of lightning, crackling arcs, violet light, `WispHum` every 0.45 s; ember bomb: a glowing slag ball with a smoke and ember trail, light, `EmberFlight`; needle: a long glowing streak; `PARRIED` → white-hot tint + streak; `PENDING` → hide (it is "in" the victim). Every projectile is drawn at least ×2 with a halo, core and streak; parryable ones carry a parry-colour ring that flares in punch reach (`docs/feel.md`) |
| **EMBER_BURST** | glowing embers (`palette().heal`) rising off a faint glow over a wisp of smoke, `state/10` m radius, burst size from `health` (pool), re-puffing while the pool lasts; `BIG` → the same, looser (the heat release itself is drawn by the dying enemy) |
| **THORN** | flying: the glass spike along its path; `DROPPING`: tumbling; `PLANTED`: tip-first in the surface along `velocity` (quivers after landing), drawn ×1.5 with a steady halo and a pulse ring so the relay reads at a distance; `LYING`: flat on the floor; disappears on removal (shatter via `Ricochet` / `ThornExpired`) |
| `Damaged` | red screen flash scaled by amount, direction indicator towards `-direction`, `Hurt` sound, camera kick; `blocked: Invulnerable` → dodge flash (no damage); hard-damage segment grows; `killed` → death |
| `HitConfirm` | hit marker (`Hitmarker`; markers pop in), `Headshot` + `WeakPointTick` and a red marker for `HEADSHOT`/`WEAK_POINT`, dull marker for `ARMOURED`, damage number at `point`, `KillConfirm` (+ a ring) when `killed` |
| `Killed` | kill feed (`KILLER [ATTACK] VICTIM 38m HEADSHOT RETURNED RICOCHET xN AIR`: the distance whenever the killer has a body and it wasn't a suicide; only kills involving a player; red when it involves you); `Death` + red flash + death screen for you; the heat release (embers, wax and slag lumps, smoke, a scorch) comes from the enemy entity entering `Dying` (also `EnemyDeath` / `EmberRelease` sounds) |
| `Parried` | for the parrier: usually already shown on the punch's frame (the client tests the same rule on what it draws, `docs/feel.md`), then only `Heal` when `healed`; otherwise now: hitstop 0.16 s, white flash, shake, `PARRY` banner, `Parry` + `ParryImpact`, a shock ring from the fist; for others: `Parry` sound at `position`; everyone: white glow + sparks at `position` |
| `Respawned` | `Respawn` sound/effect at `position` |
| `Fx::Shot` | remote muzzle flash + 3D weapon sound at the shooter (interpolated position), hitscan tracers (jagged arcs for `Arc`) from the shooter's muzzle to each end, impact sparks/decals at the ends; `Gust` → the air puff along the muzzle direction |
| `Fx::Ricochet` | bright tracer through `points`, thorn shatter at each thorn point, `Ricochet` sound (pitch up per bounce) |
| `Fx::Explosion` | by `attack`: bomb / enemy fireball (`Explosion`), tight white-hot Breach with debris (`BreachBlast`, not for its owner), Gravewell implosion (motes rushing in, green flash, closing ring, `GravewellImplode`), Wisp pop (violet arcs, `DischargeRelease` pitched up); light flash, shake by distance, scorch |
| `Fx::Splinters` | ice-cyan splinter tracers from `origin` to each end, glass glints, `SplinterBurst` |
| `Fx::ArcChain` / `WispZap` | jagged arcs between the points + glows / sparks, `ArcChain` (pitch up per jump) / `WispZap` |
| `Fx::Branded` / `Deflected` | brand flash + `Branded` / white flash, sparks, `Deflect` |
| `Fx::Discharge` | remote nova: rings rushing out along the floor, arcs, `DischargeRelease` |
| `Fx::ThornThrown` / `ThornPlanted` / `ThornExpired` | remote throw anim + `ThornThrow` / glass chips + `ThornPlant` / crumble + `ThornShatter` |
| `Fx::Punch` | remote arm swing, `Punch`/`PunchHit` |
| `Fx::Heal` | kindling: a stream of sparks (`palette().heal`) from `from` into the player's chest lantern (others: the lantern flares); for you: `Feedback::heal_flash` + `Heal` (an ember inhale and crackle) |
| `Fx::SlamShockwave` | ground ring of `radius_dm`, dust, `SlamImpact` |
| `Fx::ProjectileBounce` / `ProjectileImpact` | sparks (+ `BombBounce`) / impact puff + `ProjectileImpact` |
| `Fx::EnemyAttack` | `strike == false`: the attack's telegraph sound (by attack name: `GnawerBiteWindup`, `CenserWhirl`, `HarrierShriek`, `GaolerBashWindup`, `GaolerChain`, `HeronLaser`...) + glint in `colors.telegraph`; `parryable` → yellow flash; `strike == true`: its strike sound + bash dust ring / sweep arc / leap dust |
| `Fx::Explosion` with `Attack::Enemy` | an ember bomb bursting: flash, embers, smoke ring, light, `EmberBurst` (no fireball) |
| `Fx::EnemyFired` / `EnemyShockwave` / `EnemyAlert` | muzzle flash + `CenserLob` / `HeronNeedle` / slam ring / the type's alert bark |

## Dev commands (chat, `ServerConfig::dev_commands`)

On in single-player (`ServerConfig::single_player`) and on listen servers (`ServerConfig::host`,
for the host only); off on dedicated servers unless started with `--dev-commands` (then for
the lobby host; for everyone with `--no-lobby`). Elsewhere `/...` is answered with "dev
commands are disabled on this server" and never broadcast. Each command costs a chat token
(burst 5, then 1/s, shared with chat).


`/spawn <enemy> [count]` (8 m in front, on the ground, facing you), `/enemies`, `/killall`,
`/melt` (every ground Mould melts and re-forms near its target), `/god`, `/notarget`, `/give` (full health, no hard damage, thorns, cooldowns), `/kill`,
`/hurt <hp>`, `/ff [on|off]`, `/needle [distance]` (alias `/orb`: a Heron needle slowed to
22 m/s flies at you: parry practice), `/ember [distance]` (an ember bomb is lobbed onto you),
`/encounter <n>`, `/encounters`, `/tp <x> <y> <z>`, `/goto <exit|start|checkpoint
n|encounter n>` (teleports: `SimWorld::teleport_player`, the client snaps like a rally),
`/help`. On lobby servers (hosted / dedicated games) only the lobby host may use them; guests
get "dev commands are for the host only". The reply goes to the sender only; commands are not
broadcast. Programmatic: `ServerCommand::{SpawnEnemy, KillAllEnemies, SetFriendlyFire}`,
`Server::spawn_enemies`, `Server::kill_all_enemies`, `Server::drain_sim_events`.

## Measurements (tests, virtual clock)

| | |
|---|---|
| lag-compensated Quill shot at a target strafing 16 m/s, ~100 ms RTT, default tuning | target was 3.3 m from where the shooter saw it; server rewound 205 ms; **hit** (24). Same shot with `lag_comp_max_ms = 0`: miss |
| same at ~200 ms RTT (`lag_comp_max_ms = 400`) | 4.75 m away, rewound 305 ms, **hit** |
| parry of a 22 m/s orb, punching when the *displayed* orb is 2 m away | 0 ms: parried (2 ticks after the server impact, thanks to interpolation delay); ~100 ms RTT: parried 8 ticks late; ~200 ms RTT: parried 14 ticks late; punching 300 ms after the displayed touch: fails, 20 damage |
| breach jump (Breach at the feet + jump), 50 ms one-way | apex 17.9 m, self damage 24.3, 1 correction (0.40 m at the ack), no snaps, prediction == server afterwards |
| crowded frame: 8 players, 50 enemies, 150 projectiles | full 3.6 KB; delta 74 B with idle enemies and flying projectiles |
| bandwidth: 4 clients + 4 bots, 50 enemies chasing, 150 projectiles, Arclash zapping 10/s each | **33 KB/s per client** server→client (snapshot 1020 B: 722 B entities, 6.8 FX); server tick 0.4 ms (dev build) |
| 2 players (`net_loopback`) | snapshot 197 B (own state 155 B of it: movement 123 + combat 32); 0 prediction corrections with one player shooting |
| client weapon prediction (`client_combat`), 100 ms RTT, 587 inputs of every weapon / mode, Discharge charges, switching, punching, thorns | 0 re-base mismatches; predicted `WeaponState` == server's |
| client frame + render with combat effects (`combat_shots`, headless 1280×720, release) | avg 0.30 ms, p95 0.44 ms over 2419 frames |

Run: `cargo test -p shared --test combat_weapons --test combat_parry --test combat_ricochet
--test combat_enemy --test combat_replication` and `cargo test -p server --test combat_net
-- --nocapture`.

## Known limitations / next steps

* No interest management: every client gets every enemy (~10 B per moving enemy per
  snapshot). Distance-based update rates would cut the crowded-scene 28 KB/s a lot.
* Players and enemies don't collide (enemy navigation: `docs/enemies.md`).
* Client: hitscan tracers of your own pellets use client-side spread (the server's spread
  decides damage); splinters and arc chains are drawn from the server's events (one
  interpolation delay late).
* Default `net.lag_comp_max_ms` (350) compensates up to ~245 ms RTT (see above).
* Pending (parry-delayed) hits favour the victim by up to `lag_comp_max_ms + 80 ms`.
* A Gravewell sticks to the level only (it passes through targets); a stuck one is drawn
  claws-down (clients don't know the surface normal). Bombs never come to rest: a rolling bomb
  uses its contacts up quickly and explodes.
* Explosion LOS tests the body centre and head only (a foot sticking out of cover is safe).
