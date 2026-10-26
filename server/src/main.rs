//! Headless dedicated server.
//!
//! ```text
//! velocide-server [--port 47420] [--bind 0.0.0.0] [--level proving_ground] [--max-players 8]
//!                 [--mode campaign|deathmatch|gungame|grind] [--difficulty normal]
//!                 [--frag-limit 20] [--time-limit 600] [--friendly-fire] [--autostart] [--no-lobby]
//!                 [--tick-rate 60] [--snapshot-rate 30] [--netsim 100,20,2] [--tuning PATH]
//!                 [--no-watch] [--no-delta] [--name NAME] [--dev-commands]
//! ```
//!
//! The server starts in the lobby (docs/lobby.md): the first player to join is the host and
//! starts the match from their lobby screen (the role passes on when they leave). With
//! `--autostart` the match starts by itself once every connected player is ready.
//!
//! Dev chat commands (`/god`, `/spawn`, `/tp`...) are off on dedicated servers unless
//! `--dev-commands` is given; then only the lobby host may use them (everyone with
//! `--no-lobby`).

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use server::{LobbyConfig, NetSimConfig, Server, ServerConfig};
use shared::lobby::{Difficulty, GameMode, LevelInfo, MatchSettings};

const USAGE: &str = "\
velocide-server: headless VELOCIDE dedicated server

USAGE:
    velocide-server [OPTIONS]

OPTIONS:
    --port <PORT>            UDP port (default 47420)
    --bind <IP>              address to bind (default 0.0.0.0)
    --level <NAME>           first level (default proving_ground)
    --max-players <N>        player slots (default 8; co-op at most 4)
    --mode <MODE>            campaign | deathmatch | gungame | grind (default: the level's
                             kind)
    --difficulty <LEVEL>     campaign / grind: easy | normal | hard | brutal | inferno
                             (default normal; the lobby host can change it)
    --frag-limit <N>         deathmatch frags to win, 0 = none (default 20)
    --time-limit <SECONDS>   deathmatch length, 0 = none (default 600)
    --friendly-fire          co-op: players can hurt each other
    --autostart              start the match when every connected player is ready
    --no-lobby               no lobby: run the level right away (old behaviour)
    --tick-rate <HZ>         simulation rate (default 60)
    --snapshot-rate <HZ>     snapshot rate (default: tuning.ron net.snapshot_rate, 30)
    --netsim <L,J,LOSS[,D]>  simulate latency ms, jitter ms, loss %, duplicate %
    --tuning <PATH>          tuning file (default assets/tuning.ron), hot-reloaded
    --no-watch               don't watch the tuning file
    --no-delta               disable snapshot delta compression (bandwidth testing)
    --name <NAME>            server name
    --dev-commands           allow dev chat commands (/god, /spawn, /tp...) for the lobby
                             host (for everyone with --no-lobby); off by default
    -h, --help               print this help";

fn parse_args() -> Result<ServerConfig, String> {
    let mut config = ServerConfig {
        lobby: LobbyConfig::enabled(),
        // Opt-in on dedicated servers (`--dev-commands`).
        dev_commands: false,
        ..Default::default()
    };
    let mut mode: Option<GameMode> = None;
    let mut ip: IpAddr = config.bind.ip();
    let mut port = config.bind.port();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--port" => {
                port = value("--port")?
                    .parse()
                    .map_err(|e| format!("--port: {e}"))?
            }
            "--bind" => {
                ip = value("--bind")?
                    .parse()
                    .map_err(|e| format!("--bind: {e}"))?
            }
            "--level" => config.level = value("--level")?,
            "--max-players" => {
                config.max_players = value("--max-players")?
                    .parse()
                    .map_err(|e| format!("--max-players: {e}"))?
            }
            "--tick-rate" => {
                config.tick_rate = value("--tick-rate")?
                    .parse()
                    .map_err(|e| format!("--tick-rate: {e}"))?
            }
            "--snapshot-rate" => {
                config.snapshot_rate = Some(
                    value("--snapshot-rate")?
                        .parse()
                        .map_err(|e| format!("--snapshot-rate: {e}"))?,
                )
            }
            "--netsim" => config.netsim = Some(NetSimConfig::parse(&value("--netsim")?)?),
            "--tuning" => {
                config.tuning_path = Some(server::resolve_tuning_path(value("--tuning")?.as_ref()))
            }
            "--mode" => {
                let v = value("--mode")?;
                mode = Some(GameMode::parse(&v).ok_or_else(|| {
                    format!("--mode: '{v}' is not campaign, deathmatch, gungame or grind")
                })?)
            }
            "--difficulty" => {
                let v = value("--difficulty")?;
                config.lobby.settings.difficulty = Difficulty::parse(&v).ok_or_else(|| {
                    format!("--difficulty: '{v}' is not easy, normal, hard, brutal or inferno")
                })?;
            }
            "--frag-limit" => {
                config.lobby.settings.frag_limit = value("--frag-limit")?
                    .parse()
                    .map_err(|e| format!("--frag-limit: {e}"))?
            }
            "--time-limit" => {
                config.lobby.settings.time_limit_secs = value("--time-limit")?
                    .parse()
                    .map_err(|e| format!("--time-limit: {e}"))?
            }
            "--friendly-fire" => config.lobby.settings.friendly_fire = true,
            "--autostart" => config.lobby.autostart = true,
            "--no-lobby" => config.lobby.enabled = false,
            "--no-watch" => config.watch_tuning = false,
            "--no-delta" => config.delta_compression = false,
            "--name" => config.server_name = value("--name")?,
            "--dev-commands" => config.dev_commands = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    config.bind = SocketAddr::new(ip, port);
    config.max_players = config
        .max_players
        .clamp(1, shared::lobby::MAX_PLAYERS as usize);
    check_match(&mut config, mode)?;
    Ok(config)
}

/// Validates the first match: the level must exist and suit `--mode` (without `--mode`, the
/// level's own kind decides).
fn check_match(config: &mut ServerConfig, mode: Option<GameMode>) -> Result<(), String> {
    let def = shared::sim::load_level(&config.level).map_err(|e| format!("--level: {e:#}"))?;
    let info = LevelInfo::from_def(&config.level, &def);
    let mode = mode.unwrap_or_else(|| GameMode::for_level_kind(info.kind));
    let mut settings = MatchSettings {
        mode,
        level: config.level.clone(),
        max_players: config.max_players as u8,
        ..config.lobby.settings.clone()
    };
    settings.clamp();
    settings
        .validate(std::slice::from_ref(&info))
        .map_err(|e| format!("{e} (--mode {} --level {})", mode.id(), config.level))?;
    config.max_players = settings.max_players as usize;
    config.lobby.settings = settings;
    Ok(())
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = match parse_args() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("error: {err}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    log::info!(
        "{} dedicated server '{}'",
        shared::GAME_NAME,
        config.server_name
    );
    if config.dev_commands {
        log::warn!(
            "dev chat commands enabled ({})",
            if config.lobby.enabled {
                "lobby host only"
            } else {
                "every player: no lobby"
            }
        );
    }

    let mut server = match Server::new(config) {
        Ok(s) => s,
        Err(err) => {
            log::error!("failed to start: {err:#}");
            std::process::exit(1);
        }
    };

    let running = Arc::new(AtomicBool::new(true));
    {
        let running = running.clone();
        if let Err(err) = ctrlc::set_handler(move || running.store(false, Ordering::SeqCst)) {
            log::warn!("no Ctrl-C handler: {err}");
        }
    }

    let mut last = Instant::now();
    let mut last_report = Instant::now();
    while running.load(Ordering::SeqCst) {
        let now = Instant::now();
        server.update((now - last).min(Duration::from_millis(250)));
        last = now;
        if last_report.elapsed() >= Duration::from_secs(30) {
            last_report = Instant::now();
            let s = server.stats();
            log::info!(
                "tick {} | {} players | out {:.1} KB/s in {:.1} KB/s | tick {:.0} us",
                s.tick,
                s.players,
                s.transport.bytes_sent_per_sec / 1024.0,
                s.transport.bytes_received_per_sec / 1024.0,
                s.tick_time_us
            );
        }
        std::thread::sleep(server.suggested_sleep());
    }
    log::info!("shutting down");
    server.shutdown();
}
