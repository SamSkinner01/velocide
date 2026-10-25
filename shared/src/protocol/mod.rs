//! Network protocol shared by client and server. See `docs/network.md` for the full design.
//!
//! Transport: renet over netcode (UDP, `Unsecure` authentication, fixed [`PROTOCOL_ID`]).
//! Each direction has two channels:
//!
//! | channel                | client -> server        | server -> client                 |
//! |------------------------|-------------------------|----------------------------------|
//! | [`CHANNEL_RELIABLE`]   | [`ClientMessage`]       | [`ServerMessage`] (events)       |
//! | [`CHANNEL_UNRELIABLE`] | [`ClientUnreliable`]    | [`ServerUnreliable`] (snapshots) |
//!
//! Messages are serde enums encoded with bincode (standard config: little endian, varint
//! integers). Everything received from the network is decoded with a size limit through
//! [`decode_client_message`] / [`decode_server_message`], which never panic; client data is
//! additionally validated with [`sanitize_input`], [`sanitize_name`] and [`sanitize_chat`].
//!
//! The client's protocol version and player name travel in the netcode connect token's user
//! data ([`ConnectRequest`]), so the server can reject incompatible clients right away.

mod combat;
mod snapshot;

use glam::Vec3;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub use combat::*;
pub use snapshot::*;

use crate::input::PlayerInput;
use crate::level::LevelDef;
use crate::movement::MoveState;
pub use crate::sim::PlayerId;
use crate::sim::{KillInfo, ModeEvent, ParryEvent, PlayerCombat, StyleEvent, StyleView};
use crate::tuning::Tuning;

/// netcode protocol id ("VELOCIDE"). Clients with a different id cannot even handshake.
pub const PROTOCOL_ID: u64 = 0x5645_4C4F_4349_4445;
/// Bumped on every incompatible protocol / simulation change. Checked on connect.
/// 2: combat (OwnState::combat, SnapshotMessage::fx, combat GameEvents, entity kinds).
/// 3: style & game modes (OwnState::style, GameEvent::Style / GameEvent::Mode, DOWNED).
pub const PROTOCOL_VERSION: u32 = 3;

/// Reliable-ordered channel id (both directions).
pub const CHANNEL_RELIABLE: u8 = 0;
/// Unreliable channel id (both directions).
pub const CHANNEL_UNRELIABLE: u8 = 1;

/// Size of netcode's connect-token user data.
pub const CONNECT_USER_DATA_BYTES: usize = 256;

/// Inputs per input packet: the newest plus up to `INPUT_REDUNDANCY - 1` older unacknowledged
/// ones, so a burst of up to `INPUT_REDUNDANCY - 1` lost packets loses no input.
pub const INPUT_REDUNDANCY: usize = 6;
/// The server ignores inputs beyond this many in one packet.
pub const MAX_INPUTS_PER_PACKET: usize = 16;

pub const MAX_NAME_CHARS: usize = 24;
pub const MAX_CHAT_CHARS: usize = 160;
/// Chat messages longer than this (bytes, before trimming) are dropped outright.
pub const MAX_CHAT_BYTES: usize = 1024;

/// Decode limit for anything a client sends (bytes).
pub const MAX_CLIENT_MESSAGE_BYTES: usize = 16 * 1024;
/// Decode limit for anything a server sends (bytes; level data can be large).
pub const MAX_SERVER_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// Weapon slots / alt-fire modes a client may select. Invalid selections become 0.
pub const MAX_WEAPON_SLOTS: u8 = 8;
pub const MAX_WEAPON_MODES: u8 = 4;
/// Button bits a client may set (see [`crate::input::Buttons`]).
pub const VALID_BUTTONS: u16 = 0x00FF;
/// Pitch limit (radians, 89 degrees).
pub const MAX_PITCH: f32 = 1.553_343;
/// Yaw is not wrapped (that would change the value the client predicted with) but clamped to
/// a sane range.
pub const MAX_ABS_YAW: f32 = 1.0e6;

// ---------------------------------------------------------------------------------------------
// Client -> server
// ---------------------------------------------------------------------------------------------

/// Reliable client -> server messages.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientMessage {
    Chat {
        text: String,
    },
    /// Lobby ready-up toggle.
    SetReady {
        ready: bool,
    },
    // Lobby / host commands (mode & level selection, kick...) go here.
}

/// Unreliable client -> server messages.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientUnreliable {
    Input(InputPacket),
}

/// Sent every client tick: the newest input plus a few older unacknowledged ones (redundancy
/// against packet loss), and the newest snapshot tick the client has received (the delta
/// baseline acknowledgement). `inputs` may be empty (ack-only packet).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InputPacket {
    pub ack_tick: Option<u32>,
    /// Ascending `seq`.
    pub inputs: Vec<PlayerInput>,
}

/// Packed into the netcode connect token's user data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectRequest {
    pub protocol_version: u32,
    pub name: String,
}

const CONNECT_MAGIC: &[u8; 4] = b"VLCD";

impl ConnectRequest {
    pub fn new(name: &str) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            name: sanitize_name(name),
        }
    }

    pub fn to_user_data(&self) -> [u8; CONNECT_USER_DATA_BYTES] {
        let mut data = [0u8; CONNECT_USER_DATA_BYTES];
        data[..4].copy_from_slice(CONNECT_MAGIC);
        data[4..8].copy_from_slice(&self.protocol_version.to_le_bytes());
        let name = truncate_utf8(&self.name, CONNECT_USER_DATA_BYTES - 9);
        data[8] = name.len() as u8;
        data[9..9 + name.len()].copy_from_slice(name.as_bytes());
        data
    }

    /// `None` if the data was not produced by a VELOCIDE client.
    pub fn from_user_data(data: &[u8; CONNECT_USER_DATA_BYTES]) -> Option<Self> {
        if &data[..4] != CONNECT_MAGIC {
            return None;
        }
        let protocol_version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let len = data[8] as usize;
        let name = data.get(9..9 + len)?;
        let name = String::from_utf8_lossy(name);
        Some(Self {
            protocol_version,
            name: sanitize_name(&name),
        })
    }
}

fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ---------------------------------------------------------------------------------------------
// Server -> client
// ---------------------------------------------------------------------------------------------

/// Reliable-ordered server -> client messages (events).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerMessage {
    /// First message after a successful connect.
    Welcome(Welcome),
    /// The connection is refused (version mismatch, server full...). The server disconnects
    /// shortly after sending this.
    Rejected {
        reason: String,
    },
    /// The level to play. Sent after `Welcome` and on every level change. The full level is
    /// sent (not just its name) so client prediction collides against exactly the server's
    /// geometry, and clients can join servers running levels they don't have.
    LoadLevel {
        name: String,
        level: Box<LevelDef>,
    },
    /// The authoritative tuning. Sent on connect and whenever the server reloads it; clients
    /// must predict with these values.
    Tuning(Box<Tuning>),
    /// Everyone currently in the game (sent on connect).
    PlayerList(Vec<PlayerInfo>),
    PlayerJoined(PlayerInfo),
    PlayerLeft {
        id: PlayerId,
    },
    /// Name / ready state changed.
    PlayerUpdated(PlayerInfo),
    /// `from == None` is a server message.
    Chat {
        from: Option<PlayerId>,
        text: String,
    },
    /// Gameplay events that must not be lost (kills, respawns, level complete...).
    Game(GameEvent),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Welcome {
    pub protocol_version: u32,
    pub player_id: PlayerId,
    /// Simulation ticks per second. Clients must run prediction at exactly this rate.
    pub tick_rate: u32,
    pub snapshot_rate: f32,
    pub server_tick: u32,
    pub max_players: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub id: PlayerId,
    pub name: String,
    pub ready: bool,
    /// Server-side bot (no connection).
    pub bot: bool,
}

/// Reliable gameplay events (`ServerMessage::Game`). Who receives each one and what the client
/// shows for it: `docs/combat.md` ("client presentation contract").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GameEvent {
    /// A player was (re)spawned / teleported by the server. For the local player the client
    /// should adopt `yaw` as its view angle. Everyone.
    Respawned {
        player: PlayerId,
        position: Vec3,
        yaw: f32,
    },
    /// The recipient was hurt (or a hit was blocked by i-frames / spawn protection). Victim
    /// only.
    Damaged(DamageReport),
    /// The recipient's attacks hit something (merged per target per tick). Attacker only.
    HitConfirm(HitReport),
    /// A player or an enemy died (kill feed, gibs, death cam, style). Everyone.
    Killed(KillInfo),
    /// A successful parry (hitstop + white flash for the parrier, `Sfx::Parry` for all).
    /// Everyone.
    Parried(ParryEvent),
    /// A style bonus or rank change of the recipient (style feed, rank-up banner and sound).
    /// Owner only. `docs/style_modes.md`.
    Style(StyleEvent),
    /// Game mode: checkpoint, downed / revive, team wipe, exit countdown, scoreboard, match end
    /// with results. Everyone. `docs/style_modes.md`.
    Mode(ModeEvent),
}

/// Unreliable server -> client messages.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerUnreliable {
    Snapshot(SnapshotMessage),
}

/// One snapshot, encoded per client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotMessage {
    /// Server tick the snapshot describes (state *after* simulating this tick).
    pub tick: u32,
    /// Tick of the frame `entities` is a delta against, `None` = full snapshot.
    pub baseline_tick: Option<u32>,
    /// The recipient's last processed input sequence number.
    pub last_input_seq: u32,
    /// The recipient's own authoritative state after `last_input_seq` (for reconciliation).
    /// `None` while the recipient has no body (spectating / dead in the future).
    pub own: Option<OwnState>,
    /// Delta-encoded entity block, see [`encode_delta`].
    pub entities: Vec<u8>,
    /// Cosmetic events since the previous snapshot to this client (unreliable; see
    /// [`FxEvent`]).
    pub fx: Vec<FxEvent>,
}

/// Owner-only state, sent unquantised so reconciliation is exact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OwnState {
    pub movement: MoveState,
    /// Health, hard damage, death/respawn timer, spawn protection, weapon cooldowns and
    /// charges, parry window, prisms, stickies — after `last_input_seq` (the weapon state is
    /// predictable: compare it like the movement state).
    pub combat: PlayerCombat,
    /// Style meter, rank and freshness (HUD).
    pub style: StyleView,
}

// ---------------------------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------------------------

/// Error decoding a message.
#[derive(Debug)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "malformed message: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

/// Encodes any protocol message.
pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    match bincode::serde::encode_to_vec(msg, bincode::config::standard()) {
        Ok(bytes) => bytes,
        Err(err) => {
            // Only possible for types serde/bincode can't represent; a programming error.
            log::error!("failed to encode message: {err}");
            Vec::new()
        }
    }
}

fn decode_with<T: DeserializeOwned, C: bincode::config::Config>(
    bytes: &[u8],
    config: C,
) -> Result<T, DecodeError> {
    let (value, used) = bincode::serde::decode_from_slice::<T, C>(bytes, config)
        .map_err(|e| DecodeError(e.to_string()))?;
    if used != bytes.len() {
        return Err(DecodeError(format!(
            "{} trailing bytes",
            bytes.len() - used
        )));
    }
    Ok(value)
}

/// Decodes untrusted client data (size-limited, never panics).
pub fn decode_client_message<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, DecodeError> {
    if bytes.len() > MAX_CLIENT_MESSAGE_BYTES {
        return Err(DecodeError("message too large".into()));
    }
    decode_with(
        bytes,
        bincode::config::standard().with_limit::<MAX_CLIENT_MESSAGE_BYTES>(),
    )
}

/// Decodes server data (size-limited, never panics).
pub fn decode_server_message<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, DecodeError> {
    if bytes.len() > MAX_SERVER_MESSAGE_BYTES {
        return Err(DecodeError("message too large".into()));
    }
    decode_with(
        bytes,
        bincode::config::standard().with_limit::<MAX_SERVER_MESSAGE_BYTES>(),
    )
}

// ---------------------------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------------------------

/// Clamps an input to valid ranges. Idempotent. The client applies it before predicting (so
/// prediction uses exactly what the server will use) and the server applies it to everything
/// it receives.
pub fn sanitize_input(input: &mut PlayerInput) {
    input.forward = input.forward.max(-127);
    input.right = input.right.max(-127);
    input.yaw = if input.yaw.is_finite() {
        input.yaw.clamp(-MAX_ABS_YAW, MAX_ABS_YAW)
    } else {
        0.0
    };
    input.pitch = if input.pitch.is_finite() {
        input.pitch.clamp(-MAX_PITCH, MAX_PITCH)
    } else {
        0.0
    };
    input.buttons.0 &= VALID_BUTTONS;
    if input.weapon >= MAX_WEAPON_SLOTS {
        input.weapon = 0;
    }
    if input.weapon_mode >= MAX_WEAPON_MODES {
        input.weapon_mode = 0;
    }
}

/// Printable, trimmed, at most [`MAX_NAME_CHARS`] characters; never empty.
pub fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS * 4)
        .collect();
    let name: String = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    if name.is_empty() {
        "Shrike".to_string()
    } else {
        name
    }
}

/// `None` if the message should be dropped (empty or absurdly long).
pub fn sanitize_chat(raw: &str) -> Option<String> {
    if raw.len() > MAX_CHAT_BYTES {
        return None;
    }
    let text: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_CHAT_CHARS)
        .collect();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Buttons;

    #[test]
    fn messages_round_trip() {
        let msgs = vec![
            ServerMessage::Welcome(Welcome {
                protocol_version: PROTOCOL_VERSION,
                player_id: PlayerId(3),
                tick_rate: 60,
                snapshot_rate: 30.0,
                server_tick: 1234,
                max_players: 8,
            }),
            ServerMessage::Tuning(Box::default()),
            ServerMessage::LoadLevel {
                name: "proving_ground".into(),
                level: Box::new(crate::sim::builtin_level("proving_ground").unwrap()),
            },
            ServerMessage::Chat {
                from: None,
                text: "hi".into(),
            },
        ];
        for msg in msgs {
            let bytes = encode(&msg);
            let back: ServerMessage = decode_server_message(&bytes).unwrap();
            assert_eq!(back, msg);
        }
        let input = ClientUnreliable::Input(InputPacket {
            ack_tick: Some(77),
            inputs: vec![PlayerInput {
                seq: 9,
                forward: 127,
                yaw: 1.2345678,
                pitch: -0.1,
                buttons: Buttons::JUMP,
                ..Default::default()
            }],
        });
        let back: ClientUnreliable = decode_client_message(&encode(&input)).unwrap();
        assert_eq!(back, input);
    }

    #[test]
    fn tuning_survives_bincode_exactly() {
        let mut t = Tuning::default();
        t.movement.gravity = 37.123_45;
        t.net.interp_delay_ms = 0.1 + 0.2;
        let back: ServerMessage =
            decode_server_message(&encode(&ServerMessage::Tuning(Box::new(t.clone())))).unwrap();
        assert_eq!(back, ServerMessage::Tuning(Box::new(t)));
    }

    #[test]
    fn garbage_is_rejected_not_panicking() {
        let mut seed: u32 = 0xdead_beef;
        for len in 0..300 {
            let data: Vec<u8> = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    seed as u8
                })
                .collect();
            let _ = decode_client_message::<ClientUnreliable>(&data);
            let _ = decode_client_message::<ClientMessage>(&data);
            let _ = decode_server_message::<ServerMessage>(&data);
            let _ = decode_server_message::<ServerUnreliable>(&data);
        }
        // A vector claiming a huge length must not allocate it.
        let mut evil = vec![0u8]; // variant Input
        evil.push(0); // ack None
        evil.extend_from_slice(&[0xFD, 0xFF, 0xFF, 0xFF, 0x7F]); // varint u32 len
        assert!(decode_client_message::<ClientUnreliable>(&evil).is_err());
        assert!(
            decode_client_message::<ClientMessage>(&vec![0u8; MAX_CLIENT_MESSAGE_BYTES + 1])
                .is_err()
        );
    }

    #[test]
    fn sanitize_input_is_idempotent_and_clamps() {
        let mut input = PlayerInput {
            forward: i8::MIN,
            right: 127,
            yaw: f32::NAN,
            pitch: 10.0,
            buttons: Buttons(0xFFFF),
            weapon: 200,
            weapon_mode: 9,
            ..Default::default()
        };
        sanitize_input(&mut input);
        assert_eq!(input.forward, -127);
        assert_eq!(input.yaw, 0.0);
        assert_eq!(input.pitch, MAX_PITCH);
        assert_eq!(input.buttons.0, VALID_BUTTONS);
        assert_eq!((input.weapon, input.weapon_mode), (0, 0));
        let again = {
            let mut i = input;
            sanitize_input(&mut i);
            i
        };
        assert_eq!(again, input);
    }

    #[test]
    fn names_and_chat() {
        assert_eq!(sanitize_name("  Neo\n  Shrike\t "), "Neo Shrike");
        assert_eq!(sanitize_name("\u{7}\u{7}"), "Shrike");
        assert_eq!(
            sanitize_name(&"x".repeat(100)).chars().count(),
            MAX_NAME_CHARS
        );
        assert_eq!(sanitize_chat("  gg  "), Some("gg".into()));
        assert_eq!(sanitize_chat("   "), None);
        assert_eq!(sanitize_chat(&"a".repeat(MAX_CHAT_BYTES + 1)), None);
        assert_eq!(
            sanitize_chat(&"b".repeat(500)).map(|s| s.len()),
            Some(MAX_CHAT_CHARS)
        );
    }

    #[test]
    fn connect_request_round_trips() {
        let req = ConnectRequest::new("Ünïcode Shrike");
        let back = ConnectRequest::from_user_data(&req.to_user_data()).unwrap();
        assert_eq!(back, req);
        assert!(ConnectRequest::from_user_data(&[0u8; CONNECT_USER_DATA_BYTES]).is_none());
        let mut evil = req.to_user_data();
        evil[8] = 255; // length beyond the buffer
        assert!(ConnectRequest::from_user_data(&evil).is_none());
    }
}
