//! The tricks in the running game: the replicated world (focusing Lamplighters, wax doubles,
//! beacon arrows, revealed hostiles, guards), the own state (the focus, the tally's marks, the
//! guard, the blink window) and the server's effects (`Fx::Shot` of a Lamplighter,
//! `Wickline`, `TallyFired`, `Blink`, a Riposte's `GuardBlock`), turned into the views of the
//! other files.
//!
//! The local player's own effects play the moment they arrive (`Tricks::own_fx`, at the
//! drain: a step must snap the view on the frame the correction lands); everyone else's when
//! the render clock shows them (`Tricks::fx`, from the queue).

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::{Camera, DrawItem, PointLight, UiDrawList};
use engine::ui::Rect;
use glam::{Vec2, Vec3};
use shared::input::look_dir;
use shared::loadout::WeaponKind;
use shared::protocol::{
    EntityState, Fx, FxEvent, NetId, QPos, enemy_flags, kind, player_flags, player_weapon,
    projectile_flags, waxwork_flags,
};
use shared::sim::combat::secs_to_ticks;
use shared::sim::{Ability, AbilityPhase, Attack, FireIntent, PlayerId};

use super::super::super::effects::Effects;
use super::super::super::remote_players::{PlayerVisual, player_anim, player_color};
use super::super::Rng;
use super::super::entities::enemy_size;
use super::super::fx::{Beams, FxCtx, shake_at};
use super::{TrickFrame, Tricks, beacon, blink, lamplighter, riposte, tally, waxwork, wickline};
use crate::models::weapons::melee::SwingKind;
use crate::models::{ModelLibrary, PlayerAnim, Tint};

/// How often a lit beacon's hum is played again (it lasts 1.2 s, soft at both ends).
const BEACON_HUM_EVERY: f64 = 1.05;
/// A tally's shots: this far apart in the sound.
const TALLY_SHOT_GAP: f32 = 0.045;

/// How long a double's burst stands for the other half of it (seconds).
const BURST_MEMORY: f64 = 0.6;

/// A wax double seen in the world.
#[derive(Clone, Copy, Debug)]
pub(super) struct Double {
    /// The caster's pose, frozen when it appeared.
    anim: PlayerAnim,
    feet: Vec3,
    owner: PlayerId,
    /// When it began to slump (view time).
    slump_since: Option<f64>,
    seen: bool,
}

/// A beacon arrow seen in the world.
#[derive(Clone, Copy, Debug)]
pub(super) struct BeaconSeen {
    owner: u16,
    at: Vec3,
    /// When it stuck and lit (view time).
    lit_at: Option<f64>,
    hum_at: f64,
    seen: bool,
}

/// What the own effects asked of the caller.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OwnFx {
    /// You stepped (Blink Arrow): snap the view (`NetClient::snap_view`).
    pub snap: bool,
    /// A block landed on your guard: hold the frame this long.
    pub hitstop: f32,
}

fn q(p: QPos) -> Vec3 {
    p.to_vec3()
}

fn player_code(id: PlayerId) -> u16 {
    id.0 as u16
}

/// The chest of a replicated body (players stand 1.8 m, Moulds their own height).
fn chest(e: &EntityState) -> (Vec3, f32) {
    let h = if e.kind == kind::PLAYER {
        1.8
    } else {
        enemy_size(e.kind).1
    };
    (e.position + Vec3::Y * h * 0.55, h)
}

impl Tricks {
    /// The replicated world, once a frame (after the beams' update: lines and rings are
    /// per-frame beams).
    pub(super) fn update_world(
        &mut self,
        f: &TrickFrame,
        effects: &mut Effects,
        beams: &mut Beams,
        audio: &mut Audio,
        rng: &mut Rng,
        dt: f32,
    ) -> Vec<lamplighter::Focusing> {
        let local = f.local.map(NetId::player);
        // Other players' focusing Lamplighters: their tells.
        let focusing = f
            .entities
            .iter()
            .filter(|e| {
                e.kind == kind::PLAYER
                    && Some(e.id) != local
                    && e.flags & player_flags::DRAWING != 0
                    && e.flags & player_flags::DEAD == 0
                    && player_weapon(e.state).0 == WeaponKind::Lamplighter
            })
            .map(|e| {
                let muzzle = f
                    .remote
                    .visual(e.id)
                    .map(|v| v.muzzle)
                    .filter(|m| *m != Vec3::ZERO)
                    .unwrap_or(e.position + Vec3::Y * 1.45);
                lamplighter::Focusing {
                    player: e.id,
                    muzzle,
                    aim: look_dir(e.yaw, e.pitch),
                    focus: player_weapon(e.state).2 as f32 / 255.0,
                }
            })
            .collect();
        self.update_doubles(f, effects, beams, audio, rng);
        self.update_beacons(f, effects, beams, audio);
        self.update_marks(f, audio);
        // Sounds waiting their turn (the tally's shots).
        let mut due = Vec::new();
        self.later.retain_mut(|l| {
            l.0 -= dt.max(0.0);
            if l.0 <= 0.0 {
                due.push((l.1, l.2));
                false
            } else {
                true
            }
        });
        for (sfx, at) in due {
            match at {
                Some(p) => audio.play(sfx, PlayParams::at(p)),
                None => audio.play_2d(sfx),
            }
        }
        focusing
    }

    /// Wax doubles: the caster's pose caught as each appears (and its sound), the slump, the
    /// burst where one is gone.
    fn update_doubles(
        &mut self,
        f: &TrickFrame,
        effects: &mut Effects,
        beams: &mut Beams,
        audio: &mut Audio,
        rng: &mut Rng,
    ) {
        for d in self.doubles.values_mut() {
            d.seen = false;
        }
        // Doubles whose burst already played (`Fx::Explosion`) until their entity goes.
        self.burst_doubles
            .retain(|id| f.entities.iter().any(|e| e.id == *id));
        for e in f
            .entities
            .iter()
            .filter(|e| e.kind == kind::WAXWORK && !self.burst_doubles.contains(&e.id))
        {
            let owner = PlayerId(e.owner as u8);
            let time = self.time;
            let d = self.doubles.entry(e.id).or_insert_with(|| {
                // Its pose: the caster's as replicated with it (look, weapon, movement flags),
                // standing still.
                let mut body = *e;
                body.kind = kind::PLAYER;
                body.velocity = Vec3::ZERO;
                body.flags &= !(player_flags::WALL_CLING
                    | player_flags::DRAWING
                    | player_flags::CHARGING
                    | waxwork_flags::SLUMPING);
                let anim = player_anim(&body, &PlayerVisual::default(), 0.0, (f.look)(owner));
                if Some(owner) == f.local {
                    audio.play_2d(Sfx::WaxworkCast);
                } else {
                    audio.play(Sfx::WaxworkCast, PlayParams::at(e.position));
                }
                Double {
                    anim: waxwork::frozen(anim),
                    feet: e.position,
                    owner,
                    slump_since: None,
                    seen: true,
                }
            });
            d.seen = true;
            d.feet = e.position;
            if e.flags & waxwork_flags::SLUMPING != 0 {
                d.slump_since.get_or_insert(time);
            }
        }
        let (radius, time) = (f.tuning.waxwork.burst_radius, self.time);
        let gone: Vec<Double> = self.doubles.values().filter(|d| !d.seen).copied().collect();
        self.doubles.retain(|_, d| d.seen);
        self.bursts.retain(|b| time - b.1 < BURST_MEMORY);
        for d in gone {
            if self.burst_lately(d.feet) {
                continue;
            }
            self.bursts.push((d.feet, time));
            waxwork::burst(effects, beams, rng, d.feet, radius);
            audio.play(Sfx::WaxworkBurst, PlayParams::at(d.feet + Vec3::Y));
        }
    }

    /// Beacon arrows: the beacon catching when one sticks, then burning (its light, column and
    /// the reveal radius for its side) and humming.
    fn update_beacons(
        &mut self,
        f: &TrickFrame,
        effects: &mut Effects,
        beams: &mut Beams,
        audio: &mut Audio,
    ) {
        for b in self.beacons.values_mut() {
            b.seen = false;
        }
        self.beacon_lights.clear();
        let t = &f.tuning.beacon_arrow;
        let time = self.time;
        for e in f.entities.iter().filter(|e| e.kind == kind::BEACON_ARROW) {
            let b = self.beacons.entry(e.id).or_insert(BeaconSeen {
                owner: e.owner,
                at: e.position,
                lit_at: None,
                hum_at: f64::NEG_INFINITY,
                seen: true,
            });
            b.seen = true;
            b.at = e.position;
            if e.flags & projectile_flags::STUCK == 0 {
                continue;
            }
            if b.lit_at.is_none() {
                b.lit_at = Some(time);
                beacon::ignite(effects, beams, e.position);
                audio.play(Sfx::BeaconLight, PlayParams::at(e.position));
            }
            if time - b.hum_at >= BEACON_HUM_EVERY {
                b.hum_at = time;
                audio.play(Sfx::BeaconHum, PlayParams::at(e.position));
            }
            let life = ((time - b.lit_at.unwrap_or(time)) as f32 / t.duration.max(0.1)).min(1.0);
            let ours = !f.pvp || f.local.map(player_code) == Some(b.owner);
            let floor = super::floor_under(f.world, e.position);
            beacon::lit_beacon(
                e.position + Vec3::Y * 0.08,
                floor,
                t.radius,
                ours,
                life,
                time,
                effects,
                beams,
                &mut self.beacon_lights,
            );
        }
        self.beacons.retain(|_, b| b.seen);
    }

    /// The tally's marks from the own state: a new one scratches in with a tick.
    fn update_marks(&mut self, f: &TrickFrame, audio: &mut Audio) {
        let ids: Vec<NetId> = f.tallied.iter().flatten().copied().collect();
        self.marks.retain(|(id, _)| ids.contains(id));
        for id in ids {
            if !self.marks.iter().any(|(m, _)| *m == id) {
                self.marks.push((id, self.time));
                audio.play_2d(Sfx::TallyMark);
            }
        }
    }

    /// The beacon arrows lit for the local player's side, with their reach (the revealed
    /// silhouettes are drawn only near them).
    fn our_beacons(&self, f: &TrickFrame) -> Vec<Vec3> {
        let me = f.local.map(player_code);
        self.beacons
            .values()
            .filter(|b| b.lit_at.is_some() && (!f.pvp || Some(b.owner) == me))
            .map(|b| b.at)
            .collect()
    }

    /// With the world: the wax doubles (wax for their own side; in PvP their caster's opponents
    /// see the player), the beacons' lights, and the silhouettes of the hostiles revealed to us
    /// (into the viewmodel pass).
    pub(super) fn draw_world(
        &self,
        lib: &ModelLibrary,
        f: &TrickFrame,
        draws: &mut Vec<DrawItem>,
        lights: &mut Vec<PointLight>,
        viewmodel: &mut Vec<DrawItem>,
    ) {
        for d in self.doubles.values() {
            let waxy = !f.pvp || Some(d.owner) == f.local;
            let slump = d.slump_since.map_or(0.0, |s| {
                ((self.time - s) as f32 / waxwork::SLUMP_TIME).min(1.0)
            });
            waxwork::draw_double(
                lib,
                &d.anim,
                d.feet,
                &Tint::team(player_color(d.owner)),
                waxy,
                slump,
                self.time,
                draws,
            );
        }
        lights.extend_from_slice(&self.beacon_lights);
        let beacons = self.our_beacons(f);
        if beacons.is_empty() {
            return;
        }
        let reach = f.tuning.beacon_arrow.radius + 1.0;
        let revealed = f.entities.iter().filter(|e| {
            let lit = if e.kind == kind::PLAYER {
                e.flags & player_flags::REVEALED != 0
                    && e.flags & player_flags::DEAD == 0
                    && Some(e.id) != f.local.map(NetId::player)
            } else {
                kind::is_enemy(e.kind)
                    && e.flags & enemy_flags::REVEALED != 0
                    && e.flags & enemy_flags::DYING == 0
            };
            lit && beacons.iter().any(|b| b.distance(e.position) <= reach)
        });
        super::draw_silhouettes(lib, revealed, f, self.time, viewmodel);
    }

    /// Over the HUD: the focus ring, the held tally and its marks, the blink prompt, the guard.
    pub(super) fn hud_world(
        &self,
        list: &mut UiDrawList,
        f: &TrickFrame,
        own: &[EntityState],
        time: f64,
    ) {
        let c = Rect::from_min_size(Vec2::ZERO, list.canvas_size);
        let s = f.weapons;
        let t = f.tuning;
        let rate = f.tick_rate;
        if s.is_unarmed() {
            return;
        }
        // The Lamplighter's focus.
        if s.current() == WeaponKind::Lamplighter && s.draw > 0 {
            lamplighter::draw_focus_ring(list, c.center().floor(), s.draw_fraction(t, rate), time);
        }
        // The tally: held, and its marks on the tallied.
        let held = s.ability() == Ability::Tally && s.charge > 0;
        if held {
            let tallying = tally::Tallying {
                count: f.tallied.iter().flatten().count(),
                max: t.tally.max_targets as usize,
                assist_degrees: t.tally.assist_degrees,
                left: 1.0 - s.charge_fraction(t, rate),
            };
            tally::draw_tallying(list, c, f.camera, &tallying, time);
        }
        let marks: Vec<tally::Mark> = self
            .marks
            .iter()
            .enumerate()
            .filter_map(|(order, (id, since))| {
                let e = f.entities.iter().find(|e| e.id == *id)?;
                let (chest, height) = chest(e);
                Some(tally::Mark {
                    chest,
                    height,
                    order,
                    since: *since,
                })
            })
            .collect();
        tally::draw_marks(list, c, f.camera, &marks, time);
        // The Blink Arrow's window: while its arrow flies or has just stuck.
        if s.live_ability(s.slot) == Some(Ability::BlinkArrow)
            && let Some(arrow) = own.iter().find(|e| e.kind == kind::BLINK_ARROW)
        {
            let left = if arrow.flags & projectile_flags::STUCK != 0 {
                let window = secs_to_ticks(t.blink_arrow.window, rate, 1).max(1);
                let live = s.live.get(s.slot as usize).copied().unwrap_or(0);
                (live as f32 / window as f32).min(1.0)
            } else {
                1.0
            };
            blink::draw_prompt(list, c, left, time);
        }
        // The Riposte's guard.
        if s.ability() == Ability::Riposte && s.charge > 0 {
            let r = &t.riposte;
            let most = 1 + ((r.max_scale - r.damage_scale) / r.per_block.max(0.01)).round() as u32;
            riposte::draw_guard(
                list,
                c,
                1.0 - s.charge_fraction(t, rate),
                self.guard_blocks,
                most.clamp(1, 6),
                time,
            );
        }
    }

    /// A predicted intent of the local player that isn't a shot (`plays_as_shot`): its sound
    /// and its first-person move.
    #[allow(clippy::too_many_arguments)]
    pub fn local_intent(
        &mut self,
        intent: &FireIntent,
        viewmodel: &mut super::super::super::viewmodel::Viewmodel,
        audio: &mut Audio,
        beams: &mut Beams,
        eye: Vec3,
        yaw: f32,
    ) {
        let FireIntent::Ability { ability, phase, .. } = *intent else {
            return;
        };
        match (ability, phase) {
            // The line leaves now; where it lands is the server's (`Fx::Wickline`).
            (Ability::Wickline, AbilityPhase::Use) => {
                audio.play_2d(Sfx::WicklineFire);
                viewmodel.fire(0.35);
            }
            (Ability::Tally, AbilityPhase::Use) => {
                self.marks.clear();
                audio.play(Sfx::TallyMark, PlayParams::default().with_pitch(0.8));
            }
            // The double appears as an entity (its pour is played then).
            (Ability::Waxwork, AbilityPhase::Use) => viewmodel.fire(0.2),
            (Ability::Riposte, AbilityPhase::Use) => {
                self.guard_blocks = 0;
                viewmodel.swing(SwingKind::Guard, false);
                audio.play_2d(Sfx::RiposteGuard);
            }
            // Let go: the counter if anything was blocked, else the guard lowers.
            (Ability::Riposte, AbilityPhase::Release) => {
                if self.guard_blocks > 0 {
                    viewmodel.swing(SwingKind::Riposte, false);
                    audio.play_2d(Sfx::RiposteCounter);
                    let fwd = look_dir(yaw, 0.0);
                    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
                    riposte::counter_crescent(beams, eye, fwd, right, self.guard_blocks);
                } else {
                    viewmodel.drop_guard();
                }
                self.guard_blocks = 0;
            }
            _ => {}
        }
    }

    /// The local player's own effects, as they arrive: the step (snap the view), the line, the
    /// tally's shots, a block on the guard.
    #[allow(clippy::too_many_arguments)]
    pub fn own_fx(
        &mut self,
        fx: &[FxEvent],
        local: Option<PlayerId>,
        muzzle: Vec3,
        entities: &[EntityState],
        effects: &mut Effects,
        beams: &mut Beams,
        audio: &mut Audio,
        rng: &mut Rng,
    ) -> OwnFx {
        let mut out = OwnFx::default();
        let Some(me) = local else {
            return out;
        };
        for ev in fx {
            match &ev.fx {
                Fx::Blink { player, from, to } if *player == me => {
                    self.blink.step(
                        q(*from),
                        q(*to),
                        None,
                        Vec3::ONE.extend(1.0),
                        effects,
                        beams,
                        rng,
                    );
                    audio.play_2d(Sfx::Blink);
                    self.rush_for(0.3);
                    out.snap = true;
                }
                Fx::Wickline {
                    player,
                    to,
                    anchored,
                    ..
                } if *player == me => {
                    let anchor = anchor_of(q(*to), *anchored, entities, Some(me));
                    let pulled = matches!(anchor, wickline::Anchor::Level(_));
                    self.wickline
                        .shoot(Some(NetId::player(me)), muzzle, anchor, effects, beams);
                    if pulled {
                        audio.play_2d(Sfx::WicklineZip);
                        self.rush_for(0.35);
                    } else if matches!(anchor, wickline::Anchor::Body(..)) {
                        audio.play(Sfx::WicklineZip, PlayParams::default().with_volume(0.6));
                    }
                }
                Fx::TallyFired { shooter, ends, .. } if *shooter == me => {
                    let ends: Vec<Vec3> = ends.iter().map(|e| q(*e)).collect();
                    tally_shots(&ends, muzzle, effects, beams, rng);
                    self.queue_tally_sounds(ends.len(), None);
                    self.marks.clear();
                }
                Fx::GuardBlock {
                    player,
                    ability: Ability::Riposte,
                    position,
                    ..
                } if *player == me => {
                    let at = q(*position);
                    riposte::block_sparks(
                        effects,
                        beams,
                        rng,
                        at,
                        (at - muzzle).normalize_or_zero(),
                    );
                    audio.play_2d(Sfx::RiposteBlock);
                    self.guard_blocks += 1;
                    out.hitstop = out.hitstop.max(0.045);
                }
                _ => {}
            }
        }
        out
    }

    /// Everyone else's effects when the render clock shows them (the local player's own play
    /// at the drain, [`Self::own_fx`]).
    pub fn fx(&mut self, ctx: &mut FxCtx, ev: &FxEvent) {
        let local = ctx.local;
        let muzzle_of = |ctx: &FxCtx, player: PlayerId, fallback: Vec3| {
            ctx.remote
                .visual(NetId::player(player))
                .map(|v| v.muzzle)
                .filter(|m| *m != Vec3::ZERO)
                .unwrap_or(fallback)
        };
        match &ev.fx {
            Fx::Shot {
                shooter,
                attack: Attack::Lamplighter,
                origin,
                ends,
            } if Some(*shooter) != local => {
                let muzzle = muzzle_of(ctx, *shooter, q(*origin));
                let focus = self.lamplighter.remote_focus(NetId::player(*shooter));
                let ends: Vec<Vec3> = ends.iter().map(|e| q(*e)).collect();
                lamplighter::beam(ctx.effects, ctx.beams, ctx.rng, muzzle, &ends, focus, false);
                let (volume, pitch) = lamplighter::shot_sound(focus);
                ctx.audio.play(
                    Sfx::LamplighterFire,
                    PlayParams::at(muzzle).with_volume(volume).with_pitch(pitch),
                );
            }
            Fx::Wickline {
                player,
                from,
                to,
                anchored,
            } if Some(*player) != local => {
                let muzzle = muzzle_of(ctx, *player, q(*from));
                let anchor = anchor_of(q(*to), *anchored, ctx.entities, Some(*player));
                self.wickline.shoot(
                    Some(NetId::player(*player)),
                    muzzle,
                    anchor,
                    ctx.effects,
                    ctx.beams,
                );
                ctx.audio.play(Sfx::WicklineFire, PlayParams::at(muzzle));
                if !matches!(anchor, wickline::Anchor::Miss(_)) {
                    ctx.audio.play(Sfx::WicklineZip, PlayParams::at(muzzle));
                }
            }
            Fx::TallyFired {
                shooter,
                origin,
                ends,
            } if Some(*shooter) != local => {
                let muzzle = muzzle_of(ctx, *shooter, q(*origin));
                let ends: Vec<Vec3> = ends.iter().map(|e| q(*e)).collect();
                tally_shots(&ends, muzzle, ctx.effects, ctx.beams, ctx.rng);
                self.queue_tally_sounds(ends.len(), Some(muzzle));
            }
            Fx::Blink { player, from, to } if Some(*player) != local => {
                let id = NetId::player(*player);
                let anim = ctx.entities.iter().find(|e| e.id == id).map(|e| {
                    let v = ctx.remote.visual(id).copied().unwrap_or_default();
                    player_anim(e, &v, 0.0, Default::default())
                });
                self.blink.step(
                    q(*from),
                    q(*to),
                    anim,
                    player_color(*player),
                    ctx.effects,
                    ctx.beams,
                    ctx.rng,
                );
                ctx.audio.play(Sfx::Blink, PlayParams::at(q(*from)));
                ctx.audio
                    .play(Sfx::Blink, PlayParams::at(q(*to)).with_volume(0.7));
            }
            Fx::GuardBlock {
                player,
                ability: Ability::Riposte,
                position,
                ..
            } if Some(*player) != local => {
                let at = q(*position);
                let from = ctx
                    .entities
                    .iter()
                    .find(|e| e.id == NetId::player(*player))
                    .map_or(at, |e| e.position + Vec3::Y * 1.2);
                riposte::block_sparks(
                    ctx.effects,
                    ctx.beams,
                    ctx.rng,
                    at,
                    (from - at).normalize_or_zero(),
                );
                ctx.audio.play(Sfx::RiposteBlock, PlayParams::at(at));
            }
            // A Waxwork's burst (its slump's end, or destroyed): the hot wax where its double
            // stood, and the double gone at once (its entity may linger a frame or two: the
            // explosion of your own double plays as it arrives).
            Fx::Explosion {
                position,
                radius_dm,
                attack: Attack::Waxwork,
                ..
            } => {
                let at = q(*position);
                let near = self
                    .doubles
                    .iter()
                    .filter(|(_, d)| (d.feet + Vec3::Y * 0.9).distance(at) < 2.5)
                    .min_by(|a, b| {
                        let d = |x: &Double| (x.feet + Vec3::Y * 0.9).distance(at);
                        d(a.1).total_cmp(&d(b.1))
                    })
                    .map(|(id, d)| (*id, d.feet));
                let feet = match near {
                    Some((id, feet)) => {
                        self.doubles.remove(&id);
                        self.burst_doubles.insert(id);
                        feet
                    }
                    // Its double already went (and burst) this frame.
                    None if self.burst_lately(at - Vec3::Y * 0.9) => return,
                    None => at - Vec3::Y * 0.9,
                };
                self.bursts.push((feet, self.time));
                let radius = *radius_dm as f32 / 10.0;
                waxwork::burst(ctx.effects, ctx.beams, ctx.rng, feet, radius);
                ctx.audio
                    .play(Sfx::WaxworkBurst, PlayParams::at(feet + Vec3::Y));
                let d = at.distance(ctx.listener);
                ctx.feedback.shake(shake_at(d, radius * 3.0, 0.25));
            }
            _ => {}
        }
    }

    /// Whether a double's burst played near `feet` a moment ago (a burst shows once, whichever
    /// of its `Fx::Explosion` and its entity going comes first).
    fn burst_lately(&self, feet: Vec3) -> bool {
        self.bursts
            .iter()
            .any(|b| b.0.distance(feet) < 1.5 && self.time - b.1 < BURST_MEMORY)
    }

    /// One crack per shot of a tally, a few hundredths of a second apart.
    fn queue_tally_sounds(&mut self, shots: usize, at: Option<Vec3>) {
        for i in 0..shots.min(6) {
            self.later
                .push((i as f32 * TALLY_SHOT_GAP, Sfx::TallyShot, at));
        }
    }

    /// Other players' guards (the `GUARDING` flag with a Riposte in hand): their Shrike raises
    /// the guard and lowers it when the flag goes.
    pub fn pose_guards(
        remote: &mut super::super::super::remote_players::RemotePlayers,
        entities: &[EntityState],
    ) {
        for e in entities.iter().filter(|e| e.kind == kind::PLAYER) {
            let guarding = e.flags & player_flags::GUARDING != 0
                && player_weapon(e.state).1 == Ability::Riposte;
            let v = remote.visual_mut(e.id);
            if guarding {
                if !(v.swing.active() && v.swing.kind == SwingKind::Guard) {
                    v.swing = crate::models::weapons::melee::Swing {
                        kind: SwingKind::Guard,
                        progress: 0.12,
                        left: false,
                    };
                }
                // Held up while the flag says so.
                v.swing.progress = v.swing.progress.min(0.5);
            } else if v.swing.kind == SwingKind::Guard {
                v.swing.drop_guard();
            }
        }
    }
}

/// What a line's far end is fastened to: the level when it anchored, else the hostile nearest
/// where it ended (it holds a body), else nothing.
fn anchor_of(
    to: Vec3,
    anchored: bool,
    entities: &[EntityState],
    shooter: Option<PlayerId>,
) -> wickline::Anchor {
    if anchored {
        return wickline::Anchor::Level(to);
    }
    let me = shooter.map(NetId::player);
    entities
        .iter()
        .filter(|e| {
            (kind::is_enemy(e.kind) || e.kind == kind::PLAYER || e.kind == kind::WAXWORK)
                && Some(e.id) != me
        })
        .map(|e| (e, chest(e).0))
        .filter(|(e, c)| c.distance(to) < 1.2 + chest(e).1 * 0.5)
        .min_by(|a, b| a.1.distance(to).total_cmp(&b.1.distance(to)))
        .map_or(wickline::Anchor::Miss(to), |(e, _)| {
            wickline::Anchor::Body(e.id, to)
        })
}

/// A tally's shots from `muzzle` to each end: thin chalk-white tracers, sparks where they land.
fn tally_shots(
    ends: &[Vec3],
    muzzle: Vec3,
    effects: &mut Effects,
    beams: &mut Beams,
    rng: &mut Rng,
) {
    let chalk = glam::Vec4::new(2.0, 2.0, 1.9, 0.95);
    let glow = glam::Vec4::new(0.9, 1.3, 1.6, 0.6);
    for end in ends {
        beams.beam(muzzle, *end, 0.06, glow, 0.25);
        beams.beam(muzzle, *end, 0.02, chalk, 0.22);
        effects.glow(*end, glam::Vec4::new(1.4, 1.5, 1.6, 0.8), 0.45, 0.1);
        effects.sparks(*end, (muzzle - *end).normalize_or(Vec3::Y), 6);
        let _ = rng;
    }
}

/// The local muzzle of a frame (`camera` + the viewmodel's view-space muzzle), else in front of
/// the eye.
pub fn world_muzzle(camera: &Camera, view_muzzle: Option<Vec3>) -> Vec3 {
    view_muzzle.map_or(
        camera.position + camera.rotation * Vec3::new(0.15, -0.12, -0.6),
        |m| camera.position + camera.rotation * m,
    )
}
