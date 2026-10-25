//! Headless dedicated server.
//!
//! ```text
//! velocide-server [--port 47420] [--bind 0.0.0.0] [--level proving_ground] [--max-players 8]
//!                 [--tick-rate 60] [--snapshot-rate 30] [--netsim 100,20,2] [--tuning PATH]
//!                 [--no-watch] [--no-delta] [--name NAME]
//! ```

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use server::{NetSimConfig, Server, ServerConfig};

const USAGE: &str = "\
velocide-server: headless VELOCIDE dedicated server

USAGE:
    velocide-server [OPTIONS]

OPTIONS:
    --port <PORT>            UDP port (default 47420)
    --bind <IP>              address to bind (default 0.0.0.0)
    --level <NAME>           level to load (default proving_ground)
    --max-players <N>        player slots (default 8)
    --tick-rate <HZ>         simulation rate (default 60)
    --snapshot-rate <HZ>     snapshot rate (default: tuning.ron net.snapshot_rate, 30)
    --netsim <L,J,LOSS[,D]>  simulate latency ms, jitter ms, loss %, duplicate %
    --tuning <PATH>          tuning file (default assets/tuning.ron), hot-reloaded
    --no-watch               don't watch the tuning file
    --no-delta               disable snapshot delta compression (bandwidth testing)
    --name <NAME>            server name
    -h, --help               print this help";

fn parse_args() -> Result<ServerConfig, String> {
    let mut config = ServerConfig::default();
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
            "--no-watch" => config.watch_tuning = false,
            "--no-delta" => config.delta_compression = false,
            "--name" => config.server_name = value("--name")?,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    config.bind = SocketAddr::new(ip, port);
    Ok(config)
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
