# Feel: projectiles you can see, shots that answer, parries that land, a bow that fits

The refit's **feel** area (`docs/refit.md`). The player asked for bigger projectiles ("you
should be able to see the projectiles"), gunplay and parrying that feel responsive, shooting
that responds sooner, and a bow that doesn't block the screen. This file says what changed,
the numbers, and what the latency audit found.

## Where it lives

| what | where |
|---|---|
| press-frame shots (`WeaponPredictor::lookahead`, `PredictedShot::played`) | `client/src/ingame/combat/weapon_feel.rs` |
| the click's aim goes to the next tick's input | `client/src/controls.rs` (`Controls::press_aim`) |
| own projectiles from the muzzle at once, handed over to the server's | `client/src/ingame/combat/stand_in.rs` |
| shot playback after the camera update, flashes, tracers, punch sound, view punch | `client/src/ingame/combat/mod.rs` (`combat_after_ticks`), `fx.rs` (`muzzle_flash`, `tracer`), `client/src/ingame/mod.rs` (frame order) |
| projectile scale, halo, core, streak, the parryable ring | `client/src/ingame/combat/entities.rs` (`projectile_scale`, `projectile_look`, `projectile_glow`), `arsenal.rs` (`ArsenalProjectile::scale` / `stuck_scale`) |
| one-frame billboards (halos that follow a projectile without piling up in hitstop) | `client/src/ingame/effects.rs` (`Effects::billboard`) |
| the predicted parry and its feedback | `client/src/ingame/combat/mod.rs` (`combat_predict_parry`, `parry_feedback`) |
| hit markers, the parry-window ring, the full-draw ring | `client/src/ingame/combat/hud.rs` |
| view punch, zoom | `client/src/ingame/camera.rs` |
| recoil curve | `client/src/ingame/viewmodel.rs` |
| the bow's first-person placement | `client/src/models/weapons/longwick.rs` (`VIEW_OFFSET`, `DRAW_SHIFT`, `VIEW_SCALE`, `view_rotation`, `outline_points`), `weapons/mod.rs` (`view_scale`) |
| forgiving projectile hits | `shared/src/sim/projectiles.rs` (`projectile_target_pad`), `combat.projectile_hit_pad` |
| sounds | `engine/src/audio/sfx.rs` (the "feel" block: `ShotPunch`, `ParryImpact`, `FullDrawGlint`, `WeakPointTick`; `docs/audio.md`) |
| shots | `client/examples/combat_shots.rs` (`feel_*.png`) |

## The latency audit

The path from a click to something on screen, frame by frame, as it was:

| step | finding | now |
|---|---|---|
| sampling the button | per frame; presses are latched until the next tick (`Controls::latch`), so a click shorter than a tick still fires. No edge was lost. | unchanged |
| waiting for a tick | inputs are simulated at fixed 60 Hz ticks, and the shot's effects were played after the tick that fired it. A click between two ticks waited for the next one: up to 16.7 ms. Measured (`weapon_feel::tests`, clicks at 48 phases of a tick): **0.75 frames on average at 144 fps, 1.5 frames at 240 fps** (~5–6 ms, up to a tick). | **0 frames**: on a frame with no tick where a fire / punch / throw button changed, the next tick's `step_weapons` runs on a copy of the predicted state and the shot plays on that frame; the tick that really fires it finds it played and doesn't play it twice. The prediction and what is sent are untouched. |
| the aim of that shot | the tick's input carried the view at the tick, a few ms after the click | the next tick's input aims where the crosshair was on the click's frame (`Controls::press_aim`): the shot goes where the press-frame tracer was drawn |
| sending | `NetClient::tick` flushes the input at once | unchanged |
| where the effects start | shot effects ran inside the tick loop, *before* the camera update, so flashes and tracers left the previous frame's muzzle (0.33 m behind at 20 m/s and 60 fps), and a 60 ms flash stayed in the world while the gun moved on | shots play after the camera update (this frame's muzzle); flash particles carry the shooter's velocity |
| recoil | the kick was applied the same frame, decaying linearly over 0.3 s | still the same frame (now drawn at full strength), decaying exponentially: most of it gone in 0.1 s, so it reads as a snap |
| own projectiles | only drawn once a snapshot carrying them arrived: the input's trip, the server's input buffer and the snapshot's trip back. Measured in single player (`combat_shots`, 34 shots): **39 ms** on average with nothing on screen; online add the round trip (~140 ms at 100 ms ping) | a local stand-in leaves the muzzle on the shot's frame and hands over to the server's projectile (below) |
| parry feedback | waited for the server's `Parried`: the punch's trip, the parry delay window and the event's trip back | shown on the punch's frame (below) |
| hit confirmation | `HitConfirm` is sent reliably on the tick of the hit and handled at the start of the next client frame: a round trip, nothing added on top | unchanged (it can't be sooner without guessing hits); markers and sounds are bigger and more distinct |
| audio | `play_2d` at once; the mixer's own buffer is the engine's | unchanged |

## Responsive firing

* **Press-frame shots** (`WeaponPredictor::lookahead`): see the audit. Tests:
  `a_click_between_ticks_shows_on_its_own_frame` (every phase, 144 and 240 fps: shown on the
  click's frame, exactly once) and `a_lookahead_that_does_not_happen_is_played_by_its_tick`.
* **Stand-ins** (`stand_in.rs`): every predicted intent that launches projectiles (bombs,
  rivets, rockets, flak shells, wisps, flares, gravewells, limpets, mortar shells, every arrow,
  Triple Nock's fan, the Riveter's Volley) spawns local stand-ins mirroring
  `SimWorld::resolve_intent` (launch point, speed, lob angle, gravity; spread the client can't
  know is approximated). They are drawn by the same code as the real projectiles, from the drawn
  muzzle blending onto the eye-based path over 0.15 s. They stop at the level (bombs bounce,
  arrows / limpets / gravewells stick, the rest vanish) and inside the bodies they reach. When
  the server's projectile first appears among our own, the closest stand-in of its kind (launch
  point and direction) hands over: the real one is drawn where the stand-in was and eases onto
  its own path over 0.3 s (a time offset — the input delay, at most 15 ticks — and a position
  offset, both fading), the stand-in is removed and its trail state carries over. An unclaimed
  stand-in fades after 1.5 × RTT + 0.25 s (0.3–1.2 s). In the `combat_shots` feel pass all 34
  stand-ins were handed over, none expired. Test: `a_stand_in_hands_over_without_a_pop_or_a_double`
  (one rivet drawn every frame; no frame step bigger than its normal flight per frame).
* **Muzzle flash** (`fx::muzzle_flash`, own and remote shots): a star flash (0.85 × the
  attack's flash size, which grew from `0.25 + 0.3 kick` to `0.35 + 0.45 kick` m), a tongue
  of flame ahead of it, a white-hot core, three sparks thrown forward, and a light of radius
  6.5 m for 0.08 s (was 5 m, 0.05 s).
* **Tracers** (`fx::tracer`): every hitscan tracer is roughly 1.7× wider and lives ~1.8×
  longer, with a faint glow 2.8× wider around its core (Quill 0.03 → 0.055 m, 0.1 → 0.2 s;
  Bellows pellets 0.022 → 0.036 m, 0.07 → 0.13 s; Slug 0.06 → 0.1 m; Skewer 0.05 → 0.09 m,
  0.3 → 0.45 s; Arclash arcs 0.03 → 0.042 m). Own tracers stay thin for their first 3 m.
* **Kick**: a view punch (`CameraRig::punch`, kick² × 0.012 rad, a 40 rad/s critically damped
  spring, capped at 0.03 rad; the aim is untouched), stronger recoil on the Quill, Lancet and
  Riveter, and the exponential recoil curve.
* **Sound**: `ShotPunch` under every shot with a kick ≥ 0.3 (volume 0.4 + 0.6 kick).

## Projectiles you can see

* **Size**: every projectile model is drawn at least twice life size in flight: ×2 (bombs,
  gravewells, wisps, ember bombs, rockets, mortar shells, flares), ×2.2 arrows, ×2.4 flak shells
  and limpets, ×3.2 rivets, needles ×3 long and ×2.1 thick (head-on a fat needle would hide what
  is behind it). Stuck ones are drawn at ×1.3–1.8. Deflected ones another ×1.25.
* **Glow** (`projectile_glow`, every frame, for every projectile including stand-ins): a halo
  (0.8–1.7 m by kind, never smaller than 2.2 % of its distance, so a few pixels even far off)
  and a white-hot core (0.36 × the halo), both one-frame billboards; and a streak back along its
  velocity covering the last 0.03–0.09 s of flight (never past where it was first seen, ≤ 9 m):
  a bright core plus a wide faint glow. Colours: each kind's own (standard palette); enemy
  projectiles in `palette().hostile_projectile`, deflected ones in `palette().deflected` in the
  colour-vision palettes.
* **Parryable at a glance**: an enemy's parryable projectile (ember bombs, needles) carries a
  turning ring in `palette().parry_window` (yellow in the standard palette, pale blue for
  protanopes / deuteranopes, pale gold for tritanopes: the same colour as a Mould's parry
  strobe), 1.6× its halo, pulsing slowly. When it is within punch reach in front of you
  (`parry_range` inside `parry_cone_degrees`, or `parry_close_range` anywhere in front — the
  server's rule) or will be within 80 ms, the ring tightens, brightens to 2.4× and strobes at
  18 Hz with a white flash in its core: "punch now".
* **Deflected**: white-hot, ×1.25, a 1.6 m halo, a streak of its last 0.09 s, a 7 m light.
* Hitscan tracers: above.

## Forgiving projectile hits

Player-owned projectiles (a parried one included) test targets with their radius plus
`combat.projectile_hit_pad` (0.2 m), on top of the hit assist; the level still uses the plain
radius, so shots don't snag on corners, and enemy projectiles get no pad
(`projectile_target_pad`). A projectile that hit through the pad goes off up to 0.2 m further
from the body, so an explosive's direct hit measures its falloff 0.2 m closer: the pad never
costs a direct hit damage. Tests: `shared/tests/combat_feel.rs` (a graze 0.1 m outside the
plain reach hits with the pad and misses without it; a rivet skimming a wall inside the padded
radius keeps flying; enemy needles get no pad).

## Parrying

* **On the punch's frame**: while the punch's parry window is open (the predicted
  `WeaponState::parry`, or a punch played this frame), the client tests the server's rule on
  what it draws — a parryable enemy projectile at the render tick within `parry_range` inside
  the cone (or `parry_close_range` in front), or a Mould flashing `PARRYABLE` within
  `melee_parry_range` + its size — and shows the parry at once. The server tests the same rule
  at the same view time (`docs/combat.md`, rewound to the puncher's view), so its `Parried`
  only confirms it (within 0.9 s; then only the heal sound); a parry the client didn't see
  coming is shown when `Parried` arrives.
* **The parry itself**: hitstop 0.12 → 0.16 s, flash 0.6 → 0.65, shake 0.15 → 0.28, `Parry`
  layered with `ParryImpact`, a white shock ring bursting from the fist, a 7 m light, sparks
  and a glow where it was struck, the `PARRY` banner.
* **Readable window**: a ring in the parry colour closes on the crosshair while the window
  lasts (0.2 s); the knuckles flare as before.

## Hit confirmation

Markers pop in 3 px further out and snap in over 70 ms, last 0.3 s (kills 0.55 s), body
markers are longer and thicker, weak points longer still; kills draw an expanding ring.
Weak-point hits play `WeakPointTick` with `Headshot`; weak-point kills with `KillConfirm`.
Kill hitstop 0.045 → 0.06 s.

## The Longwick

* **Placement**: drawn at 0.72× → 0.8× of the common viewmodel scale (`VIEW_SCALE` 0.8),
  held low on the left (`VIEW_OFFSET` (−0.14, −0.14, −0.42)), canted 0.62 rad about the arrow's
  own axis so the upper limb leans left, away from the crosshair, and the lower one runs off
  the bottom of the screen; turned so the arrow points at the crosshair. Drawing brings it up
  and in (`DRAW_SHIFT` (0.115, 0.06, 0)) and cants it a little more (0.78 rad); at full draw the
  arrow rises towards the crosshair from below. Before, the bow was canted nearly flat at full
  scale and its limbs crossed the whole screen, the crosshair included.
  `models::tests::the_bow_never_covers_the_centre` projects the riser, both flexed limbs, both
  strings and the nocked arrow at every draw (and through the release) and checks nothing comes
  within 0.12 (tan units, ~70 px at 720p) of the crosshair, and that at full draw the arrow's
  line passes within 0.06 of it.
* **Full-draw cue**: `BowFullDraw` plus `FullDrawGlint`, a warm flash (0.14), a gold and white
  ring bursting from the crosshair, the existing FULL label.
* **Zoom**: the view narrows as the bow is drawn, eased, to tan(fov/2) × 0.9 at full draw
  (`CameraRig::set_zoom`, smoothed at 10/s).
* **Arrows**: ×2.2, gold halos and streaks (pinning steel blue, ember orange, brand crimson).
* **Handling** (`combat.longwick`, owned by feel; damage stays balance's): draw 0.75 → **0.62 s**,
  arrows 45–120 → **55–135 m/s**, gravity 14 → **10**. A tap no longer dribbles out; a full
  draw flies flatter and arrives sooner (40 m: 0.30 s, drop 0.44 m; was 0.33 s, 0.78 m). PvP
  (balance's ×0.85 interval) draws in 0.53 s: two full draws kill in 1.27 s, inside the band.
* **Third person**: unchanged (the replicated draw already poses the remote bow).

## Shots

`cargo run -p client --release --example combat_shots -- out/` writes, besides the old ones:
`feel_muzzle_flash` (the frame the Quill fires), `feel_tracer`, `feel_rivets`, `feel_volley`,
`feel_bomb`, `feel_bow_rest`, `feel_bow_half`, `feel_bow_full_cue`, `feel_bow_full`,
`feel_bow_arrow`, `feel_triple_nock`, `feel_needle_incoming` (a needle at ~11 m with its
parry ring), `feel_needle_in_reach` (the ring flared), `feel_parry` (the punch's own frame),
`feel_parry_deflected`; and prints the stand-ins' counters.
`cargo run -p client --release --example models_view -- --only vmx` shows the bow at rest,
half, full draw and the release.

## Known limitations

* The parry shown on the punch's frame can disagree with the server when what the client draws
  differs from what the server rewinds to (a projectile that changed course, a Mould whose
  attack has its own `parry_reach`); then the server's `Parried` never comes (no heal) — rare.
* Stand-ins can't know the server's spread (Riveter, Volley, Cinderbox): the handover eases the
  difference out over 0.3 s. A stand-in whose projectile hit something on its first tick fades
  after its timeout.
* Press-frame shots only cover button changes; a held automatic weapon still fires on ticks
  (the interval is a whole number of ticks anyway).
