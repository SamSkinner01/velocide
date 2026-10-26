# Models, rigs & procedural animation

Owner: `client/src/models/**`, `client/examples/models_*.rs`, `assets/models/**`.

Every 3D model in VELOCIDE is an original low-poly PS1-style model built in code
(`engine::assets::meshgen` + small procedural textures, 16–32 px), exported to glTF and loaded
at runtime through `engine::assets::gltf::load`. Rigged models are split into parts with
pivots and animated procedurally by pure, time-based pose functions.

## Pipeline

```text
cargo run -p client --example models_gen        # rebuild assets/models/*.gltf (+ read-back check)
cargo run -p client --example models_view       # headless renders -> target/models_view/*.png
    [-- --out DIR] [--only PREFIX]               # e.g. --only vm, --only gaoler, --only closeup
cargo test -p client                            # mesh validity, glTF round trip, finite poses
```

- `ModelId::build()` builds a model's parts in **model space, bind pose** (one mesh + one
  material per part, named after the part).
- `models::generate_all(dir)` writes one self-contained `.gltf` per model (base64 buffers,
  embedded PNG textures, nearest samplers) and reads each file back to verify it.
- `ModelLibrary::load(&mut renderer)` loads every file from `engine::assets_dir()/models`,
  checks it against the model's part table, subtracts each part's pivot (meshes become
  joint-local) and uploads meshes / textures (identical textures are shared). A missing or
  mismatching file is **regenerated in memory** with a `log::warn!` — the game always starts.
  `lib.regenerated` lists models that were not loaded from disk.
- The rig structure (part names, parents, pivots, shading) lives in code
  (`ModelId::joints()`); glTF holds geometry, textures and base colours. If you edit a file in
  Blender keep the mesh / node names; parts are matched by name.

## Conventions

- Metres, +Y up, **models face -Z**, right = +X. Characters / props: origin at the feet.
  Harrier: origin = hover centre (`EnemyKind::root_lift()` = 0.65 m above the replicated
  feet). Projectiles: centred, flying along -Z (orient with a look
  rotation towards the velocity). Weapons: **grip at the origin**, barrel along -Z.
- Colours are sRGB 0..1 (shaded in gamma space). Hues mostly come from vertex colours; the
  textures are neutral detail maps (chrome reflection bands, brushed plate, grime, veined skin,
  poured wax with drip runs, bone, knurling, hazard stripes, emissive noise).
- Part shading (`rig::Shade`):

  | shade | drawn | colour |
  |---|---|---|
  | `Lit` | lit | vertex × texture × base colour × `Tint::body` |
  | `Team` | lit | … × `Tint::team` (player / team colour slot, vertex colours near-white) |
  | `Glow` | **unlit** | … × `Pose::glow` × `Tint::glow` — **attack telegraphs**, weapon glow |
  | `Core` | **unlit** | … × `Pose::core` — steady emissive: weak points, lamps, the Shrike's blood core |
  | `TeamGlow` | **unlit** | … × `Tint::team` × `Pose::glow` (Shrike visor) |

  Emissive colours are not clamped before the framebuffer, so `glow` values > 1 push towards
  white (white-hot flashes). Translucent parts have base colour alpha < 1 (the aim laser's
  sheath, exit portal) and go through the renderer's translucent pass.

## Runtime API (`client::models`)

```rust
let lib = ModelLibrary::load(&mut renderer)?;           // once at startup

// third-person player (other players / spectating); body faces anim.yaw
let info: PlayerDrawInfo = lib.draw_player(&player_anim, feet_pos, &Tint::team(color), &mut draws);
// info.muzzle / info.muzzle_dir (world) for remote tracers & flashes, info.head for name tags

// enemies
let root = EnemyAnim::root(position + Vec3::Y * kind.root_lift(), yaw); // feet (Harrier: hover centre)
lib.draw_enemy(&enemy_anim, root, &Tint::default(), &mut draws);
let at = lib.enemy_attack_point(&enemy_anim, root); // jaw / censer pot / beak / door or flail / lens

// first-person viewmodel (view space -> FrameDesc::viewmodel, viewmodel_fov_y_degrees ~60)
let vm: ViewmodelDrawInfo = lib.draw_viewmodel(&vm_anim, &Tint::team(color), &mut viewmodel);
// vm.muzzle / vm.muzzle_dir (view space; transform by the camera for world tracers),
// vm.fist (knuckles / held prism, for punch sparks)

// projectiles and props
lib.draw_model(ModelId::Rocket, Mat4::from_rotation_translation(look_rot, pos), &Tint::default(), &mut draws);
lib.get(ModelId::Door).draw(&props::door_pose(open, locked), root, &Tint::default(), &mut draws);
```

Lower level: `lib.get(id) -> &Rig`; `Rig { parts: Vec<RigPart> }` with
`RigPart { name, mesh, texture, parent: Option<usize>, pivot, bind, shade, base_color }`;
`rig.draw(&pose, root, &tint, &mut out)`, `rig.draw_static(root, &tint, &mut out)`,
`rig.world_transforms(&pose, root, &mut Vec<Mat4>)`, `rig.part_transform(&pose, root, i)`,
`rig.find(name)`. A part's world transform is
`world[parent] * translate(pivot - parent_pivot) * pose.locals[i]` (root parts:
`root * translate(pivot) * local`). `Pose { locals: Vec<Mat4>, glow: Vec4, core: Vec4,
hidden: u64 }` (`Pose::rest(n)`, `rotate`, `rotate_move`, `set`, `hide`). Pose functions are
pure: `shrike::pose(&PlayerAnim)`, `enemies::pose(&EnemyAnim)`,
`weapons::weapon_pose(..)`, `weapons::viewmodel_transform(&ViewmodelAnim)`,
`props::{door_pose, checkpoint_pose, jump_pad_pose, exit_gate_pose}`.

`Tint { body, team, glow }`: `body` multiplies everything (hit flash > 1, fade-out alpha < 1),
`team` is the player colour (`DEFAULT_TEAM_COLOR` = blood red), `glow` lets the client flash
telegraph parts on top of what the pose already does.

## Animation inputs

### `PlayerAnim` (Shrike)

| field | meaning |
|---|---|
| `velocity` | world m/s (run cycle amplitude; legs turn up to ±0.6 rad towards the movement direction, the rest becomes side-stepping; backwards movement reverses the cycle) |
| `yaw`, `pitch` | look angles; body faces yaw, torso / head / weapon follow pitch |
| `on_ground`, `sliding`, `dashing`, `slamming`, `wall_cling: Option<normal>` | movement state (priority: slide > slam > dash > wall > air > ground) |
| `firing` | recoil 0..1 (1 on the shot, decay over ~0.2 s) |
| `punching` | punch progress 0..1 (left arm thrust; fast out in the first 25 %) |
| `downed`, `dead` | kneeling with flickering core / flat on its back |
| `time` | seconds (idle breathing, core heartbeat) |
| `weapon` | 0 Lancet, 1 Thresher, 2 Riveter, 3 Cinderbox (Lancet one-handed, others two-handed) |
| `cycle` | locomotion phase — keep per player: `cycle = PlayerAnim::advance_cycle(cycle, velocity, dt)` (phase keeps legs continuous when speed changes; leg rate capped at 2.6 cycles/s) |

### `ViewmodelAnim` (first person)

| field | meaning |
|---|---|
| `weapon`, `alt_mode` | weapon index; alt mode 0/1 changes the glow colour (Lancet teal/violet, Thresher amber/red, Riveter orange/green, Cinderbox orange/yellow) |
| `fire_kick` | 1 on fire, decay (Lancet ~0.15 s, Riveter per rivet ~0.08 s, Thresher / Cinderbox ~0.35 s); Thresher pump racks back during the decay |
| `alt_charge` | 0..1 Lancet overcharge: cell + coils brighten, turn white and the gun shakes above 0.6 |
| `switch` | 0 (just switched, below the screen) → 1 (raised); animate over ~0.12 s |
| `punch` | punch progress 0..1: the left fist thrusts in from the lower left; knuckle studs flare |
| `throw` | prism throw progress 0..1: left hand rises with a spinning prism, flicks it at ~0.5 |
| `bob_phase`, `bob_amount` | walk bob (advance phase by ~speed × 1.2 rad/s; amount = ground speed / run speed) |
| `sway` | smoothed look velocity (rad/s); the weapon lags |
| `spin` | Riveter barrel angle — keep a `BarrelSpin` per local player: `spin.update(firing, dt)` |
| `time` | seconds |

Weapons are modelled at real scale (the third-person Shrike holds the same models) and drawn
at `weapons::VIEWMODEL_SCALE` (0.52) in first person, grip around 0.3 m in front of the
camera, lower right. Each weapon exposes its muzzle (`WeaponKind::muzzle()`, model space).

### `EnemyAnim`

`{ kind, state, attack, speed (m/s), cycle, strafe (-1..1, + = moving right), aim_pitch,
aim_locked, time, seed }`; `EnemyAnim::new(kind)`; keep `cycle = kind.advance_cycle(cycle,
speed, dt)` per enemy; `seed` de-synchronises idles / hovering. `attack` is the definition's
attack index (replicated `aux` bits 8–15): which telegraph and strike to pose (each enemy
module exports its indices, e.g. `gaoler::{BASH, SWEEP, RUSH}`). `aim_pitch` aims heads (the
Heron's laser pitch) and pitches a diving Harrier down its flight; `aim_locked` makes the
Heron's lens flare. `EnemyState`: `Idle`, `Walk` (any state with speed > 0.2 walks),
`Windup(p)`, `Attack(p)`, `Recover(p)`, `Stunned`, `Dying(p)` — `p` is progress 0..1 through
the phase (elapsed / duration from the enemy definition; dying: the corpse time).
`EnemyKind::from_name("gaoler")` maps enemy definition names.

## Telegraph conventions

`Pose::glow` drives every enemy's `Glow` parts: ~0.45 calm → ramps up through `Windup` →
strobes near its end → peak on `Attack` → fades in `Recover` → dark when `Stunned` / `Dying`.
`Pose::core` drives the weak points: always ≥ 1.15 (they read through fog), flaring with the
telegraph, guttering out as the corpse cools. The client only has to pass the state; `Tint::glow`
can add extra flashing. The Moulds are grey wax-flesh, slag, brass and chain lit from inside:
weak points are hot orange / yellow / red, telegraphs orange (Gnawer, Censer), red (Harrier,
Heron, the Gaoler's chain) or yellow (the Gaoler's door). Deaths slump, split and melt (the
pelvis / robe / body squashes towards the floor; heads, braziers and doors fall away).

| enemy | telegraph (`Glow`) | steady emissive (`Core`, weak point) | wind-up poses |
|---|---|---|---|
| Gnawer | throat + gums (orange) | ember eye slits | bite: the jaw unhinges (~1 rad), head rears, throat lights; snap = head drives down, jaw slams. Pounce: coils low with fists spread, quivers; airborne stretch, fists forward, jaw wide |
| Censer | censer embers + flames over the brazier (orange) | the coal bed in the caged brazier head | volley: raises the censer overhead and whirls it faster (the chain is solved to a horizontal circle whatever the arm does); lobs lean in. Swing: twists away with the censer drawn back, then a wide sweep |
| Harrier | eyes + the beak's edge (red) | the heart sac under the chest (flares white-hot when it rears, during the dive) | rears up (body pitched up), wings folded back, neck coiled; the dive pitches the body down its line (`aim_pitch`), wings swept, talons forward; recover beats hard; swatted: tumbling |
| Gaoler | the door's rim (yellow; **parry window**: last 35 % of the bash wind-up, `gaoler::PARRY_WINDOW_FROM` = 0.65, white-hot / yellow strobe) and the chain (red) — the inactive one is hidden | the helm's grille, the back lantern | the door always stays upright (the forearm counter-rotates the body): bash draws it back and slams it forward; sweep crouches, arm out, chain spun low then a 300° turn with the body; rush hunkers behind the door then charges; parried: the door knocked open like a gate |
| Heron | the ring round the lens (red; strobes white-hot once `aim_locked`) | the lens (≥ 2 while aiming) | needle: braces with the legs splayed, body low, neck forward, the head following `aim_pitch`; recoil on the shot. Kick: leans in with a leg drawn back, then stamps it forward |

## Models and parts

| model | file | parts (parents in brackets) |
|---|---|---|
| Shrike (~1.88 m incl. crest) | `shrike.gltf` | pelvis, torso[pelvis], core[torso] *Core*, trim[torso] *Team* (pauldrons, back blades, chest V), head[torso], visor[head] *TeamGlow*, crest[head] *Team*, upper_arm_l/r[torso], forearm_l/r, hand_l/r, thigh_l/r[pelvis], shin_l/r, foot_l/r |
| Gnawer (2.0 m to the hump, 1.8 m long) | `enemy_gnawer.gltf` | pelvis, chest, head, jaw[head], throat[head] *Glow*, eyes[head] *Core*, upper_arm_l/r[chest], forearm_l/r, fist_l/r, thigh_l/r[pelvis], shin_l/r |
| Censer (2.7 m incl. the flue) | `enemy_censer.gltf` | robe, torso, head (brazier cage), coals[head] *Core*, flames[head] *Glow*, arm_l/r, forearm_l/r, chain[forearm_r], censer[chain], embers[censer] *Glow* |
| Harrier (3.05 m wingspan) | `enemy_harrier.gltf` | body, neck, head, glare[head] *Glow*, heart[body] *Core*, wing_l/r, tip_l/r[wing], legs, tail |
| Gaoler (3.73 m) | `enemy_gaoler.gltf` | pelvis, torso, lantern[torso] *Core*, helm, grille[helm] *Core*, arm_l, forearm_l, shield[forearm_l] (the cell door), rim[shield] *Glow*, arm_r, forearm_r, chain[forearm_r], chain_glow[chain] *Glow*, flail[chain], thigh_l/r, shin_l/r |
| Heron (3.35 m) | `enemy_heron.gltf` | body, neck, head, lens[head] *Core*, ring[head] *Glow*, thigh_l/r[body], shin_l/r |
| Lancet | `weapon_lancet.gltf` | body, cell *Glow* |
| Thresher | `weapon_thresher.gltf` | body, pump (slides back), glow *Glow* |
| Riveter | `weapon_riveter.gltf` | body, barrels (rotates about the bore), drum (side magazine, rotates), glow *Glow* |
| Cinderbox | `weapon_cinderbox.gltf` | body, stripe (hazard band), core *Glow* (side windows + maw) |
| right arm (1st person) | `vm_arm.gltf` | forearm, hand, trim *Team* |
| left fist (1st person) | `vm_fist.gltf` | forearm, fist, trim *Team*, knuckles *Glow* |
| projectiles | `proj_rivet/sawblade/rocket/flak/sticky/mortar/ember_bomb/needle/prism.gltf`, `fx_blood_core.gltf`, `fx_aim_laser.gltf` | `body` + `glow` (rivet, rocket, flak, sticky, mortar); `blade` + `edge` (sawblade, horizontal disc: spin it about Y); `shell` + `cracks` *Glow* (ember bomb, a 0.3 m slag ball with glowing seams); `shaft` + `glow` *Glow* (needle, 0.95 m, white-hot point, red fletch); `shard` *Core* + `rim` *Glow* (prism, ~0.5 m, stands along Y); `core`; `core` + `sheath` (translucent) *Glow* (aim laser: a unit beam 1 m along -Z, scaled to width × width × length and tinted through `Tint::glow`; the Heron's laser and needle streaks) |
| props | `prop_jump_pad/checkpoint/exit_gate/door.gltf` | jump pad: base, rim, glow; checkpoint: base, lamp, halo; exit gate: frame, portal (translucent), lights *Core*; door: frame, panel_l, panel_r, light |

Prop poses: `door_pose(open 0..1, locked)` slides the panels apart (unit door
`props::door::WIDTH` 3 m × `HEIGHT` 3.5 m — scale to the door brush; light red = locked,
amber = closed, green = open), `checkpoint_pose(active, time)` (dim red → bright cyan, halo
bobs and spins), `jump_pad_pose(time)`, `exit_gate_pose(time)`. Glow parts of static models
drawn with `draw_model` use the vertex colours as-is (`Pose::glow` = 1).

Budget: ~1–1.4k triangles per character, 20–900 per weapon / projectile / prop; 2.0 MB of
glTF in total (vertex data dominates: flat-shaded faces need their own vertices).

## Visual verification

`models_view` renders (1280×720 = the game's 640×360 internal image at 2×, retro settings
on): `lineup` / `lineup_back` (scale and silhouettes, back = the Gaoler's lantern),
`closeup_<enemy>` (front 3/4 + back), `shrike_0..3` + `shrike_closeup` (every movement state
in profile, team colour), `gnawer_1/2`, `censer_1/2`, `harrier_1/2`, `gaoler_1/2/3`,
`heron_1/2` (idle, walk, every attack's wind-up early / late and strike, recover, stunned,
melting, dead), `vm_00..19` (each weapon rest /
kick / alt mode, overcharge, riveter spin, switch, punch, prism throw, bob + sway),
`projectiles`, `props`.

## Known limitations

- Rigs are rigid parts (no skinning); elbows / knees show small gaps or overlaps at extreme
  angles. Joint pivots are checked by a test to sit on their part's geometry.
- Pivots / hierarchy are code-defined; the glTF files carry no node hierarchy (the engine's
  writer has no node transforms), so an artist edit can change geometry but not the rig.
- The Harrier's `Dying` pose only sags; the corpse falls server-side (a dead flyer has
  gravity).
- The Censer's and the Gaoler's chains are rigid rods (no rope simulation): the pose solves
  their whirl directions, the links don't sag.
- Viewmodel arms are generic: one right arm grips every weapon (fingers don't wrap each grip
  exactly), and the left fist doubles as the prism-throwing hand.
- Labels in `models_view` sit at floor level (hovering Harriers are labelled below themselves).
