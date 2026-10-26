# Style, game modes and results

The per-player **style meter** (ranks, bonuses, freshness, the style feed), the **game-mode
rules** (campaign checkpoints, solo checkpoint restarts, co-op downed / revive / team wipes, the
level exit, deathmatch frags and limits, grind waves, the gun game ladder — i.e. *the* death /
respawn rule), the
**PvE difficulty** and co-op scaling, the **scoreboard** and the **results screens** with
grades. Everything is server-authoritative and runs inside `SimWorld::step`; numbers live in
`assets/tuning.ron` (`style`, `modes`, `difficulty`, `grind`, `gun_game`). Base rules: `docs/gameplay.md`
("Style", "Game modes"); match flow / lobby: `docs/lobby.md`; the expansion's contract:
`docs/expansion.md`.

| where | what |
|---|---|
| `shared::sim::style` | `PlayerStyle` (meter, rank, freshness, totals), `StyleSystem` (`SimWorld::style`), `StyleBonus`, `StyleKey`, `StyleEvent`, `StyleView`, scoring (`kill_bonuses`, `is_close_call`, `ricochet_points`, `decay_rate`, `SimWorld::score_style`) |
| `shared::sim::modes` | `ModeRules`, `ModeState` (`SimWorld::mode`), `ModePlayer`, `LifeState`, `MatchPhase`, `PlayerStats`, `ModeEvent`, `Scoreboard`, `MatchResults`, `Grade` / `Grades` / `grade_player`, the death rule, checkpoints, revives, exit, deathmatch, `end_match`, results |
| `shared::sim::difficulty` | `PveScaling` (difficulty × players: health, count, elite chance, `EnemyScale`, downed time, revive health), `SimWorld::pve_scaling`, `spawn_pve_enemy`, `ModeRng` |
| `shared::sim::grind` | `GrindState` (`ModeState::grind`), `GrindStatus`, `GrindEvent`, `GrindResult`, `plan_wave` / `wave_budget`, the grind step, spawn points |
| `shared::sim::gungame` | `GunGameEvent`, `GunGameResult`, `Tier` / `tier_of` / `tier_loadout` / `fist_tier`, the gun game step (promotions, punch demotions, the ladder win) |
| `shared::tuning` | `StyleTuning`, `ModeTuning`, `DifficultyTuning`, `GrindTuning`, `GunGameTuning` (sections `style`, `modes`, `difficulty`, `grind`, `gun_game`) |
| `server::modes` | routing (`GameEvent::Style` to the owner, `GameEvent::Mode` to everyone), match end → `Server::end_match` |
| `client::ingame::style_hud` | `StyleHud`: meter, rank animation, STALE + freshness pips, style feed, rank-up banner, rank sounds, music floor |
| `client::ingame::mode_hud` | `ModeHud`: client mode state (scoreboard, downed, revives, exit, wipe), mode HUD (incl. the grind block and the difficulty tag), `InGame` glue (events, score cells, results, the grind record) |
| `client::menus::results` | `ResultsScreen`: campaign solo tally, co-op table, deathmatch standings, grind (solo / co-op, best wave) |
| `client::settings` | `Settings::{sp_mode, difficulty, grind_best}` (single-player page, the local grind records) |
| `client::ingame::scoreboard` | score columns (`columns_for`) + per-cell colours |

## Where it runs

`SimWorld::step` gained a step **6. game mode + style** (`SimWorld::step_modes`) after the world
rules. It takes the tick's events so far and:

1. registers joined / forgets left players (a late joiner is moved to the checkpoint / the
   farthest deathmatch spawn); the match clock starts with the first player,
2. while `Playing`: stats from the events (damage taken, kills, deaths, frags, enemies
   spawned), style scoring (`score_style`) and decay, then campaign checkpoints, revives, team
   wipe, exit — or the deathmatch limits,
3. while `Ended`: everyone stays protected,
4. broadcasts the scoreboard when something on it changed (throttled),
5. appends its own events (`SimEvent::Style`, `SimEvent::Mode`, `Respawned`) after the tick's.

The death rule is called from the damage pipeline: `SimWorld::on_player_died` /
`respawn_player` (`combat.rs`) delegate to `mode_player_died` / `mode_respawn_player`.

**Match clock**: `ModeState::start_tick` is the tick the first player was registered (worlds are
rebuilt per match but `world.tick` keeps counting); durations and the deathmatch time limit are
`tick − start_tick`. A paused single-player server runs no ticks, so the clock stops.

## Style

### Meter and ranks

Points fill the current rank's bar (`points_per_rank` = 100). Overflow ranks up (several ranks
at once if needed); at VELOCIDE the bar caps full. The meter decays continuously, linearly from
`decay_lowest` (4/s, DULL) to `decay_highest` (30/s, VELOCIDE). Falling below zero drops one
rank and restarts the lower bar at `rank_down_refill` (60 %). A fresh rank can't be lost for
`rank_grace` (1 s), so a bonus that exactly reaches a rank doesn't flicker up and down. Dying (or
going down in co-op) empties the meter to DULL (`reset_on_death`).

| # | rank | HUD colour (`Theme::rank_colors`) | animation |
|---|---|---|---|
| 0 | DULL | ash grey | — |
| 1 | CRUDE | tarnished brass | — |
| 2 | FERAL | orange | — |
| 3 | VICIOUS | hot orange-red | — |
| 4 | MERCILESS | hot red | music at full intensity from here |
| 5 | GRISLY | magenta | — |
| 6 | SEARING | red-hot | slow throb |
| 7 | VELOCIDE | white-hot | flickers red, speed streaks stream off the name |

(The theme's palette replaces the placeholder colour list in `docs/gameplay.md`.)

### Bonuses

| bonus | points | trigger |
|---|---|---|
| KILL | 30 | an enemy killed by the player |
| FRAG | 100 | another player killed (instead of KILL); also the deathmatch knock-off credit |
| HEADSHOT | +15 | killing blow flagged `HEADSHOT` |
| AIRKILL | +40 | `killer_airborne` |
| AIRSHOT | +20 | `victim_airborne`, not for flying enemies (`Locomotion::Flying`) |
| MULTIKILL | +40 per extra kill | kills by the same player within `multikill_window` (0.6 s, sliding); *n* kills = 40 × (n − 1) |
| PARRY | 60 | `SimEvent::Parried` |
| RETURN TO SENDER | +80 | kill with a parried projectile (`parried_projectile`) |
| RICOCHET | 30 + 20 per extra bounce | a ricochet shot dealt damage (once per shooter per tick, deepest chain counts) |
| EXPLODED | +20 | explosive killing blow |
| SLAM KILL | +40 | `attack == Slam` |
| FISTED | +30 | `attack == Fist` |
| CLOSE CALL | ×1.5 | kill while the killer's health < 30; the extra half is its own feed line |
| LONGSHOT | +50 | a kill from ≥ `longshot_distance` (40 m, `KillInfo::distance`) with a hitscan shot or a non-explosive projectile of the killer's own (arrows); not explosions or returned projectiles (`style::is_longshot`, docs/potg.md) |
| STALE | (info) | the gain was cut by staleness; `points` = what was lost |

Suicides score nothing. Kill bonuses are awarded together (feed order: KILL/FRAG, HEADSHOT,
AIRKILL, AIRSHOT, RETURN TO SENDER, SLAM KILL / EXPLODED, FISTED, MULTIKILL, LONGSHOT, CLOSE CALL,
STALE).

### Freshness

Every weapon fire mode plus the fist, the slam, parries and thorns is a `StyleKey` (16 keys:
Quill, Brand, Splinter, Bellows, Gust, Both Barrels, Arclash, Wisp, Discharge, Tumbler, Breach,
Gravewell, Fist, Slam, Parry, Thorn) with a
freshness in `[freshness_min, 1]` (0.2..1). A scoring use returns the key's freshness as the
multiplier for that award, then costs it `freshness_cost` (0.12) and restores every other key by
`freshness_restore` (0.04); everything regenerates `freshness_regen` (0.02/s). Several uses of
the same key in one tick are charged once (one bomb killing three is one use). Below
`stale_threshold` (0.5) a key is **STALE**: the HUD shows "STALE <KEY>" under the meter when the
held weapon's primary or selected alt mode is stale, and gains with it add "- STALE" to the feed.
A kill with a parried projectile scores under `Parry` (one turned around by a Gust under
`Gust`), ricochets under the shot's weapon mode. A Gust deflection itself scores DEFLECT (30)
under `Gust`.

### Replication

* `GameEvent::Style(StyleEvent)` — reliable, **owner only**: `Bonus { player, bonus, points,
  count }` (count = multikill size / ricochet bounces) and `Rank { player, rank, up }`.
* `OwnState::style: StyleView` — every snapshot: rank, meter fill 0..255, freshness per key
  0..255 (sent as a bitmask of the keys below full freshness and their values: 4 bytes while
  everything is fresh, `docs/arsenal.md`). `NetClient::own_style()`.
* Everyone sees ranks through the scoreboard (`ScoreEntry::rank`, `best_rank`, `style`).

## Game modes

### Rules

`ModeRules { kind: GameMode, frag_limit, time_limit_secs, difficulty }`. Servers set them from the lobby's
`MatchSettings` in `SimWorld::apply_match_settings` (`ModeRules::from_settings`; this also
restarts the mode state). Worlds that never get settings (sim tests, client prediction copies)
use the level kind (`ModeRules::for_level`: campaign levels → campaign, arenas → deathmatch
20 frags / 10 min). `SimWorld::set_mode_rules` replaces them directly (tests, tools; also sets
the combat rules). Phases: `Waiting` (no players) → `Playing` → `Ended` (results).

### The death rule

| situation | state | respawn | where |
|---|---|---|---|
| match over (results) | dead | `combat.respawn_delay` | spawn; kill volumes don't kill at all (put back instead) |
| deathmatch, gun game | dead | `combat.respawn_delay` (2 s) | spawn farthest from the living opponents, `combat.spawn_protection` (1.5 s); gun game: the tier's build |
| campaign, one player | dead | `modes.sp_death_time` (1.5 s death cam) | last checkpoint (or the start); the level is reset to the checkpoint state first |
| campaign co-op, kill volume | dead (no body to revive) | `modes.fall_respawn_time` (4 s) | last checkpoint |
| campaign co-op | **downed** | `modes.downed_time` (15 s, × difficulty / players) unless revived | last checkpoint |
| co-op, everyone dead / downed at once | **team wipe** | `modes.wipe_respawn_time` (3 s), all together | last checkpoint, level reset |
| grind co-op | **downed** | as the campaign; when it runs out: out until the next wave | spawn point farthest from the Moulds, at the next wave's start |
| grind, solo or kill volume | dead (out) | the next wave's start | as above |
| grind, everyone dead / downed at once | **overrun** | — | the run ends (results after `grind.end_delay`) |

Every respawn: full health, no hard damage, fresh weapons (selection kept), spawn protection,
`SimEvent::Respawned`. Deaths cancel the victim's pending parryable hits and empty their style.

### Campaign

* **Checkpoints**: a living player's feet (or body centre) inside a checkpoint's trigger sets
  the team's respawn point (`ModeState::checkpoint`, only forwards: walking back through an
  earlier one changes nothing). `ModeEvent::Checkpoint` → "CHECKPOINT" banner + sound on every
  client; the scoreboard carries the index and the client draws reached checkpoint props active.
* **Downed** players have `combat.dead` set (no hitboxes, no input, AI ignores them, prediction
  uses `neutral_input`) and replicate `player_flags::DOWNED` *instead of* `DEAD`, so teammates
  draw the downed pose and the revive marker. They can look around (their view sinks to 0.6 m,
  `camera::DOWNED_EYE_HEIGHT`, as it does in the death view, and rises again on revive /
  respawn; no weapon in hand); they can't move or shoot.
* **Revive**: a living, non-downed teammate holding `USE` within `revive_range` (2.5 m, feet to
  feet) for `revive_time` (1.5 s). The nearest helper takes it; letting go or stepping out of range
  cancels (`ModeEvent::Reviving { reviver: None }`). Success: back up where they lie with
  `revive_health` (40), no hard damage, spawn protection, `ModeEvent::Revived` + `Respawned` (ends
  the death view on that client). Revives are counted per reviver.
* **Team wipe**: when every player is dead / downed (two or more players, or a lone downed player
  whose teammates left) → `ModeEvent::TeamWiped { respawn_tick }`; downed players become dead (no
  more revives) and everyone respawns together at the checkpoint.
* **Level reset** (solo death, team wipe): `SimWorld::reset_to_checkpoint` puts every encounter
  in progress back to idle through the enemies' hook `SimWorld::reset_uncleared` (its enemies
  removed, its doors back to their level-start state, its trigger re-armed — the checkpoint sits
  just before the trigger, so walking in restarts the fight), takes back the kills and spawns of
  those encounters (they'll be fought again, so nothing counts twice), and clears projectiles,
  parry-delayed hits and a pending exit countdown.
* **Enemy hooks** (`docs/enemies.md`): downed players have `combat.dead` set, so
  `enemy::is_targetable` (enemies never target or hit them) and
  `encounter::counts_for_encounters` (they don't trigger encounters or get rallied) already
  exclude them.
* **Exit**: solo — reaching the `exit` volume ends the level. Co-op — the first living player in
  starts an **extraction countdown** (`exit_countdown`, 10 s, `ModeEvent::ExitCountdown`, beeps
  for the last 5 s); the level ends early as soon as every living player is inside, otherwise when
  it runs out. Downed players don't hold the team up. Anyone not inside when it ends is **LEFT
  BEHIND** on the results. Why: no deadlock waiting for a straggler or an AFK player, a reason to
  sprint the last corridor together, and a little drama for whoever dawdles.
* **Late joiners** start at the reached checkpoint. Total enemies for the results =
  `SimWorld::expected_enemies` (the encounters' waves scaled for `coop_player_count` like the
  encounter system does) or the number actually spawned if larger.

### Deathmatch

* Frags: +1 per player killed, −1 per suicide (own explosion) or kill-volume / dev death. A
  kill-volume death within `knockoff_credit` (3 s) of being hit by another player gives that
  player the frag (+ FRAG style) instead: knocking someone into the coolant counts, and the
  victim loses nothing (only uncredited self-kills / world deaths cost a frag).
* Respawn after `combat.respawn_delay` (2 s) at the level spawn whose nearest living opponent is
  farthest away; `combat.spawn_protection` (1.5 s) blocks damage (`Blocked::SpawnProtection`).
* Friendly fire is PvP (all players are enemies; `CombatRules::pvp`), self damage on. Ember-burst
  healing competition comes from combat.
* `frag_limit` reached (frags ≥ limit) or `time_limit_secs` elapsed → match over. Standings: frags,
  then fewer deaths, then style. The HUD shows frags / limit, the place (1ST / TIED / 2ND…) and the
  time left.

### Difficulty and co-op scaling (`sim::difficulty`)

The player asked for a difficulty for co-op / PvE, and said co-op was too easy with several
players. `MatchSettings::difficulty` (EASY, NORMAL, HARD, BRUTAL, INFERNO) is picked by the host
in the lobby's MATCH panel (campaign and grind only), on the single-player page, with
`--difficulty` (client and dedicated server); `ModeRules::difficulty` carries it into the sim, the
scoreboard and the results. The lobby summary reads `GRIND // KNELL YARD // HARD`; in game the
campaign shows the difficulty under the level timer, grind in its wave block, and the Tab
scoreboard and the results subtitles name it.

`PveScaling::compute(t, difficulty, players)` (players = `coop_player_count`, with
`rules.coop_enemy_scaling`; PvP modes: no scaling at all) decides, for spawns happening now:

| what | value | NORMAL, 1 / 2 / 4 players |
|---|---|---|
| enemy health (at spawn) | `health[d] × (1 + player_health (p − 1))` | ×1 / ×1.6 / ×2.8 |
| enemies per authored spawn, grind budget | `count[d] × (1 + player_count (p − 1))`, rounded, ≥ 1 per wave that has any | ×1 / ×1.6 / ×2.8 |
| damage enemies deal (`EnemyScale::damage`) | `damage[d] × (1 + player_damage (p − 1))` | ×1 / ×1.1 / ×1.3 |
| aggression (`EnemyScale::aggression`: cooldowns / recoveries ÷) | `aggression[d] × (1 + player_aggression (p − 1))` | ×1 / ×1.08 / ×1.24 |
| enemy projectile speed | `projectile_speed[d]` | ×1 |
| elite chance per spawned Mould | `elite_chance[d] + player_elite_chance (p − 1)` | 2 % / 4 % / 8 % |
| downed time | `modes.downed_time × downed_time[d] × (1 − player_downed_time (p − 2))`, ≥ `min_downed_time` | 15 / 15 / 12 s |
| revive health | `modes.revive_health × revive_health[d]` | 40 |

| table (`difficulty`) | EASY | NORMAL | HARD | BRUTAL | INFERNO |
|---|---|---|---|---|---|
| `health` | 0.7 | 1.0 | 1.3 | 1.65 | 2.1 |
| `count` | 0.75 | 1.0 | 1.2 | 1.4 | 1.6 |
| `damage` | 0.6 | 1.0 | 1.25 | 1.5 | 1.85 |
| `aggression` | 0.8 | 1.0 | 1.15 | 1.3 | 1.5 |
| `projectile_speed` | 0.85 | 1.0 | 1.1 | 1.2 | 1.3 |
| `elite_chance` | 0 | 0.02 | 0.06 | 0.12 | 0.2 |
| `downed_time` (×) | 1.5 | 1.0 | 0.8 | 0.65 | 0.5 |
| `revive_health` (×) | 1.5 | 1.0 | 0.85 | 0.7 | 0.55 |

Per extra player: `player_health` 0.6, `player_count` 0.6, `player_damage` 0.1,
`player_aggression` 0.08, `player_elite_chance` 0.02, `player_downed_time` 0.1 (from the third
player), `min_downed_time` 4 s.

Why these: NORMAL solo is exactly the game as tuned before (the reference). The old co-op rule
was health ×2.5 and count ×2.5 for four players and nothing else — the Moulds' attention was
spread over four targets, so every player took far less pressure than solo, and three teammates
to revive you made deaths cheap. Now four players face ×2.8 the Moulds with ×2.8 the health
(the contract's +0.6 per player), and the Moulds hit 30 % harder and 24 % more often, so the
pressure per player is back above solo (≈ 2.8 / 4 × 1.3 × 1.24 ≈ 1.13×), and the downed timer
shrinks with a full squad. Wind-ups never change: harder Moulds are still readable. EASY is
gentler than the old solo game; BRUTAL / INFERNO are for full squads that want to be punished.

* **Where it applies**: `SimWorld::step_modes` puts `pve_scaling(t).enemy` into
  `SimWorld::enemy_scale` every tick (players joining / leaving change it at once; the enemies
  area applies it inside the AI). Encounter waves (`encounter::spawn_wave`) and grind waves read
  `pve_scaling` when they spawn: a wave of `n` authored spawns has `scaled_count(n)` Moulds (EASY
  drops copies from the end; extra copies are placed around the authored points), each with
  health × `health`. `mode_player_died` / `revive` use the downed time and revive health.
  `SimWorld::expected_enemies(t)` counts the scaled waves for the results.
* **Elites** are the enemies area's (a replicated flag, more health, a searing glow). Every
  PvE spawn rolls the elite chance (`ModeRng`, deterministic per tick) and goes through
  `SimWorld::spawn_pve_enemy(name, pos, yaw, health, elite)`, which spawns it through the
  enemies area's `SimWorld::spawn_enemy_with` (an elite gets its type's elite health bonus,
  ×2.5 by default, on top of `health`; docs/enemies.md).
* Dev `/spawn` uses the same difficulty × players health (`SimWorld::pve_scaling`; the old
  `enemy_health_scale` hook, 1 + 0.5 per extra player, is no longer used by the server).

### Grind (`sim::grind`)

Endless numbered waves on an arena (every arena, and the sandboxes) for 1–4 players with the
co-op rules, until everyone is down or dead at once. Host it from the lobby (MODE GRIND), play it
alone from the single-player page (MODE GRIND, the arenas are listed) or `velocide --sp
knell_yard --mode grind --difficulty hard`; dedicated: `velocide-server --mode grind --level
cistern_nine`.

* **Breather → wave**: `first_delay` (5 s) before wave 1, `breather` (6 s) between waves; the
  HUD counts down ("WAVE 8 IN 3", beeps for the last three seconds).
* **Wave contents** (`plan_wave`, pure): budget `budget_base + budget_per_wave (w − 1) +
  budget_per_wave_sq (w − 1)²` (6 + 2.4 (w − 1) + 0.06 (w − 1)²: wave 1 = 6, wave 10 = 32.5,
  wave 20 = 73) × the difficulty × players `count` (× `heavy_budget` 1.25 on heavy waves). Moulds
  are picked by weight from the roster entries unlocked by the wave (and affordable), until the
  budget is spent; at most `max_wave_enemies` (150). Health: the difficulty × players `health`
  × (1 + `health_per_wave` (w − 1)) (+3 % per wave). The roster (names the server's registry
  doesn't know are skipped, so the new Moulds join when the enemies area merges):

  | Mould | cost | from wave | weight | |
  |---|---|---|---|---|
  | gnawer | 1 | 1 | 10 | |
  | harrier | 1.5 | 2 | 4 | |
  | censer | 2 | 3 | 5 | |
  | snuffer | 1.5 | 4 | 4 | (enemies area) |
  | heron | 3 | 6 | 3 | |
  | bellwether | 4 | 7 | 2 | (enemies area) |
  | gaoler | 6 | 8 | 2 | heavy |
  | smelter | 6 | 12 | 2 | heavy (enemies area) |

* **Heavy waves** (every `heavy_every` = 5th): bigger budget, `1 + (w − 1) / boss_every`
  guaranteed heavies (heavies debut on the heavy wave before their own unlock: the Gaoler on
  wave 5), the elite chance + `heavy_elite_chance` (15 %). **Boss waves** (every `boss_every` =
  10th): a boss-grade elite leads the wave, a heavy with `boss_health` (×4) health and the elite
  roll. The HUD and banner say HEAVY WAVE / BOSS WAVE.
* **Spawning**: a wave opens with `spawn_burst` (6) Moulds, then one every `spawn_interval`
  (0.35 s) while fewer than `max_alive + alive_per_player (p − 1)` (14 + 5 per extra player) of
  its Moulds live; waiting Moulds count as "left". Spawn points: the level's `grind_spawns` (a
  random one of those at least `min_spawn_distance` (14 m) from every living player, else the
  farthest; Moulds that would overlap go on a ring around it, flyers 4.5 m up), else walkable nav
  nodes that can reach the player spawns (found once per run by a backwards search over the nav
  links; never in or by lava), at least 14 m from every living player when the arena allows;
  as a last resort the level spawn farthest from the players, so a wave never stalls. Grind
  Moulds are aware (they hunt the players).
* **Deaths**: co-op deaths are *downed* (revivable, the difficulty's downed time and revive
  health, as in the campaign). A downed player nobody revives, a kill-volume death or a solo
  death is **out** (`LifeState::Dead`, status OUT on the scoreboard) until the **next wave
  starts**: then everyone dead or downed is back at the level spawn farthest from the Moulds
  with full health and spawn protection. While a wave is on their death view has no countdown
  (`respawn_in` = `u16::MAX`); in the breather it counts down to the wave.
* **The end**: everyone down or dead at once → `GrindEvent::Overrun { end_tick }` (banner
  OVERRUN, no respawn countdowns) → `end_delay` (2.5 s) later `end_match(EndReason::Overrun)`.
  The host / lobby can end a run too (RUN OVER).
* **Results**: `MatchResults::grind = GrindResult { wave, cleared, grade }`: the wave reached,
  waves cleared, the wave grade (`grind.grade_wave` (20, 15, 10, 5) for S, A, B, C). Per player
  the campaign grades without a par time (kills vs the player's share of the Moulds that
  spawned, style and damage per share) and an overall grade that averages in the wave grade; a
  team rank. The client keeps the best wave per arena and difficulty in its settings
  (`Settings::grind_best`, `"knell_yard/hard"`), shows it on the single-player page and in the
  HUD ("BEST: WAVE 23") and on the results (NEW BEST!).
* **Network**: `ModeEvent::Grind(GrindEvent::{WaveStarted { wave, enemies, heavy, boss },
  WaveCleared { wave, next_tick }, Overrun { end_tick }})` to everyone (banners, sounds) and the
  run's `GrindStatus { wave, left, total, next_wave_tick, heavy, boss, overrun_tick }` on every
  scoreboard (the HUD and late joiners).

### Gun game (`sim::gungame`)

A PvP free-for-all on an arena (up to 8 players) where everyone climbs the same fixed ladder of
weapon builds (`gun_game.ladder`), then the **Fist**. Host it from the lobby (MODE GUN GAME, a
TIME LIMIT, no frag limit), `velocide-server --mode gungame --level knell_yard`.

* **The ladder** (one tier per weapon, each with its default build, from forgiving to
  demanding): 1 Arclash, 2 Kiln, 3 Thresher, 4 Riveter, 5 Bellows, 6 Tumbler, 7 Cinderbox, 8
  Lancet, 9 Quill, 10 Longwick, 11 the Fist. Each player carries only their tier's build: a
  forced loadout (`SimWorld::force_loadout`, the arsenal's API), so it survives respawns and
  wins over the player's own loadout; `step_gun_game` keeps everyone's forced loadout on their
  tier every tick (joins, restarts, a hot-reloaded ladder).
* **Promotion**: a kill with the weapon of your tier (`KillInfo::weapon`: the weapon that fired
  the killing blow, its abilities included) moves you up a tier at once (the next build
  replaces the weapon in your hands, slot 1 selected) — `GunGameEvent::Promoted`. Kills with
  anything else don't count: a bomb of your previous weapon still bouncing, slams, thorns
  (off anyway: the PvP balance profile), knocking someone into a kill volume.
* **Punch kills** (`Attack::Fist`) knock the victim down `punch_demotes` (1) tier, whatever
  tiers either player is on (never below the first) — `GunGameEvent::Demoted`. The puncher only
  climbs if the Fist is their tier: **a punch kill on the Fist tier wins** the match
  (`EndReason::Ladder`; the winner's tier is set one past the Fist).
* **The Fist tier** is fists only: `tier_loadout` forces `Loadout::unarmed()` (no weapon: the
  trigger and abilities do nothing, the fist punches; replicated as unarmed,
  `protocol::player_armed`; `docs/arsenal.md`). Kills with anything but the fist on the Fist
  tier (a projectile of the last weapon still flying) count for nothing; the HUD says PUNCH A
  KILL TO WIN.
* **Respawns, protection, time limit**: deathmatch's. When time runs out the most tiers wins,
  then kills, then fewer deaths, then style. Gun game has no frag limit
  (`ModeRules::gun_game(time_limit_secs)`); frags aren't counted, kills are.
* **Late joiners** start on the lowest tier anyone holds (a chance to catch up).
* **Network**: `ModeEvent::GunGame(GunGameEvent::{Promoted { player, tier, victim },
  Demoted { player, tier, by }})` to everyone; tiers on the scoreboard (`ScoreEntry::tier`) and
  in the results (`PlayerStats::tier`, `MatchResults::gun_game = GunGameResult { ladder }`);
  clients read the ladder from the tuning the server sent (`Tuning::gun_game`).

### Match end

`SimWorld::end_match(t, reason)` (exit, frag limit, time limit, grind overrun, the gun game
ladder, or `EndReason::Host`) builds the
`MatchResults`, emits `ModeEvent::Ended(results)`, switches to `Ended` and sets the shared
results freeze **`SimWorld::frozen`** (also set by `Server::end_match`; a world frozen from
outside ends its match at the next tick): nothing scores, enemies idle where they stand and
encounters don't trigger (enemies code), parry-delayed enemy hits don't land, players are
protected, kill volumes put players back instead of killing them. `restart_level` unfreezes. On the server the event goes to every client and then
`Server::end_match()` moves the lobby flow to its **Results** phase (lobby servers return to the
lobby by themselves after 30 s, single-player waits for RESTART / MAIN MENU). Ending a match
from the lobby side (`LobbyCommand::EndMatch`, `Server::end_match`) also ends the sim's match
(`end_sim_match`), so results always exist. `restart_level` (R) resets the mode state.

## Results and grades

`MatchResults { rules, reason, level, duration_secs, par_secs, total_enemies, team_kills,
players: [PlayerResult], team_grade, winner, grind: Option<GrindResult> }`, `PlayerResult { player, name, stats:
PlayerStats { kills, deaths, frags, damage_taken, revives, tier }, style_points, best_rank,
extracted, grades }`, `gun_game: Option<GunGameResult>`. Deathmatch and gun game players are in
standings order.

Campaign grades per player (`grade_player`). Thresholds are S / A / B / C, anything worse is D
(`modes.grade_*`); *share* = level enemies / players:

| grade | measure | S | A | B | C |
|---|---|---|---|---|---|
| time | level time ÷ par (≤) | 1.0 | 1.25 | 1.65 | 2.2 |
| kills | kills ÷ share (≥) | 100 % | 90 % | 75 % | 50 % |
| style | style points ÷ share (≥) | 120 | 85 | 60 | 35 |
| damage | damage taken ÷ share (≤) | 1 | 3 | 6 | 10 |

For Crucible of Veins (par 5:30, 86 enemies) solo: S ≤ 5:30, A ≤ 6:52, B ≤ 9:04, C ≤ 12:06; style
S ≥ 10,320; damage S ≤ 86. Without a par time / enemies those grades are left out. **Overall** =
the average of the available grades, rounded (halves up). **Team rank** = the average of the
players' overall grades. Grind grades the same way without a par time (the share is the Moulds
that spawned) and folds the run's wave grade into every overall grade (see Grind).

### Results screens (`menus::results`)

Shown in the lobby's Results phase (`InGame::results_ui` in `match_flow.rs` draws the screen,
then the lobby's choices in one row: RESTART / MAIN MENU solo, CONTINUE TO LOBBY / LEAVE GAME
for the host, WAITING FOR THE HOST + LEAVE GAME otherwise, with the auto-return countdown).
Enter / Esc / click skips the reveal; the buttons appear when it's done.

* **Solo campaign**: LEVEL COMPLETE title sliding in with a crimson extrusion; TIME (vs par), KILLS
  (of the level, %), STYLE (best rank), DAMAGE TAKEN (deaths) slide in one by one, numbers count up
  and each grade is *stamped* (pop, white flash, `KillConfirm`); then the big RANK letter slams in
  (`RankUp`) with its word (SUPREME / ADEPT / BRUTAL / CLUMSY / DULL). S grades get a pulsing
  white-hot halo.
* **Co-op campaign**: team line (time, par, kills %), one row per player (kills, style, damage,
  deaths, revives, the four small grades T K S D, overall), LEFT BEHIND tags, TEAM RANK stamp.
* **Grind**: OVERRUN (RUN OVER when the host ended it), the level and difficulty under it. Solo:
  the campaign tally with WAVE (the wave reached, "SURVIVED 21:52", the wave grade) in place of
  TIME, then KILLS (of the Moulds that spawned), STYLE, DAMAGE TAKEN and the RANK, with "NEW
  BEST!" (flashing) or "BEST: WAVE 25" under it. Co-op: the team line (WAVE, TIME, KILLS, the
  best wave on the right) and the co-op table with W K S D (the wave grade first). The PvE
  layouts name the difficulty in the subtitle ("CRUCIBLE OF VEINS // HARD").
* **Gun game**: the deathmatch layout ("THE LADDER IS CLIMBED" / TIME LIMIT) with TIER in place of
  FRAGS: "9 QUILL", "FIST" (on the last tier), "WON" (white-hot, the ladder's winner); the
  winner banner and DRAW / TOP OF THE TABLE compare tiers, then kills.
* **Deathmatch**: MATCH OVER + level and reason, winner banner in the winner's colour (VICTORY
  for you; DRAW / DEAD EVEN when frags, deaths and style are all level), standings (#, frags — negative in red —, deaths, best rank in its colour, style),
  rows revealed one by one.

`LevelComplete` (campaign) / `MatchEnd` (deathmatch) play when the results arrive and the music
switches to the `Results` track; the 3D world stays behind a darkened vignette, the viewmodel
and HUD are hidden.

## Scoreboard

`ModeEvent::Scoreboard(Scoreboard { rules, phase, start_tick, checkpoint, rows: [ScoreEntry
{ player, kills, deaths, frags, rank, best_rank, style, life, revives }], grind })` is broadcast when something
on it changes (joins / leaves, kills, deaths, rank changes, checkpoint, life state, phase), at most
every `scoreboard_interval` (0.25 s). Tab columns (`scoreboard::columns_for`): deathmatch FRAGS,
KILLS, DEATHS, RANK, STYLE; campaign KILLS, DEATHS, RANK, STYLE; grind KILLS, DEATHS, REVIVES,
RANK, STYLE; gun game TIER ("5 BELLOWS", "FIST"), KILLS, DEATHS, RANK, STYLE, rows by tier then
kills; then the lobby's PING and STATUS (ALIVE / DOWNED / DEAD / OUT (grind) / BOT /
HOST). The title names the mode and, in PvE, the difficulty ("KNELL YARD // SOLO GRIND // HARD"). `InGame::score_rows` fills the cells
(`fill_score_row`, rank in its colour, negative frags red) and sorts deathmatch rows by frags.
Adding a column: an entry in `columns_for` and a match arm in `fill_score_row`.

## Protocol (version 5)

| message | to | notes |
|---|---|---|
| `GameEvent::Style(StyleEvent)` | owner | bonuses, rank changes |
| `GameEvent::Mode(ModeEvent::Checkpoint { index, player })` | all | |
| `…::Downed { player, respawn_tick }` | all | countdowns are computed from server ticks (`NetClient::server_tick_estimate`) |
| `…::Reviving { player, reviver, start_tick, done_tick }` | all | start (`Some`) / stop (`None`): progress bars on every client |
| `…::Revived { player, by }`, `…::TeamWiped { respawn_tick }`, `…::ExitCountdown { player, end_tick }` | all | |
| `…::Scoreboard(Scoreboard)` | all | throttled |
| `…::Ended(Box<MatchResults>)` | all | then the lobby's Results phase |
| `…::Grind(GrindEvent::{WaveStarted, WaveCleared, Overrun})` | all | banners and sounds; the state rides on the scoreboard (`Scoreboard::grind`) |
| `…::GunGame(GunGameEvent::{Promoted, Demoted})` | all | banners, sounds, feed lines; tiers ride on the scoreboard (`ScoreEntry::tier`) |
| `OwnState::style: StyleView` | owner | every snapshot |
| `player_flags::DOWNED` | all | on the player entity, `DEAD` cleared |

Typical traffic: a kill = 2–4 small style events to the killer + one scoreboard (~15 B per
player) to everyone.

## Client HUD (`style_hud`, `mode_hud`)

* **Style block** (right side, under the event feed): a dark slanted slab with a rank-coloured
  edge, the rank name (scale 2, `hud::style_meter`) with the eased meter fill and rank pips; a
  white flash on rank-up, a red flash + shake on rank-down; the held weapon's three freshness pips
  (primary, alt A, alt B; red when stale) and the blinking STALE line; the style feed below
  (newest on top; bursts are grouped: "+ KILL x3", "+ EXPLODED x3", one "+ MULTIKILL x3" line that
  keeps counting), separate from the top-right kill / event feed.
* **Rank-up banner** (`hud::banner` in the rank colour, its own slot above the game banner) +
  `RankUp`; `RankDown` on drops; `StyleBonus` for notable bonuses.
* **Music**: intensity floor by rank (DULL 0.2 … VICIOUS 0.6, MERCILESS+ 1.0).
* **Campaign**: CHECKPOINT banner + `Checkpoint`; co-op team list top left (DOWN 12 / DEAD);
  over downed teammates a pulsing cross with "NAME DOWN 12", the revive progress bar or
  "HOLD [E] TO REVIVE" (within reach) / the distance; the local downed panel (YOU ARE DOWN,
  checkpoint countdown, "NAME IS REVIVING YOU" + bar); the reviver's bar under the crosshair;
  TEAM WIPED banner + "REGROUPING AT THE CHECKPOINT" (the death view counts down); EXTRACTION countdown (urgent red in the
  last seconds, `Countdown` beeps); solo death: "BACK TO THE CHECKPOINT".
* **Deathmatch**: frags / limit and place (top left), TIME LEFT (top centre).
* **Gun game** (top left): "TIER 5 / 11 BELLOWS", "NEXT: TUMBLER" (on the Fist tier "FIST",
  flashing, and "PUNCH A KILL TO WIN"), the place (1ST / TIED / 2ND…), under it the leader
  ("LEADER: VEX - 8 LANCET") or "YOU LEAD - NEXT BEST: …"; TIME LEFT top centre. Banners: your
  promotion "TIER 5 - BELLOWS" (`RankUp`), "THE FIST" (`RankUp`, lower), "PUNCHED DOWN"
  (`RankDown`); feed lines "VEX HAS THE FIST", "VEX PUNCHED YOU DOWN A TIER".
* **Difficulty** (campaign): the difficulty under the level timer.
* **Grind** (top centre, under the timer): "WAVE 7" (+ HEAVY / BOSS) with "HARD // 12 LEFT //
  BEST: WAVE 23" under it, or in the breather "WAVE 8 IN 3" (red for the last three seconds,
  `Countdown` beeps) and "WAVE 7 CLEARED"; banners WAVE n / HEAVY WAVE n / BOSS WAVE n
  (`WaveStart`, lower pitched for heavy / boss), WAVE n CLEARED (`EncounterClear`), OVERRUN
  (`Death`); the co-op team list as in the campaign; the downed panel says "OUT IN 12 - HOLD ON
  FOR A REVIVE"; players who are out read "OUT UNTIL THE WAVE IS CLEARED" / "BACK FOR WAVE 8"
  (the death view hides waits longer than ten minutes, `combat::hud`).

## Tuning

`style` (`StyleTuning`): `points_per_rank` 100, `decay_lowest` 4, `decay_highest` 30,
`rank_down_refill` 0.6, `rank_grace` 1.0, `reset_on_death` true, `kill` 30, `frag` 100,
`headshot` 15, `airkill` 40, `airshot` 20, `multikill` 40, `multikill_window` 0.6, `parry` 60,
`return_to_sender` 80, `ricochet` 30, `ricochet_per_bounce` 20, `exploded` 20, `slam_kill` 40,
`fisted` 30, `close_call_health` 30, `close_call_multiplier` 1.5, `freshness_cost` 0.12,
`freshness_restore` 0.04, `freshness_regen` 0.02, `freshness_min` 0.2, `stale_threshold` 0.5,
`longshot` 50, `longshot_distance` 40.

`modes` (`ModeTuning`): `sp_death_time` 1.5, `downed_time` 15, `revive_range` 2.5,
`revive_time` 1.5, `revive_health` 40, `fall_respawn_time` 4, `wipe_respawn_time` 3,
`exit_countdown` 10, `knockoff_credit` 3, `scoreboard_interval` 0.25, `grade_time`
(1.0, 1.25, 1.65, 2.2), `grade_kills` (1.0, 0.9, 0.75, 0.5), `grade_style` (120, 85, 60, 35),
`grade_damage` (1, 3, 6, 10). Deathmatch respawn delay / spawn protection reuse
`combat.respawn_delay` / `combat.spawn_protection`; frag and time limits are match settings.

`difficulty` (`DifficultyTuning`): the per-difficulty tables and per-player factors in the
Difficulty section above.

`grind` (`GrindTuning`): `first_delay` 5, `breather` 6, `budget_base` 6, `budget_per_wave` 2.4,
`budget_per_wave_sq` 0.06, `health_per_wave` 0.03, `heavy_every` 5, `heavy_budget` 1.25,
`heavy_elite_chance` 0.15, `boss_every` 10, `boss_health` 4, `max_alive` 14,
`alive_per_player` 5, `spawn_burst` 6, `spawn_interval` 0.35, `max_wave_enemies` 150,
`min_spawn_distance` 14, `end_delay` 2.5, `grade_wave` (20, 15, 10, 5), `roster` (the table in
the Grind section).

`gun_game` (`GunGameTuning`): `ladder` (the ten builds above, first to last), `punch_demotes` 1.

## Tests and tools

```text
cargo test -p shared --test style      # style math + scoring real combat in the sim (15)
cargo test -p shared --test modes      # checkpoints, deaths, revive, wipe, exit, DM, grades, difficulty, grind, gun game (43)
cargo test -p shared --lib difficulty grind gungame   # scaling numbers, wave plans, budgets, unlocks, heavies / bosses, the ladder
cargo test -p server --test modes_net  # over UDP with real clients (7)
cargo run -p client --release --example style_shots -- out/   # PNGs of the HUD / results / co-op revive / grind
PLAYTHROUGH=grind,gungame cargo run -p client --example playthrough   # solo grind from the menu, a gun game against bots
```

* `style.rs`: every kill bonus trigger (incl. no AIRSHOT on flyers, FRAG vs KILL), ricochet chain
  points, decay rates, rank up / grace / rank down with refill / DULL floor / VELOCIDE cap,
  freshness cost / same-tick charge / floor / restore / regen, staleness cuts gains and reports
  STALE, close call ×1.5, rank-up events, the multikill sliding window; in the sim: a headshot
  kill (KILL + HEADSHOT, view, stats), a bomb into three enemies (3 × KILL + EXPLODED, 2 ×
  MULTIKILL, one freshness charge, two ranks), a parry, a ricochet kill, death empties the
  meter; `assets/tuning.ron` spells out every style / modes value equal to the defaults.
* `modes.rs`: checkpoints set the respawn point and only move forward; solo death → 1.5 s → last
  checkpoint with the level reset; start spawn before any checkpoint; co-op downed (DOWNED flag,
  not DEAD) → revive cancelled by letting go → completed at 40 HP where they lay; too far away
  doesn't revive; 15 s timeout → checkpoint; everyone down → team wipe → all back together and
  the fight resets; kill volumes aren't revivable; solo exit → graded results and a frozen,
  protected world; co-op extraction countdown with a straggler left behind / everyone in ends at
  once; grade thresholds (incl. co-op shares, missing par / enemies); Crucible's authored enemy
  count; arenas default to deathmatch, the lobby's settings drive the rules (frag limit, co-op
  friendly fire); frags +1 / −1 suicide / −1 kill volume, knock-off credit and its window; respawn
  after 2 s at the farthest spawn, spawn protection then damage; frag limit → standings; time
  limit; the scoreboard broadcast; late joiners at the checkpoint; restart resets the match; AI
  and kill volumes frozen after the end (`SimWorld::frozen`); a solo death in the middle of
  Crucible's first arena resets that encounter (idle, enemies gone) and takes its kill back.
  Difficulty: an encounter wave of 4 is 4 × 45 HP on NORMAL solo, 11 × 126 HP for four players,
  3 × 31.5 HP on EASY, 8 × 93.6 HP on HARD for two; `enemy_scale` follows the difficulty and a
  player joining / leaving at once, PvP has none; HARD downs for 12 s and revives at 34 HP, four
  players on NORMAL down for 12 s; the tuning file spells out every `difficulty` / `grind`
  value. Grind: wave 1 = six Gnawers after the breather, cleared → WaveCleared and the breather
  on the scoreboard, waves grow, every 5th is heavy, all kills counted; Moulds spawn ≥ 14 m away
  on the floor (nav) or around the far `grind_spawns`; a big wave trickles in under the alive
  cap with the queue counted as left; a downed player nobody revives is out (no countdown) until
  the next wave, counts down in the breather and is back, protected, when it starts; both
  players down → Overrun → results 2.5 s later (wave 2 reached, 1 cleared, grade D, per-player
  grades without a par time); a solo death ends the run; lobby settings start a grind run;
  without any usable spawn point the wave still comes (at the farthest level spawn). Gun game:
  everyone starts with the first build only; a kill with another weapon or a slam counts
  nothing, a kill with the tier's weapon promotes at once (event, the next build in hand) and
  survives a respawn, tiers ride on the scoreboard; a punch kill knocks the victim down a tier
  (never below the first) without promoting the puncher; on the Fist tier weapon kills count for
  nothing and a punch kill wins (winner one past the Fist, standings by tier, the ladder in the
  results, a frozen world); the Fist tier is fists only (unarmed, replicated as such, nothing
  but punches fires, still unarmed after a respawn, armed again when punched down); the time
  limit ranks by tier, not kills, and gun game never ends on frags; late joiners start on the
  lowest tier; lobby settings start a gun game.
* `modes_net.rs`: style events reach only the killer, `OwnState` carries the meter, everyone's
  scoreboard shows the frag and rank; a frag limit ends the match into the Results phase on every
  client; ending from the lobby flow builds results; a client holding USE revives a downed
  teammate (both clients see DOWNED, Reviving, Revived); the campaign exit sends graded results;
  a grind run on HARD for two: wave 1 (eleven Gnawers) and its scoreboard status on both
  clients, the breather, wave 2, both killed → Overrun → grind results and the Results phase;
  a gun game for two: the PvP profile (no Thorns), the first build in every owner's own state,
  a promotion reaching everyone and the next build in the owner's prediction, a punch kill on
  the Fist tier → results (`Ladder`) and the Results phase.
* `style_shots`: the style meter at DULL / FERAL / MERCILESS (STALE) / SEARING (grouped
  multikill feed) / VELOCIDE, the rank-up banner, CHECKPOINT, the deathmatch HUD and Tab
  scoreboard, solo / perfect / co-op / deathmatch results mid-reveal and finished, and a real
  two-client co-op revive (teammate marker with the prompt, the downed panel, both progress bars,
  REVIVED); a real solo grind run on Knell Yard (HARD, record 12): the first breather, wave 1
  with its banner, the Tab scoreboard, WAVE 1 CLEARED, an injected BOSS WAVE 10, the overrun,
  its results, a deep run with NEW BEST!, a co-op grind table; a gun game on a real server with
  two bots (tiers injected): the tier block with a promotion banner, the Tab scoreboard, the Fist
  tier, PUNCHED DOWN, results won on the ladder.

## Known limitations / next steps

* Downed players can look around but don't crawl (crawling would need predicted downed movement
  on the client); the third-person downed pose comes from the models.
* Players who leave mid-match drop off the results.
* Elites: the difficulty rolls them, but they spawn as normal Moulds until the enemies area's
  elite spawn exists (`SimWorld::spawn_pve_enemy` is the one place to hook it up). Boss-grade
  elites already have their ×4 health.
* Dev `/spawn` still uses the enemies area's simple co-op health hook (1 + 0.5 per extra
  player), not the difficulty.
* Grind spawn points without `grind_spawns` come from the nav grid: walkable floors that can
  reach the player spawns; a Mould may still appear somewhere awkward on an arena with odd
  geometry (the maps area adds `grind_spawns` to every arena).
* Grind best waves are per machine (settings file), not per player profile or server.
* Gun game's Fist tier is fists only in the sim, the replicated state and the prediction; until
  the arsenal-client work lands the HUD's weapon block and the viewmodel don't follow forced
  loadouts yet (an unarmed player is still drawn with the default weapon).
