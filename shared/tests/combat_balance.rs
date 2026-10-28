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

/// The rework's pools give every class four jobs (docs/rework.md §1: a move, a control or
/// defend, a set-up and a class-flavoured damage tool; `balance_support::job`), and each
/// ability is judged by its job on every weapon that carries it.
///
/// None is dead weight: a damage or set-up tool adds at least 10 DPS somewhere (one Gnawer, six,
/// or a fight); a control tool adds 10 DPS, or spares 10 % of the damage or holds the foes 10 %
/// longer (pinned, staggered, stunned) in a fight; a defend tool spares 10 % of a fight's
/// damage; a move tool carries you at least 4 m.
///
/// None dominates: among a weapon's damage and set-up tools (the choice a build makes for its
/// damage) the best adds at most 1.5 x the second best; no move, control or defend tool
/// out-damages its weapon's damage tool by more than 2 x; on six Gnawers nothing adds more than
/// 300 DPS. Before the rework's balance pass the Tally added 5 DPS (13.5 in a fight) to the
/// Brand's 35, the Kite 18 to the Limpet's 39 and the Breach's 56, the Beacon Arrow 5 to the
/// bow's Brand at 40, the Hurl 85 to the Tamper; the Bolas' cord 101 on a pack.
#[test]
fn no_ability_dominates_its_class_and_none_is_dead_weight() {
    let base = Tuning::default();
    let a = Assume::default();
    for w in weapons() {
        let mut damage: Vec<(Ability, f32)> = Vec::new();
        let mut tool = None;
        let mut utility: Vec<(Ability, f32)> = Vec::new();
        for &ability in w.abilities() {
            let build = build_with(w, ability);
            let single = build_gain(&base, a, build, false);
            let pack = build_gain(&base, a, build, true);
            assert!(
                pack <= 300.0,
                "{ability:?} on the {w:?}: {pack:.0} DPS on a pack"
            );
            let fights: Vec<FightGain> = Foes::ALL
                .iter()
                .map(|f| fight_gain(&base, build, *f))
                .collect();
            // What it adds in damage the way its use pays: the better of one Gnawer and a
            // fight (the mean of the rush and the volley).
            let in_fights = fights.iter().map(|g| g.dealt).sum::<f32>() / fights.len() as f32;
            let worth = single.max(in_fights);
            let best_fight = |f: &dyn Fn(&FightGain) -> f32| {
                fights.iter().map(f).fold(f32::NEG_INFINITY, f32::max)
            };
            match job(ability) {
                Job::Damage | Job::SetUp => {
                    let most = single.max(pack).max(best_fight(&|g| g.dealt));
                    assert!(
                        most >= 10.0,
                        "{ability:?} on the {w:?} adds {most:.1} DPS at best"
                    );
                    damage.push((ability, worth));
                    if job(ability) == Job::Damage {
                        tool = Some(worth);
                    }
                }
                Job::Control => {
                    let dealt = best_fight(&|g| g.dealt);
                    let spared = best_fight(&|g| g.spared_share());
                    let held = best_fight(&|g| g.held);
                    assert!(
                        dealt >= 10.0 || spared >= 0.1 || held >= 0.1,
                        "{ability:?} on the {w:?}: {dealt:.1} DPS, spares {spared:.2}, holds \
                         {held:.2} ({fights:?})"
                    );
                    utility.push((ability, worth));
                }
                Job::Defend => {
                    let spared = best_fight(&|g| g.spared_share());
                    assert!(
                        spared >= 0.1,
                        "{ability:?} on the {w:?} spares {spared:.2} ({fights:?})"
                    );
                    utility.push((ability, worth));
                }
                Job::Move => {
                    let m = moved(&base, build);
                    assert!(m >= 4.0, "{ability:?} on the {w:?} moves you {m:.1} m");
                    utility.push((ability, worth));
                }
            }
        }
        damage.sort_by(|x, y| y.1.total_cmp(&x.1));
        if let [top, second, ..] = damage[..] {
            assert!(
                top.1 <= 1.5 * second.1,
                "{:?} on the {w:?} adds {:.1} DPS, the next damage pick ({:?}) {:.1}: {damage:?}",
                top.0,
                top.1,
                second.0,
                second.1
            );
        }
        let tool = tool.expect("every pool has a damage tool");
        for (ability, worth) in utility {
            assert!(
                worth <= 2.0 * tool.max(1.0),
                "{ability:?} on the {w:?} adds {worth:.1} DPS, its damage tool {tool:.1}"
            );
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
