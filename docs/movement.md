# Movement

The player is extremely fast and nearly never stops. Movement is a deterministic kinematic
character controller in `shared::movement` (client prediction and the server run the same
code) built on the swept-capsule queries of `engine::physics::CollisionWorld`. Every number
is in `MovementTuning` (`shared/src/tuning.rs`) and exposed in `assets/tuning.ron`
(hot-reloaded).

Code: `shared/src/movement/mod.rs` (state machine, moves), `collide.rs` (collide-and-slide,
step-up, ground snapping, depenetration), `camera.rs` (client camera-feel helpers).

## API

```rust
pub struct MoveEnv<'a> { pub world: &'a CollisionWorld, pub jump_pads: &'a [JumpPad] }
pub fn step(state: &mut MoveState, input: &PlayerInput, env: &MoveEnv, t: &MovementTuning, dt: f32) -> MoveEvents;

impl MoveState {
    pub fn spawn(position: Vec3) -> Self;                       // feet position, full stamina
    pub fn capsule(&self, t) -> (Vec3 /*centre*/, f32 /*inner half-height*/, f32 /*radius*/);
    pub fn eye_position(&self, t) -> Vec3;                      // standing or slide eye height
    pub fn height(&self, t) -> f32;                              // 1.8 standing, 0.9 crouched
    pub fn is_invulnerable(&self) -> bool;                       // dash i-frames
    pub fn stamina(&self, t) -> f32;                             // dash charges (fractional)
    pub fn is_sliding/is_dashing/is_slamming(&self) -> bool;
    pub fn horizontal_speed(&self) -> f32;
}
pub fn view_roll(state, yaw, t) -> f32;      // target camera roll (radians, + = counter-clockwise)
pub fn fov_kick(state, t) -> f32;            // extra FOV (degrees)
pub fn landing_dip(landing_speed, t) -> f32; // eye dip (metres) for MoveEvents::landed
```

`MoveState` holds **everything** that affects future movement (it is `Copy`, `PartialEq`,
serde): `position` (feet), `velocity`, `on_ground`, `prev_buttons`, `mode`, `ground_normal`,
`crouched`, `stamina_used` (spent charges, 0 = full, so `Default`/`spawn` start full),
`dash_time`, `dash_dir`, `dash_exit_speed`, `invuln_time`, `slide_dir`, `slam_start_y`,
`slam_bounce_time`, `slam_bounce_power`, `coyote_time`, `jump_buffer`, `wall_jumps_used`,
`wall_normal` (for view tilt; zero if none), `wall_clinging`, `jump_pad` (pad currently
inside), `footstep_accum`, `air_time`.

`MoveEvents` (per tick, `Copy`): `jumped` (any ground/coyote jump, incl. slide-/dash-jumps and
slam bounces), `slide_jumped`, `dash_jumped`, `slam_bounced`, `wall_jumped`, `dashed`,
`dash_failed` (no stamina), `stamina_recharged`, `slide_started`, `slide_ended`,
`slam_started`, `slam_landed: Option<SlamLanded { position, power }>`, `landed: Option<f32>`
(downward speed), `left_ground` (walked off / knocked up), `jump_pad: Option<u16>`,
`footstep`, `hit_ceiling`, `wall_impact: Option<f32>` (speed lost into a surface, > 8 m/s),
`stepped_up: Option<f32>` (camera smoothing on stairs).

**External impulses** (explosions, knockback) are added by the server directly to
`state.velocity` between steps. An upward impulse while grounded lifts the player off on the
next step (never re-snapped while `velocity.y > 0`), keeps horizontal knockback in the air,
and grants coyote time (so jump + rocket stacks, Quake-style).

## State machine

| from | event | to |
|---|---|---|
| Walk | crouch held | Slide |
| Walk / Slide | jump (ground or coyote) | Air (slide-jump keeps slide speed) |
| Walk / Slide | walk/slide off an edge, upward impulse | Air (coyote time starts) |
| Slide | crouch released and the standing capsule fits | Walk |
| Air | land on walkable ground | Walk, or Slide if crouch held |
| Air | crouch *press*, ≥ `slam_min_height` above ground | Slam |
| Air | jump next to a wall (wall jumps left) | Air (wall jump) |
| Slam | land | Walk / Slide, opens the slam-bounce window |
| Slam | dash / wall jump | Dash / Air |
| any | dash press with ≥ 1 charge | Dash |
| Dash | timer ends | Walk / Slide (crouch held) if grounded, else Air |
| Dash | jump (ground, coyote or wall) | Air (dash-jump) |
| any | enter a jump pad | Air |

`MoveMode` is one of `Walk`, `Air`, `Slide`, `Dash`, `Slam`; `on_ground` is tracked separately
(a dash can be grounded or airborne). `crouched` (low capsule) is true while sliding and while
something overhead prevents standing (e.g. after sliding into a low tunnel and releasing
crouch, or dashing out of a slide under a ceiling).

### Tick order (`step`)

1. Depenetrate (spawned/pushed into geometry, or resting exactly on a surface).
2. Grounded with `velocity.y > 0` (external impulse) → leave the ground.
3. Timers: i-frames, coyote, slam-bounce window, jump buffer (a jump press sets it), stamina
   regen (not while dashing; `stamina_recharged` when a charge completes); grounded → reset
   wall jumps.
4. Airborne: probe walls within `wall_detect_distance` and steep slopes being touched.
5. Actions, in priority order: **dash** (press), **slam** (crouch press in the air; a dash
   pressed the same tick wins), **jump** (buffered: ground/coyote first, else wall), **slide**
   start/stop, stand up if possible.
6. Velocity by mode (below), clamp to `max_speed`.
7. Move: grounded → collide-and-slide along the ground with step-up; airborne →
   collide-and-slide (landing on walkable ground keeps horizontal speed and continues along
   the ground).
8. Ground: snap down (`step_height` reach if we were grounded and not moving up, else a 5 cm
   probe); landing → `landed`, slam landing, slide if crouch held; losing ground → coyote.
   Final depenetration safety net.
9. Dash timer ended → exit velocity.
10. Jump pads (fire on entry).
11. Footsteps, air time.

## Moves

**Running.** Wish velocity = input direction × `ground_speed`; accelerate towards it at
`ground_accel` (0 → 16 m/s in 0.1 s; same rate for braking/turning). When faster than
`ground_speed` with input held (landing from a dash-jump, leaving a slide) the extra speed
bleeds off at only `ground_overspeed_decel` while the direction still steers quickly, so chains
don't feel like hitting a wall. Friction (Quake-style, `ground_friction` with a
`ground_stop_speed` floor) applies **only without input**: a full stop takes ~0.25 s. On slopes
the horizontal speed is kept (motion is re-aimed along the ground plane) and vertical velocity
is always 0 while grounded.

**Air.** Gravity, `max_fall_speed`. Quake-style air acceleration (`air_accel` towards
`air_max_speed` along the wish direction). Below `air_max_speed` it accelerates (and can
brake); above it, it only **redirects**: the speed magnitude is preserved exactly, so momentum
from slides, dashes and explosions survives any input, and there is no strafe-jump speed gain
either. Air control can't push into a steep slope you're touching (you slide off it instead
of slowly climbing it).

**Jump.** `velocity.y = max(velocity.y, 0) + jump_speed`; horizontal speed untouched (never
capped). Allowed on the ground and for `coyote_time` after leaving it without jumping. Jump
presses are buffered for `jump_buffer_time` and fire as soon as any jump is possible (on
landing: the tick after touching down, before ground friction, so bunny-hops lose nothing; into
a slam bounce; into a wall jump).

**Dash.** Dash press with ≥ 1 charge: horizontal burst in the input direction (facing
direction if no input) at `dash_speed` for `dash_duration` (11 ticks at 60 Hz, ≈ 8.8 m),
vertical velocity zeroed and **no gravity** (air dashes are flat). Grants `dash_invuln_time`
of invulnerability. Costs one of `dash_charges`; charges regenerate continuously at
`dash_regen_rate`/s. Below one charge the press emits `dash_failed`. A dash can re-dash, cancels
a slam, ends a slide (stands up if there's room) and keeps following the ground if it started
grounded (steps, ramps). When it ends, horizontal velocity is scaled to `dash_exit_speed` =
`max(dash_end_speed, speed you dashed in with)` (capped at `dash_speed`), proportionally if a
wall cut it short. Jumping during a dash (ground, coyote or wall jump) is a **dash-jump**:
horizontal speed `max(dash_speed × dash_jump_keep, exit speed)` = 28.8 m/s in the dash
direction.

**Slide.** Crouch held on the ground (also on landing with crouch held, and when a grounded
dash ends with crouch held): low capsule (`slide_height`), moving in the current movement
direction (input, then facing, if standing still) at `max(horizontal speed, slide_min_speed)`.
No friction: speed is kept **indefinitely** on flat ground. Steering: sideways input turns
the slide at up to `slide_steer_degrees`/s. Slopes add `gravity × slide_slope_accel ×
sin(slope)` downhill (gains capped at `slide_max_speed`; entry speed above it is kept); speed
never drops below `slide_min_speed`, so slides also climb walkable ramps. Releasing crouch
stands up only if the standing capsule fits; otherwise the slide continues until there's
headroom. Jumping is a **slide-jump** that keeps the slide speed (× `slide_jump_boost`);
landing with crouch still held slides again, so slide-hops chain without losing speed.
Sliding off a ledge keeps momentum; holding crouch through the air doesn't slam (only a press
does).

**Slam.** Crouch *press* in the air, at least `slam_min_height` above the ground (pressing
closer just slides on landing): falls at `slam_speed`, horizontal speed × `slam_horizontal_keep`
(0 = straight down). On landing: `slam_landed { position, power }` with `power =
clamp(fall height / slam_full_power_height, 0, 1)` (the server applies a shockwave of
`slam_shockwave_radius` scaled by power), and a `slam_bounce_window` opens: a jump within it
(including a buffered one pressed just before impact) is a **slam bounce** with vertical speed
`slam_bounce_speed + slam_bounce_power_bonus × power` (≈ 5.8–8.9 m vs a 2.5 m jump). Bouncing
out of a slide (crouch still held) keeps the slide's horizontal speed. Dashing and wall jumps
cancel a slam.

**Wall jump.** Airborne with a wall (|normal.y| < 0.35) within `wall_detect_distance`:
jump removes velocity into the wall, keeps speed along it, adds `wall_jump_out_speed` away
from it plus `wall_jump_input_speed` in the input direction (not into the wall), and sets
`velocity.y = max(velocity.y, wall_jump_up_speed)` (≈ 2.2 m of rise). Limited to `wall_jumps`
per airtime, reset on touching the ground. Out of a dash it carries dash-jump speed along the
wall. **Wall cling:** airborne, falling, input pushing into the wall (wish · normal < -0.3):
fall speed is capped at `wall_cling_max_fall_speed`. `wall_normal`/`wall_clinging` are in the
state for the client's view tilt.

**Jump pads.** Entering a pad's trigger volume (capsule AABB overlap) sets `velocity` to the
pad's velocity, cancels dash/slam/slide, and emits `jump_pad`. Fires once per entry.

## Character controller

- **Skin.** Sweeps keep the capsule `SKIN` = 1 cm from surfaces. Every sweep result is
  validated with a closest-point query at the stopping position: that gives the exact
  surface normal (GJK sweep normals carry noise) and restores exactly the skin distance if
  the sweep stopped inside it. Resting inside the skin band would make every later sweep with
  the slightest component into the surface report a zero-distance hit and eat the motion.
- **Collide-and-slide.** Up to 6 iterations; velocity and remaining motion are clipped
  against all planes touched this move (crease of two planes in corners, stop in a
  three-plane corner). Grounded moves treat non-walkable surfaces as vertical walls (no
  climbing them, no being pushed into the floor) and follow walkable ones keeping
  horizontal motion. A 0.1 mm nudge off each touched surface keeps tangential sweeps clean.
- **Step-up.** When a grounded move is blocked, also try *up `step_height` → across → down*
  and keep it if it got further and lands on walkable ground whose surface is at most
  `step_height` above the feet. Landing on the rim of the step is fine (a slow walk may not
  clear it in one tick) but the rim of a higher ledge never counts. Works walking, sliding
  and dashing.
- **Ground snapping.** While grounded and not moving up, the capsule is swept down by
  `step_height` + 5 cm onto walkable ground (stairs, ramp crests). The rounded capsule bottom
  resting on an edge counts as ground if the surface just inside the edge is walkable (a
  ray), so running down stairs stays grounded. If the way down is blocked by a steep slope
  (standing at its foot) the snap slides along it. No snapping while `velocity.y > 0`.
- **Slopes.** Walkable up to `max_ground_slope_degrees`. Steeper: not ground (you slide off,
  can't walk up; air control can't push into it).
- **Ceilings** (normal.y < -0.7) kill upward velocity (`hit_ceiling`).
- **Depenetration** at the start and end of every tick pushes the capsule out of anything it
  penetrates or exactly touches (contact probes use a 5 mm thinner capsule because parry's
  normals are undefined at exactly zero distance).
- **No tunnelling.** All motion is swept continuously, so dash speed, `max_fall_speed`, slams
  and even `max_speed` (150 m/s, 2.5 m per tick) can't pass through 0.2 m walls/floors
  (tested).
- **Standing up** tests the standing capsule (bottom lifted 2 cm) for overlap.

### `engine/src/physics.rs` precision fix

parry's GJK-based queries use tolerances relative to the shapes' size, so against large boxes
(a 400 m floor brush) a capsule sweep that starts just inside the skin distance could report
no hit at all (or overshoot by centimetres), and contact normals were off by several degrees.
`CollisionWorld` now clips box colliders (in their own frame) to the query region plus a 2 m
margin before running cast/contact/intersection queries. The clipped-away faces lie outside
anything the query can touch, so results are unchanged except for precision. Covered by
`physics::tests::huge_boxes_stay_precise`.

## Camera feel (client)

Pure functions of the state, so they work for spectated players too. The client smooths
towards them.

- `view_roll(state, yaw, t)`: sliding → `slide_tilt_degrees`, leaning towards the side you
  slide to (left when sliding straight); airborne near a wall and clinging or moving along it
  (> 6 m/s) → `wall_tilt_degrees` away from the wall. Positive = counter-clockwise.
- `fov_kick(state, t)`: smoothstep from 0 at `ground_speed` to `fov_kick_degrees` at
  `fov_kick_speed` (total speed, so slams and dashes kick too).
- `landing_dip(speed, t)`: 0 below 4 m/s, full `landing_dip` at 30 m/s. Use with
  `MoveEvents::landed`. `stepped_up` lets the client smooth the eye over stairs.

## Tuning values

| value | default | why |
|---|---|---|
| `radius` / `standing_height` / `slide_height` | 0.4 / 1.8 / 0.9 m | Human-scale capsule; slide height fits under 1.2 m tunnels. |
| `eye_height` / `slide_eye_height` | 1.6 / 0.75 m | |
| `step_height` | 0.45 m | Climbs stairs (0.3 m) and curbs at full speed; 0.6 m ledges need a jump. Also the snap-down reach. |
| `max_ground_slope_degrees` | 50° | Generous so ramps of 45° are runnable; 60° is a wall you slide off. |
| `ground_speed` | 16 m/s | Fast base pace (≈ 3× a realistic run). |
| `ground_accel` | 160 m/s² | Full speed in 0.1 s: "instant" but not a teleport. |
| `ground_overspeed_decel` | 30 m/s² | Landing at 28.8 m/s from a dash-jump bleeds back to 16 over ~0.4 s, rewarding chains. |
| `ground_friction` / `ground_stop_speed` | 10 / 4 m/s | Stop in ~0.25 s, only when you let go. |
| `air_accel` / `air_max_speed` | 70 m/s² / 16 m/s | Strong steering (≈ 130°/s turn at 30 m/s) without speed gain or loss. |
| `gravity` | 38 m/s² | Snappy arcs (≈ 4× real) so jumps don't feel floaty at this speed. |
| `max_fall_speed` | 90 m/s | Long falls get fast but stay readable. |
| `max_speed` | 150 m/s | Safety cap for stacked explosions. |
| `jump_speed` | 14 m/s | ≈ 2.5 m apex, 0.74 s airtime. |
| `coyote_time` / `jump_buffer_time` | 0.12 s | ~7 ticks of forgiveness either side of a ledge/landing. |
| `footstep_stride` | 2.6 m | ~6 steps/s at full speed. |
| `dash_speed` / `dash_duration` | 48 m/s / 0.17 s | ≈ 8.8 m burst: crosses most gaps, clears a 20 m pit with a dash-jump. |
| `dash_invuln_time` | 0.2 s | Covers the dash plus a couple of ticks of latency slack. |
| `dash_charges` / `dash_regen_rate` | 3 / 0.85 per s | Three back-to-back dashes, ~1.2 s per refill: dash often, not always. |
| `dash_end_speed` | 20 m/s | Leaves a dash faster than running (no dead stop) but not at 48 m/s. |
| `dash_jump_keep` | 0.6 | Dash-jump at 28.8 m/s: the fastest single move, worth chaining. |
| `slide_min_speed` | 24 m/s | Sliding is always faster than running. |
| `slide_max_speed` | 60 m/s | Downhill gains stop here. |
| `slide_steer_degrees` | 70°/s | Slight correction only; commit to the line. |
| `slide_slope_accel` | 1.0 | Full gravity component downhill. |
| `slide_jump_boost` | 1.0 | Slide-jumps keep exactly the slide speed. |
| `slam_speed` | 75 m/s | Near-instant ground pound. |
| `slam_min_height` | 1.0 m | Crouching just before a landing slides instead of wasting speed on a tiny slam. |
| `slam_horizontal_keep` | 0.0 | Straight down, precise. |
| `slam_full_power_height` | 15 m | Full shockwave from about two storeys. |
| `slam_bounce_window` | 0.2 s | Generous enough for a buffered jump. |
| `slam_bounce_speed` / `slam_bounce_power_bonus` | 21 / 5 m/s | 5.8 m bounce (2.3× a jump), up to 8.9 m from a full-power slam. |
| `slam_shockwave_radius` | 6 m | Server-side shockwave size. |
| `wall_jumps` | 3 | Enough to climb a shaft, not to fly. |
| `wall_jump_up_speed` / `wall_jump_out_speed` / `wall_jump_input_speed` | 13 / 11 / 4 m/s | ≈ 2.2 m rise per wall jump; crosses a 3 m shaft in ~0.15 s. |
| `wall_cling_max_fall_speed` | 4 m/s | Slow, controllable wall slide. |
| `wall_detect_distance` | 0.4 m | Forgiving at speed. |
| `fov_kick_degrees` / `fov_kick_speed` | 10° / 45 m/s | Kick ramps in from running speed to dash speed. |
| `slide_tilt_degrees` / `wall_tilt_degrees` | 4° / 7° | |
| `landing_dip` | 0.25 m | |

## Determinism rules

- `step` reads only its arguments and the static world; all state is in `MoveState` (no
  globals, no time, no randomness, no frame-rate dependence). `dt` is the fixed tick.
- Inputs are quantised (`i8` axes) and the predicted input is exactly the one sent, so client
  and server see identical values (including the `f32` yaw).
- Edge-triggered actions come from `prev_buttons` stored in the state; timers (coyote, buffer,
  bounce window, dash, i-frames) and charges are in the state.
- Collision queries iterate colliders in a deterministic order (grid cells in order, then
  insertion order); ties keep the first hit.
- Same binary + same inputs + same start state ⇒ bit-identical state every tick, and replaying
  from any serialised snapshot reproduces the original run (tested, with impulses applied
  between steps the way the server does). Across different CPUs/platforms `sin`/`cos` may
  differ in the last bit; reconciliation corrects that.
- The client's collision world must match the server's: same level file and the same door
  states at the same ticks.

## Tests and tools

`cargo test -p shared --test movement` (50 tests, ~0.05 s): bit-identical determinism on the
test level (4 spawns × 1500 ticks of chaotic input), replay from serialised snapshots,
chaotic play never penetrates geometry or leaves the world (and exercises every mode and wall
jumps), ground speed in 0.12 s, friction stop, air strafing neither loses nor gains speed, air
control to `air_max_speed`, jump height, ceiling bonk, dash distance/duration/i-frames/exit
speed, dash direction, charges/regen/`dash_failed`, flat air dash, dash-jump speed, slide
speed held for 5 s, entry speed above minimum kept, slide steering, downhill acceleration, low
ceiling keeps you crouched until clear, slide-jump, slam speed/power/`slam_landed`, bounce >
2× jump, bounce window expiry, buffered slam bounce, crouch near the ground slides instead of
slamming, dash cancels slam, wall jumps limited to 3 and reset on landing, wall-jump
direction, wall cling cap + view tilt, coyote jump (and its expiry), buffered jump on
landing, 0.3 m step climbed walking/sliding/dashing while 0.6 m blocks, slow stair climbing,
running down stairs stays grounded, ramps climbed at full speed, 60° slope not walkable (and
you slide off it), landing on a ramp keeps speed, no tunnelling (dash and max speed into a
0.2 m wall; free fall, slam and max speed onto a 0.2 m floor), 10 s of resting without drift
(flat and on a 30° ramp), small bumps don't launch, upward impulse leaves the ground, jump
pads fire once, depenetration, spawning exactly on the floor, no lost motion across brush
seams and into the tunnel, test-level spawns, camera helpers, serialisation, and that
`assets/tuning.ron` spells out every value and matches the defaults.

`cargo run -p shared --example movement_trace -- <scenario> [--every N]` simulates scripted
input on the test level and prints a per-tick trace (position, velocity, horizontal speed,
mode, flags G=grounded C=crouched I=invulnerable W/w=clinging/near wall, stamina, events).
Scenarios: `run jump dash slide slam walljump stairs ramps steep tunnel pad gap` (`list`).

### Test level (`assets/levels/movement_test.ron`)

Floor 200 × 260 m with outer walls; around the spawns: a 246 m slide corridor (west edge),
15°/30°/45° ramps and a 60° unwalkable one (east) plus a long 10° slide hill, 0.3 m-rise stairs
up to a 3 m landing and back down, a 0.3 m step, a 0.6 m ledge and a 5 cm bump, a 1.2 m-high
tunnel only passable by sliding, a 20 m kill pit between two 2 m platforms (dash-jump gap), a
3 m wall-jump shaft (32 m tall) and a wider 5 m one, a tall single wall for clinging, rising
grate platforms (2–10 m), a pillar field, a jump pad onto a 10 m platform, a second kill
volume below the world, materials concrete/brick/metal/tile/rust/grate/hazard, 8 lights and
10 spawns.

## Known limitations

- Moving platforms aren't supported (the world is static apart from doors).
- Standing on a ledge rim: the rounded capsule bottom can rest up to its radius beyond an edge
  before rolling off (standard for capsules).
- A slide stopped head-on against a wall keeps pushing into it until steered away.
- Crouching in the air doesn't shrink the capsule (crouch in the air means slam).
- Cross-platform bit-identity isn't guaranteed (transcendental functions); same-binary
  determinism is.
