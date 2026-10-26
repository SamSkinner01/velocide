//! Target dummies for weapon tests: enemies defined right here (not from `assets/enemies/`), so
//! weapon tests don't depend on the enemy roster. A dummy stands still (speed 0, no attacks),
//! has a capsule body up to 1.45 m and a head weak point (×2), and `knockback_scale` 1.

#![allow(dead_code)]

use glam::Vec3;
use shared::protocol::NetId;
use shared::sim::EnemyDef;

use super::combat_support::Arena;

/// Body radius of a dummy.
pub const DUMMY_RADIUS: f32 = 0.45;
/// Top of a dummy's body capsule (the head sits above it).
pub const DUMMY_BODY_TOP: f32 = 1.45;
/// Head sphere: centre height and radius.
pub const DUMMY_HEAD_Y: f32 = 1.66;
pub const DUMMY_HEAD_RADIUS: f32 = 0.26;

/// A dummy definition with `health`.
pub fn dummy_def(health: f32) -> EnemyDef {
    let text = format!(
        "(
            name: \"Dummy\",
            net_kind: 60,
            model: \"dummy\",
            health: {health:?},
            speed: 0.0,
            acceleration: 60.0,
            radius: {DUMMY_RADIUS:?},
            height: 1.9,
            body_top: Some({DUMMY_BODY_TOP:?}),
            eye_height: 1.65,
            knockback_scale: 1.0,
            weak_points: [
                (name: \"head\", kind: Head, offset: (0.0, {DUMMY_HEAD_Y:?}, 0.0), radius: {DUMMY_HEAD_RADIUS:?}, multiplier: 2.0),
            ],
        )"
    );
    EnemyDef::parse(&text).expect("dummy definition parses")
}

/// Spawns a dummy with `health` at `pos` (feet), facing `yaw`.
pub fn dummy(a: &mut Arena, pos: Vec3, yaw: f32, health: f32) -> NetId {
    let name = format!("dummy_{}", health.to_bits());
    if a.world.enemy_defs.get(&name).is_none() {
        a.world.enemy_defs.insert(&name, dummy_def(health));
    }
    a.world.spawn_enemy(&name, pos, yaw).expect("dummy spawns")
}

/// Centre of a dummy's body.
pub fn dummy_body(a: &Arena, id: NetId) -> Vec3 {
    a.world.enemies[&id].position + Vec3::Y * 0.9
}

/// Centre of a dummy's head.
pub fn dummy_head(a: &Arena, id: NetId) -> Vec3 {
    a.world.enemies[&id].position + Vec3::Y * DUMMY_HEAD_Y
}
