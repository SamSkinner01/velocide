//! The server browser: fetches the public listings of our protocol version from the service.
//! Owns its own UDP socket (non-blocking); call [`Browser::poll`] every frame.

use std::collections::BTreeMap;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::proto::{self, ErrorCode, ListEntry, Message, encode};
use crate::wire::canonical;
use crate::{Pending, Resolver};

const RETRY: Duration = Duration::from_millis(400);
const TIMEOUT: Duration = Duration::from_secs(6);
/// Pages fetched at most (14 games each).
pub const MAX_PAGES: u16 = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowseState {
    Loading,
    Done(Vec<ListEntry>),
    Failed(String),
}

pub struct Browser {
    service_name: String,
    protocol: u32,
    socket: Option<UdpSocket>,
    resolver: Resolver,
    service: Option<SocketAddr>,
    epoch: Instant,
    started: Duration,
    cookie: Option<[u8; 16]>,
    hello: Option<Pending>,
    requests: BTreeMap<u16, Pending>,
    pages: Option<u16>,
    entries: BTreeMap<u16, Vec<ListEntry>>,
    state: BrowseState,
}

impl Browser {
    pub fn start(service: &str, protocol: u32) -> Browser {
        let socket = UdpSocket::bind(("0.0.0.0", 0)).and_then(|s| {
            s.set_nonblocking(true)?;
            Ok(s)
        });
        let (socket, state) = match socket {
            Ok(s) => (Some(s), BrowseState::Loading),
            Err(e) => (None, BrowseState::Failed(format!("no UDP socket: {e}"))),
        };
        Browser {
            service_name: service.to_string(),
            protocol,
            socket,
            resolver: Resolver::start(service),
            service: None,
            epoch: Instant::now(),
            started: Duration::ZERO,
            cookie: None,
            hello: None,
            requests: BTreeMap::new(),
            pages: None,
            entries: BTreeMap::new(),
            state,
        }
    }

    /// Fetches the list again.
    pub fn refresh(&mut self) {
        if self.socket.is_none() {
            return;
        }
        self.started = self.epoch.elapsed();
        self.requests.clear();
        self.pages = None;
        self.entries.clear();
        self.state = BrowseState::Loading;
    }

    pub fn state(&self) -> &BrowseState {
        &self.state
    }

    pub fn poll(&mut self) -> &BrowseState {
        if self.state == BrowseState::Loading
            && let Err(e) = self.step()
        {
            self.state = BrowseState::Failed(e);
        }
        &self.state
    }

    fn step(&mut self) -> Result<(), String> {
        let now = self.epoch.elapsed();
        if self.service.is_none() {
            match self.resolver.poll() {
                Some(Ok(addr)) => self.service = Some(canonical(*addr)),
                Some(Err(e)) => return Err(e.clone()),
                None => return Ok(()),
            }
        }
        let (Some(service), Some(socket)) = (self.service, &self.socket) else {
            return Ok(());
        };
        let mut buf = [0u8; proto::MAX_DATAGRAM];
        let mut replies = Vec::new();
        let mut received = Ok(());
        for _ in 0..64 {
            match socket.recv_from(&mut buf) {
                Ok((len, from)) if canonical(from) == service => {
                    replies.push(buf.get(..len).unwrap_or_default().to_vec());
                }
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => continue,
                Err(e) => {
                    received = Err(format!("UDP receive: {e}"));
                    break;
                }
            }
        }
        received?;
        for data in &replies {
            self.reply(data)?;
        }

        if now.saturating_sub(self.started) > TIMEOUT {
            return Err(format!(
                "no answer from the online service ({})",
                self.service_name
            ));
        }
        let mut send = Vec::new();
        match self.cookie {
            None => {
                let hello = self.hello.get_or_insert_with(Pending::new);
                if hello.send_now(now, RETRY) {
                    send.push(encode(&Message::Hello { nonce: hello.nonce }));
                }
            }
            Some(cookie) => {
                let wanted = self.pages.unwrap_or(1).min(MAX_PAGES);
                for page in 0..wanted {
                    if self.entries.contains_key(&page) {
                        continue;
                    }
                    let pending = self.requests.entry(page).or_insert_with(Pending::new);
                    if pending.send_now(now, RETRY) {
                        send.push(encode(&Message::List {
                            cookie,
                            nonce: pending.nonce,
                            protocol: self.protocol,
                            page,
                        }));
                    }
                }
                if self.pages.is_some() && self.entries.len() >= wanted as usize {
                    let mut all: Vec<ListEntry> =
                        self.entries.values().flatten().cloned().collect();
                    // Pages fetched at different times may overlap or skip a game.
                    all.sort_by_key(|e| e.code);
                    all.dedup_by(|a, b| a.code == b.code);
                    all.sort_by(|a, b| b.players.cmp(&a.players).then_with(|| a.name.cmp(&b.name)));
                    self.state = BrowseState::Done(all);
                }
            }
        }
        if let Some(socket) = &self.socket {
            for data in send {
                let _ = socket.send_to(&data, service);
            }
        }
        Ok(())
    }

    fn reply(&mut self, data: &[u8]) -> Result<(), String> {
        let Ok(msg) = proto::decode(data) else {
            return Ok(());
        };
        let nonce = msg.nonce();
        match msg {
            Message::Cookie { cookie, .. } if self.hello.is_some_and(|h| h.nonce == nonce) => {
                self.hello = None;
                self.cookie = Some(cookie);
            }
            Message::ListPage {
                page,
                pages,
                entries,
                ..
            } => {
                if self.requests.get(&page).is_some_and(|p| p.nonce == nonce) {
                    self.requests.remove(&page);
                    self.pages = Some(pages.max(1));
                    self.entries.insert(page, entries);
                }
            }
            Message::Error { code, .. } => {
                let ours = nonce == 0
                    || self.hello.is_some_and(|h| h.nonce == nonce)
                    || self.requests.values().any(|p| p.nonce == nonce);
                if !ours {
                    return Ok(());
                }
                match code {
                    ErrorCode::BadCookie => {
                        self.cookie = None;
                        self.requests.clear();
                    }
                    ErrorCode::UnsupportedVersion => {
                        return Err(
                            "the online service needs a newer version of the game: update it"
                                .into(),
                        );
                    }
                    ErrorCode::RateLimited => {
                        return Err("the online service is rate limiting us; try again soon".into());
                    }
                    _ => return Err("the online service refused the request".into()),
                }
            }
            _ => {}
        }
        Ok(())
    }
}
