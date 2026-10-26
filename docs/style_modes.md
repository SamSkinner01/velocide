# Style, game modes and results

The per-player **style meter** (ranks, bonuses, freshness, the style feed), the **game-mode
rules** (campaign checkpoints, solo checkpoint restarts, co-op downed / revive / team wipes, the
level exit, deathmatch frags and limits — i.e. *the* death / respawn rule), the **scoreboard**
and the **results screens** with grades. Everything is server-authoritative and runs inside
`SimWorld::step`; numbers live in `assets/tuning.ron` (`style`, `modes`). Base rules:
`docs/gameplay.md` ("Style", "Game modes"); match flow / lobby: `docs/lobby.md`.

| where | what |
|---|---|
| `shared::sim::style` | `PlayerStyle` (meter, rank, freshness, totals), `StyleSystem` (`SimWorld::style`), `StyleBonus`, `StyleKey`, `StyleEvent`, `StyleView`, scoring (`kill_bonuses`, `is_close_call`, `ricochet_points`, `decay_rate`, `SimWorld::score_style`) |
| `shared::sim::modes` | `ModeRules`, `ModeState` (`SimWorld::mode`), `ModePlayer`, `LifeState`, `MatchPhase`, `PlayerStats`, `ModeEvent`, `Scoreboard`, `MatchResults`, `Grade` / `Grades` / `grade_player`, the death rule, checkpoints, revives, exit, deathmatch, `end_match`, results |
| `shared::tuning` | `StyleTuning`, `ModeTuning` (sections `style`, `modes`) |
| `server::modes` | routing (`GameEvent::Style` to the owner, `GameEvent::Mode` to everyone), match end → `Server::end_match` |
| `client::ingame::style_hud` | `StyleHud`: meter, rank animation, STALE + freshness pips, style feed, rank-up banner, rank sounds, music floor |
| `client::ingame::mode_hud` | `ModeHud`: client mode state (scoreboard, downed, revives, exit, wipe), mode HUD, `InGame` glue (events, score cells, results) |
| `client::menus::results` | `ResultsScreen`: campaign solo tally, co-op table, deathmatch standings |
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

`ModeRules { kind: GameMode, frag_limit, time_limit_secs }`. Servers set them from the lobby's
`MatchSettings` in `SimWorld::apply_match_settings` (`ModeRules::from_settings`; this also
restarts the mode state). Worlds that never get settings (sim tests, client prediction copies)
use the level kind (`ModeRules::for_level`: campaign levels → campaign, arenas → deathmatch
20 frags / 10 min). `SimWorld::set_mode_rules` replaces them directly (tests, tools; also sets
the combat rules). Phases: `Waiting` (no players) → `Playing` → `Ended` (results).

### The death rule

| situation | state | respawn | where |
|---|---|---|---|
| match over (results) | dead | `combat.respawn_delay` | spawn; kill volumes don't kill at all (put back instead) |
| deathmatch | dead | `combat.respawn_delay` (2 s) | spawn farthest from the living opponents, `combat.spawn_protection` (1.5 s) |
| campaign, one player | dead | `modes.sp_death_time` (1.5 s death cam) | last checkpoint (or the start); the level is reset to the checkpoint state first |
| campaign co-op, kill volume | dead (no body to revive) | `modes.fall_respawn_time` (4 s) | last checkpoint |
| campaign co-op | **downed** | `modes.downed_time` (15 s) unless revived | last checkpoint |
| co-op, everyone dead / downed at once | **team wipe** | `modes.wipe_respawn_time` (3 s), all together | last checkpoint, level reset |

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

### Match end

`SimWorld::end_match(t, reason)` (exit, frag limit, time limit, or `EndReason::Host`) builds the
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
players: [PlayerResult], team_grade, winner }`, `PlayerResult { player, name, stats:
PlayerStats { kills, deaths, frags, damage_taken, revives }, style_points, best_rank, extracted,
grades }`. Deathmatch players are in standings order.

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
players' overall grades.

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
* **Deathmatch**: MATCH OVER + level and reason, winner banner in the winner's colour (VICTORY
  for you; DRAW / DEAD EVEN when frags, deaths and style are all level), standings (#, frags — negative in red —, deaths, best rank in its colour, style),
  rows revealed one by one.

`LevelComplete` (campaign) / `MatchEnd` (deathmatch) play when the results arrive and the music
switches to the `Results` track; the 3D world stays behind a darkened vignette, the viewmodel
and HUD are hidden.

## Scoreboard

`ModeEvent::Scoreboard(Scoreboard { rules, phase, start_tick, checkpoint, rows: [ScoreEntry
{ player, kills, deaths, frags, rank, best_rank, style, life }] })` is broadcast when something
on it changes (joins / leaves, kills, deaths, rank changes, checkpoint, life state, phase), at most
every `scoreboard_interval` (0.25 s). Tab columns (`scoreboard::columns_for`): deathmatch FRAGS,
KILLS, DEATHS, RANK, STYLE; campaign KILLS, DEATHS, RANK, STYLE; then the lobby's PING and STATUS
(ALIVE / DOWNED / DEAD / BOT / HOST). `InGame::score_rows` fills the cells
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

## Tests and tools

```text
cargo test -p shared --test style      # style math + scoring real combat in the sim (15)
cargo test -p shared --test modes      # checkpoints, solo / co-op deaths, revive, wipe, exit, DM, grades (25)
cargo test -p server --test modes_net  # over UDP with real clients (5)
cargo run -p client --release --example style_shots -- out/   # PNGs of the HUD / results / co-op revive
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
* `modes_net.rs`: style events reach only the killer, `OwnState` carries the meter, everyone's
  scoreboard shows the frag and rank; a frag limit ends the match into the Results phase on every
  client; ending from the lobby flow builds results; a client holding USE revives a downed
  teammate (both clients see DOWNED, Reviving, Revived); the campaign exit sends graded results.
* `style_shots`: the style meter at DULL / FERAL / MERCILESS (STALE) / SEARING (grouped
  multikill feed) / VELOCIDE, the rank-up banner, CHECKPOINT, the deathmatch HUD and Tab
  scoreboard, solo / perfect / co-op / deathmatch results mid-reveal and finished, and a real
  two-client co-op revive (teammate marker with the prompt, the downed panel, both progress bars,
  REVIVED).

## Known limitations / next steps

* Downed players can look around but don't crawl (crawling would need predicted downed movement
  on the client); the third-person downed pose comes from the models.
* Players who leave mid-match drop off the results.
