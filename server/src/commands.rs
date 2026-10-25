//! Dev chat commands (`ServerConfig::dev_commands`): any chat message starting with `/` is a
//! command for the sender; the reply comes back as a server chat line to them only. Who may
//! use them: single-player, the host of a listen server (never guests), and on dedicated
//! servers started with `--dev-commands` the lobby host (everyone with `--no-lobby`). Each
//! command costs a chat token (chat's rate limit). Elsewhere `/...` is answered with a
//! refusal and never broadcast.
//!
//! | command | effect |
//! |---|---|
//! | `/spawn <enemy> [count]` | spawns enemies 8 m in front of you (on the ground) |
//! | `/enemies` | lists enemy definitions |
//! | `/killall` | kills every enemy |
//! | `/god` | toggles invulnerability |
//! | `/notarget` | toggles: enemies ignore you |
//! | `/give` | full health, no hard damage, full prisms, weapons ready |
//! | `/kill` | kills you (respawn rule applies) |
//! | `/ff [on/off]` | PvP damage between players |
//! | `/hurt <amount>` | damages you (test hard damage / healing) |
//! | `/needle [distance]` (alias `/orb`) | a slow parryable Heron needle flies at you from in front (parry practice) |
//! | `/ember [distance]` | a Censer ember bomb is lobbed onto you from in front (parry practice) |
//! | `/encounter <n>` | starts arena encounter n (1-based; stragglers incl. you go to its rally point) |
//! | `/encounters` | encounter states (docs/enemies.md) |
//! | `/tp <x> <y> <z>` | teleports you to that point (feet) |
//! | `/goto <exit / start / checkpoint n / encounter n>` | teleports you to the level exit, the start, checkpoint n's spawn or encounter n's rally point |
//! | `/help` | this list |

use glam::Vec3;
use shared::input::look_dir;
use shared::protocol::kind;
use shared::sim::{Attack, DamageInfo, HitTarget, Owner, PlayerId, ProjectileKind, WeaponState};

use crate::Server;

const HELP: &str = "/spawn <enemy> [count], /enemies, /killall, /god, /notarget, /give, /kill, /ff [on|off], /hurt <hp>, /needle [dist], /ember [dist], /encounter <n>, /encounters, /tp <x> <y> <z>, /goto <exit|start|checkpoint n|encounter n>";

impl Server {
    /// Runs one dev command for `player`; returns the reply.
    pub(crate) fn run_chat_command(&mut self, player: PlayerId, text: &str) -> String {
        let mut words = text.trim_start_matches('/').split_whitespace();
        let cmd = words.next().unwrap_or("").to_ascii_lowercase();
        let args: Vec<&str> = words.collect();
        let tuning = self.tuning.clone();
        // In the lobby there is no level being simulated (docs/lobby.md).
        if !self.flow.phase.level_running() && !matches!(cmd.as_str(), "help" | "" | "enemies") {
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
                    return "usage: /spawn <enemy> [count]".into();
                };
                let count = args
                    .get(1)
                    .and_then(|c| c.parse::<u32>().ok())
                    .unwrap_or(1)
                    .clamp(1, 64);
                let Some(p) = self.world.players.get(&player) else {
                    return "no body".into();
                };
                let forward = look_dir(p.yaw, 0.0);
                let position = p.movement.position + forward * 8.0;
                // Face the player.
                let yaw = p.yaw + std::f32::consts::PI;
                let ids = self.spawn_enemies(name, position, yaw, count);
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
                        stickies: w.stickies,
                        ..WeaponState::fresh(w.weapon, w.mode)
                    };
                    "full health, prisms and weapons".into()
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
