//! Thorns relaying every weapon (docs/rework.md §4, area **client-b**): the first player
//! attack that reaches a live thorn shatters it and is relayed at the nearest hostile. How it
//! looks depends on what reached the thorn ([`Relayed`]):
//!
//! * **A ray** (any hitscan, a pellet): the ray ricochets on through the thorns in the
//!   attack's own tracer colour, brighter and wider each bounce, each thorn shattering with
//!   `Sfx::Ricochet` rising in pitch ([`ray`]).
//! * **A projectile** (a bomb, a rivet, a rocket, an arrow, a kite, a bolas, a hurled blade):
//!   it turns at the thorn — a flare there, the broken line of its old and new heading glowing
//!   for a moment, `Sfx::ThornTurn` — and flies on at its target as itself ([`turn`]).
//! * **A blow or a blast** (a swing, a cone, a nova, an explosion that holds the thorn): the
//!   thorn is batted at the target — a glass spear streaking from it with a trail of shards,
//!   a bright core and a hit at the far end, `Sfx::ThornBat` ([`bat`]).
//!
//! Every relay shatters its thorn ([`fx::thorn_shatter`](super::fx::thorn_shatter)) and
//! counts as RICOCHET for style, like the Quill's.

use engine::audio::{PlayParams, Sfx};
use glam::{Vec2, Vec3, Vec4};
use shared::sim::Attack;

use super::super::effects::{FxTex, Particle};
use super::fx::{FxCtx, thorn_shatter};

/// What reached the thorn, for its look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relayed {
    Ray,
    Projectile,
    /// A melee swing or strike.
    Blow,
    /// A cone, a nova, an explosion.
    Blast,
}

/// The relay's look for `attack` (rays by default: every hitscan primary and ability).
pub fn relayed(attack: Attack) -> Relayed {
    match attack {
        Attack::Tumble
        | Attack::Gravewell
        | Attack::Riveter
        | Attack::Cinderbox
        | Attack::Longwick
        | Attack::PinningArrow
        | Attack::Limpet
        | Attack::Flak
        | Attack::Hurl
        | Attack::Thorn => Relayed::Projectile,
        Attack::Trimmer | Attack::Tamper | Attack::Lunge | Attack::Whirl | Attack::Fist => {
            Relayed::Blow
        }
        Attack::Kiln | Attack::Discharge | Attack::Breach | Attack::Gust | Attack::Slam => {
            Relayed::Blast
        }
        _ => Relayed::Ray,
    }
}

/// The thorn's glass, pushed hot.
fn glass(k: f32, a: f32) -> Vec4 {
    Vec4::new(0.85 * k, 0.95 * k, 1.0 * k, a)
}

/// The gold of a relay, for mixing into an attack's own colour.
const RELAY_GOLD: Vec4 = Vec4::new(1.6, 1.5, 0.9, 0.95);

/// Plays a relay through `points` (each thorn it passed, then where it ended) of `attack`.
pub fn play(ctx: &mut FxCtx, attack: Attack, points: &[Vec3]) {
    match relayed(attack) {
        Relayed::Ray => ray(ctx, attack, points),
        Relayed::Projectile => {
            for w in points.windows(2) {
                turn(ctx, attack, w[0], w[1]);
            }
        }
        kind @ (Relayed::Blow | Relayed::Blast) => {
            for (i, w) in points.windows(2).enumerate() {
                bat(ctx, kind, w[0], w[1], i);
            }
        }
    }
}

/// A ray ricocheting through `points` in the attack's tracer colour.
pub fn ray(ctx: &mut FxCtx, attack: Attack, points: &[Vec3]) {
    let style = super::attack_style(attack);
    let own = if style.beam_width > 0.0 {
        style.beam_color
    } else {
        RELAY_GOLD
    };
    for (i, w) in points.windows(2).enumerate() {
        // The attack's colour, warming to the relay's gold as it chains.
        let k = (0.35 + 0.2 * i as f32).min(0.9);
        let color = own.lerp(RELAY_GOLD, k) * Vec4::new(1.2, 1.2, 1.2, 1.0);
        let width = style.beam_width.max(0.05) * (1.4 + 0.35 * i as f32);
        if style.jagged {
            ctx.beams.arc(w[0], w[1], width, color, 0.3, ctx.rng);
        } else {
            ctx.beams.beam(w[0], w[1], width, color, 0.35);
            ctx.beams
                .beam(w[0], w[1], width * 0.35, Vec4::new(2.4, 2.3, 2.0, 0.9), 0.2);
        }
    }
    shatter_thorns(ctx, points, Sfx::Ricochet);
    if let Some(last) = points.last() {
        ctx.effects.sparks(*last, Vec3::Y, 10);
    }
}

/// Every point but the last is a thorn: shattered, with `sfx` rising in pitch along the chain.
fn shatter_thorns(ctx: &mut FxCtx, points: &[Vec3], sfx: Sfx) {
    for (i, p) in points
        .iter()
        .enumerate()
        .take(points.len().saturating_sub(1))
    {
        thorn_shatter(ctx, *p);
        ctx.audio
            .play(sfx, PlayParams::at(*p).with_pitch(1.0 + 0.18 * i as f32));
    }
}

/// A projectile turning at the thorn `at` towards `to`: a flare, the new heading lit for a
/// moment, the sound. (The projectile itself flies on as drawn.)
pub fn turn(ctx: &mut FxCtx, attack: Attack, at: Vec3, to: Vec3) {
    let style = super::attack_style(attack);
    let color = style.beam_color.lerp(RELAY_GOLD, 0.6);
    let dir = (to - at).normalize_or(Vec3::NEG_Z);
    thorn_shatter(ctx, at);
    ctx.effects.glow(at, glass(1.6, 0.9), 0.9, 0.18);
    // The new line, bright at the thorn and fading out along the way.
    let reach = at.distance(to).min(6.0);
    for (from, width, alpha) in [
        (0.0f32, 0.09f32, 1.0f32),
        (0.35, 0.06, 0.7),
        (0.7, 0.035, 0.4),
    ] {
        ctx.beams.beam(
            at + dir * reach * from,
            at + dir * reach * (from + 0.3),
            width,
            color * Vec4::new(1.0, 1.0, 1.0, alpha),
            0.25,
        );
    }
    ctx.beams
        .ring(at, dir, 0.1, 0.9, 0.04, glass(1.8, 0.9), 0.18);
    ctx.audio.play(Sfx::ThornTurn, PlayParams::at(at));
}

/// The thorn at `at` batted at `to` by a blow or a blast (`i`: its place in a chain): a glass
/// spear streaking there with a trail of shards, a hit at the far end, the sound.
pub fn bat(ctx: &mut FxCtx, kind: Relayed, at: Vec3, to: Vec3, i: usize) {
    let tint = match kind {
        Relayed::Blast => Vec4::new(1.0, 0.7, 0.35, 1.0),
        _ => Vec4::ONE,
    };
    let dir = (to - at).normalize_or(Vec3::NEG_Z);
    thorn_shatter(ctx, at);
    // The spear: a broad glassy streak and a white-hot core.
    ctx.beams.beam(at, to, 0.16, glass(1.5, 0.85) * tint, 0.3);
    ctx.beams
        .beam(at, to, 0.05, Vec4::new(2.6, 2.6, 2.4, 0.95), 0.2);
    // Shards strewn along its path.
    let len = at.distance(to);
    let n = ((len * 1.5) as usize).clamp(4, 18);
    for k in 0..n {
        let p = at + dir * (len * k as f32 / n as f32);
        ctx.effects.spawn(Particle {
            position: p,
            velocity: dir * 4.0 + ctx.rng.dir() * 1.5,
            gravity: 9.0,
            drag: 1.0,
            max_life: ctx.rng.range(0.25, 0.45),
            size: Vec2::splat(0.07),
            color: glass(1.3, 1.0) * tint,
            end_alpha: 0.2,
            texture: FxTex::Shard,
            additive: true,
            rotation: ctx.rng.angle(),
            spin: ctx.rng.range(-12.0, 12.0),
            ..Default::default()
        });
    }
    ctx.effects.glow(to, glass(1.4, 0.9) * tint, 1.0, 0.15);
    ctx.effects.sparks(to, -dir, 12);
    ctx.audio.play(
        Sfx::ThornBat,
        PlayParams::at(at).with_pitch(1.0 + 0.15 * i as f32),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_of_attack_relays_its_own_way() {
        assert_eq!(relayed(Attack::Quill), Relayed::Ray);
        assert_eq!(relayed(Attack::Blast), Relayed::Ray);
        assert_eq!(relayed(Attack::Thresher), Relayed::Ray);
        assert_eq!(relayed(Attack::Riveter), Relayed::Projectile);
        assert_eq!(relayed(Attack::Longwick), Relayed::Projectile);
        assert_eq!(relayed(Attack::Hurl), Relayed::Projectile);
        assert_eq!(relayed(Attack::Trimmer), Relayed::Blow);
        assert_eq!(relayed(Attack::Kiln), Relayed::Blast);
    }
}
