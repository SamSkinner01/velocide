//! Balance report (`docs/balance.md`): every weapon and ability measured in the real
//! simulation (see `tests/balance_support` for how), printed as Markdown tables.
//!
//! ```text
//! cargo run -p shared --release --example balance_report               # everything
//! cargo run -p shared --release --example balance_report -- --refit    # with the refit's
//!     bigger Mould hit shapes / hit assist and the projectile hit pad assumed
//! cargo run -p shared --release --example balance_report -- primaries ttk swarm pvp oneshots abilities
//! cargo run -p shared --release --example balance_report -- --tuning path/to/tuning.ron
//! ```

#[path = "../tests/balance_support/mod.rs"]
mod balance_support;

use balance_support::*;
use shared::loadout::{WeaponClass, WeaponKind};
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
         included). Sustained: the trigger held for 10 s on a Gnawer, aimed at the body / the \
         head (DPS).\n"
    );
    let ranges: Vec<String> = RANGES
        .iter()
        .map(|r| format!("body / head {r:.0} m"))
        .collect();
    let mut cols = vec!["weapon", "per trigger", "interval s"];
    cols.extend(ranges.iter().map(String::as_str));
    header(&cols);
    let c = &effective(base, Side::Pve, assume).combat;
    for w in WeaponKind::ALL {
        let mut cells = vec![
            w.name().to_string(),
            num(per_trigger(base, Side::Pve, w, 5.0, Aim::Body, 1.0)),
            format!("{:.2}", w.primary_interval(c)),
        ];
        for r in RANGES {
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
            for w in WeaponKind::ALL {
                let mut cells = vec![w.name().to_string()];
                for (m, behind) in mould_rows() {
                    let health = mould_health(base, m, difficulty);
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
    println!("## Swarm: six Gnawers (NORMAL) in a pack, front row at 10 m\n");
    println!(
        "Seconds until the last one falls (aim: the nearest one's body). The builds are the \
         weapon's defaults, ability A / B used whenever ready.\n"
    );
    header(&["weapon", "primary", "+ ability A", "+ ability B"]);
    for w in WeaponKind::ALL {
        let b = w.default_build();
        let mut cells = vec![w.name().to_string()];
        for trigger in [Trigger::Primary, Trigger::Both(0), Trigger::Both(1)] {
            let t = on_pack(base, assume, b, trigger, 10.0, 20.0);
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
         every shot aimed at the body / the head (s; 0 = one pull).\n"
    );
    let ranges: Vec<String> = RANGES
        .iter()
        .map(|r| format!("TTK body / head {r:.0} m"))
        .collect();
    let mut cols = vec!["weapon", "per trigger"];
    cols.extend(ranges.iter().map(String::as_str));
    header(&cols);
    for w in WeaponKind::ALL {
        let pull = |aim| kill(per_trigger(base, Side::Pvp, w, 5.0, aim, 1.0));
        let mut cells = vec![
            w.name().to_string(),
            format!("{} / {}", pull(Aim::Body), pull(Aim::Weak(0))),
        ];
        for r in RANGES {
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
         the scatterguns and the Kiln, 15 m for the rest).\n"
    );
    header(&["weapon", "range m", "primary", "+ ability A", "+ ability B"]);
    for w in WeaponKind::ALL {
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
         full-draw bow headshot, a Slug headshot, a full-charge Skewer headshot.\n"
    );
    header(&["attack", "carrier", "3 m", "5 m", "10 m", "5 m, 90 % draw"]);
    let cell = |f: &dyn Fn(f32, Aim) -> f32, r: f32| {
        format!("{} / {}", kill(f(r, Aim::Body)), kill(f(r, Aim::Weak(0))))
    };
    for w in WeaponKind::ALL {
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
    for class in WeaponClass::ALL {
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
    for class in WeaponClass::ALL {
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let assume = Assume {
        refit: args.iter().any(|a| a == "--refit"),
    };
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
        None => Tuning::load_or_default(&Tuning::default_path()),
    };
    let sections: Vec<&str> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && Some(i.saturating_sub(1)) != tuning_at)
        .map(|(_, a)| a.as_str())
        .collect();
    let want = |name: &str| sections.is_empty() || sections.contains(&name);
    println!(
        "# VELOCIDE balance report{}\n",
        if assume.refit {
            " (refit shapes and pads assumed)"
        } else {
            ""
        }
    );
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
}
