//! PvE balance bands (`docs/balance.md`), measured in the real simulation by the harness in
//! `balance_support` (what `examples/balance_report.rs` prints): every primary's sustained
//! damage inside one band, every Mould killable by every weapon in a sane time, no ability
//! dominating its class and none dead weight, projectiles scoring weak points like rays. The
//! PvP rules are measured in `combat_arsenal.rs`. The shipped `assets/tuning.ron` must be the
//! defaults these run on.

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
/// the Kiln, 15 m for the rest): every primary between 60 and 150 DPS. Before the balance
/// pass the Kiln dealt 48 (nobody's pick) and the Arclash 60.
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
        assert!(
            (60.0..=150.0).contains(dps),
            "{w:?}: {dps:.1} DPS ({out:?})"
        );
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

/// Abilities whose job isn't damage: the Gust (knockback, deflection, the gust-jump) and the
/// Pinning Arrow (the root).
fn utility(a: Ability) -> bool {
    matches!(a, Ability::Gust | Ability::PinningArrow)
}

/// On every weapon, what each ability of its class adds to the primary (used whenever ready,
/// one Gnawer at the ability's range): the best adds at most 1.5 × the second best (a build's
/// two abilities share one cooldown, so this is the choice). Before the balance pass Both
/// Barrels added 92 / 129 DPS to the Bellows / Thresher against Flak's 52, the Limpet 85 to the
/// explosives against the Breach's 54 and Triple Nock 105 to the bow against the Brand's 34.
/// On six Gnawers no ability adds more than 300 DPS (the Limpet added 380), and every
/// damage ability adds at least 10 DPS (alone or on the pack) on some weapon.
#[test]
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
