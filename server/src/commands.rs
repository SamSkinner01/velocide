//! Dev chat commands (`ServerConfig::dev_commands`): any chat message starting with `/` is a
//! command for the sender; the reply comes back as a server chat line to them only.
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
//! | `/help` | this list |

use glam::Vec3;
use shared::input::look_dir;
use shared::sim::{Attack, DamageInfo, HitTarget, Owner, PlayerId, WeaponState};

use crate::Server;

const HELP: &str = "/spawn <enemy> [count], /enemies, /killall, /god, /notarget, /give, /kill, /ff [on|off], /hurt <hp>";

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
                if ids.is_empty() {
                    format!("unknown enemy '{name}' (try /enemies)")
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
            other => format!("unknown command '/{other}'. {HELP}"),
        }
    }
}
