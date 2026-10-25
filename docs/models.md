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
    [-- --out DIR] [--only PREFIX]               # e.g. --only vm, --only husk, --only closeup
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
  Gnat: origin = hover centre. Projectiles: centred, flying along -Z (orient with a look
  rotation towards the velocity). Weapons: **grip at the origin**, barrel along -Z.
- Colours are sRGB 0..1 (shaded in gamma space). Hues mostly come from vertex colours; the
  textures are neutral detail maps (chrome reflection bands, brushed plate, grime, veined skin,
  bone, knurling, hazard stripes, emissive noise).
- Part shading (`rig::Shade`):

  | shade | drawn | colour |
  |---|---|---|
  | `Lit` | lit | vertex × texture × base colour × `Tint::body` |
  | `Team` | lit | … × `Tint::team` (player / team colour slot, vertex colours near-white) |
  | `Glow` | **unlit** | … × `Pose::glow` × `Tint::glow` — **attack telegraphs**, weapon glow |
  | `Core` | **unlit** | … × `Pose::core` — steady emissive: weak points, lamps, the Shrike's blood core |
  | `TeamGlow` | **unlit** | … × `Tint::team` × `Pose::glow` (Shrike visor) |

  Emissive colours are not clamped before the framebuffer, so `glow` values > 1 push towards
  white (white-hot flashes). Translucent parts have base colour alpha < 1 (Gnat wings, Spitter
  orb shell, exit portal, Gnat bolt) and go through the renderer's translucent pass.

## Runtime API (`client::models`)

```rust
let lib = ModelLibrary::load(&mut renderer)?;           // once at startup

// third-person player (other players / spectating); body faces anim.yaw
let info: PlayerDrawInfo = lib.draw_player(&player_anim, feet_pos, &Tint::team(color), &mut draws);
// info.muzzle / info.muzzle_dir (world) for remote tracers & flashes, info.head for name tags

// enemies
let root = EnemyAnim::root(position, yaw);              // feet (Gnat: hover centre)
lib.draw_enemy(&enemy_anim, root, &Tint::default(), &mut draws);
let spit_from = lib.enemy_attack_point(&enemy_anim, root); // maw / emitters / claws / fist

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

`{ kind, state, speed (m/s), cycle, strafe (-1..1, + = moving right), aim_pitch, time, seed }`;
`EnemyAnim::new(kind)`; keep `cycle = kind.advance_cycle(cycle, speed, dt)` per enemy; `seed`
de-synchronises idles / hovering. `EnemyState`: `Idle`, `Walk` (any state with speed > 0.2
walks), `Windup(p)`, `Attack(p)`, `Recover(p)`, `Stunned`, `Dying(p)` — `p` is progress 0..1
through the phase (elapsed / duration from the enemy definition; dying: ~1 s).
`EnemyKind::from_name("husk")` maps enemy definition names.

## Telegraph conventions

`Pose::glow` drives every enemy's `Glow` parts: ~0.45 calm → ramps up through `Windup` →
strobes near its end → peak on `Attack` → fades in `Recover` → dark when `Stunned` / `Dying`.
The client only has to pass the state; `Tint::glow` can add extra flashing.

| enemy | telegraph (`Glow`) | steady emissive (`Core`) | wind-up pose |
|---|---|---|---|
| Husk | blood-red bone claws flare red-hot (strobe from 60 %) | slit eyes (flare with the claws) | both arms rise overhead, chest arches, jaw opens; attack rakes down + lunge |
| Spitter | throat sac, back pustules, maw (acid green) | — | sac swells to 1.6× and pulses, neck rears; spit thrusts the head, sac deflates |
| Gnat | compound eyes + emitter tips (magenta) | caged abdomen core (weak point) | abdomen curls under (presents the core); attack = three recoil kicks |
| Bastion | visor, hump beacon, fist cuff rings + knuckle bars (yellow) | back furnace (unarmoured side) | both fists overhead; **parry window** (last 35 %, `bastion::PARRY_WINDOW_FROM` = 0.65): strobes white-hot / yellow at 18 Hz; attack = slam into the floor and hold |

## Models and parts

| model | file | parts (parents in brackets) |
|---|---|---|
| Shrike (~1.88 m incl. crest) | `shrike.gltf` | pelvis, torso[pelvis], core[torso] *Core*, trim[torso] *Team* (pauldrons, back blades, chest V), head[torso], visor[head] *TeamGlow*, crest[head] *Team*, upper_arm_l/r[torso], forearm_l/r, hand_l/r, thigh_l/r[pelvis], shin_l/r, foot_l/r |
| Husk (~1.9 m) | `enemy_husk.gltf` | pelvis, spine, chest, head, eyes *Core*, jaw, upper_arm_l/r, forearm_l/r, hand_l/r, claws_l/r *Glow*, thigh_l/r, shin_l/r |
| Spitter (~1.6 m) | `enemy_spitter.gltf` | pelvis, body, pustules *Glow*, neck, head, jaw, maw *Glow*, sac[neck] *Glow* (scaled by the pose), arm_l/r, forearm_l/r, thigh_l/r, shin_l/r |
| Gnat (~1 m span) | `enemy_gnat.gltf` | thorax, head, emitters[head] *Glow*, abdomen, core[abdomen] *Core*, wing_fl/fr/bl/br (translucent), legs |
| Bastion (~3 m) | `enemy_bastion.gltf` | pelvis, torso, armor[torso], furnace[torso] *Core*, head, visor *Glow*, beacon[torso] *Glow*, upper_arm_l/r, forearm_l/r, fist_l/r, lamp_l/r[fist] *Glow*, thigh_l/r, shin_l/r |
| Lancet | `weapon_lancet.gltf` | body, cell *Glow* |
| Thresher | `weapon_thresher.gltf` | body, pump (slides back), glow *Glow* |
| Riveter | `weapon_riveter.gltf` | body, barrels (rotates about the bore), drum (side magazine, rotates), glow *Glow* |
| Cinderbox | `weapon_cinderbox.gltf` | body, stripe (hazard band), core *Glow* (side windows + maw) |
| right arm (1st person) | `vm_arm.gltf` | forearm, hand, trim *Team* |
| left fist (1st person) | `vm_fist.gltf` | forearm, fist, trim *Team*, knuckles *Glow* |
| projectiles | `proj_rivet/sawblade/rocket/flak/sticky/mortar/spitter_orb/gnat_bolt/prism.gltf`, `fx_blood_core.gltf` | `body` + `glow` (rivet, rocket, flak, sticky, mortar); `blade` + `edge` (sawblade, horizontal disc: spin it about Y); `core` + `shell` (orb); `bolt` + `core`; `shard` *Core* + `rim` *Glow* (prism, ~0.5 m, stands along Y); `core` |
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
on): `lineup` / `lineup_back` (scale and silhouettes, back = Bastion furnace weak point),
`closeup_<enemy>` (front 3/4 + back), `shrike_0..3` + `shrike_closeup` (every movement state
in profile, team colour), `husk_1/2`, `spitter_1/2`, `gnat_1/2`, `bastion_1/2` (idle, walk,
wind-up early / late, attack, recover, stunned, dying, dead), `vm_00..19` (each weapon rest /
kick / alt mode, overcharge, riveter spin, switch, punch, prism throw, bob + sway),
`projectiles`, `props`.

## Known limitations

- Rigs are rigid parts (no skinning); elbows / knees show small gaps or overlaps at extreme
  angles. Joint pivots are checked by a test to sit on their part's geometry.
- Pivots / hierarchy are code-defined; the glTF files carry no node hierarchy (the engine's
  writer has no node transforms), so an artist edit can change geometry but not the rig.
- Gnat wing flaps at 23 Hz alias at 60 fps on purpose (reads as a buzz blur).
- The Gnat `Dying` pose only drops ~0.5 m; let the corpse fall / remove it client-side.
- Viewmodel arms are generic: one right arm grips every weapon (fingers don't wrap each grip
  exactly), and the left fist doubles as the prism-throwing hand.
- Labels in `models_view` sit at floor level (hovering Gnats are labelled below themselves).
