//! What the LOADOUT screen says about the arsenal (`docs/arsenal.md`, `docs/roster.md`): what
//! each weapon is for, what each ability does, how it is fired, and their key numbers.
//!
//! The words never carry numbers (the balance moves them); every number comes from a tuning,
//! the effective one of a mode (`Tuning::for_mode`), through [`weapon_stats`] /
//! [`ability_stats`]. Ability numbers are the ones the server uses for that carrier
//! (`weapons::ability_tuning`: an ability on a lighter weapon than its home hits softer).
//! Intervals are counted in whole simulation ticks like `step_weapons` does, so a rate reads
//! what the gun really fires.
//!
//! Sentences are checked against the sim (`shared::sim::{weapons, area, payload, status}`);
//! keep them true when a mechanic changes. Lengths are budgeted for the screen's boxes (a test
//! wraps them at the 16:9 widths).

use shared::loadout::{Ability, Knack, WeaponClass, WeaponKind};
use shared::sim::DEFAULT_TICK_RATE;
use shared::sim::combat::secs_to_ticks;
use shared::sim::weapons::{AbilityStyle, ability_tuning};
use shared::tuning::{CombatTuning, Tuning};

/// One row of numbers: a label and its value in one tuning.
pub type Stat = (&'static str, String);

/// What a weapon is for and how to use it (sentence case; the screen draws it in capitals).
pub fn weapon_role(w: WeaponKind) -> &'static str {
    match w {
        WeaponKind::Quill => {
            "Exact hitscan spikes for careful aim. A shot passing a Thorn ricochets into the \
             next target, harder each bounce."
        }
        WeaponKind::Lancet => {
            "Fast hitscan needles. The first shot is exact; holding the trigger heats it and \
             the spread opens. Tap at range."
        }
        WeaponKind::Bellows => {
            "A wide wall of pellets that shoves what it hits. Devastating up close, weak at \
             range, slow between blasts."
        }
        WeaponKind::Thresher => {
            "An automatic scattergun: lighter, wider blasts for as long as you hold the \
             trigger. Close to mid range."
        }
        WeaponKind::Arclash => {
            "Hold for a stream of zaps. A hit jumps on to the nearest hostile nearby, weaker \
             each jump: made for packs."
        }
        WeaponKind::Riveter => {
            "A fast stream of hot rivets. They are projectiles with a slight drop: lead moving \
             targets at range."
        }
        WeaponKind::Kiln => {
            "A short cone of embers: everything inside catches fire and keeps burning after \
             you stop. No weak points."
        }
        WeaponKind::Tumbler => {
            "Bouncing bombs: they blow after a few bounces, on a target or when the fuse runs \
             out. Bank them round corners."
        }
        WeaponKind::Cinderbox => {
            "Each pull fires a salvo of small rockets: splash damage at mid range, then a \
             long wait for the next salvo."
        }
        WeaponKind::Longwick => {
            "Hold to draw, release to loose: a full draw flies fastest and hits hardest, a \
             tap is weak and slow. Arrows drop."
        }
        WeaponKind::Trimmer => {
            "Quick slashes through several targets in front of you; a slash turns parryable \
             shots back like a punch."
        }
        WeaponKind::Tamper => {
            "A slow, heavy hammer swing through everything in a wide arc, knocking it far \
             back. Melee has no weak points."
        }
        WeaponKind::Lamplighter => {
            "Hold to focus, release one exact hitscan beam: a full focus hits hardest and \
             pierces a second target."
        }
    }
}

/// What an ability does and how to use it (on `weapon`: the bow's abilities ride its next
/// arrow).
pub fn ability_text(a: Ability, weapon: WeaponKind) -> &'static str {
    let bow = weapon.class() == WeaponClass::Draw;
    match a {
        Ability::Brand if bow => {
            "Your next arrow marks what it hits: it takes extra damage from everyone for a \
             while, and Thorn bounces and Arclash chains seek it first."
        }
        Ability::Brand => {
            "Marks what it hits: it takes extra damage from everyone for a while, and Thorn \
             bounces and Arclash chains seek it first. One brand each."
        }
        Ability::Wickline => {
            "A burning thread: caught in the level, it zips you along it; on a light target it \
             yanks it to you, a heavy one yanks you to it."
        }
        Ability::Tally => {
            "Hold and sweep your crosshair over targets to mark them; let go and every marked \
             target still in sight takes a shot."
        }
        Ability::Waxwork => {
            "Leaves a wax double of you where you stand: Moulds go for it instead of you. It \
             bursts in hot wax when destroyed or spent."
        }
        Ability::Gust => {
            "A blast of air in front of you: shoves what it hits and turns hostile projectiles \
             around as your own. Gust the floor to jump."
        }
        Ability::Bolas => {
            "Weighted balls on a cord: the first hostile it touches is bound, and one close to \
             it too, sharing the damage either takes."
        }
        Ability::Bulwark => {
            "Hold up a slab of hardened wax: hits from the front are absorbed until it \
             shatters. You move slower behind it."
        }
        Ability::Flak => {
            "A shell that bursts into a ring of pellets after a short flight, just short of a \
             target or off a wall: it reaches behind cover."
        }
        Ability::Wisp => {
            "A ball of lightning orbits you, zapping the nearest hostile. Press again to fling \
             it: it pops on the first thing it touches."
        }
        Ability::Discharge => {
            "Charge a shock nova around you that throws everything away. More charge, bigger \
             and harder. It never hurts you."
        }
        Ability::Stoke => {
            "Stoke the weapon: for a few seconds it fires faster and sets what it hits alight, \
             then it overheats and can't fire for a moment."
        }
        Ability::Lantern => {
            "Set down a lantern that shoots the nearest hostile with your weapon for a while. \
             Moulds may attack it. One at a time."
        }
        Ability::Breach => {
            "A point-blank blast just in front of you. It throws you hard and hurts you a \
             little: the blast-jump and the panic button."
        }
        Ability::Gravewell => {
            "Lobs a canister that sticks where it lands, drags enemies in (players at half \
             strength), then implodes on the crowd."
        }
        Ability::Limpet => {
            "Tap to throw a charge that sticks to walls and targets; hold to blow every charge \
             you have out at once."
        }
        Ability::Kite => {
            "Launch an ember kite and steer it with your crosshair while you hold: it explodes \
             on contact. Let go and it flies straight."
        }
        Ability::BlinkArrow => {
            "Your next arrow can be stepped to: press again while it flies, or just after it \
             sticks, and you are there."
        }
        Ability::PinningArrow => {
            "Your next arrow pins what it hits: a Mould is rooted to the spot (it still \
             attacks; heavies only briefly), a player is slowed."
        }
        Ability::BeaconArrow => {
            "Your next arrow lights a beacon where it sticks: everything near it is revealed \
             through walls and takes extra damage."
        }
        Ability::Lunge => {
            "A low leap along your aim: for a moment the first hostile your body reaches \
             takes a heavy blow. Close the gap and strike at once."
        }
        Ability::Whirl => {
            "A spin in place of your next swing that hits everything around you, front and \
             back, and knocks it outward. Room to breathe when you're surrounded."
        }
        Ability::Hurl => {
            "Throws a spinning blade or hammer-head that hits on the way out and on the way \
             back. Catch it for part of the cooldown."
        }
        Ability::Riposte => {
            "Hold to guard: blows and shots from the front are blocked. Let go to counter-swing, \
             harder for every block."
        }
    }
}

/// What a knack does and how to use it.
pub fn knack_text(k: Knack) -> &'static str {
    match k {
        Knack::Thorn => {
            "A barbed glass spike that plants itself where it lands. A shot or blow that reaches \
             a live Thorn is relayed off it into the next target (another Thorn first), harder \
             each bounce. Throw it at a Mould: it hurts and drops, still live."
        }
        Knack::KindleFlask => {
            "Drink a flask of kindling: for a few seconds you run, slide and dash faster and \
             heal a little every second. It's yours only, in every mode."
        }
        Knack::Snare => {
            "Throw a wax trap to the floor. Once armed, the first hostile that comes close is \
             pinned (a Mould rooted, a player slowed) and hurt. Guard a doorway, your back or \
             a flank."
        }
        Knack::SootPot => {
            "A pot that bursts into a thick cloud of soot. Moulds can't see into, out of or \
             through it: they lose you, and a Heron's lock breaks. Players see a dense cloud."
        }
        Knack::TallowSkin => {
            "Wax over: for a few seconds you take much less damage and no knockback (your own \
             blasts still push you, so blast-jumps work). Use it before the big hit lands."
        }
        Knack::Surge => {
            "A burst of speed along your look, mostly level: cross a gap, dodge a shot or \
             close in. It works in every mode and stands in for PvE-only knacks in PvP."
        }
    }
}

/// How a knack is used (one short line).
pub fn knack_input(k: Knack) -> &'static str {
    match k {
        Knack::Thorn | Knack::SootPot => "KNACK: THROWS IT ALONG YOUR AIM",
        Knack::KindleFlask => "KNACK: AT ONCE, ON YOU",
        Knack::Snare => "KNACK: THROWS IT TO THE FLOOR",
        Knack::TallowSkin => "KNACK: AT ONCE, ON YOU",
        Knack::Surge => "KNACK: AT ONCE, ALONG YOUR LOOK",
    }
}

/// A knack in a few words for its card: charges and recharge, or PvE only.
pub fn knack_line(k: Knack, t: &CombatTuning) -> String {
    if !k.pvp_allowed() {
        return "PVE ONLY".into();
    }
    let n = k.charges(t);
    let s = if n == 1 { "" } else { "S" };
    format!("{n} CHARGE{s}, {}", secs(k.recharge_time(t)))
}

/// A knack's key numbers in one effective tuning (`pvp`: the PvP column, where a knack that
/// isn't allowed shows `-`: Surge stands in): charges, recharge, then one or two more.
pub fn knack_stats(k: Knack, tuning: &Tuning, pvp: bool) -> Vec<Stat> {
    let c = &tuning.combat;
    let n = &c.knacks;
    let mut rows = vec![
        ("CHARGES", k.charges(c).to_string()),
        ("RECHARGE", secs(k.recharge_time(c))),
    ];
    let radius = |r: f32| format!("{}M", num(r, 1));
    rows.extend(match k {
        Knack::Thorn => vec![("BOUNCE", format!("X{}", num(c.thorn.damage_multiplier, 2)))],
        Knack::KindleFlask => {
            let f = &n.kindle_flask;
            vec![
                (
                    "HEALS",
                    format!("{}/{}", num(f.haste_heal, 1), secs(f.haste_heal_interval)),
                ),
                (
                    "HASTE",
                    format!("X{} {}", num(f.haste_scale, 2), secs(f.haste_time)),
                ),
            ]
        }
        Knack::Snare => {
            let s = &n.snare;
            vec![
                ("DAMAGE", num(s.damage, 1)),
                ("AT ONCE", s.max_live.to_string()),
            ]
        }
        Knack::SootPot => {
            let p = &n.soot_pot;
            vec![(
                "CLOUD",
                format!("{} {}", radius(p.radius), secs(p.duration)),
            )]
        }
        Knack::TallowSkin => {
            let t = &n.tallow_skin;
            vec![
                ("DAMAGE", format!("X{}", num(t.damage_scale, 2))),
                ("LASTS", secs(t.duration)),
            ]
        }
        Knack::Surge => vec![("SPEED", format!("{} M/S", num(n.surge.speed, 1)))],
    });
    if pvp && k.for_world(true) != k {
        for (_, value) in &mut rows {
            *value = "-".into();
        }
    }
    rows
}

/// How an ability is fired from `weapon` (one short line).
pub fn ability_input(a: Ability, weapon: WeaponKind) -> &'static str {
    if weapon.class() == WeaponClass::Draw {
        return "ALT FIRE ARMS THE NEXT ARROW";
    }
    match a.style() {
        AbilityStyle::Charge => "HOLD ALT FIRE; RELEASE FIRES",
        AbilityStyle::Hold => "TAP: THROW   HOLD: DETONATE",
        AbilityStyle::Channel => "HOLD ALT FIRE, LET GO",
        AbilityStyle::Press if a.recasts() => "ALT FIRE; AGAIN: FLING",
        AbilityStyle::Press => "ALT FIRE; HOLD TO STEER",
        AbilityStyle::Instant => "ALT FIRE (HOLD TO REPEAT)",
    }
}

/// `x` with at most `decimals` decimals and no trailing zeros ("24", "19.2", "0.28").
pub fn num(x: f32, decimals: usize) -> String {
    let s = format!("{x:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// Seconds ("0.28S", "2.5S").
fn secs(x: f32) -> String {
    format!("{}S", num(x, 2))
}

/// A time in whole simulation ticks (seconds), as `step_weapons` counts it.
fn ticked(secs: f32) -> f32 {
    secs_to_ticks(secs, DEFAULT_TICK_RATE, 1) as f32 / DEFAULT_TICK_RATE as f32
}

/// Seconds from one primary shot (salvo, release) to the next.
fn cycle(w: WeaponKind, t: &CombatTuning) -> f32 {
    match w {
        WeaponKind::Longwick | WeaponKind::Lamplighter => {
            ticked(w.draw_time(t).unwrap_or(0.0)) + ticked(w.primary_interval(t))
        }
        _ => ticked(w.primary_interval(t)),
    }
}

/// Damage of one primary trigger pull (all pellets / rockets; the bow at full draw).
fn trigger_damage(w: WeaponKind, t: &CombatTuning) -> f32 {
    let d = w.primary_damage(t);
    match w {
        WeaponKind::Bellows => d * t.bellows.pellets as f32,
        WeaponKind::Thresher => d * t.thresher.pellets as f32,
        WeaponKind::Cinderbox => d * t.cinderbox.rockets as f32,
        WeaponKind::Longwick => t.longwick.max_damage,
        WeaponKind::Lamplighter => t.lamplighter.max_damage,
        _ => d,
    }
}

/// Damage of a primary shot in words: pellets × damage, the bow's draw range, the Kiln's
/// spray plus Burn.
pub fn damage_text(w: WeaponKind, t: &CombatTuning) -> String {
    let d = num(w.primary_damage(t), 1);
    match w {
        WeaponKind::Bellows => format!("{}X{d}", t.bellows.pellets),
        WeaponKind::Thresher => format!("{}X{d}", t.thresher.pellets),
        WeaponKind::Cinderbox => format!("{}X{d}", t.cinderbox.rockets),
        WeaponKind::Longwick => format!("{d}-{}", num(t.longwick.max_damage, 1)),
        WeaponKind::Lamplighter => format!("{d}-{}", num(t.lamplighter.max_damage, 1)),
        WeaponKind::Kiln => format!("{d}+BURN"),
        _ => d,
    }
}

/// How often it fires: shots per second when fast, else the interval (the bow: its draw).
pub fn rate_text(w: WeaponKind, t: &CombatTuning) -> String {
    if w == WeaponKind::Longwick {
        return format!("DRAW {}", secs(t.longwick.draw_time));
    }
    if w == WeaponKind::Lamplighter {
        return format!("FOC {}", secs(t.lamplighter.focus_time));
    }
    let interval = cycle(w, t).max(1e-3);
    if interval < 0.2 {
        format!("{}/S", num(1.0 / interval, 1))
    } else {
        secs(interval)
    }
}

/// A weapon's key numbers in one tuning: damage, rate and damage per second on one target
/// with everything landing (the Kiln with a full Burn, the bow at full draws).
pub fn weapon_stats(w: WeaponKind, t: &CombatTuning) -> Vec<Stat> {
    let mut dps = trigger_damage(w, t) / cycle(w, t).max(1e-3);
    if w == WeaponKind::Kiln {
        dps += t.burn.dps * t.burn.max_stacks as f32;
    }
    vec![
        ("DAMAGE", damage_text(w, t)),
        ("RATE", rate_text(w, t)),
        ("DPS", num(dps, 0)),
    ]
}

/// An ability's key numbers on `weapon` in one effective tuning (`pvp`: the column is the PvP
/// one): the cooldown first, then at most two more.
pub fn ability_stats(a: Ability, weapon: WeaponKind, tuning: &Tuning, pvp: bool) -> Vec<Stat> {
    let t = ability_tuning(tuning, weapon, a);
    let c = &t.combat;
    let arrow = |scale: f32| {
        format!(
            "{}-{}",
            num(c.longwick.min_damage * scale, 1),
            num(c.longwick.max_damage * scale, 1)
        )
    };
    let radius = |r: f32| format!("{}M", num(r, 1));
    let primary = weapon.primary_damage(c);
    let mut rows = vec![("COOLDOWN", secs(a.cooldown(c)))];
    let bow = weapon.class() == WeaponClass::Draw;
    let more: [Stat; 2] = match a {
        Ability::Brand => {
            let damage = if bow {
                arrow(1.0)
            } else {
                num(c.quill.brand_damage, 1)
            };
            let q = &c.quill;
            let mark = format!("X{} {}", num(q.brand_multiplier, 2), secs(q.brand_duration));
            [("DAMAGE", damage), ("MARK", mark)]
        }
        Ability::Wickline => [
            ("DAMAGE", num(c.wickline.damage, 1)),
            ("RANGE", radius(c.wickline.range)),
        ],
        Ability::Tally => {
            let k = &c.tally;
            [
                ("DAMAGE", format!("{}X{}", k.max_targets, num(k.damage, 1))),
                ("HOLD", secs(k.hold_time)),
            ]
        }
        Ability::Waxwork => {
            let w = &c.waxwork;
            [
                (
                    "HEALTH",
                    format!("{} {}", num(w.health, 1), secs(w.duration)),
                ),
                ("BURST", num(w.burst_damage, 1)),
            ]
        }
        Ability::Gust => {
            let b = &c.bellows;
            [
                ("DAMAGE", num(b.gust_damage, 1)),
                (
                    "DEFLECT",
                    format!("X{} DMG", num(b.deflect_damage_multiplier, 2)),
                ),
            ]
        }
        Ability::Bolas => {
            let b = &c.bolas;
            [
                ("SHARES", format!("{}%", num(b.link_share * 100.0, 0))),
                (
                    "LINK",
                    format!("{} {}", radius(b.link_range), secs(b.link_time)),
                ),
            ]
        }
        Ability::Bulwark => {
            let b = &c.bulwark;
            [("ABSORBS", num(b.absorb, 1)), ("HOLD", secs(b.hold_time))]
        }
        Ability::Flak => {
            let f = &c.flak;
            [
                (
                    "DAMAGE",
                    format!("{}X{}", f.pellets, num(f.pellet_damage, 1)),
                ),
                ("BURST AT", radius(f.burst_distance)),
            ]
        }
        Ability::Wisp => {
            let a = &c.arclash;
            [
                (
                    "ZAPS",
                    format!(
                        "{} / {}",
                        num(a.wisp_zap_damage, 1),
                        secs(a.wisp_zap_interval)
                    ),
                ),
                ("POP", num(a.wisp_pop_damage, 1)),
            ]
        }
        Ability::Discharge => {
            let a = &c.arclash;
            [
                (
                    "DAMAGE",
                    format!(
                        "{}-{}",
                        num(a.discharge_min_damage, 1),
                        num(a.discharge_max_damage, 1)
                    ),
                ),
                (
                    "RADIUS",
                    format!(
                        "{}-{}",
                        num(a.discharge_min_radius, 1),
                        radius(a.discharge_max_radius)
                    ),
                ),
            ]
        }
        Ability::Stoke => {
            let k = &c.stoke;
            [
                ("RATE", format!("X{} {}", num(k.rate, 2), secs(k.duration))),
                ("OVERHEAT", secs(k.overheat)),
            ]
        }
        Ability::Lantern => {
            let l = &c.lantern;
            [
                ("DAMAGE", format!("X{}", num(l.damage_scale, 2))),
                ("LASTS", secs(l.duration)),
            ]
        }
        Ability::Breach => {
            let b = &c.tumbler;
            [
                ("DAMAGE", num(b.breach_damage, 1)),
                ("RADIUS", radius(b.breach_radius)),
            ]
        }
        Ability::Gravewell => {
            let g = &c.tumbler;
            [
                ("DAMAGE", num(g.gravewell_damage, 1)),
                (
                    "PULL",
                    format!(
                        "{} {}",
                        radius(g.gravewell_pull_radius),
                        secs(g.gravewell_pull_time)
                    ),
                ),
            ]
        }
        Ability::Limpet => {
            let l = &c.limpet;
            [
                (
                    "DAMAGE",
                    format!("{} {}", num(l.damage, 1), radius(l.explosion_radius)),
                ),
                ("AT ONCE", l.max_live.to_string()),
            ]
        }
        Ability::Kite => {
            let k = &c.kite;
            [
                ("DAMAGE", num(k.damage, 1)),
                ("RADIUS", radius(k.explosion_radius)),
            ]
        }
        Ability::BlinkArrow => {
            let b = &c.blink_arrow;
            [
                ("DAMAGE", arrow(b.damage_scale)),
                ("WINDOW", secs(b.window)),
            ]
        }
        Ability::PinningArrow => {
            let p = &c.pinning_arrow;
            let pins = if pvp {
                format!("SLOW {}", secs(p.player_time))
            } else {
                format!("ROOT {}", secs(p.enemy_time))
            };
            [("DAMAGE", arrow(p.damage_scale)), ("PINS", pins)]
        }
        Ability::BeaconArrow => {
            let b = &c.beacon_arrow;
            [
                ("DAMAGE", arrow(b.damage_scale)),
                (
                    "REVEALS",
                    format!("{} {}", radius(b.radius), secs(b.duration)),
                ),
            ]
        }
        // Melee: multiples of the carrier's swing.
        Ability::Lunge => [
            ("DAMAGE", num(primary * c.lunge.damage_scale, 1)),
            ("DASH", format!("{} M/S", num(c.lunge.speed, 1))),
        ],
        Ability::Whirl => [
            ("DAMAGE", num(primary * c.whirl.damage_scale, 1)),
            ("RADIUS", radius(c.whirl.radius)),
        ],
        Ability::Hurl => [
            ("DAMAGE", num(primary * c.hurl.damage_scale, 1)),
            ("SPEED", format!("{} M/S", num(c.hurl.speed, 1))),
        ],
        Ability::Riposte => {
            let r = &c.riposte;
            [
                (
                    "COUNTER",
                    format!(
                        "{}-{}",
                        num(primary * r.damage_scale, 0),
                        num(primary * r.max_scale, 0)
                    ),
                ),
                ("GUARD", secs(r.hold_time)),
            ]
        }
    };
    rows.extend(more);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::ui::text::wrap_text;
    use shared::lobby::GameMode;

    use crate::menus::loadout::{BOX_LINES, KNACK_LINES, Layout, ROLE_LINES};
    use engine::ui::Rect;
    use glam::{Vec2, vec2};

    fn lines(text: &str, width: f32) -> usize {
        wrap_text(&text.to_uppercase(), width, 1.0).len()
    }

    #[test]
    fn every_text_fits_its_box() {
        // From 16:10 (the canvas is 576 × 360) up: nothing is cut.
        let lay = Layout::new(Rect::from_min_size(Vec2::ZERO, vec2(576.0, 360.0)));
        let (role, boxes) = (lay.role_text_width(), lay.box_text_width());
        for w in WeaponKind::ALL {
            let n = lines(weapon_role(w), role);
            assert!(n <= ROLE_LINES, "{w:?}: {n} lines");
            for &a in w.abilities() {
                let n = lines(ability_text(a, w), boxes);
                assert!(n <= BOX_LINES, "{a:?} on {w:?}: {n} lines");
                assert!(lines(ability_input(a, w), boxes - 7.0) == 1, "{a:?} input");
            }
        }
    }

    #[test]
    fn every_knack_is_explained_and_fits() {
        let lay = Layout::new(Rect::from_min_size(Vec2::ZERO, vec2(576.0, 360.0)));
        let width = lay.knack_detail().width() - 12.0;
        let file = Tuning::default();
        let (pve, pvp) = (
            file.for_mode(GameMode::Campaign),
            file.for_mode(GameMode::Deathmatch),
        );
        for k in Knack::ALL {
            let n = lines(knack_text(k), width);
            assert!(n <= KNACK_LINES, "{k:?}: {n} lines");
            assert!(lines(knack_input(k), width - 7.0) == 1, "{k:?} input");
            let line = knack_line(k, &pve.combat);
            let card = lay.slot_card(4).width() - 28.0;
            assert!(lines(&line, card) == 1, "{k:?}: {line}");
            let (a, b) = (knack_stats(k, &pve, false), knack_stats(k, &pvp, true));
            assert_eq!(a.len(), b.len());
            assert!(
                a.iter().all(|(l, v)| l.len() <= 8 && v.len() <= 11),
                "{a:?}"
            );
            // In PvP the Thorn gives way to Surge: its column is empty.
            let blank = b.iter().all(|(_, v)| v == "-");
            assert_eq!(blank, !k.pvp_allowed(), "{k:?}");
        }
    }

    #[test]
    fn numbers_come_from_the_tuning_and_stay_short() {
        let file = Tuning::default();
        let pve = file.for_mode(GameMode::Campaign);
        let pvp = file.for_mode(GameMode::Deathmatch);
        for w in WeaponKind::ALL {
            for t in [&pve, &pvp] {
                let rows = weapon_stats(w, &t.combat);
                assert_eq!(rows.len(), 3);
                for (label, value) in &rows {
                    assert!(
                        !value.is_empty() && value.len() <= 10,
                        "{w:?} {label}: {value}"
                    );
                }
            }
            for &a in w.abilities() {
                let (a_pve, a_pvp) = (
                    ability_stats(a, w, &pve, false),
                    ability_stats(a, w, &pvp, true),
                );
                assert_eq!(a_pve.len(), 3, "{a:?}");
                let labels = |r: &[Stat]| r.iter().map(|(l, _)| *l).collect::<Vec<_>>();
                assert_eq!(labels(&a_pve), labels(&a_pvp), "{a:?}: same rows");
                for (label, value) in a_pve.iter().chain(&a_pvp) {
                    assert!(label.len() <= 8, "{a:?} {label}");
                    assert!(
                        !value.is_empty() && value.len() <= 11,
                        "{a:?} {label}: {value}"
                    );
                }
            }
        }
        let c = CombatTuning::default();
        assert_eq!(damage_text(WeaponKind::Bellows, &c), "12X11");
        assert_eq!(damage_text(WeaponKind::Longwick, &c), "22-105");
        // 0.09 s is five ticks at 60 Hz: twelve rivets a second.
        assert_eq!(rate_text(WeaponKind::Riveter, &c), "12/S");
        assert_eq!(rate_text(WeaponKind::Quill, &c), "0.28S");
        assert_eq!(num(19.2, 1), "19.2");
        assert_eq!(num(3.0, 2), "3");
    }

    #[test]
    fn ability_numbers_follow_the_carrier() {
        let t = Tuning::default();
        // The Kite's home is the Cinderbox; the Tumbler carries it as listed, never more.
        let c = &t.combat;
        let home = ability_stats(Ability::Kite, WeaponKind::Cinderbox, &t, false);
        assert_eq!(home[1].1, num(c.kite.damage, 1));
        // The Wickline's home is the Quill; the Lancet carries it at its ability power.
        let lancet = ability_stats(Ability::Wickline, WeaponKind::Lancet, &t, false);
        let power = c.lancet.ability_power / c.quill.ability_power;
        assert_eq!(lancet[1].1, num(c.wickline.damage * power, 1));
        // The PvP column of the Pinning Arrow is the players' slow.
        let pvp = ability_stats(Ability::PinningArrow, WeaponKind::Longwick, &t, true);
        assert!(pvp[2].1.starts_with("SLOW"), "{:?}", pvp[2]);
        // Melee abilities hit for a multiple of their carrier's swing.
        let lunge = |w| ability_stats(Ability::Lunge, w, &t, false)[1].1.clone();
        let scale = c.lunge.damage_scale;
        assert_eq!(lunge(WeaponKind::Trimmer), num(c.trimmer.damage * scale, 1));
        assert_eq!(lunge(WeaponKind::Tamper), num(c.tamper.damage * scale, 1));
    }
}
