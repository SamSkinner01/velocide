//! The rework's tricks (docs/rework.md, area client-a): everything you see and hear of the
//! Lamplighter and of the reworked Precision, Draw and Melee abilities.
//!
//! * [`lamplighter`]: the scope (zoom, lens vignette, focus ring / meter), the focus feel, the
//!   beam, and the tell other players see on a focusing Lamplighter.
//! * [`wickline`]: the burning line and its bite.
//! * [`tally`]: the held tally round the crosshair and the marks on the tallied targets.
//! * [`waxwork`]: the wax double, its slump and its burst.
//! * [`blink`]: the Blink Arrow's step (afterimage, streak, arrival) and its prompt.
//! * [`beacon`]: the lit beacon and the revealed hostiles' silhouettes through walls.
//! * [`riposte`]: the guard on the HUD, the sparks off a block, the counter's crescent.
//! * [`play`]: all of it in the running game (the replicated world, the own state, the
//!   server's effects).
//!
//! [`Tricks`] holds their per-frame memory; the glue in `combat/mod.rs` calls it like the
//! other views (`arsenal`, `melee`, `knacks`): [`Tricks::update`] once a frame (effects,
//! sounds, the tells), [`Tricks::draw`] with the world (models, lights, and the silhouettes
//! into the viewmodel pass), [`Tricks::hud`] over it. [`TrickPreview`] /
//! `InGame::preview_tricks` stage any of it client-side for tools (`combat_shots`).

pub mod beacon;
pub mod blink;
pub mod lamplighter;
mod play;
pub mod riposte;
pub mod tally;
pub mod waxwork;
pub mod wickline;

use std::collections::{HashMap, HashSet};

use engine::audio::{Audio, Sfx};
use engine::gfx::{Camera, DrawItem, PointLight, UiDrawList};
use engine::physics::CollisionWorld;
use engine::ui::Rect;
use glam::Vec3;
use shared::cosmetics::Cosmetics;
use shared::protocol::{EntityState, NetId, kind};
use shared::sim::{PlayerId, WeaponState};
use shared::tuning::CombatTuning;

pub use play::{OwnFx, world_muzzle};

use super::super::InGame;
use super::super::effects::Effects;
use super::super::remote_players::RemotePlayers;
use super::Rng;
use super::entities::{enemy_anim, enemy_kind, enemy_size};
use super::fx::Beams;
use crate::models::{EnemyAnim, ModelLibrary, PlayerAnim, Tint};

/// Something staged client-side for tools and screenshots (no simulation involved).
#[derive(Clone, Debug, PartialEq)]
pub enum TrickPreview {
    /// Your own Lamplighter held at `focus` for `seconds`: the scope's zoom and overlay, the
    /// focus ring.
    Focus { focus: f32, seconds: f32 },
    /// Someone's focusing Lamplighter: its tell at `muzzle`, aiming along `aim`.
    Tell {
        muzzle: Vec3,
        aim: Vec3,
        focus: f32,
        seconds: f32,
    },
    /// The Lamplighter's beam from `from` through `ends`.
    Beam {
        from: Vec3,
        ends: Vec<Vec3>,
        focus: f32,
    },
    /// A Wickline from `from` to `anchor`.
    Wickline {
        from: Vec3,
        anchor: wickline::Anchor,
    },
    /// The tally held (`held`) with marks on `targets` (chest, height) for `seconds`.
    Tally {
        held: Option<tally::Tallying>,
        targets: Vec<(Vec3, f32)>,
        seconds: f32,
    },
    /// A wax double at `feet` facing `yaw` (`waxy`: as its own side sees it), standing for
    /// `seconds`, then slumping and bursting.
    Waxwork {
        feet: Vec3,
        yaw: f32,
        waxy: bool,
        seconds: f32,
    },
    /// A Blink Arrow step from `from` to `to` (feet).
    Blink { from: Vec3, to: Vec3 },
    /// The Blink prompt, its window `left` 0..1, for `seconds`.
    BlinkPrompt { left: f32, seconds: f32 },
    /// A beacon lit at `at` revealing every Mould within `radius` for `seconds`.
    Beacon { at: Vec3, radius: f32, seconds: f32 },
}

/// What the tricks need from the frame.
pub struct TrickFrame<'a> {
    pub entities: &'a [EntityState],
    pub remote: &'a RemotePlayers,
    pub world: Option<&'a CollisionWorld>,
    /// This frame's camera.
    pub camera: &'a Camera,
    pub local: Option<PlayerId>,
    /// A player's look (their cosmetics).
    pub look: &'a dyn Fn(PlayerId) -> Cosmetics,
    /// The viewmodel pass's field of view (the silhouettes are drawn in it).
    pub viewmodel_fov: f32,
    pub tuning: &'a CombatTuning,
    pub tick_rate: u32,
    /// A PvP world: a wax double is wax only to its caster, a beacon reveals only to its
    /// shooter (in PvE everyone is on one side).
    pub pvp: bool,
    /// The own state: the Tally's marks, the predicted weapons.
    pub tallied: [Option<NetId>; 4],
    pub weapons: &'a WeaponState,
    /// Your weapon's muzzle this frame (world): your own lines leave it.
    pub muzzle: Vec3,
}

/// A staged thing and how long it has left.
#[derive(Clone, Debug)]
struct Staged {
    preview: TrickPreview,
    age: f32,
}

/// The tricks' views.
#[derive(Default)]
pub struct Tricks {
    pub lamplighter: lamplighter::LamplighterView,
    pub wickline: wickline::WicklineView,
    pub blink: blink::BlinkView,
    /// The Lamplighter is in hand (or a preview holds its focus): its scope shows as the view
    /// zooms (another weapon's zoom, the bow's, has no vignette).
    pub scoped: bool,
    /// The scope's view at full focus (`lamplighter.zoom` of the tuning in play).
    pub full_zoom: f32,
    /// Your own zip or step (Wickline, Blink Arrow): seconds left of its speed lines and their
    /// length.
    rush: (f32, f32),
    /// Wax doubles and beacon arrows in the world, and the beacons' lights this frame.
    doubles: HashMap<NetId, play::Double>,
    /// Doubles burst by their `Fx::Explosion` whose entities are still in the frame.
    burst_doubles: HashSet<NetId>,
    /// Where and when doubles burst lately.
    bursts: Vec<(Vec3, f64)>,
    beacons: HashMap<NetId, play::BeaconSeen>,
    beacon_lights: Vec<PointLight>,
    /// The tally's marks (who, since when).
    marks: Vec<(NetId, f64)>,
    /// Blows your guard took since it went up (the counter grows with them).
    guard_blocks: u32,
    /// Sounds due later: seconds to wait, the sound, where (`None`: yours).
    later: Vec<(f32, Sfx, Option<Vec3>)>,
    staged: Vec<Staged>,
    /// Previews waiting for the next update.
    pending: Vec<TrickPreview>,
    time: f64,
}

impl Tricks {
    pub fn new() -> Self {
        Self::default()
    }

    /// New level / restart: drop every transient visual.
    pub fn clear(&mut self) {
        self.lamplighter.clear();
        self.wickline.clear();
        self.blink.clear();
        self.staged.clear();
        self.pending.clear();
        self.doubles.clear();
        self.burst_doubles.clear();
        self.bursts.clear();
        self.beacons.clear();
        self.beacon_lights.clear();
        self.marks.clear();
        self.later.clear();
        self.guard_blocks = 0;
    }

    /// Stages a preview; one-shot ones (a beam, a line, a step) happen at once.
    fn preview(
        &mut self,
        p: TrickPreview,
        effects: &mut Effects,
        beams: &mut Beams,
        rng: &mut Rng,
        audio: &mut Audio,
    ) {
        match &p {
            TrickPreview::Beam { from, ends, focus } => {
                lamplighter::beam(effects, beams, rng, *from, ends, *focus, false);
                audio.play_2d(Sfx::LamplighterFire);
            }
            TrickPreview::Wickline { from, anchor } => {
                self.wickline.shoot(None, *from, *anchor, effects, beams);
                audio.play_2d(Sfx::WicklineFire);
            }
            TrickPreview::Blink { from, to } => {
                let anim = PlayerAnim {
                    on_ground: true,
                    ..PlayerAnim::default()
                };
                let team = crate::models::DEFAULT_TEAM_COLOR;
                self.blink
                    .step(*from, *to, Some(anim), team, effects, beams, rng);
                audio.play_2d(Sfx::Blink);
            }
            TrickPreview::Beacon { at, .. } => {
                beacon::ignite(effects, beams, *at);
                audio.play_2d(Sfx::BeaconLight);
            }
            _ => {}
        }
        self.staged.push(Staged {
            preview: p,
            age: 0.0,
        });
    }

    /// You were yanked or stepped somewhere: the HUD's speed lines for `seconds`.
    pub fn rush_for(&mut self, seconds: f32) {
        self.rush = (seconds, seconds);
    }

    /// Your zip's / step's rush (1 = just started → 0), for the HUD's speed lines (with the
    /// Lunge's and the Surge's: `knacks::KnackView::hud`).
    pub fn rush(&self) -> f32 {
        if self.rush.0 > 0.0 {
            self.rush.0 / self.rush.1.max(0.01)
        } else {
            0.0
        }
    }

    /// The local focus a preview holds (the caller zooms with it).
    pub fn staged_focus(&self) -> Option<f32> {
        self.staged.iter().find_map(|s| match s.preview {
            TrickPreview::Focus { focus, .. } => Some(focus),
            _ => None,
        })
    }

    /// Once a frame: the tells of other players' Lamplighters, the lines and steps ageing, the
    /// staged previews.
    pub fn update(
        &mut self,
        f: &TrickFrame,
        effects: &mut Effects,
        beams: &mut Beams,
        audio: &mut Audio,
        rng: &mut Rng,
        dt: f32,
    ) {
        self.time += dt.max(0.0) as f64;
        self.rush.0 = (self.rush.0 - dt.max(0.0)).max(0.0);
        for p in std::mem::take(&mut self.pending) {
            self.preview(p, effects, beams, rng, audio);
        }
        let view = f.camera.rotation * Vec3::NEG_Z;
        let mut tells = self.update_world(f, effects, beams, audio, rng, dt);
        tells.extend(self.staged.iter().filter_map(|s| match s.preview {
            TrickPreview::Tell {
                muzzle, aim, focus, ..
            } => Some(lamplighter::Focusing {
                player: NetId(u32::MAX),
                muzzle,
                aim,
                focus,
            }),
            _ => None,
        }));
        self.lamplighter.update_tells(
            &tells,
            f.world,
            f.camera.position,
            view,
            effects,
            beams,
            audio,
            dt,
        );
        // Lines leave their shooter's muzzle wherever it is now and hold what they bit.
        let local = f.local.map(NetId::player);
        let ends = |l: &wickline::Line| {
            let from = match l.shooter {
                Some(id) if Some(id) == local => f.muzzle,
                Some(id) => f
                    .remote
                    .visual(id)
                    .map(|v| v.muzzle)
                    .filter(|m| *m != Vec3::ZERO)
                    .unwrap_or(l.from),
                None => l.from,
            };
            let to = match l.anchor {
                wickline::Anchor::Body(id, at) => {
                    f.entities.iter().find(|e| e.id == id).map_or(at, |e| {
                        e.position + Vec3::Y * (at.y - e.position.y).clamp(0.3, 2.0)
                    })
                }
                a => a.point(),
            };
            (from, to)
        };
        self.wickline.update(dt, &ends, effects, rng);
        self.wickline.draw(beams);
        self.blink.update(dt);
        self.blink.streaks(beams);
        // The staged things: lit beacons, doubles slumping and bursting.
        let mut lights = Vec::new();
        for s in &mut self.staged {
            s.age += dt.max(0.0);
            match s.preview {
                TrickPreview::Beacon {
                    at,
                    radius,
                    seconds,
                } => beacon::lit_beacon(
                    at,
                    floor_under(f.world, at),
                    radius,
                    true,
                    s.age / seconds.max(0.01),
                    self.time,
                    effects,
                    beams,
                    &mut lights,
                ),
                TrickPreview::Waxwork { feet, seconds, .. } => {
                    let end = seconds + waxwork::SLUMP_TIME;
                    if s.age >= end && s.age - dt < end {
                        waxwork::burst(effects, beams, rng, feet, 3.5);
                        audio.play_2d(Sfx::WaxworkBurst);
                    }
                }
                _ => {}
            }
        }
        self.staged.retain(|s| s.age < staged_life(&s.preview));
    }

    /// With the world: the steps' afterimages, staged doubles and beacons' lights, and the
    /// silhouettes of revealed hostiles (into `viewmodel`, drawn over the world). (The lines
    /// and streaks are per-frame beams, added by [`Self::update`].)
    pub fn draw(
        &self,
        lib: Option<&ModelLibrary>,
        f: &TrickFrame,
        draws: &mut Vec<DrawItem>,
        lights: &mut Vec<PointLight>,
        viewmodel: &mut Vec<DrawItem>,
    ) {
        self.blink.draw(lib, draws);
        let Some(lib) = lib else {
            return;
        };
        self.draw_world(lib, f, draws, lights, viewmodel);
        for s in &self.staged {
            match s.preview {
                TrickPreview::Waxwork {
                    feet,
                    yaw,
                    waxy,
                    seconds,
                } => {
                    let anim = waxwork::frozen(PlayerAnim {
                        yaw,
                        on_ground: true,
                        weapon: shared::loadout::WeaponKind::Quill.id(),
                        ..PlayerAnim::default()
                    });
                    let slump = ((s.age - seconds) / waxwork::SLUMP_TIME).clamp(0.0, 1.0);
                    if s.age < seconds + waxwork::SLUMP_TIME {
                        let team = crate::models::DEFAULT_TEAM_COLOR;
                        waxwork::draw_double(
                            lib,
                            &anim,
                            feet,
                            &Tint::team(team),
                            waxy,
                            slump,
                            self.time,
                            draws,
                        );
                    }
                }
                TrickPreview::Beacon { at, radius, .. } => {
                    lights.push(PointLight {
                        position: at,
                        color: Vec3::new(1.4, 1.1, 0.5),
                        radius: radius * 0.6,
                    });
                    let revealed = f
                        .entities
                        .iter()
                        .filter(|e| kind::is_enemy(e.kind) && e.position.distance(at) <= radius);
                    draw_silhouettes(lib, revealed, f, self.time, viewmodel);
                }
                _ => {}
            }
        }
    }

    /// Under the HUD: the Lamplighter's lens vignette at the view's current zoom scale
    /// (`CameraRig::zoom_scale`; nothing unscoped).
    pub fn scope(&self, list: &mut UiDrawList, zoom: f32, time: f64) {
        if !self.scoped {
            return;
        }
        let c = Rect::from_min_size(glam::Vec2::ZERO, list.canvas_size);
        let full = if self.full_zoom > 0.0 {
            self.full_zoom
        } else {
            lamplighter::FULL_ZOOM
        };
        lamplighter::draw_scope(list, c, lamplighter::scope_strength(zoom, full), time);
    }

    /// Over the HUD: the focus ring, the tally, the blink prompt, the guard (`own`: your own
    /// projectiles, for the Blink Arrow's window).
    pub fn hud(&mut self, list: &mut UiDrawList, f: &TrickFrame, own: &[EntityState], time: f64) {
        let c = Rect::from_min_size(glam::Vec2::ZERO, list.canvas_size);
        let camera = f.camera;
        self.hud_world(list, f, own, time);
        if let Some(focus) = self.staged_focus() {
            lamplighter::draw_focus_ring(list, c.center().floor(), focus, time);
        }
        for s in &self.staged {
            match &s.preview {
                TrickPreview::Tally { held, targets, .. } => {
                    if let Some(t) = held {
                        tally::draw_tallying(list, c, camera, t, time);
                    }
                    let marks: Vec<tally::Mark> = targets
                        .iter()
                        .enumerate()
                        .map(|(i, (chest, height))| tally::Mark {
                            chest: *chest,
                            height: *height,
                            order: i,
                            since: time - s.age as f64 - i as f64 * 0.2,
                        })
                        .collect();
                    tally::draw_marks(list, c, camera, &marks, time);
                }
                TrickPreview::BlinkPrompt { left, .. } => {
                    blink::draw_prompt(list, c, *left, time);
                }
                _ => {}
            }
        }
    }
}

/// How long a staged preview stays.
fn staged_life(p: &TrickPreview) -> f32 {
    match p {
        TrickPreview::Focus { seconds, .. }
        | TrickPreview::Tell { seconds, .. }
        | TrickPreview::Tally { seconds, .. }
        | TrickPreview::BlinkPrompt { seconds, .. }
        | TrickPreview::Beacon { seconds, .. } => *seconds,
        TrickPreview::Waxwork { seconds, .. } => seconds + waxwork::SLUMP_TIME + 0.05,
        TrickPreview::Beam { .. } | TrickPreview::Wickline { .. } | TrickPreview::Blink { .. } => {
            0.0
        }
    }
}

/// The floor under `at` (within a few metres), for rings drawn on it.
pub(super) fn floor_under(world: Option<&CollisionWorld>, at: Vec3) -> Option<Vec3> {
    let w = world?;
    w.cast_ray(
        at + Vec3::Y * 0.2,
        Vec3::NEG_Y,
        6.0,
        engine::physics::MASK_SHOTS,
    )
    .map(|h| h.point)
}

/// Silhouettes (into the viewmodel pass) of the revealed `entities` the camera can't see:
/// Moulds posed as the world draws them, players as Shrikes.
fn draw_silhouettes<'a>(
    lib: &ModelLibrary,
    entities: impl Iterator<Item = &'a EntityState>,
    f: &TrickFrame,
    time: f64,
    viewmodel: &mut Vec<DrawItem>,
) {
    let color = beacon::silhouette_color(time);
    let mut items = Vec::new();
    for e in entities {
        let (chest, head) = if e.kind == kind::PLAYER {
            (e.position + Vec3::Y * 1.0, e.position + Vec3::Y * 1.7)
        } else {
            let h = enemy_size(e.kind).1;
            (
                e.position + Vec3::Y * h * 0.5,
                e.position + Vec3::Y * h * 0.9,
            )
        };
        if !beacon::hidden(f.world, f.camera.position, &[chest, head]) {
            continue;
        }
        items.clear();
        if e.kind == kind::PLAYER {
            let Some(v) = f.remote.visual(e.id) else {
                continue;
            };
            let look = e.id.as_player().map(f.look).unwrap_or_default();
            let anim = super::super::remote_players::player_anim(e, v, time, look);
            lib.draw_player_scaled(
                &anim,
                e.position,
                crate::models::shrike::REMOTE_SCALE,
                &Tint::default(),
                &mut items,
            );
        } else {
            let anim = enemy_anim(e, 0.0, time);
            let lift = Vec3::Y * enemy_kind(e.kind).root_lift();
            let root = EnemyAnim::root(e.position + lift, e.yaw);
            lib.draw_enemy(&anim, root, &Tint::default(), &mut items);
        }
        beacon::silhouette(&items, f.camera, f.viewmodel_fov, color, viewmodel);
    }
}

impl InGame {
    /// The scope's vignette, drawn before the HUD so the HUD stays readable over it.
    pub(crate) fn combat_scope(&self, list: &mut UiDrawList, now: f64) {
        if self.net().is_some_and(|n| n.is_in_game()) {
            self.combat
                .tricks
                .scope(list, self.camera.zoom_scale(), now);
        }
    }

    /// Stages a trick client-side (tools and screenshots; see [`TrickPreview`]): it starts on
    /// the next frame's update.
    #[doc(hidden)]
    pub fn preview_tricks(&mut self, preview: TrickPreview) {
        self.combat.tricks.pending.push(preview);
    }
}
