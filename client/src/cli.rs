//! Command line of the `velocide` binary (hand-rolled, no dependencies).

use std::path::PathBuf;

use shared::lobby::{Difficulty, GameMode};

use crate::net::NetSimConfig;

pub const USAGE: &str = "\
velocide: the VELOCIDE game client

USAGE:
    velocide [OPTIONS]            (no options: main menu)

GAME:
    --sp [LEVEL]                 single-player (in-process server) on LEVEL
    --host [PORT]                host a listen server and play (default port 47420)
    --connect <IP:PORT>          join a server
    --level <NAME>               level for --sp / --host (alone: single-player)
    --name <NAME>                player name (not saved)
    --max-players <N>            player slots when hosting (default from settings)
    --mode <MODE>                campaign | deathmatch | gungame | grind (hosting: default last
                                 used; --sp: grind plays endless waves on an arena)
    --difficulty <LEVEL>         campaign / grind: easy | normal | hard | brutal | inferno
    --ready                      ready up in the lobby automatically
    --autostart                  hosting: start the match when everyone is ready (implies --ready)
    --netsim <LAT,JIT,LOSS>      simulate one-way latency ms, jitter ms, loss % on this client

WINDOW:
    --windowed <WxH>             windowed at this size
    --fullscreen                 borderless fullscreen

TESTING:
    --bot                        auto-pilot: runs, turns, jumps, dashes, slides and slams
    --capture <PATH.png>         save a screenshot (then keep running)
    --capture-after <SECONDS>    when to take it (default 3)
    --quit-after <SECONDS>       exit cleanly after this long
    --no-audio                   don't open the audio device
    --debug                      start with the debug overlay (F3) open
    --add-bots <N>               (--host) add N standing server-side bot players
    -h, --help                   print this help";

/// What to do at startup.
#[derive(Clone, Debug, PartialEq)]
pub enum StartMode {
    Menu,
    SinglePlayer {
        level: Option<String>,
    },
    Host {
        port: Option<u16>,
        level: Option<String>,
    },
    Join {
        address: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct LaunchOptions {
    pub start: StartMode,
    pub name: Option<String>,
    pub max_players: Option<u32>,
    /// Hosting: initial game mode. Single-player: grind on an arena.
    pub mode: Option<GameMode>,
    /// PvE difficulty (single-player, hosting); default: the last one picked.
    pub difficulty: Option<Difficulty>,
    /// Ready up in the lobby automatically (`--ready`, `--bot`, `--autostart`).
    pub ready: bool,
    /// Hosting: the server starts the match once everyone is ready.
    pub autostart: bool,
    pub netsim: Option<NetSimConfig>,
    pub window_size: Option<(u32, u32)>,
    pub fullscreen: Option<bool>,
    pub bot: bool,
    pub capture: Option<PathBuf>,
    pub capture_after: f32,
    pub quit_after: Option<f32>,
    pub no_audio: bool,
    pub debug_overlay: bool,
    pub add_bots: u32,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            start: StartMode::Menu,
            name: None,
            max_players: None,
            mode: None,
            difficulty: None,
            ready: false,
            autostart: false,
            netsim: None,
            window_size: None,
            fullscreen: None,
            bot: false,
            capture: None,
            capture_after: 3.0,
            quit_after: None,
            no_audio: false,
            debug_overlay: false,
            add_bots: 0,
        }
    }
}

/// Result of parsing: run the game, or print help.
#[derive(Clone, Debug, PartialEq)]
pub enum Cli {
    Run(Box<LaunchOptions>),
    Help,
}

fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("--windowed expects WxH, got '{text}'"))?;
    let w: u32 = w
        .trim()
        .parse()
        .map_err(|_| format!("bad width in '{text}'"))?;
    let h: u32 = h
        .trim()
        .parse()
        .map_err(|_| format!("bad height in '{text}'"))?;
    if w < 160 || h < 120 || w > 16384 || h > 16384 {
        return Err(format!("window size {w}x{h} out of range"));
    }
    Ok((w, h))
}

fn parse_secs(name: &str, text: &str) -> Result<f32, String> {
    let v: f32 = text
        .parse()
        .map_err(|_| format!("{name} expects seconds, got '{text}'"))?;
    if !v.is_finite() || v < 0.0 {
        return Err(format!("{name} must be >= 0"));
    }
    Ok(v)
}

/// Parses the arguments (without the program name).
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut o = LaunchOptions::default();
    let mut level: Option<String> = None;
    let mut args = args.into_iter().peekable();
    // Value of an option that may be omitted (`--sp`, `--host`): taken if it isn't a flag.
    fn optional(args: &mut std::iter::Peekable<impl Iterator<Item = String>>) -> Option<String> {
        if args.peek().is_some_and(|a| !a.starts_with("--")) {
            args.next()
        } else {
            None
        }
    }
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => return Ok(Cli::Help),
            "--sp" => {
                let l = optional(&mut args);
                o.start = StartMode::SinglePlayer { level: l };
            }
            "--host" => {
                let port = match optional(&mut args) {
                    Some(p) => Some(p.parse().map_err(|_| format!("--host: bad port '{p}'"))?),
                    None => None,
                };
                o.start = StartMode::Host { port, level: None };
            }
            "--connect" => {
                o.start = StartMode::Join {
                    address: value("--connect")?,
                }
            }
            "--level" => level = Some(value("--level")?),
            "--name" => o.name = Some(value("--name")?),
            "--max-players" => {
                let v = value("--max-players")?;
                o.max_players = Some(
                    v.parse()
                        .map_err(|_| format!("--max-players: bad number '{v}'"))?,
                )
            }
            "--mode" => {
                let v = value("--mode")?;
                o.mode = Some(GameMode::parse(&v).ok_or_else(|| {
                    format!("--mode: '{v}' is not campaign, deathmatch, gungame or grind")
                })?);
            }
            "--difficulty" => {
                let v = value("--difficulty")?;
                o.difficulty = Some(Difficulty::parse(&v).ok_or_else(|| {
                    format!("--difficulty: '{v}' is not easy, normal, hard, brutal or inferno")
                })?);
            }
            "--ready" => o.ready = true,
            "--autostart" => {
                o.autostart = true;
                o.ready = true;
            }
            "--netsim" => {
                let v = value("--netsim")?;
                o.netsim = Some(NetSimConfig::parse(&v)?);
            }
            "--windowed" => {
                o.window_size = Some(parse_size(&value("--windowed")?)?);
                o.fullscreen = Some(false);
            }
            "--fullscreen" => o.fullscreen = Some(true),
            "--bot" => o.bot = true,
            "--capture" => o.capture = Some(PathBuf::from(value("--capture")?)),
            "--capture-after" => {
                o.capture_after = parse_secs("--capture-after", &value("--capture-after")?)?
            }
            "--quit-after" => {
                o.quit_after = Some(parse_secs("--quit-after", &value("--quit-after")?)?)
            }
            "--no-audio" => o.no_audio = true,
            "--debug" => o.debug_overlay = true,
            "--add-bots" => {
                let v = value("--add-bots")?;
                o.add_bots = v
                    .parse()
                    .map_err(|_| format!("--add-bots: bad number '{v}'"))?
            }
            other => return Err(format!("unknown option '{other}' (try --help)")),
        }
    }
    if let Some(l) = level {
        match &mut o.start {
            StartMode::SinglePlayer { level } | StartMode::Host { level, .. } => {
                level.get_or_insert(l);
            }
            StartMode::Menu => o.start = StartMode::SinglePlayer { level: Some(l) },
            StartMode::Join { .. } => {
                return Err("--level can't be combined with --connect".into());
            }
        }
    }
    Ok(Cli::Run(Box::new(o)))
}
