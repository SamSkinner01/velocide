# VELOCIDE combat simulation

Server-authoritative combat: weapons with alt fires, lag-compensated hitscan, projectiles and
explosions, parry with network tolerance, ricochet prisms, blood-burst healing, hard damage,
the first enemy (Husk) and how all of it reaches clients. Rules and default numbers come from
`docs/gameplay.md`; every number is in `CombatTuning` (`assets/tuning.ron`, section `combat`)
or in `assets/enemies/*.ron`. Networking basics: `docs/network.md`.

The last section is the **client presentation contract**: what the client must draw and play
for every entity kind and event (the spec for phase 2, client side).

## Where things live

| module | contents |
|---|---|
| `shared::sim` (`mod.rs`) | `SimWorld` (players, enemies, projectiles, prisms, blood, pending hits, rules), the tick pipeline, `SimEvent`, `ShotEvent`, `neutral_input`, replication, hitboxes |
| `shared::sim::combat` | `Owner`, `Attack`, `DamageFlags`, `DamageInfo`, `DamageEvent`, `Blocked`, `KillInfo`, `PlayerCombat`, `CombatRules`, **the damage pipeline** (`SimWorld::apply_damage`), death/respawn rule, vitals |
| `shared::sim::weapons` | `Weapon`, `WeaponState`, `FireIntent`, **`step_weapons`** (predictable), `SimWorld::resolve_intent` (what each mode does) |
| `shared::sim::hitscan` | `ShotSpec`, `ShotView`, lag-compensated traces, piercing, ricochet |
| `shared::sim::projectiles` | `ProjectileKind`, `Projectile`, `Explosive`, projectile step, explosions, stickies, `replicated_position` (client helper) |
| `shared::sim::parry` | punch, parry checks, deflection, auto-aim, parry-delayed damage, melee parry hook |
| `shared::sim::prism` / `blood` | ricochet prisms, blood bursts (healing) |
| `shared::sim::trail` | short per-tick position history (rewinding non-hitbox things: projectiles, prisms, enemies) |
| `shared::sim::enemy` | `EnemyDef` (RON format) + `EnemyRegistry`, `Enemy`, `AiState`, damage/death; `ai.rs`: state machine, targeting, attacks, movement |
| `shared::protocol` | entity kinds and flag bits (`snapshot.rs`), `GameEvent` combat variants, `OwnState::combat`, `SnapshotMessage::fx`, `DamageReport`, `HitReport`, `Fx`/`FxEvent`, `QPos` (`combat.rs`) |
| `server::combat` | routing `SimEvent`s to reliable `GameEvent`s per recipient and batched `Fx` (`FxLog`) |
| `server::commands` | dev chat commands |
| `assets/enemies/husk.ron` | the Husk (also compiled in as the built-in definition) |

## Tick pipeline (`SimWorld::step`)

1. **Inputs → movement → weapons**, per player, per input (one input = one tick):
   movement step (dead players use `neutral_input`: no axes/buttons, view kept), then
   `step_weapons(&mut combat.weapons, input, alive, tuning, tick_rate)` → `FireIntent`s, each
   resolved immediately (`resolve_intent`): hitscan rewound to the input's view time,
   projectile spawns, punch + first parry check, prism throws, sticky detonation. A
   `MoveEvents::slam_landed` triggers the slam shockwave.
2. **Projectiles** move/collide/explode; parry-delayed impacts that ran out apply. Then
   **open parry windows** are re-checked against the moved projectiles.
3. **Enemies**: targeting, AI, attacks, movement; then due **pending enemy hits** apply.
4. **Prisms** (ballistic, bounce, expire), **blood bursts** (healing), **vitals** (hard damage
   recovery, spawn protection, respawn timers).
5. **World rules**: below the kill floor → put back at a spawn immediately (safety net);
   inside a kill volume → `kill_player(…, Attack::World)` (dies, respawns by the rule).
6. Returns every `SimEvent` of the tick.

The server records `SimWorld::hitboxes` (living players: body capsule + head sphere; living
enemies: body capsule + weak point spheres) into `LagCompHistory` after every tick, so enemies
are lag-compensated exactly like players. Projectiles, prisms and enemies also keep a
`Trail` of their end-of-tick positions (same moment) for parry/ricochet rewinding.

## Data model

### `PlayerCombat` (per player; `OwnState::combat`, 32 B in a snapshot)

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

`weapon` (slot 0–3), `mode` (0 = alt A, 1 = alt B), `prev_buttons` (own edge detection),
`primary_cooldown[4]`, `alt_cooldown[4]` (ticks, per weapon: switching mid-combo is free),
`charge` (Overcharge ticks held), `punch_cooldown`, `parry` (window ticks left),
`prisms_used` + `prism_recharge`, `stickies` (live count, server-authoritative) +
`sticky_since` (ticks since the last throw).

**Input mapping.** `PlayerInput::weapon` 0 Lancet, 1 Thresher, 2 Riveter, 3 Cinderbox (other
slots keep the current weapon); `weapon_mode` 0/1 (others → 0); switching weapon or mode is
instant and cancels an Overcharge charge. `FIRE` held = automatic primary at the weapon's
interval. `ALT_FIRE` held = alt at its cooldown, except Overcharge (hold to charge, fires on
**release**) and Sticky (on **press**: see Cinderbox). `PUNCH` press = fist + parry window.
`THROW` press = prism (if a charge is left; 1 per `recharge_time`).

`step_weapons` is pure and deterministic. The owning client should run it on every predicted
input (exactly like `player_move_step`) for instant muzzle flashes/sounds/recoil, keep the
predicted `WeaponState` per input, and compare it with `OwnState::combat.weapons` for
`last_input_seq` (mismatch → reset to the server's and replay; only `stickies` normally
differs, when a sticky exploded server-side).

### Rules (`SimWorld::rules: CombatRules`)

`friendly_fire` (PvP) and `coop_enemy_scaling`. From the level kind: `Arena` → PvP
(friendly fire on), `Campaign` → co-op (off, enemy health scaled). Game modes / lobby set
them; dev `/ff on|off` toggles.

## Damage pipeline (`SimWorld::apply_damage(t, target, DamageInfo) -> DamageEvent`)

Everything that hurts goes through this one function.

```rust
pub struct DamageInfo {
    amount: f32,          // base damage (multipliers are applied by the pipeline)
    source: Owner,        // World | Player(id) | Enemy(net id)
    attack: Attack,       // Lancet, Overcharge, Fan, Pellet, Slug, Flak, Rivet, Sawblade, Volley,
                          // Rocket, Sticky, Mortar, Fist, Slam, Enemy{kind, attack}, World, Dev
    point: Vec3, dir: Vec3,   // where, and the direction the damage travels
    part: HitPart,        // Body | Head | Weak(i)
    knockback: Vec3,      // velocity change
    flags: DamageFlags,   // HITSCAN PROJECTILE EXPLOSIVE MELEE PARRIED RICOCHET SLAM HEADSHOT WEAK_POINT
                          // SELF PIERCED RETURNED ENVIRONMENT ARMOURED DIRECT
    ricochets: u8,        // prism bounces before this hit
    projectile: Option<NetId>,
}
```

**Players**, in order: missing/dead → `Blocked::NoTarget` (shots pass through corpses).
Self: explosive → `× self_damage_fraction` (0.35) and flag `SELF`; non-explosive self damage
→ blocked. Another player while `!friendly_fire` → `Blocked::FriendlyFire`. Head hit
(non-explosive) → `× player_head_multiplier` (2), `HEADSHOT`. Then `god` → `God`, spawn
protection → `SpawnProtection`, dash i-frames (`MoveState::is_invulnerable`) →
`Invulnerable` (kill volumes ignore the last two). **Knockback** is applied when the hit
counts *or* is explosive (rocket jumps through i-frames, teammates' explosions in co-op).
Counted hits: `damage += amount`, `hard += amount × hard_damage_fraction` (capped at
`damage`), `hard_delay` reset, last attacker recorded; `damage ≥ max_health` → killed. In PvP
a counted hit spawns a blood burst the victim can't drain.

**Enemies**: enemy-sourced damage → `Blocked::SameTeam`. Weak point `Weak(i)` (non-explosive)
→ `× weak_points[i].multiplier`, flags `WEAK_POINT` (+ `HEADSHOT` for `kind: Head`). Armour:
damage arriving from the front arc (`-dir` within `front_arc/2` of the facing) `×
front_multiplier` unless the weak point is in `bypass` (explosions too if `explosions`),
flag `ARMOURED`. Knockback `× knockback_scale` into the enemy's velocity (airborne if it has
an upward component). Aggro bookkeeping. Blood burst (`heal_fraction` of damage dealt,
`melee_heal_fraction` for punches/parry counters). `health ≤ 0` → killed.

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
    overkill: f32,                // gib size
    tick: u32,                    // MULTIKILL timing
}
```

Style mapping (for the style agent): HEADSHOT = `flags.HEADSHOT`; AIRKILL =
`killer_airborne`; AIRSHOT = `victim_airborne`; MULTIKILL = kills by the same killer within
0.6 s of `tick`; PARRY = `SimEvent::Parried`; RETURN TO SENDER = `parried_projectile`;
RICOCHET = `ricochets`; EXPLODED = `flags.EXPLOSIVE`; SLAM KILL = `attack == Slam`;
FISTED = `attack == Fist`; CLOSE CALL = `killer_health < 30`; FRAG = `victim_kind == PLAYER`.
Variety/freshness can key off `attack` (weapon + mode). The server exposes every event via
`Server::drain_sim_events()`.

### Death and respawn (replaceable)

`SimWorld::on_player_died` (sets `dead`, `respawn_in = respawn_delay`, cancels pending hits
on the player) and `SimWorld::respawn_player` (spawn point, full health, fresh weapons, spawn
protection, `SimEvent::Respawned`) are the whole default rule, driven by `tick_vitals`. The
modes agent replaces these two for checkpoints, downed/revive and deathmatch spawns. While
dead, the body is simulated with `neutral_input` (falls/slides naturally) and fires nothing.

## Weapons

| weapon / mode | implementation |
|---|---|
| **Lancet** primary | hitscan from the eye along the aim, 22, every 0.30 s, ricochets |
| Overcharge (alt A) | charge while alt held (0.8 s max); on release a beam `25 + 50 × charge`, pierces 5 targets (`PIERCED` on all but the first), ricochets; 0.5 s cooldown |
| Fan (alt B) | 3 hitscans at −6°, 0°, +6° (yaw), 15 each, ricochet; 0.9 s |
| **Thresher** primary | 10 pellets in a 7° cone (deterministic spread: splitmix of tick/player/seq), 9 each, knockback 2 m/s each along the ray, 120 m range, no ricochet; 0.75 s |
| Slug (alt A) | hitscan 70, knockback 14 m/s, **recoil** −6 m/s along the aim on the shooter (server impulse → reconciled), ricochets; 1.0 s |
| Flak (alt B) | grenade 28 m/s, gravity 25, explodes on impact or after the 1.2 s fuse (r 5, 55, kb 18); parryable, **also by its owner** (2× speed, 2× damage); 1.0 s |
| **Riveter** primary | rivet projectile 90 m/s, 8, 12/s, 1.5° spread |
| Sawblade (alt A) | disc 40 m/s, r 0.35, pierces: 10 per target per 0.1 s of contact, bounces off the level 3× (reflect), 3 s life; 1.0 s |
| Volley (alt B) | 8 rivets on a 10° ring around the aim, 12 each; 1.2 s |
| **Cinderbox** primary | rocket 45 m/s, explodes on contact: r 4.5, 70 → 30 % at the edge, kb 22; 0.9 s |
| Sticky (alt A) | **press alt**: no stickies out → throw (30 m/s, gravity 20); stickies out and the last throw < 0.4 s ago and < 3 out → throw another; otherwise → **detonate all** (r 4, 60, kb 18 each). Sticks to the level or to whatever it hits (follows enemies/players; falls off when they die); 20 s self-destruct |
| Mortar (alt B) | lob: aim rotated 35° up, 22 m/s, gravity 25, r 7, 90, kb 24; 2.0 s |
| **Fist** (`PUNCH`) | lag-compensated melee: nearest hittable target along the aim within 2.6 m, or whose nearest surface is within 2.6 m and inside the 50° cone, with line of sight; 25, `MELEE`, knockback 10 m/s (flattened aim + up); 0.4 s; opens the parry window |
| **Prism** (`THROW`) | thrown from eye + 0.5 m: aim × 18 + up 6 (+ half the thrower's velocity), gravity 20, bounces (restitution 0.35), rests on the floor, 4 s life; 4 charges, 1 per 2.5 s |

## Hitscan and lag compensation

1. `shot_view`: hitboxes rewound through `HitboxHistory` to `input.view_time_ticks()` (the
   tick the client was rendering, `view_tick + view_frac/256`), clamped by the history to
   `[newest − net.lag_comp_max_ms, newest]`; without a history (or before it has frames)
   current hitboxes. Prisms are rewound to the same (clamped) time through their trails.
2. Ray vs level (`MASK_SHOTS`: world + doors) gives the far limit; ray vs every rewound
   hitbox the shooter `can_hit` (alive and hostile: not itself, not co-op teammates, not
   corpses) — nearest first, one hit per target, up to `1 + pierce` targets.
3. The first prism within `ricochet_radius` of the ray before that limit (if the shot can
   ricochet) cuts the ray: targets before it are hit, it shatters, and the shot re-fires
   from it (see Ricochet).
4. Each hit → `apply_damage` with `HITSCAN` (+ `RICOCHET`, `PIERCED`), part from the shape,
   knockback along the ray.

**Rewind budget.** The server has to rewind ≈ one-way latency × 2 + interpolation delay +
input buffer: ~205 ms at 100 ms RTT, ~305 ms at 200 ms RTT (measured). The networking
default `net.lag_comp_max_ms = 350` therefore fully compensates players up to ~245 ms RTT;
raise it (e.g. 400) to support 200 ms+ players at the cost of "shot behind cover" moments for
their victims.

## Projectiles

Swept spheres, simulated on the server only. Per tick: exact ballistic step
(`p += v·dt − ½g·dt²ŷ`, `v −= g·dt·ŷ`), sphere cast against the level (`MASK_SHOTS`) and a
swept-sphere test against the **current** hitboxes of everything the owner `can_hit` (owners
and co-op teammates are never hit; enemy projectiles never hit enemies); the earliest contact
wins.

| kind (net kind) | gravity | radius | life | on target | on level | parryable |
|---|---|---|---|---|---|---|
| Rivet (64) | 0 | 0.08 | 2 s | 8 / 12 damage | vanish (`ProjectileImpact`) | no |
| Sawblade (65) | 0 | 0.35 | 3 s | pierce, 10 per 0.1 s contact | bounce ×3 (`ProjectileBounced`) | no |
| Rocket (66) | 0 | 0.2 | 5 s (explodes) | explode | explode | yes |
| Flak (67) | 25 | 0.2 | 1.2 s fuse (explodes) | explode | explode | yes, owner too |
| Sticky (68) | 20 | 0.15 | 20 s (explodes) | stick to it | stick | no |
| Mortar (69) | 25 | 0.25 | 10 s | explode | explode | yes |
| SpitterOrb (70) | `enemy_projectiles.orb_*` (0) | 0.35 | 6 s | direct damage (attack's) | vanish | yes |
| GnatBolt (71) | `bolt_*` (0) | 0.15 | 3 s | direct damage | vanish | yes |

Enemy projectiles: `SimWorld::spawn_enemy_projectile(t, kind, enemy, attack, pos, vel,
damage)` (used by `AttackKind::Projectile` attacks; speed/damage/count/spread/lead/muzzle come
from the enemy definition).

**Parry-delayed impacts.** A parryable projectile touching a *player* freezes there
(`PENDING`) for `parry_delay_ticks(victim)` and only then applies (direct damage or
explosion) — unless a parry deflects it first (next section).

### Explosions (`SimWorld::explode`)

For every living player and enemy hitbox: distance from the centre to the nearest shape
surface `d ≤ radius` (or the direct-hit target); line of sight from the centre to the body
centre or head (blocked → unaffected, unless touching); `frac = 1 − 0.7 · d/radius`; damage
`× frac`; knockback `normalize(dir_from_centre + 0.3ŷ) × knockback × frac` (full for the
owner: rocket jumps; teammates in co-op get knockback only). Flags `EXPLOSIVE` (+ `DIRECT`,
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
* Eligible: parryable kinds not stuck; others' projectiles (enemies; opponents in PvP); own and
  co-op teammates' only for `owner_can_parry` kinds (flak). A projectile you just deflected
  can't be re-parried by you.
* **Deflection**: owner → the puncher (`original_owner` kept), `parried`, no gravity, position
  eye + 1 m along the new direction = aim, auto-aimed at the nearest hostile (head/weak point
  if visible) within `parry_auto_aim_degrees` (15°) / 80 m; speed `× 1.5` (flak `× 2`),
  damage `× 2` (explosion too), lifetime reset, pending impact cancelled.
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
  `Enemy::parry_window = { start, end = strike tick, seq }` (entity flag `PARRYABLE` while
  `start ≤ tick ≤ end`). A punch whose view time — or the current tick — falls in it, with the
  enemy (rewound through its trail) within `melee_parry_range` + its radius, stuns it for the
  attack's `parry_stun`, deals `parry_damage` (`Fist`, `MELEE | PARRIED`) and cancels the
  strike's pending damage. Strikes of parryable attacks queue their damage as `PendingHit`s
  keyed `ParryKey::Melee(enemy, seq)` with the same delay, so lagging players can parry too.

## Ricochet

From a shattered prism the shot re-fires at: the nearest other live prism within
`chain_range` (30 m) in line of sight (chain), else the nearest hittable target within
`target_range` (40 m) — aiming at its first visible weak point / head, else its body. No
target → continues in the original direction. Each bounce multiplies damage by 1.5 (`22 →
33 → 49.5`, ×2 on a head). Bounce legs use current hitboxes/prisms (they auto-aim at current
positions). Hitscan modes that ricochet: Lancet primary, Overcharge, Fan, Slug (not pellets).
In PvP the targets are opposing players. Events: `SimEvent::Ricochet { shooter, attack,
points }` (prism positions…, final end) and `PrismShattered`.

## Healing, hard damage, i-frames

* Blood bursts: see `blood.rs` docs. A damage instance dealt *by a player* to an enemy (or
  another player) spawns/merges a burst at the hit point; each tick the closest living player
  within the radius who is below their effective max drains it (the PvP victim can't drain
  their own). Enemy death → big burst (25 HP, r 4.5, 1 s).
* Hard damage: 35 % of each hit; caps healing at `max − hard`; recovers at 25/s after 1.5 s
  without damage. Parries clear half.
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
| `name`, `net_kind` | display name; replicated kind (16..64, unique: Husk 16, Spitter 17, Gnat 18, Bastion 19) |
| `model` | client model name |
| `health` | base health (co-op: `× (1 + 0.5 (players − 1))`, hook `SimWorld::enemy_health_scale`) |
| `speed`, `acceleration`, `turn_rate` | m/s, m/s², deg/s |
| `radius`, `height`, `body_top` | movement capsule; top of the body hitbox (`None` = height) so a head can sit above it |
| `eye_height`, `step_height` | sight origin; climbable ledge |
| `locomotion` | `Ground` or `Flying(min_height, max_height)` (hovers, no gravity) |
| `knockback_scale` | light enemies > 1 |
| `weak_points` | `[(name, kind: Head/Core, offset, radius, multiplier)]` → `HitPart::Weak(i)` |
| `armour` | `Some((front_multiplier, front_arc, bypass: ["head"], explosions))` |
| `attacks` | `[AttackDef]`, in priority order (below) |
| `ai` | `sight_range, reaction_time, lose_target_time, preferred_range: Option<(min,max)>, strafe, target_stickiness, aggro_switch_fraction, retarget_interval` |
| `drops` | `(blood_pool, blood_radius)` of the death burst |
| `colors` | `body, accent, glow, telegraph, parry` (sRGB) |
| `death_time` | corpse duration (Dying state) |

`AttackDef`: `name`, `kind` (`Melee` / `Projectile` / `Slam`), `range`, `min_range`, `arc`
(melee cone), `damage`, `knockback`, `windup`, `strike`, `recover`, `cooldown`,
`windup_move_scale`, `track_target`, `parry_window: Option<f32>` (+ `parry_stun`,
`parry_damage`), `shockwave_radius` (Slam), `projectile: Option<(kind, speed, count,
interval, spread, muzzle, lead)>`.

The server loads `assets/enemies/*.ron` over the compiled-in built-ins
(`EnemyRegistry::load_default`; `ServerConfig::enemies_dir` overrides); invalid files are
logged and skipped. Adding the Spitter/Gnat/Bastion is a RON file each: Spitter =
`Projectile` attack with `kind: SpitterOrb` + `preferred_range: Some((12, 25))`; Gnat =
`Flying(4, 8)` + `Projectile` burst of 3 `GnatBolt` + `strafe`; Bastion = `armour` + `Slam`
with `parry_window: Some(0.35)`. Navigation plugs in at `chase_velocity` (`ai.rs`).

### AI (`ai.rs`)

`Idle` → (a target is visible) `Alert` for `reaction_time` → `Chase` (straight at the target;
ranged enemies hold their band and may strafe; stop at 60 % of melee range) → first usable
attack (cooldown ready, in range, line of sight) → `Windup` (telegraph; turns towards the
target if `track_target`, creeps at `windup_move_scale`) → `Strike` (melee arc / slam radius
damage, or the projectile burst; ≥ `strike` s) → `Recover` → `Chase`. `Stunned` (parry) →
`Chase`. `Dying` (corpse, no hitboxes) → removed after `death_time`.

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
| `PLAYER` 0 | player id | feet / velocity | view | `LOW_PROFILE 1, INVULNERABLE 2, DEAD 4, PARRY 8, CHARGING 16, PROTECTED 32, DOWNED 64 (reserved)` | weapon (bits 0–3) \| mode (4–5) \| Overcharge charge 0–255 (8–15) | health (ceil) | hard (bits 0–7) \| prism charges (8–11) \| live stickies (12–15) |
| enemies 16..64 | target player or `OWNER_NONE` | feet / velocity (¼ m/s steps) | facing (~0.7° steps) | `ALERT 1, PARRYABLE 2, STUNNED 4, DYING 8, AIRBORNE 16, ARMOURED 32` | `AiState` (Idle 0, Alert 1, Chase 2, Windup 3, Strike 4, Recover 5, Stunned 6, Dying 7) | health (ceil) | progress through the timed state 0–255 (0–7) \| attack index (8–15) \| max health (16–31) |
| projectiles 64..72 | shooter id, `OWNER_ENEMY` (0xFFFE), `OWNER_NONE` | **at launch tick** | – | `PARRIED 1, STUCK 2, PENDING 4, ENEMY 8` | bounces left | – | launch tick |
| `BLOOD_BURST` 128 | – | burst centre | – | `BIG 1` | radius (dm) | pool left (ceil) | spawn tick |
| `PRISM` 160 | thrower | **at launch tick** | – | `RESTING 1` | – | – | launch tick |

**Trajectory replication** (projectiles, prisms): `pos(t) = p0 + v0·Δ − ½·g·Δ²·ŷ`,
`Δ = (t − aux) / tick_rate`, g = the kind's gravity (`ProjectileKind::gravity`,
`prism.gravity`), 0 when `PARRIED`; `STUCK`/`PENDING`/`RESTING` → `position` as is.
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

### Cosmetic events (`SnapshotMessage::fx: Vec<FxEvent>`, unreliable)

Batched into each client's next snapshot; important ones (`Explosion`, `Ricochet`,
`SlamShockwave`, `EnemyShockwave`, `EnemyAttack`) are repeated in the following snapshot as
well, so clients **must de-duplicate by `seq`** (global, +1 per event). `tick` = when it
happened (play it when the render clock reaches it). At most 256 per snapshot (important first,
then newest). Positions are `QPos` (1/64 m varints).

| `Fx` | recipients |
|---|---|
| `Shot { shooter, attack, origin, ends }` | everyone but the shooter |
| `Ricochet { shooter (owner code), attack, points }` | everyone |
| `Explosion { position, radius_dm, attack }` | everyone |
| `Punch { player, hit }` | everyone but the puncher |
| `Heal { player, amount_tenths, from }` | everyone |
| `SlamShockwave { player, position, power, radius_dm }` | everyone |
| `PrismThrown { player }` | everyone but the thrower |
| `PrismExpired { id }` | everyone |
| `StickiesDetonated { player }` | everyone but the owner |
| `ProjectileBounce { kind, position }` | everyone |
| `ProjectileImpact { kind, position, normal }` | everyone |
| `EnemyAttack { enemy, attack, strike, parryable }` | everyone |
| `EnemyFired { enemy, attack, muzzle }` | everyone |
| `EnemyShockwave { enemy, position, radius_dm }` | everyone |
| `EnemyAlert { enemy }` | everyone |

## Client presentation contract (phase 2)

Prediction duties first:

* **Weapons**: run `step_weapons` on each predicted input; play muzzle flash, weapon sound
  (`LancetFire`, `ThresherFire`, `ThresherSlug`, `FlakLaunch`, `RiveterFire`, `SawLaunch`,
  `VolleyFire`, `CinderFire`, `StickyArm`, `MortarFire`, `LancetFan`, `LancetBeam`),
  viewmodel recoil and a local tracer (raycast your own collision world) immediately for
  every `FireIntent`; `LancetCharge` loop + charge glow while `charge > 0`; `Punch` anim +
  `Sfx::Punch` on `FireIntent::Punch`; `PrismThrow` on `ThrowPrism`; `StickyDetonate` click on
  `DetonateStickies`; `WeaponSwitch` / `ModeSwitch` on selection changes. Reconcile
  `WeaponState` against `OwnState::combat.weapons` like movement.
* **Death**: while `OwnState::combat.dead`, predict movement with `sim::neutral_input` and
  show the death view (killer from `last_attacker` / the `Killed` event); `Respawned` /
  `NetEvent::Spawned` ends it.
* **Projectiles/prisms**: draw at `replicated_position(e, render_tick, …)` (consistent with
  lag compensation and parry validation), using the newest received frame whose `aux ≤
  render_tick`. Optionally spawn a predicted local copy of your own projectiles on the
  intent and swap to the entity when it arrives.

HUD (from `OwnState::combat` + tuning): health bar `max − damage` with the hard-damage
segment `[max − hard, max]` in another colour; respawn timer; spawn protection; current
weapon + alt mode; per-weapon cooldown pips; Overcharge charge; prism charges + recharge
progress; live stickies (`n/3`); parry window flash.

| thing | render / play |
|---|---|
| **PLAYER** entity | body/stance from `LOW_PROFILE`; weapon model from `state & 0xF` and alt mode; `CHARGING` → Lancet glow (charge in bits 8–15); `PARRY` → punch pose; `INVULNERABLE` → dash afterimage; `PROTECTED` → spawn shimmer; `DEAD` → corpse/ragdoll, no name tag; name tag health bar from `health` (+ hard from `aux`) for teammates |
| **enemy** entity (16..64) | model/colours from the `EnemyDef` with that `net_kind` (same RON files); animation by `state` (Idle, Alert: alert bark `HuskAlert`, Chase: walk/run by speed, Windup: telegraph pose + `colors.telegraph` glint driven by `aux & 0xFF` progress + `HuskWindup`, Strike: swipe + `HuskSwipe`, Recover, Stunned: dazed, Dying: death anim/gibs); `PARRYABLE` → flash `colors.parry` (yellow); `ARMOURED` → armour plates; health bar from `health` / `aux >> 16`; hurt flash + `EnemyHurt` when `health` drops; `AIRBORNE` → flail |
| **projectiles** | rivet: thin bolt + trail; sawblade: spinning disc (`SawBounce` on `ProjectileBounce`); rocket: smoke trail + light; flak: tumbling grenade + fuse beep; sticky: blinking charge (`STUCK` → attached, beeping); mortar: arcing shell; spitter orb / gnat bolt: glowing (parryable) with `ENEMY` tint; `PARRIED` → white-hot tint + streak; `PENDING` → hide (it is "in" the victim) |
| **BLOOD_BURST** | red mist cloud of `state/10` m radius, density from `health` (pool); `BIG` → gib shower + big mist; fades over its life |
| **PRISM** | spinning mirrored shard with glints; `RESTING` → lying/hovering still; disappears on removal (shatter shown via `Ricochet`/`PrismExpired`) |
| `Damaged` | red screen flash scaled by amount, direction indicator towards `-direction`, `Hurt` sound, camera kick; `blocked: Invulnerable` → dodge flash (no damage); hard-damage segment grows; `killed` → death |
| `HitConfirm` | hit marker (`Hitmarker`), `Headshot` sound + red marker for `HEADSHOT`/`WEAK_POINT`, dull marker for `ARMOURED`, damage number at `point`, `KillConfirm` when `killed` |
| `Killed` | kill feed (killer, attack name, flags icons); victim gibs at `position` scaled by `overkill`; `EnemyDeath`/`Death`; style feed (style system) |
| `Parried` | for the parrier: client hitstop 0.12 s, white flash, `Parry` sound, heal flash (`healed`); for others: `Parry` sound at `position` |
| `Respawned` | `Respawn` sound/effect at `position` |
| `Fx::Shot` | remote muzzle flash + 3D weapon sound at the shooter (interpolated position), hitscan tracers from the shooter's muzzle to each end, impact sparks/decals at the ends |
| `Fx::Ricochet` | bright tracer through `points`, prism shatter at each prism point, `Ricochet` sound (pitch up per bounce) |
| `Fx::Explosion` | fireball sized by `radius_dm`, light flash, screen shake by distance, scorch decal, `Explosion` |
| `Fx::Punch` | remote arm swing, `Punch`/`PunchHit` |
| `Fx::Heal` | red streak from `from` to the player; for you: green-ish heal flash + `Heal` |
| `Fx::SlamShockwave` | ground ring of `radius_dm`, dust, `SlamImpact` |
| `Fx::PrismThrown` / `PrismExpired` | remote throw anim + `PrismThrow` / small fizzle |
| `Fx::StickiesDetonated` | remote detonator click |
| `Fx::ProjectileBounce` / `ProjectileImpact` | sparks (+ `SawBounce`) / impact puff + `ProjectileImpact` |
| `Fx::EnemyAttack` | `strike == false`: telegraph sound (`HuskWindup`, `SpitterWindup`, `BastionTelegraph`) + glint; `parryable` → yellow flash; `strike == true`: `HuskSwipe` / `BastionSlam` |
| `Fx::EnemyFired` / `EnemyShockwave` / `EnemyAlert` | spit/muzzle flash (`SpitterFire`, `GnatFire`) / slam ring / `HuskAlert` bark |

## Dev commands (chat, `ServerConfig::dev_commands`, default on)

`/spawn <enemy> [count]` (8 m in front, on the ground, facing you), `/enemies`, `/killall`,
`/god`, `/notarget`, `/give` (full health, no hard damage, prisms, cooldowns), `/kill`,
`/hurt <hp>`, `/ff [on|off]`, `/help`. The reply goes to the sender only; commands are not
broadcast. Programmatic: `ServerCommand::{SpawnEnemy, KillAllEnemies, SetFriendlyFire}`,
`Server::spawn_enemies`, `Server::kill_all_enemies`, `Server::drain_sim_events`.

## Measurements (tests, virtual clock)

| | |
|---|---|
| lag-compensated Lancet shot at a target strafing 16 m/s, ~100 ms RTT, default tuning | target was 3.3 m from where the shooter saw it; server rewound 205 ms; **hit** (22). Same shot with `lag_comp_max_ms = 0`: miss |
| same at ~200 ms RTT (`lag_comp_max_ms = 400`) | 4.75 m away, rewound 305 ms, **hit** |
| parry of a 22 m/s orb, punching when the *displayed* orb is 2 m away | 0 ms: parried (2 ticks after the server impact, thanks to interpolation delay); ~100 ms RTT: parried 8 ticks late; ~200 ms RTT: parried 14 ticks late; punching 300 ms after the displayed touch: fails, 20 damage |
| rocket jump (rocket at the feet + jump), 50 ms one-way | apex 14.6 m, self damage 23.2, 1 correction (0.35 m at the ack), no snaps, prediction == server afterwards |
| crowded frame: 8 players, 50 husks, 150 projectiles | full 3.5 KB; delta 74 B with idle husks and flying projectiles |
| bandwidth: 4 clients + 4 bots, 50 husks chasing, 150–240 projectiles, riveters firing | **28 KB/s per client** server→client (snapshot 844 B: 615 B entities ≈ 10 B per moving enemy, 3.4 FX); server tick 0.8 ms (dev build) |
| 2 players (`net_loopback`) | snapshot 197 B (own state 155 B of it: movement 123 + combat 32); 0 prediction corrections with one player shooting |

Run: `cargo test -p shared --test combat_weapons --test combat_parry --test combat_ricochet
--test combat_enemy --test combat_replication` and `cargo test -p server --test combat_net
-- --nocapture`.

## Known limitations / next steps

* No interest management: every client gets every enemy (~10 B per moving enemy per
  snapshot). Distance-based update rates would cut the crowded-scene 28 KB/s a lot.
* Enemies chase directly (no navigation yet); players and enemies don't collide.
* Weapon prediction, projectile drawing and FX de-duplication are client work (phase 2);
  today the client ignores `OwnState::combat` and `SnapshotMessage::fx`.
* Default `net.lag_comp_max_ms` (350) compensates up to ~245 ms RTT (see above).
* Pending (parry-delayed) hits favour the victim by up to `lag_comp_max_ms + 80 ms`.
* Rivets that leave the level vanish silently after 2 s; sawblades only bounce off the level.
* Explosion LOS tests the body centre and head only (a foot sticking out of cover is safe).
