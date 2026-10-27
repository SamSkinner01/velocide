# Refit: melee, knacks, feel, the loadout screen, readable Moulds, spectating, balance

The fourth content drop. Like `docs/expansion.md`, this file is the **contract** between the
areas building it in parallel: names, wire ids, who owns which files, and the rules everyone
keeps. Numbers are starting values (tuned in play and by **balance**); names, ids, slots and
behaviour described here are the contract. If an implementation has to deviate, update this
file in the same commit and say why.

What the player asked for, and where it lands:

| request | area |
|---|---|
| Gun game's Fist tier is rough: add melee weapons, make the last tier one of them | **arsenal** (sim) + **arsenal-client** |
| Replace the Thorn (Q) with abilities the player picks (make them up) | **arsenal** (sim) + **arsenal-client** + **loadout-ui** (picker) |
| Bigger projectiles: gunplay and parrying should feel responsive, you should *see* them | **feel** |
| Shooting should feel more responsive | **feel** |
| The bow blocks most of the screen and doesn't feel good | **feel** |
| PvE: bigger Mould models, hit shapes larger than the model | **pve** |
| Spectate when dead in PvE | **pve** |
| Loadout screen: a 3D model of the gun, two boxes for the abilities, abilities explained; redesign the page's UX | **loadout-ui** |
| Spend time balancing the weapons | **balance** |

## Design rules (unchanged, `docs/expansion.md`)

- Everything stays **original** (names, silhouettes, mechanic combos). Archetypes are fine;
  someone else's signature is not.
- Determinism: everything `step_weapons` decides is integer-tick and predicted by the owning
  client. The server stays authoritative for everything else.
- Quality bar for every branch: `cargo test --workspace` green, `cargo clippy --workspace
  --all-targets` with **zero** warnings, `cargo fmt --all`, `cargo run -p client --example
  models_gen` leaves assets unchanged unless you meant to change models (then commit the
  regenerated assets), `cargo run -p client --example level_gen -- --check` clean if you touched
  levels. No `unwrap()` on anything network or file driven. Match the surrounding code's
  comment density and idiom.
- Don't grow the per-snapshot own state (`OwnState`): the `net_loopback` delta-compression test
  has a thin margin.
- `PROTOCOL_VERSION` becomes **9** (**arsenal** bumps it once; nobody else touches it).

## Arsenal: melee and knacks (area **arsenal**: `shared/` + `server/`)

### Melee weapons (new class)

`WeaponClass::Melee` (appended to `WeaponClass::ALL`). Two new weapons, ids appended:

| id | weapon | primary (starting values, PvE) |
|---|---|---|
| 10 | **Trimmer** — hooked wick-shears, two blades | fast alternating slashes: every 0.32 s a 3.0 m reach / 70° arc in front of the eye, 30 dmg to up to 3 targets in the arc (nearest first), line of sight from the eye, lag-compensated like hitscan. A swing also **deflects** parryable hostile projectiles inside its arc (as a punch parry would, same reward): the melee identity |
| 11 | **Tamper** — a foundry ramming hammer (the tool that packs sand into moulds) | heavy swing: every 0.85 s a 3.4 m / 100° arc, 75 dmg to everything in it, heavy knockback (kb 16); no deflect |

Hit shapes are inflated by the usual hit assist; melee ignores weak points (no headshots)
unless **arsenal** finds a reason, documented. `PrimaryStyle::Auto` (hold to keep swinging).

Melee ability pool (`WeaponClass::Melee.abilities()`, menu order), ids appended:

| id | ability | starting rule |
|---|---|---|
| 19 | **Lunge** | an impulse along the aim (mostly horizontal, ~16 m/s) and, for the next 0.35 s, the first target your body reaches takes 1.5 × the primary's damage. 3.0 s |
| 20 | **Whirl** | a 360° spin: everything within 3.5 m takes 1.2 × primary damage and is knocked outward. 4.0 s |
| 21 | **Hurl** | throws a spinning blade / hammer-head projectile (30 m/s, light gravity), 1.5 × primary damage on contact. 2.5 s |
| 22 | **Upswing** | a rising strike in the arc: 0.8 × primary damage and a big upward launch on what it hits (and a small hop for you). 3.0 s |

Defaults: Trimmer `[Lunge, Hurl]`, Tamper `[Whirl, Upswing]`. Abilities scale with
`ability_power` like the other classes (home weapons: Lunge / Hurl → Trimmer, Whirl / Upswing
→ Tamper). Every new primary and ability gets an `Attack` variant (appended), a style key, a
kill-feed name, `CombatTuning` sections (`trimmer`, `tamper`, `lunge`, `whirl`, `hurl`,
`upswing`), entries in the balance multiplier table, and PvP numbers inside the PvP
time-to-kill rules (`docs/arsenal.md`).

Remote players and the replay must be able to show a swing: **arsenal** replicates it (a
`SimEvent` / `Fx` per swing, or bits in the player entity — its choice, documented in
`docs/arsenal.md` "Replication and wire ids").

### Gun game: the melee finale

The Fist tier is replaced by a **melee finale**: `gun_game.finale` (a `WeaponBuild`, default
Trimmer `[Lunge, Hurl]`) is the tier after the ladder; it is a forced
`Loadout::single(finale)` like every other tier. A kill **with the finale weapon** (primary or
ability, `KillInfo::weapon`) on the finale tier wins the match (`EndReason::Ladder`). Punch kills
still knock the victim down `punch_demotes` tiers. `Loadout::unarmed` may stay as an API but
gun game no longer hands it out. `docs/style_modes.md` is updated by **arsenal**.

### Knacks (the Q slot)

The Thorn stops being fixed: the Q slot (`Buttons::THROW`, the control is renamed **KNACK**)
holds one **knack** the player picks in the loadout screen. `shared::loadout::Knack`, ids:

| id | knack | starting rule | charges / recharge |
|---|---|---|---|
| 0 | **Thorn** (default) | the relay spike as today (Quill ricochets). **PvE only** | 3 / 3 s |
| 1 | **Kindle Flask** | a lobbed flask; where it lands a 3.5 m kindling pool for 4 s heals Shrikes standing in it 10 HP/s (teammates and you; in PvP only you), through the normal healing rules | 1 / 14 s |
| 2 | **Snare** | a wax trap thrown to the floor; it arms after 0.6 s and the first hostile within 1.8 m is **Pinned** (the Pinning Arrow's status) and takes 15 dmg. Up to 2 out, each lasts 20 s | 2 / 10 s |
| 3 | **Soot Pot** | bursts into a 5 m soot cloud for 7 s: Moulds can't see into, out of or through it (lose their target, a Heron's lock breaks); players see a dense cloud | 1 / 15 s |
| 4 | **Tallow Skin** | for 3 s you take 50 % less damage and no knockback | 1 / 16 s |
| 5 | **Surge** | an instant burst of speed along your look (an impulse like Both Barrels' recoil, ~18 m/s, mostly horizontal) | 2 / 6 s |

- `Loadout` gains `knack: Knack` (`#[serde(default)]`, so saved presets keep loading).
  `Loadout::default()` carries the Thorn. `Knack::{ALL, from_id, id, name, blurb, pvp_allowed}`
  in `shared::loadout`, ids append-only.
- **PvP**: a knack that isn't `pvp_allowed` (the Thorn) is replaced by **Surge** when the
  loadout applies in a PvP world (the stored request keeps the player's pick).
- Wire: the packed loadout `u64` keeps its 4 × 16-bit slots; the knack id rides in the spare
  bit 15 of each slot (4 bits, slot 0 lowest). `WeaponState::{thorns_used, thorn_recharge}`
  become `knack_used: u8, knack_recharge: u16` (same size; charges = the knack's `charges` −
  `knack_used`). `FireIntent::ThrowThorn` becomes `FireIntent::Knack(Knack)`. Gun game keeps
  the requested knack inside its forced loadouts.
- Tuning: `combat.knacks` with a section per knack (`charges`, `recharge_time` + its numbers);
  the Thorn keeps `combat.thorn`. The `thorn.charges: 0` PvP override is replaced by the
  `pvp_allowed` rule.
- New projectile / hazard kinds, statuses and flag bits are **arsenal**'s to allocate and
  document in `docs/arsenal.md` ("Replication and wire ids"); **arsenal-client** draws them.

### As built (arsenal), where it adds to or departs from the above

- Wire: projectile kinds 77 `HURL` (`state` = the carrier's weapon id), 78 `KINDLE_FLASK`,
  79 `SNARE` (lies `STUCK`, projectile flag bit 4 `ARMED`), 96 `SOOT_POT` (77–79 were full);
  hazards 137 `KINDLE_POOL`, 138 `SOOT_CLOUD`; player flag bit 16 `TALLOW`; swings and knack
  uses are events (`Fx::Swing`, `Fx::LungeStruck`, `Fx::Knack`, `Fx::SnareSprung`). Details:
  `docs/arsenal.md`, "Melee and knacks on the wire".
- Gun game API: `gungame::Tier::Fist` became `Tier::Finale(WeaponBuild)`, `fist_tier` became
  `finale_tier`. A punch kill still demotes but no longer wins anything.
- Embers: melee weapons' strikes heal at a new `embers.melee_weapon_heal_fraction` (0.5), not
  the fist's 1.0 (with three targets a swing that made the Trimmer near-immortal).
- Tallow Skin keeps your own blasts' knockback (blast jumps still work).
- PvP: the Tamper's damage × 0.8 (its abilities are multiples of the swing; 1.5 × 75 was a
  body one-shot).
- Client settings: a loadout's text gains a `"Knack: <name>"` entry after the slots (files
  without it load with the Thorn).

### Milestones

1. **API** (merge early; others build on it): `WeaponClass::Melee`, `WeaponKind::{Trimmer,
   Tamper}`, the four melee `Ability`s, `Knack` + `Loadout::knack`, packing, `WeaponState`
   fields, `FireIntent::Knack`, `Attack` variants, tuning sections, `PROTOCOL_VERSION` 9, every
   exhaustive `match` in the workspace (client included) compiling with simple placeholder
   arms, behaviour stubbed if needed. Report it as soon as it is green.
2. **Behaviour**: everything above working, tested (`shared/tests/combat_melee.rs`,
   `shared/tests/combat_knacks.rs`, gun game in `shared/tests/modes.rs`, `server/tests`), docs.

**arsenal** owns `shared/src/loadout.rs`, `shared/src/sim/{weapons,thorn,loadouts,payload,
status,area,hitscan,gungame}.rs`, new sim files (`melee.rs`, `knacks.rs`), `shared/src/sim/
combat.rs` + `style.rs` (new variants), the tuning *structs* for its sections, `server/src/
commands.rs` (`/loadout` learns the knack: `knack:<name>`), `docs/arsenal.md` (new sections),
`docs/style_modes.md` (gun game).

## Arsenal client (area **arsenal-client**: what you see and hear of melee and knacks)

- Viewmodels and third-person models for the **Trimmer** and **Tamper** (procedural like the
  others, original silhouettes), swing animations (the Trimmer alternates sides, the Tamper is a
  big committed arc), whooshes / hit sounds (new `Sfx`), remote players' swings, the replay and
  play of the game.
- Melee ability visuals (Lunge streak, Whirl ring, the Hurl projectile, Upswing).
- Knacks: the HUD's Q slot (icon, name, charges, recharge) replacing the thorn pips; the flask
  and its pool, the snare (armed / sprung), the soot cloud (dense enough to actually hide
  players), Tallow Skin (a waxy screen-edge sheen for you, a sheen on remote players), the Surge
  streak; sounds.
- Gun game: every "Fist tier" string and banner becomes the finale (`FINAL TIER: TRIMMER`),
  results included. Controls: "THROW THORN" → "KNACK" (`client/src/controls.rs`).
- Put new code in new files (`client/src/models/weapons/{trimmer,tamper}.rs`,
  `client/src/ingame/combat/{melee,knacks}.rs`) and keep hunks in shared client files to match
  arms and hook calls; **feel** is reworking `fx.rs`, `entities.rs`, `weapon_feel.rs`,
  `models/projectiles.rs` and the Longwick at the same time.
- Docs: `docs/arsenal.md` (a "Client" section for melee and knacks), `docs/models.md`,
  `docs/audio.md`. Shots in `client/examples/combat_shots.rs`.

## Loadout screen (area **loadout-ui**)

Redesign `client/src/menus/loadout.rs` for usability. Requirements:

- A **3D model of the selected weapon** turning on a card (drawn into the viewmodel pass like
  the CUSTOMISE preview, `menus::customize::draw_preview`; drag to turn), on every screen that
  shows the loadout (main menu, lobby, pause menu, the warm-up overlay).
- **Two ability boxes** (A and B) under / beside it: name, which input fires it, cooldown, and a
  real explanation (what it does, how to use it, key numbers) — not a one-line blurb. Clicking a
  box opens the class's pool with every option explained the same way.
- The four slots as clear cards (key, weapon, its two abilities), a weapon picker grouped by
  class with each weapon explained (role, numbers in PvE / PvP), the **knack (Q)** card and
  picker (from **arsenal**'s milestone 1: `Loadout::knack`, `Knack`; PvE-only knacks marked),
  presets kept but compact.
- Works with mouse, keyboard and gamepad; readable at the default resolution; colour-vision
  palette respected.
- Long explanations live in a new `client/src/menus/loadout_text.rs` (owned by **loadout-ui**),
  written from `docs/arsenal.md` and checked against the code, so they are true. Melee and
  knack texts once **arsenal**'s milestone 1 / 2 is on main.
- Owns `client/src/menus/loadout.rs`, `loadout_text.rs`; small hunks in the callers
  (`menus/{main_menu,lobby}.rs`, `ingame/{pause,warmup}.rs`) to hand the screen the viewmodel
  draw list; `docs/ui.md` (loadout section); shots in `client/examples/lobby_shots.rs` /
  `client_shots.rs`.

## Feel: projectiles, firing, parrying, the bow (area **feel**)

- **Projectiles you can see**: every player and Mould projectile drawn substantially bigger
  (roughly ×2 at least, more for the small fast ones), with a bright core, a glow halo and a
  trail; parryable Mould projectiles read as parryable at a glance (a distinct rim / colour from
  the semantic palette) and visibly flare when they come into punch reach. Hitscan tracers
  thicker and longer-lived.
- **Forgiving projectile hits**: player projectiles test targets with their radius plus a new
  `combat.projectile_hit_pad` (start 0.2 m); world collision keeps the plain radius so shots
  don't snag on edges. **feel** owns this hunk in `shared/src/sim/projectiles.rs` and the
  projectile radius / speed fields of existing weapons in `assets/tuning.ron`.
- **Responsive firing**: audit input → fire → feedback. Own projectiles should appear at the
  muzzle on the frame the shot is predicted (a local stand-in that hands over to the
  server's projectile with no pop or double), plus stronger muzzle flash, viewmodel kick,
  firing sounds and hit confirmation (hit markers / sounds as soon as the server reports a hit).
  Look for added latency (input sampled late, a frame of delay, interpolation of own effects).
- **Parrying**: a clear, satisfying parry (flash, hitstop, sound, the deflected projectile
  visibly fast and bright).
- **The Longwick**: a viewmodel that never covers the centre of the screen (smaller, canted,
  low-left; at full draw the arrow lines up under the crosshair), a clear full-draw cue
  (sound + flash), slight zoom at full draw, bigger brighter arrows with trails, and handling
  numbers that feel good (**feel** owns `longwick.draw_time`, arrow speeds and gravity; damage
  stays **balance**'s).
- Owns `client/src/ingame/combat/{fx,entities,weapon_feel,arsenal}.rs`,
  `client/src/models/projectiles.rs`, `client/src/models/weapons/longwick.rs`,
  `client/src/ingame/{viewmodel,feedback,camera}.rs` (camera: shared with **pve**'s spectate,
  keep hunks small), the projectile hunk above. Doc: `docs/feel.md` (new). Shots in
  `client/examples/combat_shots.rs`.

## PvE: readable Moulds and spectating (area **pve**)

- **Bigger Moulds**: draw every Mould noticeably bigger (around ×1.25, per type as it reads
  best) and make its hit shapes cover **more** than the drawn model: body shapes sized to the
  scaled silhouette plus a bigger hit assist (`enemy_body_pad` 0.2 → ~0.35, `enemy_weak_pad`
  0.1 → ~0.15); weak points re-placed on the scaled models. The movement capsule may stay as it
  is (doors, nav and level tests must stay green) — split hit shapes from the movement capsule
  if needed. Enemy attacks still test the real (un-padded) player shapes.
- **Spectate when dead (PvE)**: while the local player is dead, downed or out (campaign co-op,
  grind) and a teammate is alive, the camera follows a teammate: first person from their eyes by
  default (their held weapon from the replicated state if feasible), JUMP toggles a third-person
  chase camera, FIRE / ALT_FIRE cycle teammates; HUD "SPECTATING NAME" with the controls; the
  revive / respawn countdowns stay visible; audio listener follows the view. Solo (nobody to
  watch) keeps today's death cam.
- Owns `assets/enemies/*.ron`, `client/src/models/enemies/*`, `shared/src/sim/hitbox.rs` (and
  the hit-shape parts of `sim/enemy/*`), the `hit_assist` numbers, a new
  `client/src/ingame/spectate.rs`, hunks in `client/src/ingame/{mod,camera,mode_hud}.rs`.
  Docs: `docs/enemies.md` (sizes, hit shapes), `docs/style_modes.md` (a short "Spectating"
  section). Shots in `client/examples/enemy_shots.rs`.

### As built (pve)

- Enemy definitions gain `model_scale` (the drawn size; the client applies it in
  `ModelLibrary::draw_enemy`), `hit_radius` (the body hit capsule's radius, `None` = the
  movement radius) and `body_shapes` (extra body capsules); `EnemyDef::hitbox(target, feet,
  yaw)` builds every hit shape and `Enemy::hitbox()` (same signature as before) calls it, so
  hitscan, projectiles, explosions and melee pick the new shapes up unchanged. Scales: Harrier
  and Snuffer ×1.3, Gnawer and Effigy ×1.25, Censer, Heron and Bellwether ×1.2, Gaoler and
  Smelter ×1.12 (the heavies read already; bigger crowds their rooms). Numbers and reasons:
  `docs/enemies.md` "Sizes and hit shapes".
- Spectating waits 1.5 s of the death view first (FIRE / ALT_FIRE skip it) and draws your own
  body where you lie; no `camera.rs` change was needed (it owns its own `CameraRig`). Hunks
  outside the listed files: `remote_players.rs` (a hidden player), `replay.rs` (its viewpoint
  helpers made `pub(super)`), `combat/{mod,hud}.rs` (`CombatHudInfo::spectating`: no death
  screen / low-health vignette over the watched view), `combat/enemy_fx.rs` (tracer ends use
  the hit radius; the latched Snuffer keeps its size in the victim's view).

## Balance (area **balance**)

- Build a **balance harness** (`shared/examples/balance_report.rs` or a test module) that
  computes from the effective tuning, per weapon and build, in PvE and PvP: damage per trigger,
  sustained and burst DPS, time to kill per Mould type (body / weak point, per difficulty) and
  vs a PvP player (body / head) at each weapon's effective range, with spread / pellet hit
  fractions against the real hit shapes, projectile travel time, ability cooldown value.
- Use it (and reading the code paths) to rebalance: find outliers (weapons nobody would pick,
  abilities that dominate, dead abilities), tune `combat` numbers and the `balance` profiles in
  `assets/tuning.ron`, keep the PvP one-shot rules and time-to-kill band (or change them with
  reasons), and add PvE bands to the tests so it stays balanced.
- Once **arsenal**'s milestone 2 is on main (the coordinator tells you), balance the melee
  weapons, their abilities and the knacks too.
- Owns the *numbers* in `assets/tuning.ron` (`combat`, `balance`) except **feel**'s projectile
  radius / speed / Longwick handling fields; the PvP balance test in
  `shared/tests/combat_arsenal.rs`; a new `docs/balance.md` (method, the harness's tables, every
  change with its reason) and the numbers in `docs/arsenal.md`'s "Balance profiles" tables.

## Shared files (several areas touch them; keep hunks small and local)

`assets/tuning.ron`, `shared/src/tuning.rs` (+ `tuning/balance.rs`), `shared/src/sim/mod.rs`,
`shared/src/protocol/*`, `engine/src/audio/sfx.rs` (append new `Sfx` variants in a block with a
comment naming your area), `client/src/ingame/mod.rs`, `client/src/ingame/combat/{mod,hud}.rs`,
`client/src/models/{mod,library}.rs`, `client/src/models/weapons/mod.rs`, `client/src/settings.rs`,
`client/src/menus/*`, `server/src/lib.rs`, `docs/arsenal.md`. Add, don't reorganise.

## Process

- Each area works in its own worktree branch and commits there; the coordinator merges into
  main. Don't merge into main yourself. When the coordinator tells you main has something you
  need, `git merge main` into your branch and fix it up.
- Keep main's tests passing on your branch at every commit you report.
- README / DESIGN are updated by the coordinator at the end.
