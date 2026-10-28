//! Balance report (`docs/balance.md`): every weapon and ability measured in the real
//! simulation (see `tests/balance_support` for how), printed as Markdown tables.
//!
//! ```text
//! cargo run -p shared --release --example balance_report               # everything
//! cargo run -p shared --release --example balance_report -- primaries ttk swarm pvp oneshots
//!     abilities knacks healing shotguns
//! cargo run -p shared --release --example balance_report -- --tuning path/to/tuning.ron
//! ```

#[path = "../tests/balance_support/mod.rs"]
mod balance_support;

use balance_support::*;
use shared::loadout::{Knack, WeaponClass, WeaponKind};
use shared::tuning::Tuning;

/// Seconds, or a dash when it never happened.
fn secs(t: Option<f32>) -> String {
    t.map_or("—".into(), |t| format!("{t:.2}"))
}

fn num(v: f32) -> String {
    if v.abs() < 0.05 {
        "—".into()
    } else if v.abs() >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// A PvP damage number, bold when it kills from full health.
fn kill(v: f32) -> String {
    if v >= 100.0 {
        format!("**{}**", num(v))
    } else {
        num(v)
    }
}

fn header(cols: &[&str]) {
    println!("| {} |", cols.join(" | "));
    println!("|{}", cols.iter().map(|_| "---|").collect::<String>());
}

fn row(cells: &[String]) {
    println!("| {} |", cells.join(" | "));
}

/// A weapon's row label: melee weapons are measured at contact.
fn weapon_label(w: WeaponKind) -> String {
    if w.class() == WeaponClass::Melee {
        format!("{} (contact)", w.name())
    } else {
        w.name().to_string()
    }
}

fn mould_label(name: &str, behind: bool) -> String {
    let mut s = name.to_string();
    if let Some(c) = s.get_mut(0..1) {
        c.make_ascii_uppercase();
    }
    if name == "gaoler" {
        s.push_str(if behind { " (back)" } else { " (front)" });
    }
    s
}

/// The Moulds of the time-to-kill tables (the Gaoler from both sides).
fn mould_rows() -> Vec<(&'static str, bool)> {
    let mut v: Vec<(&'static str, bool)> = MOULDS.iter().map(|m| (*m, false)).collect();
    v.push(("gaoler", true));
    v
}

fn primaries(base: &Tuning, assume: Assume) {
    println!("## Primaries (PvE)\n");
    println!(
        "Per trigger: one pull on a big body at 5 m (a salvo, a full draw, a blast; burns \
         included; melee: a swing at contact). Sustained: the trigger held for 10 s on a Gnawer, \
         aimed at the body / the head (DPS; melee at contact, in the first column).\n"
    );
    let ranges: Vec<String> = RANGES
        .iter()
        .map(|r| format!("body / head {r:.0} m"))
        .collect();
    let mut cols = vec!["weapon", "per trigger", "interval s"];
    cols.extend(ranges.iter().map(String::as_str));
    header(&cols);
    let c = &effective(base, Side::Pve, assume).combat;
    for w in weapons() {
        let melee = w.class() == WeaponClass::Melee;
        let first = range_on(w, "gaoler", 5.0);
        let mut cells = vec![
            weapon_label(w),
            num(per_trigger(base, Side::Pve, w, first, Aim::Body, 1.0)),
            format!("{:.2}", w.primary_interval(c)),
        ];
        for r in RANGES {
            if melee && r != RANGES[0] {
                cells.push("—".into());
                continue;
            }
            let r = range_on(w, "gnawer", r);
            let mut cell = Vec::new();
            for aim in [Aim::Body, Aim::Weak(0)] {
                let t = on_mould(
                    base,
                    assume,
                    w.default_build(),
                    Trigger::Primary,
                    "gnawer",
                    false,
                    aim,
                    r,
                    Stop::secs(10.0),
                );
                cell.push(num(t.dps()));
            }
            cells.push(cell.join(" / "));
        }
        row(&cells);
    }
    println!();
}

fn ttk(base: &Tuning, assume: Assume) {
    for (difficulty, label) in [(1usize, "NORMAL"), (2, "HARD")] {
        for r in RANGES {
            if difficulty == 2 && r != 15.0 {
                continue;
            }
            println!("## Time to kill, {label}, {r:.0} m (s, body / weak point)\n");
            let names: Vec<String> = mould_rows()
                .into_iter()
                .map(|(m, behind)| {
                    format!(
                        "{} {:.0}",
                        mould_label(m, behind),
                        mould_health(base, m, difficulty)
                    )
                })
                .collect();
            let mut cols = vec!["weapon"];
            cols.extend(names.iter().map(String::as_str));
            header(&cols);
            for w in weapons() {
                let mut cells = vec![weapon_label(w)];
                for (m, behind) in mould_rows() {
                    let health = mould_health(base, m, difficulty);
                    let r = range_on(w, m, r);
                    let mut cell = Vec::new();
                    for aim in [Aim::Body, weak_of(m, behind)] {
                        let t = on_mould(
                            base,
                            assume,
                            w.default_build(),
                            Trigger::Primary,
                            m,
                            behind,
                            aim,
                            r,
                            Stop::damage(health, 20.0),
                        );
                        cell.push(secs(t.time_to(health, None)));
                    }
                    cells.push(cell.join(" / "));
                }
                row(&cells);
            }
            println!();
        }
    }
}

fn swarm(base: &Tuning, assume: Assume) {
    println!("## Swarm: six Gnawers (NORMAL) in a pack, front row at 10 m (melee: at contact)\n");
    println!(
        "Seconds until the last one falls (aim: the nearest one's body). The builds are the \
         weapon's defaults, ability A / B used whenever ready.\n"
    );
    header(&["weapon", "primary", "+ ability A", "+ ability B"]);
    for w in weapons() {
        let b = w.default_build();
        let mut cells = vec![weapon_label(w)];
        let r = range_on(w, "gnawer", 10.0);
        for trigger in [Trigger::Primary, Trigger::Both(0), Trigger::Both(1)] {
            let t = on_pack(base, assume, b, trigger, r, 20.0);
            let all = (t.deaths.len() >= 6)
                .then(|| t.deaths.last().copied())
                .flatten();
            let label = match trigger {
                Trigger::Primary => String::new(),
                Trigger::Both(m) | Trigger::Ability(m) => format!(" ({})", b.ability(m).name()),
            };
            cells.push(format!("{}{label}", secs(all)));
        }
        row(&cells);
    }
    println!();
}

fn pvp(base: &Tuning, assume: Assume) {
    println!("## PvP (100 HP)\n");
    println!(
        "Per trigger at 5 m, aimed at the body / the head; time to kill holding the trigger, \
         every shot aimed at the body / the head (s; 0 = one pull). Melee at 2 m in the first \
         column (no headshots).\n"
    );
    let ranges: Vec<String> = RANGES
        .iter()
        .map(|r| format!("TTK body / head {r:.0} m"))
        .collect();
    let mut cols = vec!["weapon", "per trigger"];
    cols.extend(ranges.iter().map(String::as_str));
    header(&cols);
    for w in weapons() {
        let melee = w.class() == WeaponClass::Melee;
        let first = if melee { MELEE_RANGE } else { 5.0 };
        let pull = |aim| kill(per_trigger(base, Side::Pvp, w, first, aim, 1.0));
        let mut cells = vec![
            weapon_label(w),
            format!("{} / {}", pull(Aim::Body), pull(Aim::Weak(0))),
        ];
        for r in RANGES {
            let r = if melee && r == RANGES[0] {
                MELEE_RANGE
            } else {
                r
            };
            let mut cell = Vec::new();
            for head in [false, true] {
                let t = on_player(
                    base,
                    assume,
                    w.default_build(),
                    Trigger::Primary,
                    head,
                    r,
                    Stop::damage(100.0, 8.0),
                );
                cell.push(secs(t.time_to(100.0, None)));
            }
            cells.push(cell.join(" / "));
        }
        row(&cells);
    }
    println!();
    println!(
        "With the default build's ability A / B used whenever ready (the same tick as the \
         primary when both are ready): all-body time to kill at the weapon's range (5 m for \
         the scatterguns and the Kiln, 2 m for melee, 15 m for the rest).\n"
    );
    header(&["weapon", "range m", "primary", "+ ability A", "+ ability B"]);
    for w in weapons() {
        let b = w.default_build();
        let r = weapon_range(w);
        let mut cells = vec![w.name().to_string(), format!("{r:.0}")];
        for trigger in [Trigger::Primary, Trigger::Both(0), Trigger::Both(1)] {
            let t = on_player(base, assume, b, trigger, false, r, Stop::damage(100.0, 8.0));
            let label = match trigger {
                Trigger::Primary => String::new(),
                Trigger::Both(m) | Trigger::Ability(m) => format!(" ({})", b.ability(m).name()),
            };
            cells.push(format!("{}{label}", secs(t.time_to(100.0, None))));
        }
        row(&cells);
    }
    println!();
}

fn oneshots(base: &Tuning) {
    println!("## PvP one-shots: one trigger pull / one ability use (body / head)\n");
    println!(
        "Bold = kills from full health. The bow at a full draw and at 90 %. Allowed: a \
         full-draw bow headshot, a full-focus Lamplighter headshot.\n"
    );
    header(&["attack", "carrier", "3 m", "5 m", "10 m", "5 m, 90 % draw"]);
    let cell = |f: &dyn Fn(f32, Aim) -> f32, r: f32| {
        format!("{} / {}", kill(f(r, Aim::Body)), kill(f(r, Aim::Weak(0))))
    };
    for w in weapons() {
        let f = |r: f32, aim: Aim| per_trigger(base, Side::Pvp, w, r, aim, 1.0);
        let bow = w.primary_style() == shared::sim::weapons::PrimaryStyle::Draw;
        let partial = if bow {
            let g = |r: f32, aim: Aim| per_trigger(base, Side::Pvp, w, r, aim, 0.9);
            cell(&g, 5.0)
        } else {
            String::new()
        };
        row(&[
            format!("{} (primary)", w.name()),
            String::new(),
            cell(&f, 3.0),
            cell(&f, 5.0),
            cell(&f, 10.0),
            partial,
        ]);
    }
    for class in classes() {
        for &a in class.abilities() {
            for w in carriers(a) {
                if w.class() != class {
                    continue;
                }
                let f = |r: f32, aim: Aim| per_use(base, Side::Pvp, w, a, r, aim, 1.0);
                let partial = if class == WeaponClass::Draw {
                    let g = |r: f32, aim: Aim| per_use(base, Side::Pvp, w, a, r, aim, 0.9);
                    cell(&g, 5.0)
                } else {
                    String::new()
                };
                row(&[
                    a.name().to_string(),
                    w.name().to_string(),
                    cell(&f, 3.0),
                    cell(&f, 5.0),
                    cell(&f, 10.0),
                    partial,
                ]);
            }
        }
    }
    println!();
}

fn abilities(base: &Tuning, assume: Assume) {
    let c = effective(base, Side::Pve, assume).combat;
    let p = effective(base, Side::Pvp, assume).combat;
    println!("## Abilities\n");
    println!(
        "PvE on Gnawers at the ability's range (Gust, Discharge, Breach: 4 m; the rest 10 m). \
         Per use: one use on one Gnawer body / on six in a pack. Spam DPS: used whenever ready \
         for 12 s, primary idle. Build gain: primary + ability whenever ready, minus the primary \
         alone (12 s, one Gnawer): what taking it adds. A build's two abilities share one \
         cooldown, so siblings compete for the same seconds.\n"
    );
    for class in classes() {
        println!("### {}\n", class.name());
        header(&[
            "ability",
            "carrier",
            "cooldown s (PvP)",
            "per use",
            "per use, pack",
            "spam DPS",
            "build gain DPS",
            "build gain, pack",
        ]);
        for &a in class.abilities() {
            for w in carriers(a) {
                if w.class() != class {
                    continue;
                }
                let b = build_with(w, a);
                let r = ability_range(a);
                let single = |trigger, stop| {
                    on_mould(
                        base,
                        assume,
                        b,
                        trigger,
                        "gnawer",
                        false,
                        Aim::Body,
                        r,
                        stop,
                    )
                };
                let pack = |trigger, stop| on_endless_pack(base, assume, b, trigger, r, stop);
                let one = single(Trigger::Ability(0), Stop::one_use(4.0));
                let spam = single(Trigger::Ability(0), Stop::secs(12.0));
                row(&[
                    a.name().to_string(),
                    w.name().to_string(),
                    format!("{:.2} ({:.2})", a.cooldown(&c), a.cooldown(&p)),
                    num(one.total()),
                    num(pack(Trigger::Ability(0), Stop::one_use(4.0)).total()),
                    num(spam.dps()),
                    num(build_gain(base, assume, b, false)),
                    num(build_gain(base, assume, b, true)),
                ]);
            }
        }
        println!();
    }
}

fn knacks(base: &Tuning) {
    println!("## Knacks in a PvP duel\n");
    println!(
        "Real 100 HP: every primary held at the opponent's body at its range; the opponent \
         uses its knack on the first tick (the Kindle Flask drunk: its heal over time; its \
         haste doesn't act standing). Seconds to the kill. Snare, Soot Pot and Surge don't act \
         in a standing duel (a trap, Moulds' sight, mobility).\n"
    );
    header(&[
        "attacker",
        "range m",
        "no knack",
        "Tallow Skin",
        "Kindle Flask",
    ]);
    for w in weapons() {
        let b = w.default_build();
        let r = weapon_range(w);
        let mut cells = vec![w.name().to_string(), format!("{r:.0}")];
        for k in [None, Some(Knack::TallowSkin), Some(Knack::KindleFlask)] {
            cells.push(secs(duel(base, b, k, r)));
        }
        row(&cells);
    }
    println!();
}

fn healing(base: &Tuning) {
    println!("## Healing from embers (PvE)\n");
    println!(
        "HP per second the shooter draws in while 40 HP down, holding the primary for 10 s on \
         one Gnawer / on six (melee at contact; the rest at their range). Hits shed embers \
         worth `embers.heal_fraction` (melee weapons: `melee_weapon_heal_fraction`) of the \
         damage, drawn in by the closest Shrike within `embers.radius`.\n"
    );
    header(&["weapon", "range m", "one Gnawer", "six"]);
    for w in weapons() {
        let b = w.default_build();
        let r = range_on(w, "gnawer", weapon_range(w));
        row(&[
            w.name().to_string(),
            format!("{r:.1}"),
            num(heal_rate(base, b, false, r)),
            num(heal_rate(base, b, true, r)),
        ]);
    }
    println!();
}

/// The weapons the play-like tables compare (melee and the Kiln's cone have no aim to miss).
fn ranged() -> Vec<WeaponKind> {
    weapons()
        .into_iter()
        .filter(|w| w.class() != WeaponClass::Melee && *w != WeaponKind::Kiln)
        .collect()
}

fn scatter() -> [WeaponKind; 2] {
    [WeaponKind::Bellows, WeaponKind::Thresher]
}

fn shotguns(base: &Tuning) {
    println!("## Shotguns in play\n");
    println!(
        "Moulds juking across the line of fire (5 m/s, turning every 0.8 s), the crosshair \
         0.1 s behind them (a good player tracking); endless. DPS aimed at the body / the weak \
         point; the scatterguns add the share of their pellets that landed (body / weak \
         point).\n"
    );
    let moulds = ["gnawer", "censer", "heron"];
    let ranges = [5.0, 8.0, 12.0];
    let cols: Vec<String> = moulds
        .iter()
        .flat_map(|m| ranges.map(|r| format!("{} {r:.0} m", mould_label(m, false))))
        .collect();
    let mut head = vec!["weapon"];
    head.extend(cols.iter().map(String::as_str));
    header(&head);
    for w in ranged() {
        let mut cells = vec![w.name().to_string()];
        for m in moulds {
            for r in ranges {
                let t: Vec<Trace> = [Aim::Body, Aim::Weak(0)]
                    .into_iter()
                    .map(|aim| in_play(base, w.default_build(), m, r, aim, Play::STRAFE, 8.0))
                    .collect();
                let mut cell = format!("{} / {}", num(t[0].dps()), num(t[1].dps()));
                if scatter().contains(&w) {
                    cell.push_str(&format!(
                        " ({:.0} / {:.0} %)",
                        100.0 * t[0].landed(),
                        100.0 * t[1].landed()
                    ));
                }
                cells.push(cell);
            }
        }
        row(&cells);
    }
    println!();
    println!(
        "Blast by blast, juking as above, aimed at the body: pellets landed and damage per blast \
         (worst / mean), the blasts that staggered the Mould, and blasts to kill one at NORMAL \
         (most / mean over 12 Moulds placed a tick apart in the juke).\n"
    );
    header(&[
        "weapon",
        "Mould",
        "range m",
        "pellets landed",
        "damage",
        "staggered",
        "blasts to kill",
    ]);
    for w in scatter() {
        for m in [
            "gnawer",
            "censer",
            "snuffer",
            "heron",
            "harrier",
            "bellwether",
        ] {
            for r in [8.0, 12.0] {
                let t = in_play(base, w.default_build(), m, r, Aim::Body, Play::STRAFE, 8.0);
                let landed: Vec<u32> = t.volleys.iter().map(|v| v.landed.min(v.rays)).collect();
                let dealt: Vec<f32> = t.volleys.iter().map(|v| v.dealt).collect();
                let n = t.volleys.len().max(1) as f32;
                let rays = t.volleys.first().map_or(0, |v| v.rays);
                let kills: Vec<usize> = (0..12)
                    .map(|i| {
                        let mut b =
                            Bench::new(base, Side::Pve, Assume::default(), w.default_build());
                        b.set_motion(Motion::Strafe {
                            speed: 5.0,
                            period: 0.8,
                        });
                        b.aim_lag = 0.1;
                        b.mould_at(m, r, false, Some(mould_health(base, m, 1)));
                        // Start the juke `i` ticks in, the crosshair already trailing.
                        b.run_idle(i as f32 * DT + 0.1, Aim::Body);
                        b.run(Trigger::Primary, Aim::Body, Stop::secs(6.0))
                            .volleys
                            .len()
                    })
                    .collect();
                row(&[
                    w.name().to_string(),
                    mould_label(m, false),
                    format!("{r:.0}"),
                    format!(
                        "{} / {:.1} of {rays}",
                        landed.iter().min().copied().unwrap_or(0),
                        landed.iter().sum::<u32>() as f32 / n
                    ),
                    format!(
                        "{} / {}",
                        num(dealt.iter().copied().fold(f32::INFINITY, f32::min)),
                        num(dealt.iter().sum::<f32>() / n)
                    ),
                    format!("{:.0} %", 100.0 * t.stagger_rate()),
                    format!(
                        "{} / {:.1}",
                        kills.iter().max().copied().unwrap_or(0),
                        kills.iter().sum::<usize>() as f32 / kills.len() as f32
                    ),
                ]);
            }
        }
    }
    println!();
    println!(
        "A Mould running in at the shooter (its own AI), one blast at its body from 8 m \
         (endless): how much further away it ended up, how long it was staggered, and how long \
         until it ran at you again.\n"
    );
    header(&[
        "weapon",
        "Mould",
        "pellets landed",
        "pushed back m",
        "staggered s",
        "running again s",
    ]);
    for w in scatter() {
        for m in ["gnawer", "snuffer", "smelter", "gaoler"] {
            let Some(p) = blast_push(base, w, m, 8.0) else {
                continue;
            };
            row(&[
                w.name().to_string(),
                mould_label(m, false),
                format!(
                    "{} of {}",
                    p.volley.landed.min(p.volley.rays),
                    p.volley.rays
                ),
                format!("{:.1}", p.back),
                format!("{:.2}", p.staggered),
                format!("{:.2}", p.recovered),
            ]);
        }
    }
    println!();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let assume = Assume::default();
    let tuning_at = args.iter().position(|a| a == "--tuning");
    let base = match tuning_at.map(|i| args.get(i + 1)) {
        Some(Some(path)) => match Tuning::load(std::path::Path::new(path)) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{path}: {e}");
                std::process::exit(1);
            }
        },
        Some(None) => {
            eprintln!("--tuning needs a path");
            std::process::exit(1);
        }
        None => Tuning::load_default(),
    };
    let sections: Vec<&str> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && Some(i.saturating_sub(1)) != tuning_at)
        .map(|(_, a)| a.as_str())
        .collect();
    let want = |name: &str| sections.is_empty() || sections.contains(&name);
    println!("# VELOCIDE balance report\n");
    if want("primaries") {
        primaries(&base, assume);
    }
    if want("ttk") {
        ttk(&base, assume);
    }
    if want("swarm") {
        swarm(&base, assume);
    }
    if want("pvp") {
        pvp(&base, assume);
    }
    if want("oneshots") {
        oneshots(&base);
    }
    if want("abilities") {
        abilities(&base, assume);
    }
    if want("knacks") {
        knacks(&base);
    }
    if want("healing") {
        healing(&base);
    }
    if want("shotguns") {
        shotguns(&base);
    }
}
