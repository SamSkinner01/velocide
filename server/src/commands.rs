//! Dev chat commands (`ServerConfig::dev_commands`): any chat message starting with `/` is a
//! command for the sender; the reply comes back as a server chat line to them only. Who may
//! use them: single-player, the host of a listen server (never guests), and on dedicated
//! servers started with `--dev-commands` the lobby host (everyone with `--no-lobby`). Each
//! command costs a chat token (chat's rate limit). Elsewhere `/...` is answered with a
//! refusal and never broadcast.
//!
//! | command | effect |
//! |---|---|
//! | `/spawn <enemy> [count] [elite]` | spawns enemies 8 m in front of you (on the ground); `elite` makes them elites |
//! | `/enemies` | lists enemy definitions |
//! | `/killall` | kills every enemy |
//! | `/god` | toggles invulnerability |
//! | `/notarget` | toggles: enemies ignore you |
//! | `/give` | full health, no hard damage, every knack charge, weapons ready |
//! | `/kill` | kills you (respawn rule applies) |
//! | `/ff [on/off]` | PvP damage between players |
//! | `/hurt <amount>` | damages you (test hard damage / healing) |
//! | `/needle [distance]` (alias `/orb`) | a slow parryable Heron needle flies at you from in front (parry practice) |
//! | `/ember [distance]` | a Censer ember bomb is lobbed onto you from in front (parry practice) |
//! | `/encounter <n>` | starts arena encounter n (1-based; stragglers incl. you go to its rally point) |
//! | `/encounters` | encounter states (docs/enemies.md) |
//! | `/tp <x> <y> <z>` | teleports you to that point (feet) |
//! | `/goto <exit / start / checkpoint n / encounter n>` | teleports you to the level exit, the start, checkpoint n's spawn or encounter n's rally point |
//! | `/loadout [weapon[:ability,ability] ...] [knack:<name>]` | sets your loadout now, slot by slot (`-` = empty slot; abilities default to the weapon's), e.g. `/loadout longwick:pinning,ember kiln quill knack:snare`; only `knack:<name>`: keeps the weapons; no arguments: shows it (docs/arsenal.md) |
//! | `/help` | this list |

use glam::Vec3;
use shared::input::look_dir;
use shared::loadout::{Ability, Knack, LOADOUT_SLOTS, Loadout, WeaponBuild, WeaponKind};
use shared::protocol::kind;
use shared::sim::{Attack, DamageInfo, HitTarget, Owner, PlayerId, ProjectileKind, WeaponState};

use crate::Server;

const HELP: &str = "/spawn <enemy> [count] [elite], /enemies, /killall, /god, /notarget, /give, /kill, /ff [on|off], /hurt <hp>, /needle [dist], /ember [dist], /encounter <n>, /encounters, /tp <x> <y> <z>, /goto <exit|start|checkpoint n|encounter n>, /loadout [weapon[:ability,ability] ...] [knack:<name>]";

/// Lower-case letters and digits only ("Triple Nock" → "triplenock").
fn squash(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The weapon or ability whose name starts with `word` (ignoring case, spaces and `_`).
fn find_named<T: Copy>(word: &str, all: &[T], name: impl Fn(T) -> &'static str) -> Option<T> {
    let word = squash(word);
    if word.is_empty() {
        return None;
    }
    all.iter()
        .copied()
        .find(|x| squash(name(*x)) == word)
        .or_else(|| {
            all.iter()
                .copied()
                .find(|x| squash(name(*x)).starts_with(&word))
        })
}

/// Parses `/loadout` arguments: up to four `weapon[:ability[,ability]]` (or `-` for an empty
/// slot) and `knack:<name>` anywhere. Abilities left out are the weapon's defaults; without
/// weapons the slots (and without a knack the knack) are `current`'s. The result is sanitised.
fn parse_loadout(args: &[&str], current: Loadout) -> Result<Loadout, String> {
    let (knacks, args): (Vec<&str>, Vec<&str>) = args.iter().partition(|a| {
        a.split_once(':')
            .is_some_and(|(k, _)| k.eq_ignore_ascii_case("knack"))
    });
    let mut knack = current.knack;
    for arg in knacks {
        let name = arg.split_once(':').map_or("", |(_, n)| n);
        knack = find_named(name, &Knack::ALL, Knack::name).ok_or_else(|| {
            let all: Vec<&str> = Knack::ALL.iter().map(|k| k.name()).collect();
            format!("unknown knack '{name}' ({})", all.join(", "))
        })?;
    }
    if args.is_empty() {
        return Ok(current.with_knack(knack).sanitized());
    }
    if args.len() > LOADOUT_SLOTS {
        return Err(format!("at most {LOADOUT_SLOTS} slots"));
    }
    let mut loadout = Loadout::unarmed().with_knack(knack);
    for (slot, arg) in args.iter().enumerate() {
        if *arg == "-" || arg.eq_ignore_ascii_case("none") {
            continue;
        }
        let (weapon, abilities) = arg.split_once(':').unwrap_or((arg, ""));
        let weapon = find_named(weapon, &WeaponKind::ALL, WeaponKind::name)
            .ok_or_else(|| format!("unknown weapon '{weapon}'"))?;
        let mut build = weapon.default_build();
        for (i, word) in abilities.split(',').filter(|w| !w.is_empty()).enumerate() {
            let ability = find_named(word, weapon.abilities(), Ability::name).ok_or_else(|| {
                let pool: Vec<&str> = weapon.abilities().iter().map(|a| a.name()).collect();
                format!(
                    "{} can't take '{word}' ({})",
                    weapon.name(),
                    pool.join(", ")
                )
            })?;
            if let Some(a) = build.abilities.get_mut(i) {
                *a = ability;
            }
        }
        loadout.slots[slot] = Some(build);
    }
    if loadout.first_slot().is_none() {
        return Err("no weapon given".into());
    }
    Ok(loadout.sanitized())
}

/// "1 LONGWICK (Triple Nock / Pinning Arrow)  2 KILN (...)  Q SNARE".
fn describe_loadout(loadout: &Loadout) -> String {
    let builds: Vec<String> = loadout
        .slots
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.map(|b| (i, b)))
        .map(|(i, WeaponBuild { weapon, abilities })| {
            format!(
                "{} {} ({} / {})",
                i + 1,
                weapon.name().to_uppercase(),
                abilities[0].name(),
                abilities[1].name()
            )
        })
        .collect();
    format!(
        "{}  Q {}",
        builds.join("  "),
        loadout.knack.name().to_uppercase()
    )
}

impl Server {
    /// Runs one dev command for `player`; returns the reply.
    pub(crate) fn run_chat_command(&mut self, player: PlayerId, text: &str) -> String {
        let mut words = text.trim_start_matches('/').split_whitespace();
        let cmd = words.next().unwrap_or("").to_ascii_lowercase();
        let args: Vec<&str> = words.collect();
        let tuning = self.tuning.clone();
        // A lobby without a warm-up world simulates nothing (docs/lobby.md).
        if !self.world_running() && !matches!(cmd.as_str(), "help" | "" | "enemies" | "loadout") {
            return "no match is running".into();
        }
        match cmd.as_str() {
            "help" | "" => HELP.to_string(),
            "enemies" => {
                let names: Vec<&str> = self.world.enemy_defs.names().collect();
                format!("enemies: {}", names.join(", "))
            }
            "spawn" => {
                let Some(name) = args.first() else {
                    return "usage: /spawn <enemy> [count] [elite]".into();
                };
                let count = args
                    .get(1)
                    .and_then(|c| c.parse::<u32>().ok())
                    .unwrap_or(1)
                    .clamp(1, 64);
                let elite = args.iter().skip(1).any(|a| a.eq_ignore_ascii_case("elite"));
                let Some(p) = self.world.players.get(&player) else {
                    return "no body".into();
                };
                let forward = look_dir(p.yaw, 0.0);
                let position = p.movement.position + forward * 8.0;
                // Face the player.
                let yaw = p.yaw + std::f32::consts::PI;
                let ids = self.spawn_enemies(name, position, yaw, count);
                if elite {
                    for id in &ids {
                        self.world.make_elite(*id);
                    }
                }
                if self.world.enemy_defs.get(name).is_none() {
                    format!("unknown enemy '{name}' (try /enemies)")
                } else if ids.len() < count as usize {
                    format!(
                        "spawned {} {name} (enemy limit {} reached)",
                        ids.len(),
                        shared::sim::MAX_ENEMIES
                    )
                } else {
                    format!("spawned {} {name}", ids.len())
                }
            }
            "killall" => {
                let n = self.world.enemies.values().filter(|e| e.is_alive()).count();
                self.kill_all_enemies();
                format!("killed {n} enemies")
            }
            "god" => match self.world.players.get_mut(&player) {
                Some(p) => {
                    p.combat.god = !p.combat.god;
                    format!("god {}", if p.combat.god { "on" } else { "off" })
                }
                None => "no body".into(),
            },
            "notarget" => match self.world.players.get_mut(&player) {
                Some(p) => {
                    p.notarget = !p.notarget;
                    format!("notarget {}", if p.notarget { "on" } else { "off" })
                }
                None => "no body".into(),
            },
            "give" => match self.world.players.get_mut(&player) {
                Some(p) => {
                    let w = p.combat.weapons;
                    p.combat.damage = 0.0;
                    p.combat.hard = 0.0;
                    p.combat.weapons = WeaponState {
                        prev_buttons: w.prev_buttons,
                        ..w.respawned(w.loadout)
                    };
                    "full health, knack and weapons".into()
                }
                None => "no body".into(),
            },
            "needle" | "orb" | "ember" => {
                let dist = args
                    .first()
                    .and_then(|a| a.parse::<f32>().ok())
                    .unwrap_or(16.0)
                    .clamp(3.0, 60.0);
                let Some(p) = self.world.players.get(&player) else {
                    return "no body".into();
                };
                let eye = p.movement.eye_position(&tuning.movement);
                let feet = p.movement.position;
                let ember = cmd == "ember";
                let (kind, attack, from, velocity, damage) = if ember {
                    // Lobbed from a Censer's height onto your feet.
                    let from = feet + look_dir(p.yaw, 0.0) * dist + Vec3::Y * 3.0;
                    let g = ProjectileKind::EmberBomb.gravity(&tuning.combat);
                    let (v, _) = shared::sim::lob_velocity(from, feet + Vec3::Y * 0.3, g, 26.0);
                    (ProjectileKind::EmberBomb, (kind::CENSER, 1), from, v, 18.0)
                } else {
                    // A needle slowed to 22 m/s so there is time to read it.
                    let from = eye + look_dir(p.yaw, 0.0) * dist;
                    let v = (eye - from).normalize_or(Vec3::Z) * 22.0;
                    (ProjectileKind::Needle, (kind::HERON, 1), from, v, 20.0)
                };
                // A phantom enemy owns it (RETURN TO SENDER has nobody to hit).
                let spawned = self.world.alloc_net_id().and_then(|owner| {
                    self.world.spawn_enemy_projectile(
                        &tuning,
                        kind,
                        owner,
                        Attack::Enemy {
                            kind: attack.0,
                            attack: attack.1,
                        },
                        from,
                        velocity,
                        damage,
                    )
                });
                match spawned {
                    Some(_) if ember => format!("ember bomb incoming from {dist:.0} m"),
                    Some(_) => format!("needle incoming from {dist:.0} m"),
                    None => "too many projectiles in flight".into(),
                }
            }
            "kill" => {
                self.world.kill_player(&tuning, player, Attack::Dev);
                "you died".into()
            }
            "hurt" => {
                let amount = args
                    .first()
                    .and_then(|a| a.parse::<f32>().ok())
                    .unwrap_or(10.0)
                    .clamp(0.0, 10_000.0);
                let point = self
                    .world
                    .players
                    .get(&player)
                    .map_or(Vec3::ZERO, |p| p.movement.position + Vec3::Y);
                let info =
                    DamageInfo::new(amount, Owner::World, Attack::Dev).at(point, Vec3::NEG_Y);
                let e = self
                    .world
                    .apply_damage(&tuning, HitTarget::Player(player), info);
                format!(
                    "took {:.0} ({:.0} hard), health {:.0}",
                    e.dealt, e.hard, e.health_after
                )
            }
            "ff" => {
                let on = match args.first().map(|a| a.to_ascii_lowercase()) {
                    Some(a) if a == "on" || a == "1" => true,
                    Some(a) if a == "off" || a == "0" => false,
                    _ => !self.world.rules.friendly_fire,
                };
                self.world.rules.friendly_fire = on;
                format!("friendly fire {}", if on { "on" } else { "off" })
            }
            // Arena encounters (docs/enemies.md).
            "encounter" => {
                let n = self.world.level.encounters.len();
                let Some(i) = args
                    .first()
                    .and_then(|a| a.parse::<usize>().ok())
                    .filter(|i| (1..=n).contains(i))
                else {
                    return format!("usage: /encounter <1..{n}>");
                };
                self.world.reset_encounter(i - 1);
                self.world.start_encounter(i - 1);
                format!("encounter {i} started")
            }
            "encounters" => {
                let states: Vec<String> = (0..self.world.level.encounters.len())
                    .map(|i| {
                        let phase = self
                            .world
                            .encounters
                            .states
                            .get(i)
                            .map_or(shared::sim::encounter::EncounterPhase::Idle, |s| s.phase);
                        format!("{}: {phase:?}", i + 1)
                    })
                    .collect();
                if states.is_empty() {
                    "no encounters on this level".into()
                } else {
                    states.join(", ")
                }
            }
            "tp" => {
                let coords: Vec<f32> = args
                    .iter()
                    .filter_map(|a| a.parse::<f32>().ok())
                    .filter(|v| v.is_finite() && v.abs() < 100_000.0)
                    .collect();
                if coords.len() != 3 || args.len() != 3 {
                    return "usage: /tp <x> <y> <z>".into();
                }
                let to = Vec3::new(coords[0], coords[1], coords[2]);
                self.teleport(player, to, None)
            }
            "goto" => {
                let level = &self.world.level;
                let n = args
                    .get(1)
                    .and_then(|a| a.parse::<usize>().ok())
                    .unwrap_or(0);
                let target = match args.first().map(|a| a.to_ascii_lowercase()).as_deref() {
                    Some("exit") => level.exit.map(|v| (v.center(), None)),
                    Some("start") => level
                        .spawns
                        .first()
                        .map(|s| (s.position, Some(s.yaw.to_radians()))),
                    Some("checkpoint") => n
                        .checked_sub(1)
                        .and_then(|i| level.checkpoints.get(i))
                        .map(|c| (c.spawn.position, Some(c.spawn.yaw.to_radians()))),
                    Some("encounter") => n
                        .checked_sub(1)
                        .and_then(|i| level.encounters.get(i))
                        .and_then(|e| e.rally)
                        .map(|r| (r.position, Some(r.yaw.to_radians()))),
                    _ => return "usage: /goto <exit|start|checkpoint n|encounter n>".into(),
                };
                match target {
                    Some((to, yaw)) => self.teleport(player, to, yaw),
                    None => "no such place on this level".into(),
                }
            }
            "loadout" => {
                if args.is_empty() {
                    return format!("loadout: {}", describe_loadout(&self.world.loadout(player)));
                }
                let current = self
                    .world
                    .loadout_requests
                    .get(&player)
                    .copied()
                    .unwrap_or_else(|| self.world.loadout(player));
                let loadout = match parse_loadout(&args, current) {
                    Ok(l) => l,
                    Err(e) => {
                        return format!(
                            "{e}. usage: /loadout <weapon[:ability,ability]> ... [knack:<name>]"
                        );
                    }
                };
                // The usual rules store it; a dev wants it in hand right now (with the knack
                // this world allows: no Thorn in PvP).
                self.world.request_loadout(player, loadout);
                let fitted = self.world.fit_loadout(loadout);
                if let Some(p) = self.world.players.get_mut(&player) {
                    p.combat.weapons = p.combat.weapons.with_loadout(fitted);
                }
                let mut reply = format!("loadout: {}", describe_loadout(&fitted));
                if fitted.knack != loadout.knack {
                    reply.push_str(&format!(" ({} is PvE only)", loadout.knack.name()));
                }
                reply
            }
            other => format!("unknown command '/{other}'. {HELP}"),
        }
    }

    /// Dev teleport (`SimWorld::teleport_player`).
    fn teleport(&mut self, player: PlayerId, to: Vec3, yaw: Option<f32>) -> String {
        if self.world.teleport_player(player, to, yaw) {
            format!("teleported to {:.1} {:.1} {:.1}", to.x, to.y, to.z)
        } else {
            "no living body to move".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `parse_loadout` from the default loadout.
    fn parse(args: &[&str]) -> Result<Loadout, String> {
        parse_loadout(args, Loadout::default())
    }

    #[test]
    fn loadout_arguments_parse() {
        let l = parse(&["longwick:pinning,ember", "-", "Kiln", "quill:splinter"]).unwrap();
        assert_eq!(
            l.slot(0).map(|b| b.abilities),
            Some([Ability::PinningArrow, Ability::EmberArrow])
        );
        assert_eq!(l.slot(1), None);
        assert_eq!(l.slot(2), Some(&WeaponKind::Kiln.default_build()));
        // One ability given: it replaces A; B stays the default unless that repeats it.
        assert_eq!(
            l.slot(3).map(|b| b.abilities),
            Some([Ability::Splinter, Ability::Brand])
        );
        assert!(l.is_valid());
        assert_eq!(l.knack, Knack::Thorn, "the current knack");
        assert_eq!(
            parse(&["triple nock"]).unwrap_err(),
            "unknown weapon 'triple nock'"
        );
        assert!(parse(&["quill:gust"]).is_err());
        assert!(parse(&["-"]).is_err());
        assert!(parse(&["quill"; 5]).is_err());
        // The same weapon twice: the second is dropped.
        let l = parse(&["lancet", "lancet"]).unwrap();
        assert_eq!(l.slot(1), None);
        assert!(describe_loadout(&l).starts_with("1 LANCET (Skewer / Flurry)"));
        assert!(describe_loadout(&l).ends_with("Q THORN"));
        // Melee builds, and the knack by name prefix anywhere (not a slot).
        let l = parse(&["knack:kindle", "trimmer:whirl,upswing", "tamper"]).unwrap();
        assert_eq!(l.knack, Knack::KindleFlask);
        assert_eq!(
            l.slot(0).map(|b| b.abilities),
            Some([Ability::Whirl, Ability::Upswing])
        );
        assert_eq!(l.slot(1), Some(&WeaponKind::Tamper.default_build()));
        assert!(parse(&["quill", "knack:soot", "-", "-", "kiln"]).is_ok());
        assert!(
            parse(&["knack:nothing"])
                .unwrap_err()
                .starts_with("unknown knack")
        );
        // Only a knack: the current weapons keep.
        let current = Loadout::single(WeaponKind::Kiln.default_build());
        let l = parse_loadout(&["KNACK:tallow"], current).unwrap();
        assert_eq!(l, current.with_knack(Knack::TallowSkin));
    }
}
