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
  textures are neutral detail maps (leather grain, porcelain glaze, cloth wraps, reflection
  bands, brushed plate, grime, veined skin, poured wax with drip runs, bone, knurling, hazard
  stripes, emissive noise).
- Part shading (`rig::Shade`):

  | shade | drawn | colour |
  |---|---|---|
  | `Lit` | lit | vertex × texture × base colour × `Tint::body` |
  | `Team` | lit | … × `Tint::team` (player / team colour slot, vertex colours near-white) |
  | `Glow` | **unlit** | … × `Pose::glow` × `Tint::glow` — **attack telegraphs**, weapon glow |
  | `Core` | **unlit** | … × `Pose::core` — steady emissive: weak points, lamps, the heart of the Shrike's wick |
  | `TeamGlow` | **unlit** | … × `Tint::team` × `Pose::glow` (the Shrike's eye slits, first-person knuckle embers) |
  | `Primary` | lit | … × `Tint::primary` (the Shrike's coat, sleeves, coat tails, cloaks: the chosen coat colour) |
  | `Secondary` | lit | … × `Tint::secondary` (masks, crests, cloak trims: the chosen mask colour) |
  | `Flame` | **unlit** | … × `Tint::flame` × `Pose::glow` (the wick, the halo crest, lantern emblems: the chosen flame colour) |

  Emissive colours are not clamped before the framebuffer, so `glow` values > 1 push towards
  white (white-hot flashes). Translucent parts have base colour alpha < 1 (the aim laser's
  sheath, exit portal, the Arclash's glass capacitor, the Wisp's halo, the Thorn's glass) and
  go through the renderer's translucent pass.

## Runtime API (`client::models`)

```rust
let lib = ModelLibrary::load(&mut renderer)?;           // once at startup

// third-person player (other players / spectating); body faces anim.yaw
// (anim.cosmetics = the player's look; the tint's team colour is their identity colour)
let info: PlayerDrawInfo = lib.draw_player(&player_anim, feet_pos, &Tint::team(color), &mut draws);
// other players are drawn 10 % larger (visual only): shrike::REMOTE_SCALE
let info = lib.draw_player_scaled(&player_anim, feet_pos, shrike::REMOTE_SCALE, &tint, &mut draws);
// info.muzzle / info.muzzle_dir (world) for remote tracers & flashes, info.head for name tags
// any root (e.g. view space for a preview in the viewmodel pass: menus::customize)
let info = lib.draw_shrike(&player_anim, root, &tint, &mut draws);

// enemies (drawn kind.scale() = the definition's model_scale times their built size, about
// the root: docs/enemies.md "Sizes and hit shapes")
let root = EnemyAnim::root(position + Vec3::Y * kind.root_lift(), yaw); // feet (Harrier: hover centre)
lib.draw_enemy(&enemy_anim, root, &Tint::default(), &mut draws);
let at = lib.enemy_attack_point(&enemy_anim, root); // jaw / censer pot / beak / door or flail / lens

// first-person viewmodel (view space -> FrameDesc::viewmodel, viewmodel_fov_y_degrees ~60)
let vm: ViewmodelDrawInfo = lib.draw_viewmodel(&vm_anim, &Tint::team(color), &mut viewmodel);
// vm.muzzle / vm.muzzle_dir (view space; transform by the camera for world tracers),
// vm.fist (knuckles / the held thorn's tip, for punch sparks)

// projectiles and props
lib.draw_model(ModelId::Bomb, Mat4::from_rotation_translation(look_rot, pos), &Tint::default(), &mut draws);
// a planted thorn: drawn at its planted position along its flight direction, the tip sinks
// projectiles::THORN_TIP (0.22 m) into the surface
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

`Tint { body, team, glow, glow_color, core_color }`: `body` multiplies everything (hit flash
> 1, fade-out alpha < 1), `team` is the player colour (`DEFAULT_TEAM_COLOR` = deep red), `glow`
lets the client flash telegraph parts on top of what the pose already does. `glow_color` /
`core_color` (default `None`) *replace* the colours of `Glow` / `Core` parts with a palette role
(the colour-blind palettes: `palette().model_glow(palette().telegraph)`, `weak_point`,
`parry_window`, `hostile_projectile`, `aim_laser`): the library uploads a greyscale copy of
every `Glow` / `Core` mesh (`rig::neutral_mesh`) and draws that in the role colour at the
brightness of `Pose::glow × Tint::glow` (or `Pose::core`) **capped at full**, so the hue
survives where the art's HDR multipliers would wash it out to white. With `None` nothing
changes, so the standard look keeps each model's own glow colours.

## Animation inputs

### `PlayerAnim` (Shrike)

| field | meaning |
|---|---|
| `velocity` | world m/s (run cycle amplitude; legs turn up to ±0.6 rad towards the movement direction, the rest becomes side-stepping; backwards movement reverses the cycle) |
| `yaw`, `pitch` | look angles; body faces yaw, torso / head / weapon follow pitch |
| `on_ground`, `sliding`, `dashing`, `slamming`, `wall_cling: Option<normal>` | movement state (priority: slide > slam > dash > wall > air > ground) |
| `firing` | recoil 0..1 (1 on the shot, decay over ~0.2 s) |
| `punching` | punch progress 0..1 (left arm thrust; fast out in the first 25 %) |
| `downed`, `dead` | kneeling with a guttering wick / flat on its back, wick snuffed (flame hidden, eye slits dark, a dull smoulder) |
| `time` | seconds (idle breathing, the wick's flicker: flame height, eye slits) |
| `weapon` | the held weapon's wire id (`shared::loadout::WeaponKind`: 0 Quill, 1 Bellows, 2 Arclash, 3 Tumbler, 4 Lancet, 5 Thresher, 6 Riveter, 7 Cinderbox, 8 Longwick, 9 Kiln, 10 Trimmer, 11 Tamper; `weapons::two_handed`: the Quill and Lancet are one-handed) |
| `ability`, `draw`, `charge` | the selected ability id (the held weapon glows in its colour; one the weapon can't take shows its default build's first), the Longwick's draw and an alt charge 0..1 (from the replicated `player_weapon` and `DRAWING` / `CHARGING`) |
| `unarmed` | holding nothing: no weapon drawn, no two-handed grip |
| `swing` | a melee move in progress (`weapons::melee::Swing { kind, progress, left }`; keep it per player and `advance` it with the held melee weapon): both arms follow it (`melee::arms`), the Trimmer's left half in the left hand, the edges / band flaring through the blow |
| `cycle` | locomotion phase — keep per player: `cycle = PlayerAnim::advance_cycle(cycle, velocity, dt)` (phase keeps legs continuous when speed changes; leg rate capped at 2.6 cycles/s) |
| `cosmetics` | the look (`shared::cosmetics::Cosmetics`): mask, crest, cloak, emblem, coat / mask colours, flame (below) |

### `ViewmodelAnim` (first person)

| field | meaning |
|---|---|
| `weapon`, `ability` | the held `WeaponKind` and its selected `Ability`; the weapon's `Glow` parts take the ability's colour (`weapons::ability_color`, one per ability; see below); `WeaponLook::primary_color` is each gun's primary-fire colour for tracers / beams |
| `fire_kick` | 1 on fire, decay (~0.3 s); also drives the mechanisms: the Quill's crest folds flat front to back and rises again, the Bellows pumps (board swings shut half way through the decay, pleats squash, then refill), the Lancet's feed sleeve snaps back, the Riveter's nose hammers back and one air pulse runs down its hose (`riveter::pulse_position`: the kick falls 0.3 between rivets, which carries the pulse from the tank to the tool), the Longwick's string snaps and quivers and its arrow is gone until the kick fades below 0.35 (the next one nocked) |
| `alt_charge` | 0..1 alt charge (Discharge, Skewer): any gun's glow brightens towards white; the Arclash's capacitor light speeds up and arcs jump between its prongs (flickering); the gun shakes above 0.6 |
| `draw` | 0..1 Longwick draw: the nock and the arrow come back by up to `longwick::DRAW_LENGTH` (0.3 m), the limbs flex, the wick string and the arrow's flame heat up and strobe white at full draw, the left fist pinches the nock (`longwick::draw_hand`), the bow trembles past 85 % |
| `salvo` | seconds since a Cinderbox salvo started (`None` = no salvo): the tubes fire top, left, right 0.08 s apart (`cinderbox::SALVO_GAP`), each mouth flares (`tube_flash`), the box kicks once per rocket (`salvo_kick`), the back face's three "loaded" lamps go dark as their tubes fire and relight at `RELOAD` (0.9 s) |
| `spray` | 0..1 Kiln spray (smoothed while it fires): the firebox coals roar, the chimney's damper rattles, the gun shakes |
| `unarmed` | no weapon at all (the client sets it when the carried loadout is empty): the right hand (the arm's closed glove) raised and the left fist up in a guard, punching as usual |
| `swing` | a melee move (`melee::Swing`): the Trimmer / Tamper follow its keyframed track (below) |
| `throw_item` | what the left hand throws (`None` = a Thorn; a knack's flask / snare / pot model) |
| `switch` | 0 (just switched, below the screen) → 1 (raised); animate over ~0.12 s |
| `punch` | punch progress 0..1: the left fist thrusts in from the lower left; knuckle studs flare |
| `throw` | throw progress 0..1: the left hand rises with a thorn held point-forward between the knuckles (or the knack's flask / pot cupped on the fist, the snare's disc edge-on), flicks it at ~0.5 |
| `bob_phase`, `bob_amount` | walk bob (advance phase by ~speed × 1.2 rad/s; amount = ground speed / run speed) |
| `sway` | smoothed look velocity (rad/s); the weapon lags |
| `spin` | Tumbler / Thresher drum angle — keep a `DrumTurn` per local player: `drum.kick()` on every Tumbler or Thresher shot, `drum.update(dt)` every frame, `spin = drum.angle()` (one chamber = `DrumTurn::STEP`, eased in with ~10 % overshoot; the Thresher's six shell windows have the same spacing) |
| `time` | seconds |

Weapons are modelled at real scale (the third-person Shrike holds the same models) and drawn
at `weapons::VIEWMODEL_SCALE` (0.52) in first person, grip around 0.3 m in front of the
camera, lower right. Each weapon exposes its muzzle (`WeaponLook::muzzle()`, model space). The
Longwick is canted almost flat (`longwick::view_rotation`) and held further out so the bow
crosses the view; the right arm keeps its usual angle and runs off the bottom of the screen.

The client draws the shared `shared::loadout::WeaponKind` (wire ids); the visual side is the
`weapons::WeaponLook` trait (`model`, `muzzle`, `two_handed`, `primary_color`, `glow_color` of the
default build's alt modes) plus `weapons::weapon_kind(id)` (unknown ids draw the Quill) and
`weapons::ability_color`. `weapons::weapon_pose(&HeldWeapon)` poses any weapon
(`ViewmodelAnim::held()` in first person, `third_person_pose(weapon, firing, time)` for other
players until the replicated state carries their ability and draw).

| class | ability colours (`ability_color`) |
|---|---|
| Precision | Brand crimson, Splinter ice, Skewer white-gold, Flurry mint |
| Scatter | Gust sky, Both Barrels orange, Slug brass, Flak rose |
| Rapid | Wisp violet, Discharge blue, Volley amber, Flare signal red |
| Explosive | Breach orange-red, Gravewell green, Limpet yellow, Mortar lilac |
| Draw | Brand crimson, Triple Nock lime, Pinning Arrow steel blue, Ember Arrow ember |
| Melee | Lunge cyan, Whirl amber, Hurl violet, Upswing lime |

### Melee (`weapons::melee`, docs/refit.md)

The Trimmer and the Tamper are held and swung, not aimed: `melee::of(WeaponKind)` /
`of_id(id)` → `MeleeWeapon`. A swing is `Swing { kind: SwingKind::{Slash, Lunge, Whirl, Hurl,
Upswing}, progress 0..1 (0 = idle), left }` (`advance(weapon, dt)`, `flare(weapon)` for the
glow; `duration` / `impact` per move). Every move reaches its blow early (the hit is on the
click's tick: the Trimmer's slash at 0.09 s of 0.3 s, the Tamper's slam at 0.1 s of 0.78 s)
and spends the rest recovering.

- **First person**: `viewmodel_transform` / `draw_viewmodel` use a keyframed view-space track
  of the right hand per move (`melee::view_right`; the Trimmer's left blade `view_left`
  mirrors it, its slashes alternate hands); bob, sway and the switch's lowering are added on
  top. The forearms are re-aimed back at the shoulders (`aim_arm`) whatever the wrist does, so
  the arm never sweeps across the view. The left hand: the Trimmer's left blade in the left
  fist; the Tamper's haft (it slides down to the right hand for the slam and the whirl);
  punching, the left blade rides in the fist (`LeftHand::Punch`); throwing a knack, it is
  put away (`LeftHand::Busy`). A Hurl hides the thrown blade / the head's glow until the hand
  comes back up (`hurl_hidden`). `ViewmodelDrawInfo::muzzle` is the strike point (the hook,
  the foot); `strike_path` samples it through a move.
- **Third person**: `melee::arms(weapon, swing, reach, pitch)` gives both arms' rotations
  (`shrike::pose_full` applies them; a punch still takes the left arm); at rest the weapon
  points along the look like a gun. `ModelLibrary::draw_shrike` puts the Trimmer's left half in
  the left hand (`left_blade_local`).
- **Hurled**: `melee::draw_hurled` draws just the blade (Trimmer) or the head (Tamper), spun
  end over end about its middle.

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
| Bellwether | the bell's lip (orange → white-hot, strobing for the last quarter of the toll), the eyes in the hood, the right fist's brass studs | the bell's shoulder band and the clapper glowing in its mouth (≥ 1.6 while tolling) | toll: rears up, arms overhead hauling the bell back on its yoke (it trembles), then the bell swings through and rocks to rest; clapper: a fist drawn back with a twist, a backhand sweep; cracked (stunned): reels, the bell swinging wildly |
| Snuffer | the cap's rim (red-hot) | the wick under the cap (blazes as it rears) | latch: rears back, the cap tipped up off the wick, legs bunched and quivering; the leap stretches out, legs forward; `Latched`: legs wrapped round, cap pressed down, heaving; nip: cap lifts and snaps; thrown off (stunned): on its back, legs pedalling |
| Smelter | the slag brimming in the crucible and the spout (brightens as it tips) | the crucible's pour hatch, the head's furnace grate | pour: plants and leans back, tipping the crucible forward over its lip, held shaking while it pours; heave: twists away and heaves the crucible across the front; death: to its knees, the crucible tips out of its fists |
| Effigy | the rope binds (never winds up; flash on hits) | the chest sigil, the wick-knot on the head | none: a hit (`Recover`) rocks the figure on its post and bobbles the head |

## Models and parts

| model | file | parts (parents in brackets) |
|---|---|---|
| Shrike (~1.86 m incl. plume) | `shrike.gltf` | pelvis, torso[pelvis] (waist, collar, pauldrons, harness, lantern cage), wick[torso] *Flame* (flame, flickers in height), trim[torso] *Team* (scarf, pauldron edging), head[torso] (the hood; masks and crests attach here), coat[torso] *Primary* (jerkin, lapels, back), upper_arm_l/r[torso] *Primary* (sleeves), forearm_l/r, hand_l/r, thigh_l/r[pelvis], shin_l/r, foot_l/r, coat_l/r[pelvis] *Primary* (coat tails), ember[wick] *Core* (the wick's hot heart) |
| Shrike kit (cosmetics) | `shrike_kit.gltf` | 41 root parts, each attached to a Shrike joint when drawn (below): `mask_*` *Secondary* + `eyes_*` *TeamGlow* (8 each), `horns` / `grille_bars` / `crown_spikes` / `lantern_cage` (fixed colours), `crest_quills/fin/antlers` *Secondary*, `crest_tassel` *Secondary* (swings), `crest_halo` *Flame*, `cloak_short/long/tattered/scarf` *Primary* + `_trim` *Secondary* (swing), `emblem_plate` (brass medallion) + `emblem_bell/key/eye/spiral/fang/star/wing` *Flame* |
| Gnawer (2.0 m to the hump, 1.8 m long) | `enemy_gnawer.gltf` | pelvis, chest, head, jaw[head], throat[head] *Glow*, eyes[head] *Core*, upper_arm_l/r[chest], forearm_l/r, fist_l/r, thigh_l/r[pelvis], shin_l/r |
| Censer (2.7 m incl. the flue) | `enemy_censer.gltf` | robe, torso, head (brazier cage), coals[head] *Core*, flames[head] *Glow*, arm_l/r, forearm_l/r, chain[forearm_r], censer[chain], embers[censer] *Glow* |
| Harrier (3.05 m wingspan) | `enemy_harrier.gltf` | body, neck, head, glare[head] *Glow*, heart[body] *Core*, wing_l/r, tip_l/r[wing], legs, tail |
| Gaoler (3.73 m) | `enemy_gaoler.gltf` | pelvis, torso, lantern[torso] *Core*, helm, grille[helm] *Core*, arm_l, forearm_l, shield[forearm_l] (the cell door), rim[shield] *Glow*, arm_r, forearm_r, chain[forearm_r], chain_glow[chain] *Glow*, flail[chain], thigh_l/r, shin_l/r |
| Heron (3.35 m) | `enemy_heron.gltf` | body, neck, head, lens[head] *Core*, ring[head] *Glow*, thigh_l/r[body], shin_l/r |
| Bellwether (3.24 m to the yoke) | `enemy_bellwether.gltf` | body, head, eyes[head] *Glow*, yoke[body], bell[yoke] (swings about its hang point), bell_core[bell] *Core*, bell_lip[bell] *Glow*, upper_arm_l/r[body], forearm_l/r, knuckles[forearm_r] *Glow*, thigh_l/r, shin_l/r |
| Snuffer (2.4 m long, 1.6 m to the cap) | `enemy_snuffer.gltf` | body, neck[body] (the snuffer's handle), cap[neck], wick[neck] *Core* (stays as the cap lifts), rim[cap] *Glow*, leg_l0..2 / leg_r0..2[body], tail[body] |
| Smelter (3.43 m) | `enemy_smelter.gltf` | body, head, mouth[head] *Core*, crucible[body] (tips about its front lip), hatch[crucible] *Core*, slag[crucible] *Glow*, arm_l/r[body], fore_l/r, thigh_l/r, shin_l/r |
| Effigy (2.27 m) | `enemy_effigy.gltf` | post, figure[post] (rocks about its lashing), binds[figure] *Glow*, sigil[figure] *Core*, head[figure], wick[head] *Core* |
| slag line | `fx_slag_strip.gltf` | strip *Glow* (molten cells), rims *Glow* (edge lines: the telegraph): a unit strip 1 m wide, 1 m along −Z, scaled per floor segment |
| wax puddle | `fx_wax_puddle.gltf` | pool (lumpy flattened lobes, tinted with the Mould's wax), heart *Glow* (the molten middle and a drip run): 1 m in radius at the origin, scaled per Mould under a melting / re-forming one (`docs/enemies.md` "Leaps, pads, melting") |
| Quill | `weapon_quill.gltf` | body, channel *Glow* (barrel flanks, nib slit), quill_0..4 (crest quills, each folds about its socket), tip_0..4[quill_i] *Glow* |
| Bellows | `weapon_bellows.gltf` | body, board (lower bellows board, hinged at the front), pleats (squashed vertically about the hinge), glow *Glow* (gauge face, throat rings, vents) |
| Arclash | `weapon_arclash.gltf` | body, glass (translucent capacitor), crawl *Glow* (filaments, turn about the bore), tips *Glow* (prong tips, strap spark, lamps, charge eye on the back), arcs *Glow* (hidden unless charging / on a strong shot) |
| Tumbler | `weapon_tumbler.gltf` | body, drum (six chambers, turns about +Z), drum_glow[drum] *Glow* (fuse bands in the chamber mouths), glow *Glow* (muzzle throat, fuse windows) |
| Lancet (needle pistol: blued lozenge receiver, glass needle vial on top, long thin silver needle with copper fins, a scalpel-spine blade over it, syringe finger flanges; no cylinder, no hammer) | `weapon_lancet.gltf` | body, glass (translucent vial), needles *Glow*, slide (feed sleeve, snaps back on the shot), glow *Glow* (channel under the needle, point ring, eyes) |
| Thresher (drum-fed auto-scattergun: an upright brass drum like a threshing wheel under the receiver, caged barrel with heat coils, flattened duckbill muzzle; no pump) | `weapon_thresher.gltf` | body, drum (turns about +X), shells[drum] *Core* (six shell windows per face), glow *Glow* (heat coils, duckbill slit, lamps) |
| Riveter (pneumatic rivet driver: green enamel housing, steel nose with a rivet-set cup, brass feed of hot rivets, copper pressure tank under the nose, rubber hose along the left flank) | `weapon_riveter.gltf` | body, nose (hammers back), rivet[nose] *Core*, heads *Core* (the feed's rivets), glow *Glow* (gauge dial, cap lamp), hose_0..3 (bulge in turn as the pulse passes) |
| Cinderbox (squat sooty box flaring to a brass face, three short clay tubes in a triangle, cinder vents, carry handle) | `weapon_cinderbox.gltf` | body, cinders *Core* (behind the vent slats), glow *Glow* (mouth rings), flare_0..2 *Glow* (shown while that tube fires), lamp_0..2 *Glow* ("loaded" lamps on the back face) |
| Longwick (recurve bow of wax-hardened wood, ~1.1 m: dark riser with brass limb pockets, amber limbs sweeping back and flicking forward at the tips, a glowing wick string, a taper arrow nocked) | `weapon_longwick.gltf` | riser, limb_upper / limb_lower (flex about their pockets), string_upper / string_lower *Core* (each a full transform from its flexed tip to the nock), arrow (slides back with the nock), flame[arrow] *Core*, glow *Glow* (riser inlays, pocket rings) |
| Kiln (ceramic ember sprayer: long terracotta spout on top with glazed bands, firebox belly underneath with grated, glowing doors, a chimney with a hinged damper, brass feed cap) | `weapon_kiln.gltf` | body, damper (rattles), coals *Core* (door grates, chimney mouth), glow *Glow* (spout throat, cap lamp) |
| Trimmer (hooked wick-shears carried as two blades, ~0.6 m each: straight blued blades bent at the end into an angled jaw, a bright bevel, a brass pivot boss, leather-wrapped handles, brass finger loops closed round the knuckles; the right half carries the snuffer box on its spine with a wick stub smouldering in it; bind pose = the closed pair side by side) | `weapon_trimmer.gltf` | blade_r (grip at the origin), edge_r[blade_r] *Glow* (the honed edge), wick[blade_r] *Core*, blade_l (mirrored, grip at `trimmer::LEFT_GRIP`), edge_l[blade_l] *Glow* |
| Tamper (foundry ramming hammer, ~0.95 m: an ash haft up the grip's axis with a leather grip and brass pommel; a black-iron head with the rammer's broad round foot ringed in brass on its face, a wedge peen behind, langets down the haft, slag glowing in vents on its flanks) | `weapon_tamper.gltf` | haft, head[haft] (pivot `tamper::HEAD_CENTER`), band[head] *Glow* (round its waist), vent[head] *Core* |
| right arm (1st person) | `vm_arm.gltf` | forearm (cloth wraps), hand (glove), trim *Team* (cord round the bracer), bracer (brass) |
| left fist (1st person) | `vm_fist.gltf` | forearm (wraps, brass bracer, cuff), fist (glove, brass knuckle plate), trim *Team* (cords), knuckles *TeamGlow* (ember studs, flare with the wick's colour on the punch) |
| player projectiles | `proj_bomb/gravewell/wisp/thorn.gltf` (`models::projectiles`) | `body` + `glow` (bomb: studded iron ball, orange fuse band; gravewell: canister with green core rings and front clamp claws); wisp: `core` + `tendrils` + `halo` (translucent); thorn: `glass` *Core* (translucent) + `core` *Glow* (amber core line) + `collar` (brass fletching), ~0.64 m, tip `THORN_TIP` = 0.22 m in front of the origin |
| arsenal projectiles | `proj_arrow/arrow_pinning/arrow_ember/rivet/cinder_rocket/flak_shell/limpet/mortar_shell/flare.gltf` | `body` + `glow`, all flying along -Z: taper arrow (wax shaft, red cock vane, iron head, the taper's flame behind it, ~0.72 m), pinning arrow (dark shaft, barbed steel head, chain, steel-blue rings), ember arrow (iron cage round a bulb of coals), rivet (red-hot, domed head at the back), cinder rocket (clay casing, brass nose, fins, cinder exhaust), flak shell (brass case, rose timer band, ring of pellet tips), limpet (iron dome on four claws facing -Z, a lamp to blink), mortar shell (fat iron teardrop, tail fins, lilac bands, fuse nose), flare (brass-capped tube, sputtering flame head) |
| knacks (`models::knacks`) | `prop_kindle_flask/snare/soot_pot.gltf` | flask: `body` (brass cage, wax stopper, sling) + `glass` (translucent belly) + `embers` *Glow* (kindling inside); snare (~0.9 m wax trap on the floor): `base` (poured wax disc, brass ring, hinges) + `jaw_r` / `jaw_l` (toothed brass half hoops, open flat; `knacks::snare_pose(armed, sprung, time)` snaps them up past upright) + `lamp` *Glow* (the wick lamp: dull arming, pulsing armed, flashing sprung); soot pot: `body` (black clay, cord net, rag) + `fuse` *Glow* |
| enemy projectiles / fx | `proj_ember_bomb/needle.gltf`, `fx_ember_core.gltf`, `fx_aim_laser.gltf` | `shell` + `cracks` *Glow* (ember bomb, a 0.3 m slag ball with glowing seams); `shaft` + `glow` *Glow* (needle, 0.95 m, white-hot point, red fletch); `core` (ember core: a white-hot coal with glowing shards); `core` + `sheath` (translucent) *Glow* (aim laser: a unit beam 1 m along -Z, scaled to width × width × length and tinted through `Tint::glow`; the Heron's laser and needle streaks) |
| props | `prop_jump_pad/checkpoint/exit_gate/door.gltf` | jump pad: base, rim, glow; checkpoint: base, lamp, halo; exit gate: frame, portal (translucent), lights *Core*; door: frame, panel_l, panel_r, light |
| hub props (`models::hub`) | `prop_ready_board.gltf`, `prop_console.gltf` | placed by the Vestry as level props: the ready board (frame, slate, brass header plate + candle sconces; 6 × 3.4 m, the client writes on the slate: `hub::board`), the practice console (stone lectern with a slanted desk, a summoning bell and a lever, panel *Glow*; a sign board: `hub::console`) |

Prop poses: `door_pose(open 0..1, locked)` slides the panels apart (unit door
`props::door::WIDTH` 3 m × `HEIGHT` 3.5 m — scale to the door brush; light red = locked,
amber = closed, green = open), `checkpoint_pose(active, time)` (dim red → bright cyan, halo
bobs and spins), `jump_pad_pose(time)`, `exit_gate_pose(time)`. Glow parts of static models
drawn with `draw_model` use the vertex colours as-is (`Pose::glow` = 1).

Budget: ~1–2k triangles per character and weapon (the Shrike ~1.9k, Arclash / Tumbler ~1.7k),
20–650 per projectile / prop; ~2.5 MB of glTF in total (vertex data dominates: flat-shaded
faces need their own vertices).

## Cosmetics (`models::cosmetics`, `shared::cosmetics`)

A player's look (`Cosmetics`: mask, crest, cloak, emblem, coat colour, mask colour, flame; saved
in the client settings, shared through `PlayerInfo` — `docs/lobby.md`) is drawn by
`ModelLibrary::draw_shrike` / `draw_player*` from `PlayerAnim::cosmetics`:

- The **paints** go through `Tint::with_cosmetics` into the `Primary` (coat, sleeves, tails,
  cloak bodies), `Secondary` (masks, crests, trims) and `Flame` (wick, halo, emblems) shades.
  Painted meshes are near white with baked shading (`shrike::PAINT`, `PAINT_DARK` for straps and
  lapels), so the classic look (coat HIDE = the old leather, mask BONE, flame AMBER:
  `Cosmetics::default()`) matches the old model.
- **Identity stays readable**: the player's slot colour (`Tint::team`, colour-vision palettes)
  is never a cosmetic: it stays on the scarf / pauldron edging (`Team`) and on every mask's eye
  slits (`TeamGlow`), and on name tags and markers. A test checks every mask has team-coloured
  eyes.
- The **kit** (`ModelId::ShrikeKit`) holds every option as a part; `cosmetics::parts(look)` picks
  at most six (mask + eyes [+ detail], crest, cloak + trim, medallion + glyph) and
  `cosmetics::draw` attaches each to the posed Shrike (`Attach`): masks / crests / the halo ride
  the head, emblems the torso; cloaks hang from `shrike::CLOAK_HANG` behind the collar and a tassel
  from `shrike::TASSEL_KNOT`, swinging with the coat tails (`shrike::pose_full` → `Drape`: the
  tails' world swing with the torso's own pitch taken out; a shoulder mantle swings a quarter as
  much). Parts are built in the Shrike's model space; a kit part's pivot is its attachment point.
- **Masks** (each a different silhouette): SHRIKE (the long hooked porcelain beak with a ridge),
  HOOKBILL (short raptor beak under a heavy brow), LANTERN (round globe face, one lens behind a
  brass cage, a chimney cap), VISOR (flat helm face, one slit across, chin guard), HORNED (ram
  snout, round eye holes, curling bone horns), WEEPER (long oval, downturned slits, tear
  grooves), GRILLE (flat plate, glowing band behind iron bars), CROWN (serene full face, a brass
  crown of spikes). **Crests**: NONE, QUILLS (the swept plume), FIN, ANTLERS, TASSEL (top knot,
  swinging tassel), HALO (a burning ring in the flame colour). **Cloaks**: NONE, SHORT (shoulder
  mantle), LONG (to the calves), TATTERED (torn strips), SCARF (two trailing tails). **Emblems**: a
  brass medallion on the chest lantern with a glyph burning in the flame colour (BELL, KEY, EYE,
  SPIRAL, FANG, STAR, WING).
- Cosmetics never touch hit shapes or movement (purely visual). The death / respawn glows use
  the flame colour too (`ingame::combat`).
- `menus::customize::draw_preview` draws a turning Shrike on a dark card into the viewmodel pass
  (any screen: the main menu, the lobby, the warm-up overlay); `look_swatch` is the 2D swatch
  (coat | mask colour, flame dot) for lists.

## Visual verification

`models_view` renders (1280×720 = the game's 640×360 internal image at 2×, retro settings
on): `lineup` / `lineup_back` (scale and silhouettes, back = the Gaoler's lantern),
`closeup_<enemy>` (front 3/4 + back), `shrike_0..3` + `shrike_closeup` (every movement state
in profile, team colour), `gnawer_1/2`, `censer_1/2`, `harrier_1/2`, `gaoler_1/2/3`,
`heron_1/2` (idle, walk, every attack's wind-up early / late and strike, recover, stunned,
melting, dead), `look_masks` / `look_crests` / `look_cloaks` (from behind, standing and
running) / `look_emblems` / `look_random` (cosmetics), `vmx_00..17` (the arsenal in first person: Lancet rest / fire / Skewer charged,
Thresher drum turning, Riveter hose pulse at the tank and at the tool, Cinderbox salvo 1st / 3rd
tube, Longwick rest / half / full draw / release, Kiln rest / spraying / Discharge),
`weapons_x` / `weapons_x34` and `projectiles_x` (the arsenal at real scale), `melee_vm_00..16`
(the Trimmer and Tamper in first person: rest, the slashes' wind / mid / impact, the left
blade, the slam's wind / fall / impact / recovery, Lunge, Whirl, Hurl, Upswing),
`melee_weapons` / `melee_weapons_34` (real scale), `melee_hurled`, `melee_3p_trimmer` /
`melee_3p_tamper` (third person through the swings), `knack_props` (flask, pot, snare
arming / armed / sprung), `vm_00..22` (each
weapon rest / kick / alt mode, Quill crest folding, Bellows
pump, Arclash Discharge 40 % / full, Tumbler drum turning, switch, punch, thorn throw, bob +
sway), `weapons` / `weapons_34` (the guns at real scale), `projectiles`, `thorns` (planted at
angles, lying, in flight), `props`.

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
  exactly), and the left fist doubles as the throwing hand and the Trimmer's / Tamper's left
  hand (aimed at the shoulder, its fist closed round the handle).
- Melee keyframes are hand-tuned per move (`melee::track` / `arm_track`); the third-person
  Tamper's left hand only roughly follows the haft.
- In first person the Bellows' pump and the Arclash's prong arcs are mostly seen from behind
  (the pleats bulge out to the left, the capacitor and its charge eye face the camera); the
  third-person view shows them fully.
- Planted thorns are small at a distance (~0.42 m sticks out); the client can add a glow
  sprite / light to make relays easier to spot.
- Labels in `models_view` sit at floor level (hovering Harriers are labelled below themselves).
