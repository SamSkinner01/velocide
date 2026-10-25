//! Session layer: how a game is started or joined.
//!
//! | session                     | server                                   | client connects to   |
//! |-----------------------------|------------------------------------------|----------------------|
//! | [`Session::single_player`]  | in-process thread on `127.0.0.1:0`       | that loopback port   |
//! | [`Session::host`]           | in-process thread on `0.0.0.0:<port>`    | loopback, same port  |
//! | [`Session::join`]           | remote                                   | the given address    |
//!
//! Single-player is just a private listen server, so there is exactly one code path.
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
    SinglePlayer { level: String },
    Host { server: ServerConfig },
    Join { target: JoinTarget },
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
            },
            ClientConfig::new(name),
        )
    }

    /// Hosts a listen server on `port` (all interfaces) and connects to it.
    pub fn host(port: u16, level: &str, max_players: usize, name: &str) -> anyhow::Result<Session> {
        Self::start(
            SessionSpec::Host {
                server: ServerConfig::host(port, level, max_players),
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
            SessionSpec::SinglePlayer { level } => {
                let server = ServerHandle::spawn(ServerConfig::single_player(&level))?;
                let net = NetClient::connect(server.connect_addr(), client)?;
                Ok(Session {
                    kind: SessionKind::SinglePlayer,
                    server: Some(server),
                    client: net,
                })
            }
            SessionSpec::Host { server } => {
                let server = ServerHandle::spawn(server)?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
