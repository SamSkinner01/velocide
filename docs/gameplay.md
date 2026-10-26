# VELOCIDE — gameplay rules

Concrete rules and starting numbers for combat, enemies, style and game modes. Every number
here belongs in `assets/tuning.ron` (`CombatTuning` and friends) or in the enemy RON files so it
can be hot-reloaded; the values below are the defaults. All gameplay is resolved on the server.

## Player
- **Health** 100: the Shrike's *wick*, the flame in its chest lantern (HUD label `WICK`;
  `docs/premise.md`). **Hard damage** (*smother*): 35 % of every hit also smothers the wick,
  which lowers the player's *effective max health* (`max - hard`). It starts rekindling 1.5 s
  after the last hit at 25 HP/s. Healing can never exceed the effective max. The HUD shows the
  hard damage segment in a different colour (and hatched) at the end of the health bar.
- **Close-range healing (ember bursts, *kindling*)**: every damage instance dealt to an enemy
  (or, in PvP, to another player) sheds an *ember burst* at the hit point: heal pool = 40 % of
  the damage (melee/parry: 100 %), radius 3.5 m, lifetime 0.6 s. Each tick, the closest living
  player inside the radius draws the embers in (whoever is closest gets them, so players compete
  in PvP; a Shrike can't draw back the embers its own wick shed). An enemy death releases its
  heat as a big burst (pool 25, radius 4.5 m). Bursts are replicated entities so clients draw
  the embers, the stream of sparks into the healed Shrike's lantern and the heal flash.
- **Dash i-frames**: no damage while `MoveState::is_invulnerable()`.
- **Self damage**: own explosions deal 35 % damage to self (and full knockback — blast jumps).
  No friendly fire in co-op (knockback from teammates' explosions still applies).
- **Falling** never hurts. Kill volumes kill instantly.

## Weapons
Weapon switching is instant (no draw time), each weapon has its own cooldowns so switching
mid-combo is rewarded. Each weapon: primary fire + two alt-fire modes; pressing the weapon's
slot key again cycles its alt mode. Damage multipliers: head/weak point ×2 (per enemy def).
The full design (silhouettes, behaviour, net kinds) is `docs/roster.md`.

| Weapon | Fire | Numbers |
|---|---|---|
| 1 **Quill** (precision spike-caster, hitscan) | primary *Quill* | 24 dmg, 0.28 s interval, perfectly accurate, ricochets off thorns |
| | alt A *Brand* | 10 dmg marking shot, 2.5 s cooldown, ricochets; the target is **branded** for 5 s: ×1.35 damage from every source, a sigil over it, thorn ricochets and Arclash chains go for it first. One live brand per player (a new one moves it) |
| | alt B *Splinter* | 20 dmg, 1.1 s, ricochets; where it lands it bursts into 6 splinters (9 dmg, 18 m, 40° cone): on through the target it hit (not hitting it again), or reflected off a wall (corner shots) |
| 2 **Bellows** (twin-barrel blast cannon) | primary *Blast* | 12 pellets × 8 dmg, 8° cone, 60 m, 0.8 s interval, 2 m/s knockback per pellet |
| | alt A *Gust* | compressed air, 7 m / 60° cone (line of sight): 6 dmg + 18 m/s knockback along the aim (+4 up) to every hostile; every hostile projectile in it is **deflected** (yours now, along your aim, ×1.25 speed, ×1.5 damage; no heal, *DEFLECT* style); you recoil 10 m/s (gust-jump); 1.6 s cooldown |
| | alt B *Both Barrels* | 24 pellets × 8 dmg in a 5° cone, self recoil 9 m/s (blast-jump), 1.4 s cooldown, also puts the primary on its interval |
| 3 **Arclash** (arc caster) | primary *Arc* | while held a zap every 0.1 s, 30 m, 6 dmg; a hit **chains** to the nearest other hostile within 7 m of it (line of sight, branded first), 2 jumps at most, ×0.75 per jump; no thorn ricochets |
| | alt A *Wisp* | slow ball of lightning (12 m/s, no gravity, 2.5 s): every 0.2 s it zaps the nearest hostile within 6 m (8 dmg); drifts through targets; pops on the level or at the end (r 4 m, 25 dmg); 2.8 s cooldown |
| | alt B *Discharge* | hold to charge (1.0 s max), release: nova around you, r 3.5–8 m, 25–80 dmg, 12–24 m/s outward knockback by charge; never hurts you; 2.0 s cooldown; switching cancels the charge |
| 4 **Tumbler** (bouncing-bomb launcher) | primary *Tumble* | bomb 30 m/s, gravity 18, bounces (×0.6) and explodes on its 3rd contact with the level, on any hostile or after 1.6 s: r 4.5 m, 65 dmg (falloff to 30 %), knockback 20; 0.85 s interval |
| | alt A *Breach* | point-blank charge: explodes at the first surface / hostile within 3 m along the aim (pulled back 0.2 m) or 3 m out: r 4 m, 70 dmg, knockback 24, normal self damage (the blast-jump / panic button); 1.2 s cooldown |
| | alt B *Gravewell* | canister lobbed at 20 m/s, +25°, gravity 20; sticks to the level; for 1.5 s pulls every enemy within 9 m in (up to 9 m/s × their knockback scale; hostile players in PvP at half), then implodes: r 5 m, 80 dmg; 3.0 s cooldown |
| **Fist** (`PUNCH`) | | 25 dmg in a 2.6 m / 50° cone, enemy knockback 10 m/s, 0.4 s cooldown |
| **Thorn** (`THROW`) | | 3 charges, 1 per 3 s; thrown at 30 m/s along the aim (gravity 12, half your velocity); plants itself in the first surface it hits and lives 8 s there; hitting an enemy / player deals 8 and it drops to the floor as a live thorn |

### Hit assist
Shots should feel generous; being shot should feel fair. Every **player-owned** attack (each
weapon mode, splinters, arc chains, ricochet legs, projectile sweeps, explosion distances, the
fist, thorn hits) tests against hit shapes inflated by `combat.hit_assist`: enemy bodies
+0.2 m, enemy weak points +0.1 m, and in PvP player bodies +0.12 m, heads +0.06 m. A ray that
hits the plain shapes keeps that result (the padding never turns a headshot into a body hit).
**Enemy attacks** use the plain shapes. The padding is added at test time; the
lag-compensation history keeps the plain shapes.

### Parry
- Pressing `PUNCH` opens a **parry window** of 0.2 s. Any parryable enemy projectile that is
  within 3 m of the player (in front, 70° cone) during the window is **deflected**: it now
  belongs to the player, flies along the player's aim (auto-aimed at the nearest enemy within a
  15° cone), at 1.5× speed and 2× damage. Kills with it score *RETURN TO SENDER*. Punching
  the same projectile back and forth never takes it past 250 m/s or 400 damage
  (`parry_max_speed`, `parry_max_damage`).
- **Network tolerance**: the server validates against the lag-compensated (rewound) projectile
  positions at the client's view time and accepts a parry up to `parry_tolerance_ms` (80 ms)
  after the server would otherwise have applied the hit. To make that possible, damage from a
  parryable projectile is applied with a delay equal to that tolerance **plus the victim's view
  lag** (RTT + interpolation delay, clamped to `lag_comp_max_ms`): if a valid parry for it
  arrives in that window, the damage is cancelled. Without the view-lag term, parries made on
  what the player saw fail at ~100 ms RTT (`combat.parry_compensate_view_lag = false` restores
  the plain 80 ms window).
- **Lag compensation** rewinds up to `net.lag_comp_max_ms` = 350 ms (rewind needed ≈ RTT +
  interpolation delay, so this fully compensates players up to ~245 ms RTT).
- **Telegraphed melee** (the Gaoler's shield bash): punching inside the attack's parry window
  (last 0.35 s of its wind-up, the door's rim strobes white-hot) within 4.5 m of its centre
  stuns it for 2.5 s and deals 150 damage through its armour.
- **The Swat**: punching a diving Harrier (its whole dive is the window, within 3 m + its
  radius) is a parry too: heal + PARRY, it is stunned 2 s and knocked out of the air, 50 damage
  (2× the fist) on top of the punch.
- **Reward**: the wick *flares*: heal to effective max (+ clears 50 % of hard damage), +PARRY
  style, client hitstop 0.12 s and a white screen flash.
- Players' own projectiles (bombs, wisps, gravewells) are **not** parryable — no punchable
  grenades. In PvP the counter is the Bellows **Gust**, which turns any hostile projectile
  around (kills with it are RETURN TO SENDER too, scored under the Gust).

### Ricochet (Thorn)
- A Quill shot (primary, Brand or Splinter — the main shot) that passes within 0.6 m of a live
  thorn (flying, planted or lying) *ricochets*: the thorn shatters and the shot re-fires from it
  at: another live thorn within 30 m (chain), else a **branded** hostile within 40 m, else the
  nearest hostile within 40 m — always in line of sight, weak point preferred. Each bounce
  multiplies damage by 1.5. A Brand that ricochets brands what it finally hits; a Splinter
  bursts there. Bellows pellets and Arclash zaps pass thorns by. Resolved entirely on the
  server; clients get the tracer path to draw.

### Explosions
Sphere with linear falloff (100 % at centre → 30 % at radius) and line-of-sight check from the
centre. Knockback impulse away from the centre (upward bias +0.3) up to the weapon's knockback,
applied directly to `MoveState::velocity` (blast-jumping). Enemies get knocked back too (light
enemies more). The Gravewell's implosion uses the same rules after its pull.

## Enemies
The Moulds (`docs/roster.md`): things poured and set in the Foundry, grey wax-flesh, slag, brass
and chain, glowing from inside where the heat collects (weak points, telegraphs). They don't
bleed: a hit cracks them and sprays molten wax and sparks; a kill releases their heat as one big
ember burst while the body slumps and melts.

Data-driven: `assets/enemies/<name>.ron` define health, speed, collision size, weak points
(offset, radius, multiplier), armour, attacks (kind, range, damage, wind-up/telegraph time,
recovery, projectile params, parryable, leaps, dives, lobs, aim lasers), sight range, drops
(the death burst's pool) and colours. AI runs on the server only: **idle → alert → chase →
attack (wind-up with clear visual/audio telegraph → strike → recover) → chase**. Targets: the
nearest visible player, with stickiness; switches to a player who damages it heavily. All
players are valid targets. Details and measurements: `docs/enemies.md`.

| Enemy | Role | Defaults |
|---|---|---|
| **Gnawer** | fodder melee, pack crawler | 45 HP, 10 m/s, r 0.6 × 2.0 m knuckle-walker; bite 14 dmg at 2.6 m (0.4 s wind-up: the jaw unhinges, the throat glows orange); pounce on players faster than 4 m/s 3–9 m away (0.55 s crouch, leap 13 m/s + 6 m/s up, 18 dmg, 3 s cooldown); head (jaw) ×2 |
| **Censer** | ranged lobber | 80 HP, 5 m/s, r 0.6 × 2.5 m, keeps 14–30 m and side-steps; Volley (0.8 s whirl overhead): 3 ember bombs lobbed onto your predicted feet (±8°, ≤ 26 m/s, gravity 20), each bursts r 2.5, 18 dmg, parryable, 3 s cooldown; censer swing at rushers (0.5 s, 12 dmg, 160°, kb 12); brazier head ×2 |
| **Harrier** | flying dive-bomber | 40 HP, 15 m/s, 3 m wingspan, hovers 5–10 m up and circles 8–16 m out; Dive (0.7 s: rears up, beak and heart glow, shriek): a straight 3D line at 26 m/s for ≤ 0.6 s, 20 dmg, ends on the level, 2.2 s cooldown; punch it mid-dive to Swat it; heart ×2.5 |
| **Gaoler** | heavy, shielded | 520 HP, 4.5 m/s, r 1.3 × 3.7 m; front armour ×0.15 in 120° (explosions too; not the helm or the back lantern); Shield Bash 40 dmg (1.0 s, parryable in the last 0.35 s → stun 2.5 s + 150 dmg); Chain Sweep 25 dmg in 300° within ~7 m that only hits feet < 1.2 m above its own (jump it); Rush 8–20 m: 15 m/s for 0.8 s, 30 dmg, stops at walls; helm ×1.5, lantern ×2.5 |
| **Heron** | sniper | 60 HP, 7 m/s, r 0.5 × 3.4 m, keeps 25–45 m, sight 80 m, repositions after every shot; Needle: 1.3 s wind-up with a red aim laser every player sees, tracking until the last 0.25 s then locked (dodge then), 110 m/s, 30 dmg, parryable, 3.5 s cooldown; kick at 4 m (15 dmg, kb 16); lens ×2.5 |

- **Navigation**: a nav grid built at level load (1 m cells, walkable floor found with downward
  rays, clearance and headroom per agent class, step/drop limits) with A*. Flying enemies steer
  directly with feeler sweeps.
- **Encounters**: entering an encounter trigger closes its `lock_doors`, spawns waves (next wave
  when the previous is dead, after `delay`), and opens `unlock_doors` when cleared.
- **Co-op scaling**: enemy health × (1 + 0.5 × (players − 1)); each wave spawns 50 % more
  enemies per extra player (extra copies offset around the authored spawn points).

## Style
- Per-player style meter (server-side, replicated). Points fill the current rank's bar (100 per
  rank); the meter decays continuously (4/s at DULL rising to 30/s at VELOCIDE). Falling below
  zero drops a rank. Rank changes produce events (rank-up stinger).
- Ranks: DULL (grey), CRUDE (blue), FERAL (green), VICIOUS (yellow), MERCILESS (orange),
  GRISLY (red), SEARING (red-hot), VELOCIDE (white, flashing).
- Bonuses: KILL 30 · HEADSHOT +15 · AIRKILL (killer airborne) +40 · AIRSHOT (target airborne)
  +20 · MULTIKILL (n kills within 0.6 s) +40×(n−1) · PARRY 60 · DEFLECT (a Gust turned a
  hostile projectile around) 30 · RETURN TO SENDER (kill with a parried / deflected projectile)
  80 · RICOCHET 30 (+20 per extra thorn bounce) · EXPLODED 20 · SLAM KILL 40 ·
  FISTED (punch kill) 30 · CLOSE CALL (kill under 30 effective HP) ×1.5 multiplier ·
  FRAG (PvP player kill) 100.
- **Freshness**: each weapon mode (Quill, Brand, Splinter, Bellows, Gust, Both Barrels,
  Arclash, Wisp, Discharge, Tumbler, Breach, Gravewell) and action (Fist, Slam, Parry, Thorn)
  has a freshness value starting at 1.0; using it costs
  0.12, using something else restores the others by 0.04/use and slowly over time. Gains are
  multiplied by freshness (min 0.2). A stale weapon shows "STALE" next to the meter.
- **Style feed**: each bonus is pushed as an event and shown in a scrolling feed beside the meter.
- **Results screen** (campaign): time (vs par), kills (% of total), style (total points earned),
  damage taken → each graded D/C/B/A/S, overall rank; co-op scoreboard per player and a combined
  team rank (average of player grades).

## Game modes
- **Campaign** (SP / 2–4 co-op): checkpoints (trigger volumes) set the respawn point. SP: on
  death, instant restart from the last checkpoint (or `R` to restart the level). Co-op: a dead
  player is *downed* for 15 s; a teammate holding `USE` within 2.5 m for 1.5 s revives them at
  40 HP; otherwise they respawn at the last checkpoint. If everyone is down at once, all respawn
  at the checkpoint and the current encounter resets. Reaching the exit shows the results screen.
- **Deathmatch** (PvP, up to 8): frag limit 20, time limit 10 min (both set in the lobby),
  respawn after 2 s at the spawn point farthest from other players, 1.5 s spawn protection,
  scoreboard, end-of-match results.
