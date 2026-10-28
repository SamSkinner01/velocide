//! `velocide-rendezvous`: VELOCIDE's online lobby service (docs/rendezvous.md).
//!
//! A UDP loop around [`rendezvous::Service`]. The master key lives in a file so restarts
//! (deploys) don't end lobbies or relayed games. `--probe` is the client side of the health
//! check the deploy script runs.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rendezvous::crypto::{KEY_LEN, random_bytes, random_nonzero_u64};
use rendezvous::proto::{self, MAX_DATAGRAM, Message};
use rendezvous::{DEFAULT_PORT, Service, ServiceConfig};

const USAGE: &str = "\
velocide-rendezvous: VELOCIDE's online lobby service (listings, join codes, hole punching, relay)

USAGE:
    velocide-rendezvous [OPTIONS]
    velocide-rendezvous --probe <HOST:PORT> [--expect <VERSION>]

OPTIONS:
    --bind <IP:PORT>           UDP address (default 0.0.0.0:47400)
    --key-file <PATH>          the master key, made on first start (default:
                               $STATE_DIRECTORY/key, else ./rendezvous.key). Keep it: with a
                               new key every lobby and relayed game starts over
    --max-listings <N>         games listed at once (default 1024)
    --max-relays <N>           relay sessions at once (default 512)
    --relay-kib <N>            relayed KiB/s toward each end of a session (default 64)
    --relay-total-kib <N>      relayed KiB/s over all sessions (default 2048)
    --relay-daily-gb <N>       relayed GB per UTC day over all sessions (default 20)
    --exit-when-replaced <PATH>
                               exit (to be restarted) once PATH points at another file: a
                               deploy swapped the binary
    --probe <HOST:PORT>        ask a running service its version, print it and exit (status 1
                               if it doesn't answer, or not with --expect's version)
    --version                  print the version
    -h, --help                 print this help";

fn version() -> String {
    let version = option_env!("VELOCIDE_VERSION").unwrap_or("dev");
    match option_env!("VELOCIDE_COMMIT") {
        Some(c) if c.len() >= 7 => format!("{version} ({})", c.get(..7).unwrap_or(c)),
        _ => version.to_string(),
    }
}

struct Args {
    bind: SocketAddr,
    key_file: PathBuf,
    config: ServiceConfig,
    exit_when_replaced: Option<PathBuf>,
}

enum Command {
    Run(Box<Args>),
    Probe {
        addr: String,
        expect: Option<String>,
    },
    Print(String),
}

fn parse_args() -> Result<Command, String> {
    let mut bind = SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT));
    let mut key_file = std::env::var_os("STATE_DIRECTORY")
        .map(|dir| PathBuf::from(dir).join("key"))
        .unwrap_or_else(|| PathBuf::from("rendezvous.key"));
    let mut config = ServiceConfig {
        version: version(),
        ..Default::default()
    };
    let mut exit_when_replaced = None;
    let mut probe = None;
    let mut expect = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        fn number(name: &str, v: String) -> Result<u64, String> {
            v.parse().map_err(|_| format!("{name}: bad number '{v}'"))
        }
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Print(USAGE.into())),
            "--version" => return Ok(Command::Print(version())),
            "--bind" => {
                let v = value("--bind")?;
                bind = v
                    .parse()
                    .map_err(|_| format!("--bind: bad address '{v}'"))?;
            }
            "--key-file" => key_file = PathBuf::from(value("--key-file")?),
            "--max-listings" => {
                config.max_listings = number("--max-listings", value("--max-listings")?)? as usize
            }
            "--max-relays" => {
                config.max_relays = number("--max-relays", value("--max-relays")?)? as usize
            }
            "--relay-kib" => {
                let kib = number("--relay-kib", value("--relay-kib")?)?;
                config.relay_rate = kib.saturating_mul(1024);
                config.relay_burst = config.relay_burst.max(config.relay_rate);
            }
            "--relay-total-kib" => {
                config.relay_total_rate =
                    number("--relay-total-kib", value("--relay-total-kib")?)?.saturating_mul(1024)
            }
            "--relay-daily-gb" => {
                config.relay_daily_bytes = number("--relay-daily-gb", value("--relay-daily-gb")?)?
                    .saturating_mul(1_000_000_000)
            }
            "--exit-when-replaced" => {
                exit_when_replaced = Some(PathBuf::from(value("--exit-when-replaced")?))
            }
            "--probe" => probe = Some(value("--probe")?),
            "--expect" => expect = Some(value("--expect")?),
            other => return Err(format!("unknown option '{other}' (try --help)")),
        }
    }
    Ok(match probe {
        Some(addr) => Command::Probe { addr, expect },
        None => Command::Run(Box::new(Args {
            bind,
            key_file,
            config,
            exit_when_replaced,
        })),
    })
}

/// Reads the master key, or makes one (readable by the service's user only).
fn load_key(path: &Path) -> io::Result<[u8; KEY_LEN]> {
    match fs::File::open(path) {
        Ok(mut file) => {
            let mut key = [0u8; KEY_LEN];
            let mut extra = [0u8; 1];
            file.read_exact(&mut key)?;
            if file.read(&mut extra)? != 0 {
                return Err(io::Error::other(format!(
                    "{} is not a {KEY_LEN}-byte key",
                    path.display()
                )));
            }
            Ok(key)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let key = random_bytes::<KEY_LEN>();
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(path)?;
            file.write_all(&key)?;
            file.sync_all()?;
            log::info!("made a new key in {}", path.display());
            Ok(key)
        }
        Err(e) => Err(e),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn run(args: Args) -> Result<(), String> {
    let key = load_key(&args.key_file)
        .map_err(|e| format!("key file {}: {e}", args.key_file.display()))?;
    let socket =
        UdpSocket::bind(args.bind).map_err(|e| format!("binding UDP {}: {e}", args.bind))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(20)))
        .map_err(|e| e.to_string())?;
    let watched = args.exit_when_replaced.map(|path| {
        let target = fs::canonicalize(&path).ok();
        (path, target)
    });
    log::info!(
        "velocide-rendezvous {} listening on {} (limits: {} listings, {} relays, {} KiB/s per \
         relay end, {} KiB/s relayed in all, {} GB a day)",
        args.config.version,
        args.bind,
        args.config.max_listings,
        args.config.max_relays,
        args.config.relay_rate / 1024,
        args.config.relay_total_rate / 1024,
        args.config.relay_daily_bytes / 1_000_000_000
    );

    let epoch = Instant::now();
    let mut service = Service::new(&key, args.config, Duration::ZERO);
    let mut buf = vec![0u8; 2048];
    let mut out = Vec::new();
    let mut last_tick = Duration::ZERO;
    let mut last_stats = Duration::ZERO;
    let mut last_check = Duration::ZERO;
    loop {
        match socket.recv_from(&mut buf) {
            Ok((len, from)) => {
                let data = buf.get(..len).unwrap_or_default();
                service.handle(epoch.elapsed(), unix_now(), from, data, &mut out);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                        | io::ErrorKind::ConnectionReset
                ) => {}
            Err(e) => {
                log::warn!("UDP receive: {e}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let now = epoch.elapsed();
        if now.saturating_sub(last_tick) >= Duration::from_millis(50) {
            last_tick = now;
            service.tick(now, unix_now(), &mut out);
        }
        for (to, data) in out.drain(..) {
            if let Err(e) = socket.send_to(&data, to) {
                log::debug!("send to {to}: {e}");
            }
        }
        if now.saturating_sub(last_stats) >= Duration::from_secs(300) {
            last_stats = now;
            let s = service.stats();
            log::info!(
                "{} listings, {} relays; since start: {} packets ({} malformed, {} bad cookies, \
                 {} rate limited, {} refused), {} registrations, {} joins ({} failed), {} MB \
                 relayed ({} packets dropped)",
                s.listings,
                s.relays,
                s.packets,
                s.malformed,
                s.bad_cookies,
                s.rate_limited,
                s.refused,
                s.registrations,
                s.joins,
                s.join_failures,
                s.relayed_bytes / 1_000_000,
                s.relay_dropped
            );
        }
        if now.saturating_sub(last_check) >= Duration::from_secs(5) {
            last_check = now;
            if let Some((path, target)) = &watched {
                let current = fs::canonicalize(path).ok();
                if current.is_some() && current != *target {
                    log::info!("{} now points at a new binary: exiting", path.display());
                    return Ok(());
                }
            }
        }
    }
}

/// `--probe`: the version a running service reports.
fn probe(addr: &str, expect: Option<&str>) -> Result<(), String> {
    let target = rendezvous::parse_service(addr).and_then(|(host, port)| {
        use std::net::ToSocketAddrs;
        (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| e.to_string())?
            .find(SocketAddr::is_ipv4)
            .ok_or_else(|| format!("{addr} has no IPv4 address"))
    })?;
    let socket = UdpSocket::bind(("0.0.0.0", 0)).map_err(|e| e.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|e| e.to_string())?;
    let nonce = random_nonzero_u64();
    let request = proto::encode(&Message::Probe { nonce });
    let mut buf = [0u8; MAX_DATAGRAM];
    for _ in 0..6 {
        socket
            .send_to(&request, target)
            .map_err(|e| format!("send to {target}: {e}"))?;
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            let Ok((len, from)) = socket.recv_from(&mut buf) else {
                break;
            };
            let data = buf.get(..len).unwrap_or_default();
            if let (true, Ok(Message::ProbeReply { nonce: n, version })) =
                (from == target, proto::decode(data))
                && n == nonce
            {
                println!("{version}");
                return match expect {
                    Some(want) if version != want && !version.starts_with(&format!("{want} ")) => {
                        Err(format!("{target} runs {version}, not {want}"))
                    }
                    _ => Ok(()),
                };
            }
        }
    }
    Err(format!("no answer from {target}"))
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let result = match parse_args() {
        Ok(Command::Print(text)) => {
            println!("{text}");
            Ok(())
        }
        Ok(Command::Probe { addr, expect }) => probe(&addr, expect.as_deref()),
        Ok(Command::Run(args)) => run(*args),
        Err(e) => Err(e),
    };
    if let Err(e) = result {
        eprintln!("velocide-rendezvous: {e}");
        std::process::exit(1);
    }
}
