# Play of the game and kill distance

At the end of every PvP match (deathmatch, gun game: `GameMode::is_pvp()`) the game shows the
**play of the game**: a title card with the player's name and what they did ("TRIPLE KILL",
"LONGSHOT 87m", "AIRBORNE DOUBLE"), then that moment replayed **in first person from their
eyes**, with the kill feed, every sound and a slow motion on the last kill, then the results.
And every kill-feed line shows how far the kill was: `YOU [QUILL] GNAWER 38m HEADSHOT`.

| where | what |
|---|---|
| `shared::sim::highlight` | `Highlight`, `HighlightRules`, `HighlightTracker`, `KillNote`, `burst_score`, `burst_title` (pure, unit-tested) |
| `shared::sim::style` | `StyleBonus::Longshot`, `is_longshot` (shared by the style bonus and the highlight titles) |
| `shared::protocol` | `GameEvent::HighlightMark(Highlight)`, `GameEvent::PlayOfTheGame(Highlight)` |
| `server::highlights` | `Highlights` (the match's tracker), `Server::update_highlights` (called first in `Server::send_mode_events`) |
| `client::ingame::replay` | `Recorder`, `Clip`, `PotgClient`, `Playback`, the title card (`draw_card`) and replay HUD (`draw_replay_hud`), the `impl InGame` glue |
| `client::ingame::combat` | `kill_line` / `kill_line_text` / `kill_distance` (the feed line format) |
| `engine::audio::Sfx` | `PotgStinger`, `PotgWhoosh`, `PotgSlowmo` |

## Kill distance and LONGSHOT

`KillInfo::distance` is metres from the killer (player or Mould) to the victim when it died, 0
when the killer has no body (the world, a dev command, gone). The kill feed
(`combat::kill_line_text`) prints the whole metres after the victim whenever the killer has a
body and it wasn't a suicide (at least `1m`), then the existing tags, single-spaced so the line
stays short at 640×360. The bracket names the weapon (`KillInfo::weapon`): its name for a
primary, weapon and ability for an ability — several weapons share one
(`combat::weapon_attack_label`); the fist, thorns, slams, Moulds and the world keep their
attack's name:

```text
YOU [QUILL] GNAWER 38m HEADSHOT
YOU [RIVETER VOLLEY] BOB 22m
GNAWER [BITE] YOU 1m
RIVET_WITCH [LONGWICK] YOU 62m HEADSHOT RICOCHET x2 AIR
THE PIT [WORLD] BOB
```

**LONGSHOT** (+50 style, `style.longshot`) is a player's kill from `style.longshot_distance`
(40 m) or farther with a hitscan shot or a non-explosive projectile of their own (arrows,
rivets); explosions and returned (parried / deflected) projectiles don't count
(`style::is_longshot`). PvE and PvP alike. The highlight titles use the same rule and
threshold (the server copies `longshot_distance` into the tracker's rules every tick).

## Highlights (server, PvP only)

A **burst** is one player's run of kills where each kill comes within `chain_gap` (3 s) of the
previous one, spanning at most `max_span` (10 s). It **closes** once its last kill is `chain_gap`
old (nothing can join it any more) or when the next kill would make it too long (that kill
starts a new burst). Kills by the world and suicides don't count. Parries by the player during
the burst or within `chain_gap` before its first kill count too and pull its start back
("parry and punish").

**Score** (`burst_score`, starting values in `HighlightRules::default`):

| per kill | points |
|---|---|
| base: another player / a Mould | 100 / 25 |
| headshot | +40 |
| killer airborne / victim airborne | +50 / +25 |
| distance | +1.5 per metre beyond 15 m (up to 120 m) |
| LONGSHOT | +50 |
| returned projectile | +90 |
| per thorn ricochet bounce | +45 |
| slam / punch | +50 / +40 |
| close call (killer below 30 health) | +40 |
| killer's style rank at the time | +8 per rank (DULL 0 … VELOCIDE 7) |

plus, for every kill after the first, a tempo bonus of up to 60 (linear from a kill right
after the previous one down to 0 at `chain_gap`), plus 35 per parry; the sum is multiplied by
`1 + 0.35 × (kills − 1)`. So a quick triple kill (≈ 650) beats the flashiest single kill (an
87 m airborne headshot at VELOCIDE ≈ 400), and a tight double beats a slow one.

**Titles** (`burst_title`): several kills → the multi-kill word (DOUBLE, TRIPLE, QUAD, PENTA,
then RAMPAGE xN), sharpened by what *every* kill had in common: AIRBORNE, LONGSHOT, HEADSHOT or
RICOCHET ("AIRBORNE DOUBLE", "HEADSHOT TRIPLE"), else "… KILL" ("TRIPLE KILL"). One kill → what
made it special, by priority: RETURN TO SENDER, RICOCHET (xN), LONGSHOT 87m, SLAM KILL, AIRBORNE
HEADSHOT, AIRKILL, FISTED, PARRY AND PUNISH, CLOSE CALL, HEADSHOT, else FRAG (KILL for a Mould).

**Server** (`server/src/highlights.rs`): `Server::send_mode_events` first calls
`update_highlights` with the tick's events. In PvP modes it feeds every `SimEvent::Killed`
(with the killer's style rank) and `SimEvent::Parried` to the `HighlightTracker`, closes bursts
that are done, and when one beats the best of the match so far broadcasts
`GameEvent::HighlightMark(Highlight { player, start_tick, end_tick, score, title })`. When the
tick's events contain `ModeEvent::Ended` (frag / time limit, or the host ending the match: the
sim's `Ended` goes out with the next tick) it closes every open burst
(`HighlightTracker::finish`) and broadcasts `GameEvent::PlayOfTheGame(best)` **before** the
tick's mode events, so it reaches clients before the results. The play of the game is the best
highlight of a player still in the match (else the best of anyone); no kills → nothing is sent
and the results come straight away. A new match (`ModeState::start_tick` changes: a new match
world, a restart) starts the tracker over; PvE modes never feed it.

Traffic: one `HighlightMark` per new best (a few per match, ~20 bytes each) and one
`PlayOfTheGame`.

## Recording (client)

Every client gets every entity in every snapshot (no interest management), so any client can
replay anyone. During PvP matches (`InGame::match_mode().is_pvp()`) the `Recorder` keeps the
last 20 s (`RECORD_SECS`) of:

* **snapshot frames**: the decoded `Arc<NetFrame>`s of the session's interpolation buffer,
  shared, not copied (`Recorder::record_frames` after each `NetClient::update`);
* **reliable events**: `Killed` and `Parried` with their server ticks (`InGame::potg_event`,
  called for every `NetEvent` before `on_net_event`);
* **cosmetic events** (`Fx`), as `combat_update` drains them;
* **the local player's own actions**, which the server never sends to their owner (it sends
  `Fx::Shot`, `Punch`, `ThornThrown` and `Discharge` to everyone *but* the actor,
  `server::combat::FxLog::collect`): `InGame::potg_record_local_shots` turns the predicted
  shots of the frame's ticks (`CombatClient::pending_shots`, before `combat_after_ticks` plays
  them) into those same `Fx` — hitscan ends traced against the level and the drawn bodies like
  the local tracers — stamped with the server tick the input runs at (`own.tick + (seq −
  own.seq)` from the newest `OwnState`: one input per tick).

Older data is pruned every frame (`SnapshotBuffer::with_retention` / `retain_from`, events and
`Fx` by tick, capped at 1024 / 8192). 20 s of a busy match (two players and 40 projectiles in
every frame, a Bellows shot every snapshot) measure 1.7 MB (`Recorder::approx_bytes`, test
`twenty_seconds_of_a_busy_match_stay_within_a_few_megabytes`, which allows 4 MB); the newest
3 s of frames are shared with the interpolation buffer anyway. Outside PvP the recorder is
empty.

On `HighlightMark` the client cuts a **clip** (`Recorder::cut`): from `LEAD_SECS` (2 s) before
the highlight's start — later if the player was dead / downed / absent in that stretch (just
respawned) or the recording starts later — to `TAIL_SECS` (1.5 s) after its end, or 0.8 s past
the player's death if they die first, never past what is recorded; plus `VIEW_LAG_SECS` more
frames before (below). Frames are shared again; events and `Fx` of the stretch are copied,
sorted by tick. The clip replaces the previous one (marks only ever announce a new best). A
highlight the recording doesn't cover (joined late) gives no clip.

## The play of the game (client)

`GameEvent::PlayOfTheGame(h)` starts the sequence (`InGame::start_potg`); from then on
`InGame::potg_frame` runs the frame after the network update until it is over:

1. **Title card** (3 s, `draw_card`): the end of the match darkens behind slow diagonal slashes
   in the player's colour; at 0.3 s "PLAY OF THE GAME" slams in (6 → 5 → 4 scale pop, white
   flash, `PotgStinger`, a little shake) with an extrusion in the player's colour and a rule
   drawing itself out; the name slides in from the left on a band of the player's colour with
   speed streaks, the title from the right; "SPACE  SKIP" at the bottom; a white wipe into the
   replay. The clip is the marked one when it is of this highlight; otherwise (the burst was
   still open when the match ended) it is cut from the rolling recording when the card ends —
   the recording is pinned from the highlight's lead-in until then, and by then the tail is
   recorded. No clip at all (joined late, recording missing) → the card alone for 3.8 s with
   "REPLAY UNAVAILABLE".
2. **Replay** (`InGame::potg_replay_frame`): the clip's clock (`Playback::tick`, server ticks)
   advances with real time; around the last kill (0.25 s before to 0.6 s after, eased over
   0.15 s) it runs at 0.3× (`replay_speed`, `PotgSlowmo` on entering); animation and particles
   follow the replay speed. The camera is the player's eyes: their replicated entity (position,
   velocity, yaw, pitch, movement flags) sampled at the clip tick drives a fresh `CameraRig`
   (eye height, slide view roll, speed FOV kick, landing dips) and a fresh `Viewmodel` with
   their held weapon (`held_weapon`: `shared::protocol::player_weapon` decodes the weapon,
   the selected ability and the charge / draw level; the model as the third-person Shrike picks
   it, the glow by the ability's place in its class pool), the switch raise and the charge /
   draw glow while `CHARGING` / `DRAWING`. Everything else comes from the clip through
   the game's own drawers — fresh `RemotePlayers` (everyone else incl. the local player, dead /
   downed poses from the flags), `EntityView` (projectiles, embers, thorns, enemies), `fx::play`
   (explosions, ricochets, heals...), `RemoteSounds` (footsteps), and the session's `Effects`,
   `Beams`, `WorldRender` (cleared when the replay starts and when it ends: the match is over).
   * **Timing**: the player is shown at their own tick and everything else `VIEW_LAG_SECS`
     (0.1 s) behind — what a client typically shows of the others — so their shots meet what
     they were aiming at (lag compensation hit them there). Their own projectiles are drawn at
     their present from the muzzle (`EntityFrame::own`), their explosions on time. That is the
     live game's model with the viewpoint as the local player.
   * **First person**: the player's `Fx::Shot` / `Punch` / `ThornThrown` / `Discharge` /
     `SlamShockwave` (recorded or received) play at their present as the local player's own
     do: weapon sound in 2D, viewmodel kick / punch / throw / drum turn, muzzle flash and
     tracers from the viewmodel's muzzle, screen shake.
   * **Events** at the time they are seen: the kill feed (`kill_line`; the player's kills in
     their colour), `KillConfirm` + a kill marker on their kills, `Death` + the death flash and
     the sinking death view if they die, parries.
   * **HUD** (`draw_replay_hud`): thin letterbox bars, the kill feed top right, a crosshair (and
     kill marker), "PLAY OF THE GAME // NAME" with the title on a slanted plate bottom left (the
     player's colour), "SPACE  SKIP". At the end of the clip: a 0.45 s fade to black.
3. **Results**: the results screen starts its reveal (`InGame::restart_results_reveal`); the
   viewmodel and HUD stay hidden as before.

Space, Esc, Enter or gamepad A / B / Start skip the whole sequence (the frame of the press still
draws it, so the press doesn't also skip the results reveal). The cursor stays free, chat and
the pause menu are closed, inputs keep flowing (neutral) as on the results screen. The
`MatchEnd` sound and results music from `ModeEvent::Ended` play under the card; the music is
held at intensity 0. A new level (`LevelLoaded`) or going back to the lobby ends everything.

Server text is shown sanitised (`display_text`: printable ASCII, ≤ 28 characters); clip
lengths are bounded by what was recorded, never by the server's ticks.

## Tests and tools

```text
cargo test -p shared --lib highlight        # scoring, bursts, titles, announcements, finish (9)
cargo test -p client --lib replay           # recorder, clips, slow motion, kill feed lines (11)
cargo test -p server --test potg_net        # over UDP with real clients (3)
cargo run -p client --release --example potg_shots -- out/
```

* `highlight`: a plain frag closes after the gap and is announced once; a quick triple chains
  and beats an 87 m airborne VELOCIDE headshot; slow kills split, a burst stops at 10 s; titles;
  distance / LONGSHOT / ricochet / returned / rank points; suicides and world kills ignored;
  only new bests are announced; `finish` closes open bursts and prefers players still there; a
  parry just before counts and moves the start.
* `replay`: the recording rolls at 20 s and shares the session's frames, a pin holds it; a mark
  cuts the lead-in and tail (frames, events, `Fx` in order, interpolated viewpoint pose, held
  weapon); the lead-in starts after a respawn and the tail ends after a death; no clip without
  the highlight in the recording; 20 s of a busy match stay within a few MB; the slow-motion
  curve; server text sanitised; kill-feed lines with and without a distance and their weapon /
  ability brackets; the viewpoint's own `Fx` play in first person; replicated movement drives
  the camera state.
* `potg_net`: a deathmatch double kill is marked for everyone only after it closes, a later
  single frag isn't, and at the frag limit every client gets `PlayOfTheGame` (the double) before
  `ModeEvent::Ended`; a burst still open at the frag limit (two 50 m frags: LONGSHOT DOUBLE) is
  the play of the game without a mark; a co-op match with a player's kill sends neither.
* `potg_shots`: Cistern Nine against server bots, the server stepped in the same thread on the
  same virtual clock. Match 1: the local player's headshot triple kill (13, 27 and 46 m) ends
  the match → `potg_local_card_slam`, `potg_local_card`, `potg_local_replay_lead` (the targets
  standing), `potg_local_replay_kill` (first kill: kill marker, feed line with the distance),
  `potg_local_replay_final` (the last shot in slow motion: muzzle flash and tracer from the
  viewmodel), `potg_local_replay_after`, `potg_local_results`; also `potg_live_before` and
  `potg_live_feed` (the live kill feed with distances). Match 2: a second, headless client
  ("Rivet Witch") gets a double kill → the same set as `potg_remote_*` from a remote player's
  eyes (their colour, their weapon). Plus `potg_card_only` ("REPLAY UNAVAILABLE").

## Known limitations

* Doors are drawn in their current state during the replay (the level's door states aren't
  recorded; arenas have none today).
* The view lag of the replayed player is assumed (0.1 s), not known: on a very laggy client
  their aim can still lead or trail a fast target a little.
* Hit markers and damage numbers of the replayed player aren't shown (`HitConfirm` / `Damaged`
  only reach their owner); kills get the kill marker.
* Slow motion doesn't slow the sounds down.
* If the host ends a match from the lobby, the results phase arrives a tick before the play of
  the game, so the plain results can flash for a frame before the card.
* Gun game (and other new PvP modes) get the play of the game through `is_pvp()`.
* LONGSHOT keys off the killing blow's flags: damage over time (Burn, when it lands) must not
  carry `HITSCAN` / `PROJECTILE`, or a burning victim running 40 m away would count.
