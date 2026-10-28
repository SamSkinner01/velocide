//! PvE balance bands (`docs/balance.md`), measured in the real simulation by the harness in
//! `balance_support` (what `examples/balance_report.rs` prints): every primary's sustained
//! damage inside one band, every Mould killable by every weapon in a sane time, no ability
//! dominating its class and none dead weight, projectiles scoring weak points like rays, the
//! scatterguns the strongest primaries up close (in play too: juking Moulds, the pellets landed,
//! the stagger). The PvP rules are measured in `combat_arsenal.rs`. The shipped
//! `assets/tuning.ron` must be the defaults these run on.

mod balance_support;

use std::path::Path;

use balance_support::*;
use shared::loadout::{Ability, WeaponKind};
use shared::tuning::{BalanceTuning, Tuning};

#[test]
fn the_shipped_balance_profiles_are_the_defaults() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/tuning.ron");
    let text = std::fs::read_to_string(&path).expect("assets/tuning.ron exists");
    let tuning: Tuning = ron::from_str(&text).expect("assets/tuning.ron parses");
    assert_eq!(tuning.balance, BalanceTuning::default());
}

/// Holding the trigger on a Gnawer's body at the weapon's range (5 m for the scatterguns and
/// the Kiln, 15 m for the rest): every primary between 60 and 150 DPS, the scatterguns above
/// it (`scatterguns_are_the_strongest_primaries_up_close`). Before the balance pass the Kiln
/// dealt 48 (nobody's pick) and the Arclash 60.
#[test]
fn every_primary_sustains_damage_inside_the_band() {
    let base = Tuning::default();
    let mut out = Vec::new();
    for w in weapons() {
        let t = on_mould(
            &base,
            Assume::default(),
            w.default_build(),
            Trigger::Primary,
            "gnawer",
            false,
            Aim::Body,
            weapon_range(w),
            Stop::secs(8.0),
        );
        out.push((w, t.dps()));
    }
    for (w, dps) in &out {
        let max = if scatter(*w) { f32::INFINITY } else { 150.0 };
        assert!((60.0..=max).contains(dps), "{w:?}: {dps:.1} DPS ({out:?})");
    }
}

fn scatter(w: WeaponKind) -> bool {
    matches!(w, WeaponKind::Bellows | WeaponKind::Thresher)
}

/// The shotguns pass (docs/balance.md, "Shotguns that hit hard"): at 5 m each scattergun deals
/// at least 1.35 x what it did before it (132 / 142 DPS on a Gnawer's body), and on a Gnawer at
/// 5 and 8 m, standing or juking across the line of fire with the crosshair trailing it
/// (`Play::STRAFE`), each one out-damages every other primary aimed the same way (body or head;
/// melee at contact). Before, a juking Censer at 8 m took 4 of the Thresher's 7 pellets on its
/// worst blast and the Longwick out-damaged both at the head.
#[test]
fn scatterguns_are_the_strongest_primaries_up_close() {
    let base = Tuning::default();
    let a = Assume::default();
    for (w, before) in [(WeaponKind::Bellows, 132.0), (WeaponKind::Thresher, 142.0)] {
        let t = on_mould(
            &base,
            a,
            w.default_build(),
            Trigger::Primary,
            "gnawer",
            false,
            Aim::Body,
            5.0,
            Stop::secs(10.0),
        );
        assert!(
            t.dps() >= 1.35 * before,
            "{w:?}: {:.0} DPS at 5 m (was {before})",
            t.dps()
        );
    }
    for range in [5.0, 8.0] {
        for play in [Play::STILL, Play::STRAFE] {
            for aim in [Aim::Body, Aim::Weak(0)] {
                let dps: Vec<(WeaponKind, f32)> = weapons()
                    .into_iter()
                    .map(|w| {
                        let r = range_on(w, "gnawer", range);
                        let t = in_play(&base, w.default_build(), "gnawer", r, aim, play, 8.0);
                        (w, t.dps())
                    })
                    .collect();
                let best_other = dps
                    .iter()
                    .filter(|(w, _)| !scatter(*w))
                    .map(|(_, d)| *d)
                    .fold(0.0, f32::max);
                for (w, d) in dps.iter().filter(|(w, _)| scatter(*w)) {
                    assert!(
                        *d > best_other,
                        "{w:?} at {range} m ({play:?}, {aim:?}): {d:.0} DPS, {dps:?}"
                    );
                }
            }
        }
    }
}

/// A blast that connects staggers a light Mould (base health below `heavy_health`): one blast
/// from 8 m at the body of each, standing. A pellet alone never pushed hard enough
/// (`STAGGER_KNOCKBACK` is per hit), so no blast ever staggered anything. The heavies (the
/// Smelter, the Gaoler) shrug it off.
#[test]
fn a_connecting_blast_staggers_a_light_mould() {
    let base = Tuning::default();
    let heavy = base.combat.pinning_arrow.heavy_health;
    let defs = shared::sim::EnemyRegistry::builtin();
    for w in [WeaponKind::Bellows, WeaponKind::Thresher] {
        for m in MOULDS {
            let light = defs.get(m).expect("a Mould").health < heavy;
            let t = on_mould(
                &base,
                Assume::default(),
                w.default_build(),
                Trigger::Primary,
                m,
                m == "gaoler",
                Aim::Body,
                8.0,
                Stop::secs(DT),
            );
            let v = t.volleys.first().copied();
            assert!(
                v.is_some_and(|v| v.landed > 0 && v.staggered == light),
                "{w:?} on a {m} (light: {light}): {v:?}"
            );
        }
    }
}

/// The fixed pellet pattern lands most of every blast for a good aim: on a Gnawer, a Censer and
/// a Heron juking across the line of fire with the crosshair trailing them (`Play::STRAFE`),
/// every blast lands at least three quarters of its pellets at 8 m and half at 12 m. With
/// uniformly random pellets the Thresher's worst blasts at a Heron landed 4 of 7 at 8 m and 2
/// at 12 m.
#[test]
fn blasts_land_most_of_their_pellets_every_time() {
    let base = Tuning::default();
    for w in [WeaponKind::Bellows, WeaponKind::Thresher] {
        for m in ["gnawer", "censer", "heron"] {
            for (range, share) in [(8.0, 0.75), (12.0, 0.5)] {
                let t = in_play(
                    &base,
                    w.default_build(),
                    m,
                    range,
                    Aim::Body,
                    Play::STRAFE,
                    8.0,
                );
                assert!(t.volleys.len() > 8, "{w:?}: {} blasts", t.volleys.len());
                let worst = t
                    .volleys
                    .iter()
                    .map(|v| v.landed.min(v.rays) as f32 / v.rays as f32)
                    .fold(1.0, f32::min);
                assert!(
                    worst >= share,
                    "{w:?} on a juking {m} at {range} m: the worst blast landed {worst:.2}"
                );
            }
        }
    }
}

/// Every Mould dies to every weapon's primary at NORMAL within 9 s (the big ones) and 1.5 s
/// (the fodder) at the weapon's range (melee at contact), aiming at the body or the weak
/// point, whichever is faster. The Gaoler from behind: its front is armoured on purpose (flank
/// it, parry it, or shoot its head). A hovering Harrier is out of a swing's reach (it comes
/// down to dive): melee builds reach it with the Hurl.
#[test]
fn every_mould_dies_to_every_weapon_in_a_sane_time() {
    let base = Tuning::default();
    for w in weapons() {
        for m in MOULDS {
            let behind = m == "gaoler";
            let health = mould_health(&base, m, 1);
            let limit = if health >= 200.0 { 9.0 } else { 1.5 };
            let melee = w.class() == shared::loadout::WeaponClass::Melee;
            let (build, trigger, range) = if melee && m == "harrier" {
                (build_with(w, Ability::Hurl), Trigger::Ability(0), 10.0)
            } else {
                (
                    w.default_build(),
                    Trigger::Primary,
                    range_on(w, m, weapon_range(w)),
                )
            };
            let best = [Aim::Body, weak_of(m, behind)]
                .into_iter()
                .filter_map(|aim| {
                    on_mould(
                        &base,
                        Assume::default(),
                        build,
                        trigger,
                        m,
                        behind,
                        aim,
                        range,
                        Stop::damage(health, limit + 0.5),
                    )
                    .time_to(health, None)
                })
                .min_by(f32::total_cmp);
            assert!(
                best.is_some_and(|t| t <= limit),
                "{w:?} on a {m} ({health} HP): {best:?} s (limit {limit} s)"
            );
        }
    }
}

/// Abilities whose job isn't damage (docs/rework.md: every class has a move, a control or
/// defend and a set-up ability): the Gust (knockback, deflection, the gust-jump), the Pinning
/// Arrow and the Bolas (the bind), the Wickline and the Blink Arrow (movement), the Bulwark,
/// the Waxwork and the Riposte (defence), the Beacon Arrow (a reveal).
fn utility(a: Ability) -> bool {
    matches!(
        a,
        Ability::Gust
            | Ability::PinningArrow
            | Ability::Bolas
            | Ability::Wickline
            | Ability::BlinkArrow
            | Ability::Bulwark
            | Ability::Waxwork
            | Ability::Riposte
            | Ability::BeaconArrow
    )
}

/// On every weapon, what each ability of its class adds to the primary (used whenever ready,
/// one Gnawer at the ability's range): the best adds at most 1.5 × the second best (a build's
/// two abilities share one cooldown, so this is the choice). Before the balance pass Both
/// Barrels added 92 / 129 DPS to the Bellows / Thresher against Flak's 52, the Limpet 85 to the
/// explosives against the Breach's 54 and Triple Nock 105 to the bow against the Brand's 34.
/// On six Gnawers no ability adds more than 300 DPS (the Limpet added 380), and every
/// damage ability adds at least 10 DPS (alone or on the pack) on some weapon.
///
/// Ignored while the rework (docs/rework.md) lands: its abilities are being built in other
/// areas, and the balance area's second phase re-runs this band once they have merged.
#[test]
#[ignore = "the rework's abilities: re-enabled by the balance pass (docs/rework.md)"]
fn no_ability_dominates_its_class_and_none_is_dead_weight() {
    let base = Tuning::default();
    let a = Assume::default();
    let mut best_anywhere: Vec<(Ability, f32)> = Vec::new();
    for w in weapons() {
        let mut gains: Vec<(Ability, f32)> = Vec::new();
        for &ability in w.abilities() {
            let build = build_with(w, ability);
            let single = build_gain(&base, a, build, false);
            let pack = build_gain(&base, a, build, true);
            assert!(
                pack <= 300.0,
                "{ability:?} on the {w:?}: {pack:.0} DPS on a pack"
            );
            gains.push((ability, single));
            match best_anywhere.iter_mut().find(|(b, _)| *b == ability) {
                Some((_, g)) => *g = g.max(single).max(pack),
                None => best_anywhere.push((ability, single.max(pack))),
            }
        }
        gains.sort_by(|x, y| y.1.total_cmp(&x.1));
        let (top, second) = (gains[0], gains[1]);
        assert!(
            top.1 <= 1.5 * second.1,
            "{:?} on the {w:?} adds {:.1} DPS, the next best ({:?}) {:.1}: {gains:?}",
            top.0,
            top.1,
            second.0,
            second.1
        );
    }
    for (ability, gain) in best_anywhere {
        if !utility(ability) {
            assert!(gain >= 10.0, "{ability:?} adds {gain:.1} DPS at best");
        }
    }
    // Every class's pool was looked at.
    for class in classes() {
        assert!(weapons().iter().any(|w| w.class() == class));
    }
}

/// Aimed at a weak point from 15 m, an arrow scores it just as a Quill ray does. With the
/// Moulds' bigger body pads a padded body used to catch projectiles first (the Smelter's
/// crucible in front of its hatch, the Gaoler's door below its grille): the Longwick lost the
/// weak points of the Bellwether, the Smelter and the Gaoler's front
/// (`Hitbox::sweep_sphere_padded`).
#[test]
fn projectiles_score_weak_points_like_rays() {
    let base = Tuning::default();
    for w in [WeaponKind::Quill, WeaponKind::Longwick] {
        for m in MOULDS {
            for behind in [false, m == "gaoler"] {
                let t = on_mould(
                    &base,
                    Assume::default(),
                    w.default_build(),
                    Trigger::Primary,
                    m,
                    behind,
                    weak_of(m, behind),
                    15.0,
                    Stop::secs(1.2),
                );
                let first = t.hits.first();
                assert!(
                    first.is_some_and(|h| h.weak),
                    "{w:?} at a {m}'s weak point (behind: {behind}): {first:?}"
                );
            }
        }
    }
}
