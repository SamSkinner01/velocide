# Expansion: loadouts, builds, new modes, play of the game, warm-up lobby, cosmetics, content

The third big content drop. This file is the **contract** between the areas building it in
parallel: names, wire ids, message shapes, who owns which files, and the rules everyone keeps.
Numbers are starting values (tuned in play); names, ids, slots, messages and behaviour described
here are the contract. If an implementation has to deviate, update this file in the same commit
and say why.

What the player asked for, and where it lands:

| request | area |
|---|---|
| Play of the game at the end of PvP matches | **potg** |
| Kill distance in the kill feed | **potg** |
| Different balance for PvP and PvE (co-op included), per gun | **arsenal** |
| Loadout system: choose your weapons | **arsenal** (sim, server) + **arsenal-client** (UI) |
| Customisable builds: base weapon stats fixed, abilities chosen per weapon type | **arsenal** + **arsenal-client** |
| Bring back the old weapons (Lancet, Thresher, Riveter, Cinderbox), copyright-safe | **arsenal** + **arsenal-client** |
| A bow | **arsenal** + **arsenal-client** (Longwick) |
| Remove the Thorn (Q) from PvP | **arsenal** (PvP balance profile) |
| Warm-up area in the lobby to run around in | **lobby** |
| Character customisation | **lobby** (cosmetics) |
| Difficulty setting for co-op / PvE, harder with more players | **modes** + **enemies** |
| Grind mode: endless waves until everyone dies | **modes** |
| Gun game | **modes** |
| More of everything: weapons, maps, enemies | everyone; **maps**, **enemies** |

Already on main (the groundwork commit that adds this file): `shared/src/loadout.rs` (weapon
kinds, classes, abilities, builds, loadouts), `shared/src/cosmetics.rs`,
`GameMode::{GunGame, Grind}` + `is_pvp()` / `is_pve()` / `has_difficulty()`, `Difficulty` +
`MatchSettings::difficulty`, `LevelKind::Hub`, `LevelDef::grind_spawns`, `KillInfo::distance`,
`EnemyScale` + `SimWorld::enemy_scale`, `ClientMessage::{SetLoadout, SetCosmetics}` (the server
ignores them until their areas land), `PROTOCOL_VERSION = 8`. Gun game and grind run on the
deathmatch / campaign rules until **modes** gives them their own.

## Design rules (unchanged from `docs/roster.md`, they matter more now)

- Everything must stay **original**. No revolver + coin, no shotgun with a parryable grenade,
  no nailgun with saw discs, no plain rocket launcher; no orb-throwing humanoid, no hover drone
  with bolt bursts, no armoured brute whose only trick is an overhead slam, no floating face with
  a beam, no teleporting caster with homing orbs, no shield-granting idol, no sword-and-shotgun
  humanoid. Archetypes (a sidearm, a shotgun, a bow, a flyer, a heavy) are fine; their
  *signature* (silhouette, mechanic combo, name) must be ours.
- The four returning weapons keep their old names (they were always ours) but get **new
  silhouettes and new primaries**: the Lancet is a needle pistol (no cylinder, no hammer); the
  Thresher is a drum-fed auto-scattergun (no pump); the Riveter drives hot rivets (no discs, no
  magnets); the Cinderbox is a box launcher firing a three-rocket salvo. Their old alt modes do
  not come back as a fixed set: they join the shared ability pool (minus anything signature-y).
- Every enemy attack has a readable wind-up. Enemies are big (≥ 2 m), high-contrast weak points.
- Determinism: everything `step_weapons` decides is integer-tick and predictable by the owning
  client. The server stays authoritative for everything else.
- Quality bar for every branch: `cargo test --workspace` green, `cargo clippy --workspace
  --all-targets` with **zero** warnings, `cargo fmt --all`, `cargo run -p client --example
  level_gen -- --check` clean if you touched levels, `cargo run -p client --example models_gen`
  leaves assets unchanged if you touched models. No `unwrap()` on anything network or file
  driven. Match the surrounding code's comment density and idiom.

## Arsenal (area **arsenal**: sim + server; **arsenal-client**: everything you see and hear)

### Loadouts and builds (`shared/src/loadout.rs`, already on main)

- A **loadout** is up to 4 builds on the slot keys 1–4 (`PlayerInput::weapon` = slot index,
  unchanged). Empty slots are skipped by switching. No weapon twice; at least one weapon.
- A **build** = a base weapon (`WeaponKind`, fixed stats and primary) + two abilities
  (`Ability`) from the pool of the weapon's `WeaponClass`. `PlayerInput::weapon_mode` 0 / 1
  picks ability A / B (pressing the slot key again cycles, unchanged). `ALT_FIRE` uses it.
- Ids (`WeaponKind as u8`, `Ability as u8`) are wire ids: append only.
- The client sends `ClientMessage::SetLoadout` on connect and whenever the player changes it.
  The server sanitises it (`Loadout::sanitized`) and stores it per player. It takes effect:
  immediately while the player is in the warm-up lobby or dead; otherwise at the player's next
  (re)spawn (campaign checkpoint respawns included). Gun game overrides it (below).
- API **arsenal** provides (others call it, nobody else reimplements it):
  ```rust
  impl SimWorld {
      /// Stores the player's chosen loadout (sanitised); applies it per the rules above.
      pub fn request_loadout(&mut self, id: PlayerId, loadout: Loadout);
      /// Forces a loadout now (gun game tier changes): replaces the weapons immediately,
      /// selects the first slot, keeps nothing of the old weapons' state. `None` lifts the
      /// override (the requested loadout applies at the next spawn).
      pub fn force_loadout(&mut self, id: PlayerId, loadout: Option<Loadout>);
      /// The loadout the player is carrying right now.
      pub fn loadout(&self, id: PlayerId) -> Loadout;
  }
  ```

### Weapons (10)

| id | weapon | class | primary (starting values) |
|---|---|---|---|
| 0 | **Quill** | Precision | unchanged (24 dmg hitscan, ricochets off Thorns) |
| 1 | **Bellows** | Scatter | unchanged (12 × 8 dmg, 8°) |
| 2 | **Arclash** | Rapid | unchanged (chaining zaps) |
| 3 | **Tumbler** | Explosive | unchanged (bouncing bombs) |
| 4 | **Lancet** | Precision | needle pistol: 14 dmg hitscan, 0.15 s, slight bloom when spammed |
| 5 | **Thresher** | Scatter | drum auto-scattergun: 7 × 7 dmg, 11°, 0.42 s, holds to auto-fire |
| 6 | **Riveter** | Rapid | hot rivets: fast projectiles (90 m/s, light gravity), 7 dmg, 11/s |
| 7 | **Cinderbox** | Explosive | salvo: 3 small rockets 0.08 s apart, r 2.6 m, 32 dmg each, 1.3 s |
| 8 | **Longwick** (bow) | Draw | hold to draw (0.75 s to full), release: taper arrow with gravity, 45–120 m/s, 22–105 dmg by draw, weak point ×2; un-drawn tap = weak quick arrow |
| 9 | **Kiln** | Rapid | ember sprayer: 13 m 16° cone, every 0.08 s 3 dmg + **Burn**; no ricochet |

**Status effects** (new, **arsenal** owns them; replicated as flags):
- **Burn**: damage over time (start 4 dps per stack, up to 3 stacks, 3 s, refreshed by
  re-application). Applies to enemies and (PvP) players. Ember-coloured flicker on the target.
- **Pinned**: a Pinning Arrow roots an enemy for 1.4 s (heavies: 0.6 s) / slows a player 50 %
  for 1.0 s.

### Abilities (19, pools per class)

| class | pool (menu order) |
|---|---|
| Precision (Quill, Lancet) | Brand, Splinter, **Skewer**, **Flurry** |
| Scatter (Bellows, Thresher) | Gust, Both Barrels, **Slug**, **Flak** |
| Rapid (Arclash, Riveter, Kiln) | Wisp, Discharge, **Volley**, **Flare** |
| Explosive (Tumbler, Cinderbox) | Breach, Gravewell, **Limpet**, **Mortar** |
| Draw (Longwick) | Brand, **Triple Nock**, **Pinning Arrow**, **Ember Arrow** |

Existing abilities keep their rules (`docs/roster.md`); when an ability sits on another weapon
of its class it behaves the same (e.g. Splinter on the Lancet uses the Lancet's damage scale:
**arsenal** decides how ability damage relates to the base weapon and documents it). New ones:

- **Skewer** — hold to charge (≤ 0.8 s), release: hitscan that pierces up to 3 targets
  (damage 30–90 by charge, ×0.8 per target passed). Uses `charge` like Discharge.
- **Flurry** — 3 shots 0.06 s apart at 0.8× primary damage, tiny spread. 1.6 s cooldown.
- **Slug** — one hitscan slug, 55 dmg, perfectly accurate, 1.2 s cooldown.
- **Flak** — a shell (projectile) that airbursts at 14 m (or on contact) into a 10-pellet ring
  perpendicular to its flight, 9 dmg each. Not parryable. 1.8 s cooldown.
- **Volley** — dumps 10 shots of the weapon's primary at once in a 6° cone, 2.2 s cooldown.
- **Flare** — a slow arcing flare; where it lands, a 3 m burning patch for 4 s (Burn to
  hostiles inside). 3.0 s cooldown.
- **Limpet** — throws a sticky charge (sticks to world, enemies, players); up to 3 live; using
  Limpet again while charges are out detonates them all (r 3.5 m, 50 dmg). Owner-replicated.
- **Mortar** — lobbed heavy shell (+35° above aim), r 6 m, 85 dmg, big knockback, 3.0 s cooldown.
- **Triple Nock** — 3 arrows in a 7° fan at the current draw (needs ≥ 30 % draw), 1.4 s cooldown.
- **Pinning Arrow** — next release fires a pinning arrow (Pinned status), 2.5 s cooldown.
- **Ember Arrow** — next release fires an arrow that bursts on impact (r 3 m, 30 dmg + Burn).

Bow abilities change the *next release* rather than firing on their own: pressing `ALT_FIRE`
arms them (HUD shows it); the following primary release uses it. **arsenal** may adjust this if
it plays badly, documenting why.

As built (milestone 1, details in `docs/arsenal.md`): pressing `ALT_FIRE` again disarms; the
cooldown starts at the release. A request also applies at once to a body that hasn't processed
an input yet (a player joining mid-match sends `SetLoadout` a moment after spawning). Requests
live in `SimWorld::loadout_requests` (the server carries them into every new world), forced
loadouts in `SimWorld::forced_loadouts`; the warm-up flag is `SimWorld::warmup` (**lobby** sets
it). An ability is one `Attack` on every weapon, so `Attack::weapon()` is `None` for abilities
several weapons can carry: **`KillInfo::weapon`** (new field) names the weapon that fired the
killing blow — use it for the kill feed's weapon and gun-game tiers.

As built (milestone 2): **Limpet** — a tap throws a charge, holding the ability 0.25 s
detonates every charge out, and a throw with 3 already out detonates them instead. Read
literally ("using Limpet again while charges are out detonates them all") a second use would
always detonate the first charge, so "up to 3 live" could never happen; tap / hold keeps both.
Ability damage on another weapon of the class: × the carrier's `ability_power` / the home
weapon's, never above the listed numbers (`docs/arsenal.md`). Pinned players are slowed through
their predicted `MoveState` (`slow_time`, `slow_scale`).

### Balance per mode (**arsenal**)

`assets/tuning.ron` keeps the weapons' base numbers in `combat` and gains a `balance` section
with two profiles, `pve` (campaign, grind, warm-up for PvE) and `pvp` (deathmatch, gun game),
each a per-weapon and per-ability set of multipliers (damage, fire interval / cooldown, splash
radius, knockback, self-damage...) plus absolute overrides where a multiplier can't express it.
`Tuning::for_mode(GameMode) -> Tuning` returns the effective tuning; the server runs a match
with it and sends **that** to clients (`ServerMessage::Tuning`), so prediction always matches.
Hot reload recomputes it. (As built: the server recomputes it from the world's
`match_settings.mode` in `replace_world` and on reloads; whoever switches the running world's
mode without a new world calls `Server::update_effective_tuning`. Overrides are field paths
inside `combat`, e.g. `"thorn.charges": 0`.) The PvE profile starts at 1.0 everywhere (today's campaign feel is
the reference); the PvP profile is tuned for time-to-kill and counterplay between players (time
to kill in the 0.8–1.6 s band for a skilled player at the weapon's range; one-shots only from
full-draw bow headshots, Slug/Skewer headshots at full charge). Document the reasoning in
`docs/arsenal.md`.

**PvP has no Thorns**: the `pvp` profile sets `thorn.charges = 0`; `THROW` does nothing, the HUD
hides the thorn pips (and the "Q" hint) when charges are 0.

### Replication and wire ids (**arsenal** owns, **arsenal-client** reads)

- Player entity `state`: **arsenal** repacks it to carry the held `WeaponKind` id, the selected
  `Ability` id and the charge / draw level (so remote clients draw the right model, the bow's
  draw, the Discharge / Skewer glow). Provide decode helpers in `shared::protocol` (e.g.
  `player_weapon(state) -> (WeaponKind, Ability, u8 charge)`) and use them everywhere.
- `OwnState.combat.weapons` carries the loadout (so the HUD and prediction see it).
- Projectile kinds: **arsenal** allocates 64–71 and 75–79 for new player projectiles (arrows,
  rivets, cinder rockets, flak shells, limpets, mortar shells, flares); 136–139 for hazards
  (burning patches). Document each in `docs/arsenal.md` with its `state` / `aux` / flags.
- Player flags: **arsenal** owns bits 12–14 (`BURNING`, `PINNED`, `DRAWING`); enemy flags:
  **arsenal** owns `BURNING` / `PINNED` in bits 13–14; **enemies** owns bits 8–12.
- `Attack` gains a variant per new primary / ability; `Attack::weapon()` returns the
  `WeaponKind`. Kill feed names come from `Attack::name()`.
- Style freshness keys cover every weapon / ability (**arsenal** owns `sim/style.rs`).

## Modes (area **modes**)

### Difficulty and player scaling

- `MatchSettings::difficulty` (Easy, Normal, Hard, Brutal, Inferno), chosen by the host in the
  lobby (campaign and grind only), shown in the lobby summary, `--difficulty` on the dedicated
  server and in the single-player menu.
- **modes** computes from difficulty × player count: enemy health (at spawn, via the existing
  encounter scaling), spawn counts, `SimWorld::enemy_scale` (damage, aggression, projectile
  speed; **enemies** applies those inside the AI), elite chance (**enemies** provides elites),
  and co-op rules (downed time, revive health). Recompute when players join or leave.
- The player said co-op is too easy with several players: co-op scaling must get **stronger**
  than today (today: health × (1 + 0.5 (p − 1)), count +50 % per extra player). Starting point:
  per extra player health +0.6, count +0.6, aggression +0.08; Normal for 4 players should feel
  like a real fight. Numbers live in `tuning.ron` (`difficulty` section).

### Grind (`GameMode::Grind`)

- Arenas only (every arena is a grind arena; `LevelDef::grind_spawns` mark where Moulds appear,
  else nav points away from players). 1–4 players, co-op rules (downed / revive).
- Endless numbered waves. Wave budget grows with the wave number, scaled by difficulty and
  player count. Enemy types unlock as waves climb (Gnawers first, Gaolers / Herons / the new
  Moulds later). Every 5th wave is a heavy wave with elites; every 10th has a boss-grade elite.
- Short breather between waves (~6 s, "WAVE 7 IN 3..."). Players who died (not revived) come back
  at the start of the next wave. **The run ends when everyone is down or dead at once.**
- Results: wave reached, kills, time, best style rank, per-player stats. The client remembers the
  best wave per arena and difficulty locally ("BEST: WAVE 23").
- HUD: wave number, enemies left, breather countdown.

### Gun game (`GameMode::GunGame`)

- Arenas, PvP free-for-all, up to 8 players. A fixed ladder of builds (all ten weapons, one
  tier each, with fixed abilities) and finally the **Fist**. A kill with your current tier moves
  you up a tier (via `force_loadout`); a kill with the Fist on the last tier wins. A punch kill
  on someone (any tier) knocks the victim **down** a tier. Thorns are off (PvP profile).
- Respawns are quick (deathmatch respawn rules). A time limit still applies (most tiers wins).
- *As built*: the Fist tier is **fists only**: `sim::gungame::tier_loadout` forces
  `Loadout::unarmed()` (no weapon; `force_loadout` keeps it, requests from clients are still
  sanitised to at least one weapon). Unarmed, `step_weapons` fires nothing but the fist (and
  the thorn, per tuning), `WeaponState::is_unarmed()` / `held() == None` tell the owner, and
  the player entity's `state` carries the sentinel weapon id `UNARMED_WEAPON_ID` (15):
  `protocol::player_armed(state)` is false (`player_weapon` keeps its signature and decodes
  the defaults then). Drawing the fists-only viewmodel / third-person pose is
  **arsenal-client**'s.
- HUD: your tier, the next weapon, the leader's tier. Results: winner, tiers reached.

**modes** owns `shared/src/sim/modes.rs`, `shared/src/sim/encounter.rs`, new
`shared/src/sim/{grind,gungame,difficulty}.rs`, the lobby model (`shared/src/lobby.rs`),
`ModeEvent`, the `modes` / `difficulty` / `grind` / `gun_game` sections of the tuning, the
scoreboard, mode HUD and results screens for the new modes, `server/src/main.rs` flags, the
host / single-player mode and difficulty pickers. Gun game needs **arsenal**'s `force_loadout`:
build difficulty and grind first; start gun game once main has the arsenal merge (`git merge
main`), or report and wait to be resumed.

## Play of the game and kill distance (area **potg**)

- **Kill distance**: every kill-feed line with a killer that has a body shows the distance
  (`KillInfo::distance`, already filled in): `YOU [LONGWICK] GNAWER 38m HEADSHOT`. Enemy kills of
  players too. A **LONGSHOT** style bonus for player kills at ≥ 40 m with hitscan / arrows is
  welcome (add the variant at the end of `StyleBonus`; tell **arsenal** nothing, it's additive).
- **Play of the game** (PvP modes only: `rules.kind.is_pvp()`): the server scores highlights from
  kills as they happen (`shared/src/sim/highlight.rs`, pure and unit-tested): a highlight is one
  player's burst of kills (≤ ~10 s), scored by kills, multi-kill chains, headshots, airborne,
  distance, parried projectiles, ricochets. When a new best highlight closes (its last kill is
  ≥ 2 s old) the server broadcasts `GameEvent::HighlightMark(Highlight { player, start_tick,
  end_tick, score, title })`; at match end it sends `GameEvent::PlayOfTheGame(Highlight)` before
  the results.
- Clients **record** what they receive (every client gets every entity: snapshots aren't culled):
  a rolling ~20 s buffer of decoded entity frames + game events + fx, and copy the marked range
  out when a mark arrives. At match end: a "PLAY OF THE GAME" title card (player name, title like
  "TRIPLE KILL" / "LONGSHOT 87m"), then the clip replayed in first person from that player's eyes
  (their replicated pose and weapon), with the kill feed and sounds; skippable; then the normal
  results. A client without the clip (joined late) shows the card only.
- **potg** owns `shared/src/sim/highlight.rs`, the two `GameEvent` variants, the server hook,
  `client/src/ingame/replay.rs` (new) and the kill feed line format; small hooks in
  `match_flow.rs` / `results.rs` (coordinate: those files are **modes**' too; keep hunks small).

## Warm-up lobby and cosmetics (area **lobby**)

- **Warm-up**: while the server is in its lobby phase it runs a world on the hub level
  **Vestry** (`vestry`, `LevelKind::Hub`, built by **lobby**: movement course, firing range with
  Effigy dummies, a console that spawns practice Moulds, a jump-pad tower, mirrors of the arenas'
  tricks). Players spawn there and run around; death → respawn in 1 s; no scores, no results.
  Players hurt each other only if the *selected* next mode is PvP; otherwise only knockback. The
  warm-up uses the balance profile of the selected mode. The lobby panel (players, ready, chat,
  host settings, start) becomes an overlay you open over the warm-up (and the old screen stays
  usable without a world, e.g. while connecting); ready-up has a key. The countdown → match →
  results → back to the warm-up flow keeps working, including dedicated `--autostart`, bots and
  joining mid-match. Loadout changes apply instantly in warm-up.
- **Cosmetics** (`shared/src/cosmetics.rs`, already on main): mask, crest, cloak, emblem, primary
  and secondary colour, wick flame colour. Saved in the client settings, sent with
  `ClientMessage::SetCosmetics` (and on connect), sanitised by the server, shared via
  `PlayerInfo` (and the lobby player list). Drawn on remote players, in the lobby list /
  podium, the results screen and the play-of-the-game card. A **Customise** screen (main menu
  and lobby) with a turning 3D preview of your Shrike. Cosmetics never change hit shapes.
- **lobby** owns `server/src/lobby.rs`, `client/src/menus/lobby.rs`, the screen flow in
  `client/src/app.rs`, `client/src/models/shrike.rs`, `client/src/ingame/remote_players.rs`, new
  `client/src/menus/customize.rs`, `PlayerInfo` / `LobbyPlayer` fields, and `docs/lobby.md`.

## Maps (area **maps**)

New levels, generated in code like the others (`client/examples/level_gen/*.rs`):

| level | file | kind | notes |
|---|---|---|---|
| **Tallow Works** | `tallow_works` | Arena | rendering vats, conveyor catwalks, hoist chains; mid-size, vertical |
| **Wickmarket** | `wickmarket` | Arena | open candle market, stalls and awnings, a bell gallows; long sightlines for the bow |
| **Furnace Ring** | `furnace_ring` | Arena | big round tiered pit built for grind (many `grind_spawns`), fine for PvP too |
| **The Drowned Chandlery** | `drowned_chandlery` | Campaign | flooded candle factory; 4+ encounters using the new Moulds; par time, checkpoints, secrets |

Add `grind_spawns` to **every** arena (the three existing ones too). The hub (Vestry) belongs to
**lobby**. **maps** owns `client/examples/level_gen.rs` (main list), the new level modules,
`assets/levels/*.ron` for its levels, `shared/tests/levels.rs`, `docs/levels.md`, and
`shared/src/level.rs` (format additions). Until **enemies** merges, place the new Moulds by name
(`bellwether`, `snuffer`, `smelter`) and keep the level loadable without them only if the loader
needs it; coordinate through this file.

## Enemies (area **enemies**)

New Moulds (net kinds 25–28; `assets/enemies/*.ron`, models, sounds, AI):

| kind | Mould | role |
|---|---|---|
| 25 | **Bellwether** | support: a hunched bell-ringer (~3 m). Every few seconds it rings (long readable wind-up, the bell glows): nearby Moulds are *frenzied* (faster, harder-hitting) for a few seconds. Shooting the bell mid-swing staggers it. Priority target. |
| 26 | **Snuffer** | skitterer with a snuffer-cap head (~2 m long): leaps at a player and latches on, smothering the wick (hard damage over time) until punched or dashed off; weak point under the cap. |
| 27 | **Smelter** | slow heavy (~3.4 m) dragging a crucible: pours molten slag lines on the ground ahead of players (glowing telegraph, lingering hazard); hatch weak point on the crucible. |
| 28 | **Effigy** | training dummy for the warm-up (no attacks, regenerates, shows hits); never in encounters. |

- **Elites**: any Mould can spawn as an elite (more health, a searing glow, slightly faster
  attacks); a replicated enemy flag bit. API for **modes**: a spawn function taking an `elite`
  flag (and health scale).
- Apply `SimWorld::enemy_scale` (damage, aggression = cooldown / recovery ÷, projectile speed)
  everywhere enemies attack; frenzy stacks on top.
- **enemies** owns `shared/src/sim/enemy/*`, `assets/enemies/*`, `client/src/models/enemies/*`,
  `client/src/ingame/combat/enemy_fx.rs`, enemy kinds 25–31 and enemy projectile kinds 82–95,
  hazard kinds 140–143, enemy flag bits 8–12, `docs/enemies.md`.

## Shared files (several areas touch them; keep hunks small and local)

`shared/src/sim/mod.rs`, `shared/src/protocol/{mod,snapshot}.rs`, `shared/src/tuning.rs`,
`assets/tuning.ron`, `engine/src/audio/sfx.rs` (new `Sfx` variants: append yours in a block with
a comment naming your area), `client/src/ingame/mod.rs`, `client/src/settings.rs`,
`client/src/menus/main_menu.rs`, `client/src/menus/{lobby,results}.rs`, `server/src/lib.rs`.
Add, don't reorganise. `PROTOCOL_VERSION` is already 8; nobody bumps it again.

## Process

- Each area works in its own worktree branch and commits there; the coordinator merges into
  main. Don't merge into main yourself. When the coordinator tells you main has something you
  need, `git merge main` into your branch and fix it up.
- Keep main's tests passing on your branch at every commit you report.
- Each area writes its own doc (`docs/arsenal.md`, `docs/enemies.md`, `docs/levels.md`,
  `docs/lobby.md`, `docs/style_modes.md`, `docs/potg.md`) and adds screenshots / example updates
  where the area already has them (`client/examples/*_shots.rs`). README / DESIGN are updated by
  the coordinator at the end.
