//! Session layer: how a game is started or joined.
//!
//! | session                     | server                                   | client connects to   |
//! |-----------------------------|------------------------------------------|----------------------|
//! | [`Session::single_player`]  | in-process thread on `127.0.0.1:0`       | that loopback port   |
//! | [`Session::host`]           | in-process thread on `0.0.0.0:<port>`, lobby | loopback, same port |
//! | [`Session::join`]           | remote                                   | the given address    |
//!
//! Single-player is just a private listen server, so there is exactly one code path. Hosted
//! servers start in the lobby (docs/lobby.md); the session gives the in-process server a
//! random host token and the local client presents it, so only this player is the host.
//!
//! ## Plugging in Steam networking / matchmaking
//!
//! The seam has three parts:
//! - [`JoinTarget`]: *where* to connect. Add e.g. `SteamLobby(u64)` or
//!   `Matchmaking { ticket }`; [`Session::start`] resolves it to a transport.
//! - [`super::ClientTransport`]: *how* bytes reach the server (Steam's relay network instead
//!   of UDP). Build the client with [`NetClient::with_transport`].
//! - `server::ServerTransport`: the host side (listen on Steam sockets in addition to / instead
//!   of UDP), passed to `server::Server::with_transport`.
//!
//! Matchmaking services usually just hand out an address (+ optionally a connect token for
//! netcode's `Secure` mode): resolve them to [`JoinTarget::Address`].

use std::net::{SocketAddr, ToSocketAddrs};

use server::{NetSimConfig, ServerConfig, ServerHandle};
use shared::lobby::{Difficulty, GameMode};

use super::{ClientConfig, NetClient};

/// What kind of session this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    SinglePlayer,
    /// Listen server: this process hosts and plays.
    Host,
    /// Connected to a remote (dedicated or listen) server.
    Join,
}

/// Where to connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinTarget {
    Address(SocketAddr),
    // Future: SteamLobby(u64), SteamId(u64), Matchmaking { ticket: String }
}

/// How to start a session.
#[derive(Clone, Debug)]
pub enum SessionSpec {
    /// `mode`: a mode other than the level's natural one (solo grind on an arena); the
    /// difficulty applies to the PvE modes.
    SinglePlayer {
        level: String,
        mode: Option<GameMode>,
        difficulty: Difficulty,
    },
    Host {
        server: Box<ServerConfig>,
    },
    Join {
        target: JoinTarget,
    },
}

/// A running game session: the client plus, when hosting, the in-process server.
pub struct Session {
    kind: SessionKind,
    server: Option<ServerHandle>,
    client: NetClient,
}

impl Session {
    /// Spawns a private server on loopback and connects to it.
    pub fn single_player(level: &str, name: &str) -> anyhow::Result<Session> {
        Self::start(
            SessionSpec::SinglePlayer {
                level: level.to_string(),
                mode: None,
                difficulty: Difficulty::Normal,
            },
            ClientConfig::new(name),
        )
    }

    /// Hosts a listen server on `port` (all interfaces, starting in the lobby) and connects to
    /// it as the lobby host.
    pub fn host(port: u16, level: &str, max_players: usize, name: &str) -> anyhow::Result<Session> {
        Self::start(
            SessionSpec::Host {
                server: Box::new(ServerConfig::host(port, level, max_players)),
            },
            ClientConfig::new(name),
        )
    }

    /// Connects to a server.
    pub fn join(addr: SocketAddr, name: &str) -> anyhow::Result<Session> {
        Self::start(
            SessionSpec::Join {
                target: JoinTarget::Address(addr),
            },
            ClientConfig::new(name),
        )
    }

    /// General form (client options such as the network simulator go in `client`).
    pub fn start(spec: SessionSpec, client: ClientConfig) -> anyhow::Result<Session> {
        match spec {
            SessionSpec::SinglePlayer {
                level,
                mode,
                difficulty,
            } => {
                let mut config = ServerConfig::single_player(&level);
                config.lobby.settings.difficulty = difficulty;
                if let Some(mode) = mode {
                    config.lobby.settings.mode = mode;
                }
                let server = ServerHandle::spawn(config)?;
                let net = NetClient::connect(server.connect_addr(), client)?;
                Ok(Session {
                    kind: SessionKind::SinglePlayer,
                    server: Some(server),
                    client: net,
                })
            }
            SessionSpec::Host { mut server } => {
                let mut client = client;
                if server.lobby.enabled {
                    // Only our own client knows the token, so only we can be the host: it
                    // must be unguessable (OS randomness, not time ^ pid).
                    let token = *server
                        .lobby
                        .host_token
                        .get_or_insert_with(random_host_token);
                    client.host_token = Some(token);
                }
                let server = ServerHandle::spawn(*server)?;
                let net = NetClient::connect(server.connect_addr(), client)?;
                Ok(Session {
                    kind: SessionKind::Host,
                    server: Some(server),
                    client: net,
                })
            }
            SessionSpec::Join {
                target: JoinTarget::Address(addr),
            } => Ok(Session {
                kind: SessionKind::Join,
                server: None,
                client: NetClient::connect(addr, client)?,
            }),
        }
    }

    pub fn kind(&self) -> SessionKind {
        self.kind
    }

    pub fn client(&self) -> &NetClient {
        &self.client
    }

    pub fn client_mut(&mut self) -> &mut NetClient {
        &mut self.client
    }

    /// The in-process server (single-player / host).
    pub fn server(&self) -> Option<&ServerHandle> {
        self.server.as_ref()
    }

    /// Address of the in-process server for others to join (host): the bound port.
    pub fn hosted_port(&self) -> Option<u16> {
        self.server.as_ref().map(|s| s.local_addr().port())
    }

    /// Sends a command to the in-process server (single-player / host).
    pub fn command(&self, cmd: server::ServerCommand) {
        if let Some(server) = &self.server {
            server.send(cmd);
        }
    }

    /// Disconnects and stops the in-process server, if any.
    pub fn shutdown(mut self) {
        self.client.disconnect();
        if let Some(server) = self.server.take() {
            server.stop();
        }
    }
}

/// Parses "ip:port", "ip", "host:port" or "host" (default port) into an address.
pub fn resolve_address(text: &str) -> anyhow::Result<SocketAddr> {
    let text = text.trim();
    if let Ok(addr) = text.parse::<SocketAddr>() {
        return Ok(addr);
    }
    if let Ok(ip) = text.parse::<std::net::IpAddr>() {
        return Ok(SocketAddr::new(ip, shared::DEFAULT_PORT));
    }
    if text.is_empty() {
        anyhow::bail!("empty address");
    }
    let has_port = text
        .rsplit_once(':')
        .is_some_and(|(_, port)| port.parse::<u16>().is_ok());
    let candidate = if has_port {
        text.to_string()
    } else {
        format!("{text}:{}", shared::DEFAULT_PORT)
    };
    candidate
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .ok_or_else(|| anyhow::anyhow!("can't resolve '{text}'"))
}

/// Parses a `--netsim` value (`latency,jitter,loss[,dup]`).
pub fn parse_netsim(text: &str) -> anyhow::Result<NetSimConfig> {
    NetSimConfig::parse(text).map_err(|e| anyhow::anyhow!(e))
}

/// A listen server's host secret ([`server::LobbyConfig::host_token`]) from the OS's
/// cryptographic random source: whoever presents it becomes the lobby host, so a guest must
/// not be able to guess it (it used to be derived from the clock and the process id).
pub fn random_host_token() -> u64 {
    u64::from_le_bytes(renet_netcode::generate_random_bytes::<8>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_tokens_are_random() {
        let tokens: std::collections::BTreeSet<u64> =
            (0..64).map(|_| random_host_token()).collect();
        assert_eq!(tokens.len(), 64);
        // Not clustered like time-derived values: the high bits vary too.
        let high: std::collections::BTreeSet<u64> = tokens.iter().map(|t| t >> 48).collect();
        assert!(high.len() > 32, "{high:?}");
    }

    #[test]
    fn resolves_addresses() {
        assert_eq!(
            resolve_address("127.0.0.1:5000").unwrap(),
            "127.0.0.1:5000".parse().unwrap()
        );
        assert_eq!(
            resolve_address("127.0.0.1").unwrap().port(),
            shared::DEFAULT_PORT
        );
        assert_eq!(resolve_address("::1").unwrap().port(), shared::DEFAULT_PORT);
        assert_eq!(resolve_address("[::1]:7").unwrap().port(), 7);
        assert!(resolve_address("localhost:9").is_ok());
        assert!(resolve_address("").is_err());
    }
}
