//! RTMP ingest server session state machine — `connect` → `createStream` →
//! `publish` (Adobe RTMP 1.0 §7.2, `NetConnection`/`NetStream` commands).
//!
//! See [`docs/rtmp.md`](../docs/rtmp.md) §2 (Handshake), §4 (Protocol Control
//! Messages), §5.3 (User Control Messages), §7 (RTMP Message Types incl.
//! Command Message), and [`transmux/docs/codec/flv.md`](../../transmux/docs/codec/flv.md)
//! (FLV header/tag layout, Adobe FLV v10.1 Annex E) for the wire layouts this
//! module ties together.
//!
//! [`ServerSession`] is the sans-IO **publish ingest** engine: feed it
//! inbound bytes via [`ServerSession::handle_data`], get back outbound bytes
//! to write plus a list of typed [`ServerEvent`]s. It drives, in order:
//!
//! 1. the [`crate::handshake::Handshake`] sub-FSM (C0/C1/C2 → S0/S1/S2),
//! 2. the [`crate::chunk::ChunkAssembler`]/[`crate::chunk::ChunkWriter`]
//!    chunk-stream (de)assembly,
//! 3. [`crate::message::ProtocolControl`]/[`crate::message::UserControl`]
//!    interpretation and replies,
//! 4. [`crate::amf0::Command`] routing for the `connect`/`createStream`/
//!    `publish` command sequence, and
//! 5. FLV tag emission for Audio(8)/Video(9)/Data-AMF0(18) messages received
//!    while publishing.
//!
//! # Session state
//!
//! Internally tracked as `Init → Connected(app) → Publishing(stream_key) →
//! Closed`. The handshake phase itself is not duplicated in this enum — it
//! is tracked by `self.handshake.is_done()` (querying
//! [`crate::handshake::Handshake`] directly), so there is exactly one source
//! of truth for "has the handshake finished".
//!
//! # Ack accounting
//!
//! §5.4.3's Acknowledgement sequence number is a plain **modular `u32`**
//! (truncating the running total byte count) — the spec is silent on
//! wraparound behaviour for this field, so this is a documented
//! implementation choice, not a spec requirement.
//!
//! Two further implementation choices, not spec requirements:
//!
//! - At most **one** Acknowledgement is emitted per
//!   [`handle_data`](ServerSession::handle_data) call, even if the input
//!   buffer crossed `window_ack_size` multiple times over (e.g. a single
//!   call carrying several times the window in bytes). The threshold check
//!   runs once, after all messages in that call have been dispatched, not
//!   once per `window_ack_size`-sized increment.
//! - Handshake bytes are excluded from the Ack byte count: the running
//!   total only accumulates post-handshake (chunk-stream) bytes — bytes
//!   consumed while still inside the C0/C1/C2 ↔ S0/S1/S2 handshake exchange
//!   never reach the counter.
//!
//! # Reply csid convention
//!
//! Protocol control and User Control messages MUST/SHOULD use chunk stream
//! id 2 ([`crate::message::CONTROL_CHUNK_STREAM_ID`]) — enforced already by
//! [`crate::message::ProtocolControl::to_message`] and
//! [`crate::message::UserControl::to_message`]. This module's own outbound
//! AMF0 command replies (`_result`/`onStatus`) use `COMMAND_CHUNK_STREAM_ID`
//! (3) — a real-world convention (distinct from the reserved control csid),
//! not a spec-mandated value: §5.3 leaves csid choice to the sender for
//! anything other than protocol control/user control traffic.

use broadcast_common::Parse;

use crate::RtmpError;
use crate::amf0::{Amf0Value, Command};
use crate::chunk::{ChunkAssembler, ChunkWriter, Message};
use crate::handshake::Handshake;
use crate::message::{LimitType, ProtocolControl, UserControl, msg_type};

type Result<T> = core::result::Result<T, RtmpError>;

// ── Named constants (no magic numbers) ──────────────────────────────────

/// Default outbound chunk size we advertise via Set Chunk Size on `connect`
/// (§5.4.1). Larger than the §5.3 wire default (128) to reduce chunk-header
/// overhead for real audio/video payloads.
pub const DEFAULT_CHUNK_SIZE: u32 = 4096;
/// Default Window Acknowledgement Size we advertise on `connect` (§5.4.4),
/// and the default threshold for our own inbound Ack accounting (§5.4.3).
pub const DEFAULT_WINDOW_ACK_SIZE: u32 = 2_500_000;
/// Default Set Peer Bandwidth value we advertise on `connect` (§5.4.5).
pub const DEFAULT_PEER_BANDWIDTH: u32 = 2_500_000;

/// The first message stream id [`ServerSession`] allocates on `createStream`
/// (§7.2.2). Message stream id 0 is reserved for the `NetConnection`
/// (control) channel, so allocation starts at 1.
const FIRST_STREAM_ID: u32 = 1;

/// Chunk stream id this session uses for its own outbound AMF0 command
/// replies (`_result`/`onStatus`) — see the module doc's "Reply csid
/// convention" section.
const COMMAND_CHUNK_STREAM_ID: u32 = 3;

/// `fmsVer` value advertised in the `connect` `_result` Properties object
/// (§7.2.1). Not spec-mandated — a conventional placeholder value (the
/// pattern used by reference server implementations), since real clients
/// only branch on `NetConnection.Connect.Success`/`level`/`code`, not this
/// string.
const FMS_VERSION: &str = "FMS/3,0,1,123";
/// `capabilities` value advertised in the `connect` `_result` Properties
/// object (§7.2.1). Not spec-mandated (see [`FMS_VERSION`]).
const CAPABILITIES: f64 = 31.0;

// ── FLV mapping consts (transmux/docs/codec/flv.md, Annex E) ────────────

/// FLV file header `Signature` field (Annex E.2): `"FLV"`.
const FLV_SIGNATURE: [u8; 3] = *b"FLV";
/// FLV file header `Version` field (Annex E.2).
const FLV_VERSION: u8 = 1;
/// FLV file header `TypeFlags` field (Annex E.2): bit 0 (audio present) |
/// bit 2 (video present) — this ingest engine always advertises both, since
/// it does not know ahead of time which media types a publisher will send.
const FLV_TYPE_FLAGS_AUDIO_VIDEO: u8 = 0b0000_0101;
/// FLV file header `DataOffset` field (Annex E.2): header size in bytes.
const FLV_HEADER_SIZE: u32 = 9;
/// Byte width of one FLV tag's fixed header (Annex E.4.1): `TagType`(1) +
/// `DataSize`(3) + `Timestamp`(3) + `TimestampExtended`(1) + `StreamID`(3).
const FLV_TAG_HEADER_LEN: usize = 11;
/// Byte width of the `PreviousTagSize` field that follows every FLV tag
/// (Annex E.4.1), and the file header's `PreviousTagSize0` (Annex E.2).
const FLV_PREV_TAG_SIZE_LEN: usize = 4;
/// Largest value the FLV tag's 24-bit `DataSize` field can encode.
const FLV_MAX_DATA_SIZE: usize = 0x00FF_FFFF;

/// Configuration for a [`ServerSession`].
///
/// `#[non_exhaustive]`: fields may grow (e.g. a future `app` gate alongside
/// `expected_stream_key`). Construct via [`ServerConfig::default`] plus the
/// `with_*` builder methods rather than a struct literal.
#[non_exhaustive]
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ServerConfig {
    /// Outbound chunk size advertised (and adopted) on `connect` (§5.4.1).
    pub chunk_size: u32,
    /// Window Acknowledgement Size advertised on `connect` (§5.4.4); also
    /// the initial threshold for this session's own inbound Ack accounting
    /// (§5.4.3), until a peer `WindowAckSize`/`SetPeerBandwidth` updates it.
    pub window_ack_size: u32,
    /// Set Peer Bandwidth value advertised on `connect` (§5.4.5).
    pub peer_bandwidth: u32,
    /// If set, `publish`'s stream key (publishing name) must match this
    /// value exactly or the publish is rejected: `onStatus`
    /// `NetStream.Publish.BadName`, and no `Publish`/`Media` events are
    /// emitted for that connection.
    pub expected_stream_key: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            window_ack_size: DEFAULT_WINDOW_ACK_SIZE,
            peer_bandwidth: DEFAULT_PEER_BANDWIDTH,
            expected_stream_key: None,
        }
    }
}

impl ServerConfig {
    /// Set [`ServerConfig::expected_stream_key`]. `#[non_exhaustive]` forbids
    /// struct-literal construction of this type from outside the crate, so
    /// this (plus the other `with_*` builders below) is how a caller
    /// customises a field starting from [`ServerConfig::default`].
    #[must_use]
    pub fn with_expected_stream_key(mut self, expected_stream_key: Option<String>) -> Self {
        self.expected_stream_key = expected_stream_key;
        self
    }

    /// Set [`ServerConfig::chunk_size`].
    #[must_use]
    pub fn with_chunk_size(mut self, chunk_size: u32) -> Self {
        self.chunk_size = chunk_size;
        self
    }

    /// Set [`ServerConfig::window_ack_size`].
    #[must_use]
    pub fn with_window_ack_size(mut self, window_ack_size: u32) -> Self {
        self.window_ack_size = window_ack_size;
        self
    }

    /// Set [`ServerConfig::peer_bandwidth`].
    #[must_use]
    pub fn with_peer_bandwidth(mut self, peer_bandwidth: u32) -> Self {
        self.peer_bandwidth = peer_bandwidth;
        self
    }
}

/// Typed events [`ServerSession::handle_data`] surfaces to the caller.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ServerEvent {
    /// `connect` completed: the publisher's requested `app` name (§7.2.1).
    Connected {
        /// The `app` property of the `connect` command object.
        app: String,
    },
    /// `publish` was accepted (stream key matched, or no key was
    /// configured): the session is now `Publishing`.
    Publish {
        /// The `app` name captured at `connect`.
        app: String,
        /// The publishing name (stream key) passed to `publish` (§7.2.2.6).
        stream_key: String,
        /// The message stream id `publish` was invoked on (allocated by the
        /// preceding `createStream`).
        stream_id: u32,
    },
    /// One FLV tag run is ready: the payload of a single Audio(8)/Video(9)/
    /// Data-AMF0(18) message, converted to an FLV tag (+ `PreviousTagSize`).
    /// The very first `Media` event of a session is prefixed with the FLV
    /// file header, so concatenating every `Media.flv` in arrival order
    /// yields a valid FLV byte stream feedable to `transmux::FlvDemux`.
    Media {
        /// FLV bytes for this tag (file header prefix on the first event
        /// only, tag header + payload + `PreviousTagSize` every time).
        flv: Vec<u8>,
    },
    /// The publisher ended the stream (`deleteStream`/`FCUnpublish`).
    Eof,
}

/// Session state (`connect`/`publish` progress only — the handshake phase
/// is tracked separately by `self.handshake.is_done()`, see the module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Handshake done (or not yet started); `connect` not yet received.
    Init,
    /// `connect` succeeded; `app` captured. `createStream`/`publish` may
    /// now proceed.
    Connected,
    /// `publish` succeeded; Audio/Video/Data-AMF0 messages now produce
    /// [`ServerEvent::Media`].
    Publishing,
    /// `deleteStream`/`FCUnpublish` was received; the session is done.
    Closed,
}

/// The sans-IO RTMP **publish ingest** server session (see the module doc).
///
/// No sockets or clocks live here: drive it entirely by feeding inbound
/// bytes to [`handle_data`](Self::handle_data).
#[derive(Debug)]
pub struct ServerSession {
    config: ServerConfig,
    handshake: Handshake,
    /// Raw bytes accumulated across calls while the handshake is still in
    /// progress (the handshake sub-FSM does not buffer partial input
    /// itself — see [`crate::handshake::Handshake::read`]).
    handshake_buf: Vec<u8>,
    assembler: ChunkAssembler,
    writer: ChunkWriter,
    state: State,
    app: Option<String>,
    next_stream_id: u32,
    /// The message stream id allocated by the most recent successful
    /// `createStream` (§7.2.2), or `None` if none has succeeded yet.
    /// `publish` requires this to be `Some` (in addition to `state ==
    /// Connected`) — a stream must actually have been created first.
    created_stream_id: Option<u32>,
    /// Threshold (in bytes received) at which an Acknowledgement is due
    /// (§5.4.3/§5.4.4). Starts at `config.window_ack_size`; updated if the
    /// peer sends its own `WindowAckSize`/`SetPeerBandwidth`.
    ack_threshold: u32,
    /// Total bytes received on the chunk stream (post-handshake) so far.
    bytes_received: u64,
    /// `bytes_received` value as of the last Acknowledgement sent.
    bytes_acked: u64,
    /// Whether the FLV file header has already been prefixed to a `Media`
    /// event (only the very first one gets it).
    flv_header_sent: bool,
}

impl ServerSession {
    /// A new session with the given configuration.
    #[must_use]
    pub fn new(config: ServerConfig) -> Self {
        let ack_threshold = config.window_ack_size;
        Self {
            config,
            handshake: Handshake::new(),
            handshake_buf: Vec::new(),
            assembler: ChunkAssembler::new(),
            writer: ChunkWriter::new(),
            state: State::Init,
            app: None,
            next_stream_id: FIRST_STREAM_ID,
            created_stream_id: None,
            ack_threshold,
            bytes_received: 0,
            bytes_acked: 0,
            flv_header_sent: false,
        }
    }

    /// A new session using [`ServerConfig::default`].
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(ServerConfig::default())
    }

    /// Feed inbound bytes to the session. Returns `(bytes to write back,
    /// events produced)`.
    ///
    /// Buffers partial handshake input and partial chunks across calls —
    /// callers just need to forward whatever bytes arrive off the wire, in
    /// order, one call per read.
    ///
    /// # Errors
    /// [`RtmpError`] on malformed handshake/chunk/AMF0 input, or a command
    /// used out of order (e.g. `publish` before `connect`). Never panics on
    /// truncated or garbage input. On `Err` the session should be considered
    /// unrecoverable/torn down: internal state may have partially advanced
    /// past the offending input, so the caller must not keep driving it.
    pub fn handle_data(&mut self, input: &[u8]) -> Result<(Vec<u8>, Vec<ServerEvent>)> {
        let mut out = Vec::new();
        let mut events = Vec::new();

        let chunk_input = match self.drive_handshake(input, &mut out)? {
            Some(bytes) => bytes,
            None => return Ok((out, events)),
        };

        self.bytes_received = self.bytes_received.saturating_add(chunk_input.len() as u64);

        // Dispatch each message as soon as it is parsed (rather than
        // collecting a full batch from one `push` first): a Set Chunk Size
        // protocol control message (§5.4.1) must take effect for the very
        // next chunk that follows it, even when both arrive in the same
        // `handle_data` call — a real ffmpeg publisher does exactly this
        // (its own `connect`-time SetChunkSize is immediately followed, in
        // the same TCP segment, by chunks already framed at the new size).
        // Collecting the whole batch under one `ChunkAssembler::push` call
        // would parse those later chunks with the *old* chunk size and
        // misparse them.
        self.assembler.feed(&chunk_input);
        while let Some(msg) = self.assembler.next_message()? {
            self.dispatch_message(&msg, &mut out, &mut events)?;
        }

        self.maybe_ack(&mut out);

        Ok((out, events))
    }

    /// Drive the handshake sub-FSM with `input`, appending any handshake
    /// reply bytes to `out`. Returns `Some(leftover_bytes)` — the
    /// post-handshake bytes now ready for the chunk assembler — once the
    /// handshake has completed; `None` if it is still in progress (the
    /// caller should return early and wait for more input).
    fn drive_handshake(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<Option<Vec<u8>>> {
        if self.handshake.is_done() {
            return Ok(Some(input.to_vec()));
        }

        self.handshake_buf.extend_from_slice(input);
        loop {
            match self.handshake.read(&self.handshake_buf) {
                Ok((reply, consumed, done)) => {
                    out.extend_from_slice(&reply);
                    self.handshake_buf.drain(..consumed);
                    if done {
                        break;
                    }
                }
                Err(RtmpError::BufferTooShort { .. }) => break,
                Err(e) => return Err(e),
            }
        }

        if self.handshake.is_done() {
            Ok(Some(core::mem::take(&mut self.handshake_buf)))
        } else {
            Ok(None)
        }
    }

    /// Send an Acknowledgement (§5.4.3) if `bytes_received` has crossed
    /// `ack_threshold` since the last one. Sequence number is a plain
    /// modular `u32` truncation of the running total (see the module doc).
    fn maybe_ack(&mut self, out: &mut Vec<u8>) {
        let threshold = u64::from(self.ack_threshold.max(1));
        if self.bytes_received.saturating_sub(self.bytes_acked) >= threshold {
            self.bytes_acked = self.bytes_received;
            let seq = self.bytes_received as u32;
            let ack_msg = ProtocolControl::Acknowledgement(seq).to_message();
            out.extend_from_slice(&self.writer.write(&ack_msg));
        }
    }

    /// Dispatch one reassembled [`Message`] by `message_type_id`.
    fn dispatch_message(
        &mut self,
        msg: &Message,
        out: &mut Vec<u8>,
        events: &mut Vec<ServerEvent>,
    ) -> Result<()> {
        if let Some(pc) = ProtocolControl::from_message(msg)? {
            self.handle_protocol_control(pc);
            return Ok(());
        }

        match msg.message_type_id {
            msg_type::USER_CONTROL => {
                // Publish-only ingest: no inbound user control event needs
                // a reply from us. Malformed/unrecognised event types are
                // tolerated (accepted, not fatal) rather than aborting the
                // whole session over a benign/unknown control event.
                let _ = UserControl::parse(&msg.payload);
                Ok(())
            }
            msg_type::COMMAND_AMF0 => {
                let command = Command::parse(&msg.payload)?;
                self.handle_command(&command, msg, out, events)
            }
            msg_type::AUDIO | msg_type::VIDEO | msg_type::DATA_AMF0 => {
                self.emit_media_if_publishing(msg.message_type_id, msg, events)
            }
            // Command-AMF3(17), Data-AMF3(15), Shared Object(19/16),
            // Aggregate(22), and anything unrecognised: out of scope for
            // this ingest engine (see the crate's non-goals) — accepted
            // and ignored rather than treated as an error.
            _ => Ok(()),
        }
    }

    /// Apply a protocol control message's effect (§5.4). Never errors: a
    /// malformed payload is rejected earlier, in
    /// [`ProtocolControl::from_message`].
    fn handle_protocol_control(&mut self, pc: ProtocolControl) {
        match pc {
            ProtocolControl::SetChunkSize(n) => self.assembler.set_chunk_size(n),
            ProtocolControl::WindowAckSize(w) => self.ack_threshold = w,
            ProtocolControl::SetPeerBandwidth {
                ack_window_size, ..
            } => self.ack_threshold = ack_window_size,
            ProtocolControl::Abort { .. } | ProtocolControl::Acknowledgement(_) => {}
        }
    }

    /// Route a Command Message (§7.1.1) by `command.name` (§7.2).
    fn handle_command(
        &mut self,
        command: &Command,
        msg: &Message,
        out: &mut Vec<u8>,
        events: &mut Vec<ServerEvent>,
    ) -> Result<()> {
        match command.name.as_str() {
            "connect" => self.handle_connect(command, msg, out, events),
            "releaseStream" | "FCPublish" => {
                self.reply_result(command, msg, vec![Amf0Value::Undefined], out);
                Ok(())
            }
            "createStream" => self.handle_create_stream(command, msg, out),
            "publish" => self.handle_publish(command, msg, out, events),
            "deleteStream" | "FCUnpublish" => {
                self.state = State::Closed;
                events.push(ServerEvent::Eof);
                Ok(())
            }
            // Unrecognised command name: ignore rather than error, per the
            // "never error on OBS/extra commands" design goal.
            _ => Ok(()),
        }
    }

    /// `connect` (§7.2.1, `NetConnection`): capture `app`, reply
    /// WindowAckSize + SetPeerBandwidth + SetChunkSize + `_result`, emit
    /// [`ServerEvent::Connected`].
    ///
    /// # Errors
    /// [`RtmpError::Malformed`] if the command has no command-object
    /// argument (arg0), or arg0 is not an AMF0 Object, or the object has no
    /// `app` property of type String — a present-but-empty `app` string
    /// (`""`) is tolerated (it is a valid, if unhelpful, application name).
    /// [`RtmpError::UnexpectedState`] if the session has already reached
    /// [`State::Closed`].
    fn handle_connect(
        &mut self,
        command: &Command,
        msg: &Message,
        out: &mut Vec<u8>,
        events: &mut Vec<ServerEvent>,
    ) -> Result<()> {
        if self.state == State::Closed {
            return Err(RtmpError::UnexpectedState {
                what: "connect received after the session was closed",
            });
        }

        let Some(Amf0Value::Object(pairs)) = command.arguments.first() else {
            return Err(RtmpError::Malformed {
                what: "connect command object / app",
            });
        };
        let Some(app) = pairs.iter().find_map(|(k, v)| {
            if k == "app" {
                match v {
                    Amf0Value::String(s) => Some(s.clone()),
                    _ => None,
                }
            } else {
                None
            }
        }) else {
            return Err(RtmpError::Malformed {
                what: "connect command object / app",
            });
        };

        self.app = Some(app.clone());
        self.state = State::Connected;

        let window_ack = ProtocolControl::WindowAckSize(self.config.window_ack_size).to_message();
        out.extend_from_slice(&self.writer.write(&window_ack));

        let peer_bandwidth = ProtocolControl::SetPeerBandwidth {
            ack_window_size: self.config.peer_bandwidth,
            limit_type: LimitType::Dynamic,
        }
        .to_message();
        out.extend_from_slice(&self.writer.write(&peer_bandwidth));

        let set_chunk_size = ProtocolControl::SetChunkSize(self.config.chunk_size).to_message();
        out.extend_from_slice(&self.writer.write(&set_chunk_size));
        self.writer.set_chunk_size(self.config.chunk_size);

        let result = Command {
            name: "_result".to_string(),
            transaction_id: command.transaction_id,
            arguments: vec![
                Amf0Value::Object(vec![
                    (
                        "fmsVer".to_string(),
                        Amf0Value::String(FMS_VERSION.to_string()),
                    ),
                    ("capabilities".to_string(), Amf0Value::Number(CAPABILITIES)),
                ]),
                Amf0Value::Object(vec![
                    ("level".to_string(), Amf0Value::String("status".to_string())),
                    (
                        "code".to_string(),
                        Amf0Value::String("NetConnection.Connect.Success".to_string()),
                    ),
                    (
                        "description".to_string(),
                        Amf0Value::String("Connection succeeded.".to_string()),
                    ),
                ]),
            ],
        };
        out.extend_from_slice(&self.writer.write(&self.command_message(msg, &result)));

        events.push(ServerEvent::Connected { app });
        Ok(())
    }

    /// `createStream` (§7.2.2, `NetConnection`): allocate a message stream
    /// id, reply `_result` with it.
    ///
    /// # Errors
    /// [`RtmpError::UnexpectedState`] unless `state == State::Connected` —
    /// this rejects `createStream` before a successful `connect`, and also
    /// after [`State::Closed`] (post-`deleteStream`/`FCUnpublish`).
    fn handle_create_stream(
        &mut self,
        command: &Command,
        msg: &Message,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        if self.state != State::Connected {
            return Err(RtmpError::UnexpectedState {
                what: "createStream received before a successful connect (or after the session was closed)",
            });
        }

        let stream_id = self.next_stream_id;
        self.next_stream_id = self.next_stream_id.saturating_add(1);
        self.created_stream_id = Some(stream_id);

        let result = Command {
            name: "_result".to_string(),
            transaction_id: command.transaction_id,
            arguments: vec![Amf0Value::Null, Amf0Value::Number(f64::from(stream_id))],
        };
        out.extend_from_slice(&self.writer.write(&self.command_message(msg, &result)));
        Ok(())
    }

    /// `publish` (§7.2.2.6, `NetStream`): capture the stream key, enforce
    /// `expected_stream_key` if configured, reply StreamBegin + `onStatus`,
    /// transition to [`State::Publishing`], emit [`ServerEvent::Publish`].
    ///
    /// # Errors
    /// [`RtmpError::UnexpectedState`] unless `state == State::Connected`
    /// *and* a stream was actually allocated by a preceding `createStream`
    /// (`created_stream_id.is_some()`) — this rejects `publish` before
    /// `connect`, `publish` before `createStream`, and `publish` after
    /// [`State::Closed`].
    fn handle_publish(
        &mut self,
        command: &Command,
        msg: &Message,
        out: &mut Vec<u8>,
        events: &mut Vec<ServerEvent>,
    ) -> Result<()> {
        if self.state != State::Connected || self.created_stream_id.is_none() {
            return Err(RtmpError::UnexpectedState {
                what: "publish received before a successful connect+createStream (or after the session was closed)",
            });
        }
        let app = self
            .app
            .clone()
            .expect("state == Connected implies app was captured by connect");

        let stream_key = match command.arguments.get(1) {
            Some(Amf0Value::String(s)) => s.clone(),
            _ => {
                return Err(RtmpError::Malformed {
                    what: "publish command missing its publishing-name (string) argument",
                });
            }
        };
        let stream_id = msg.message_stream_id;

        if let Some(expected) = &self.config.expected_stream_key {
            if expected != &stream_key {
                let on_status = Command {
                    name: "onStatus".to_string(),
                    transaction_id: 0.0,
                    arguments: vec![
                        Amf0Value::Null,
                        Amf0Value::Object(vec![
                            ("level".to_string(), Amf0Value::String("error".to_string())),
                            (
                                "code".to_string(),
                                Amf0Value::String("NetStream.Publish.BadName".to_string()),
                            ),
                            (
                                "description".to_string(),
                                Amf0Value::String("Stream key mismatch.".to_string()),
                            ),
                        ]),
                    ],
                };
                out.extend_from_slice(&self.writer.write(&self.command_message(msg, &on_status)));
                // No Publish/Media events; state unchanged (not Publishing).
                return Ok(());
            }
        }

        let stream_begin = UserControl::StreamBegin(stream_id).to_message();
        out.extend_from_slice(&self.writer.write(&stream_begin));

        let on_status = Command {
            name: "onStatus".to_string(),
            transaction_id: 0.0,
            arguments: vec![
                Amf0Value::Null,
                Amf0Value::Object(vec![
                    ("level".to_string(), Amf0Value::String("status".to_string())),
                    (
                        "code".to_string(),
                        Amf0Value::String("NetStream.Publish.Start".to_string()),
                    ),
                    (
                        "description".to_string(),
                        Amf0Value::String(format!("{stream_key} is now published.")),
                    ),
                ]),
            ],
        };
        out.extend_from_slice(&self.writer.write(&self.command_message(msg, &on_status)));

        self.state = State::Publishing;
        events.push(ServerEvent::Publish {
            app,
            stream_key,
            stream_id,
        });
        Ok(())
    }

    /// Reply a benign `_result` command echoing `command`'s transaction id
    /// (used for the OBS extras `releaseStream`/`FCPublish`, which never
    /// error even when tolerated rather than fully implemented).
    fn reply_result(
        &mut self,
        command: &Command,
        msg: &Message,
        arguments: Vec<Amf0Value>,
        out: &mut Vec<u8>,
    ) {
        let result = Command {
            name: "_result".to_string(),
            transaction_id: command.transaction_id,
            arguments,
        };
        out.extend_from_slice(&self.writer.write(&self.command_message(msg, &result)));
    }

    /// Wrap `command` in a [`Message`] on [`COMMAND_CHUNK_STREAM_ID`],
    /// echoing `request`'s message stream id (replies travel back on the
    /// same `NetConnection`/`NetStream` channel the request arrived on).
    fn command_message(&self, request: &Message, command: &Command) -> Message {
        Message {
            chunk_stream_id: COMMAND_CHUNK_STREAM_ID,
            timestamp: 0,
            message_type_id: msg_type::COMMAND_AMF0,
            message_stream_id: request.message_stream_id,
            payload: command.to_body(),
        }
    }

    /// Convert an Audio(8)/Video(9)/Data-AMF0(18) message to an FLV tag and
    /// emit it as [`ServerEvent::Media`], but only while
    /// [`State::Publishing`] (messages arriving before `publish` succeeds,
    /// or after a stream-key mismatch, are silently dropped — no event).
    fn emit_media_if_publishing(
        &mut self,
        tag_type: u8,
        msg: &Message,
        events: &mut Vec<ServerEvent>,
    ) -> Result<()> {
        if self.state != State::Publishing {
            return Ok(());
        }

        let mut flv = if self.flv_header_sent {
            Vec::new()
        } else {
            self.flv_header_sent = true;
            flv_file_header()
        };
        flv.extend(flv_tag(tag_type, msg.timestamp, &msg.payload)?);
        events.push(ServerEvent::Media { flv });
        Ok(())
    }
}

/// Build the 13-byte FLV file header: 9-byte header (Annex E.2) + the first
/// `PreviousTagSize0` (always `0`).
fn flv_file_header() -> Vec<u8> {
    let mut v = Vec::with_capacity(FLV_HEADER_SIZE as usize + FLV_PREV_TAG_SIZE_LEN);
    v.extend_from_slice(&FLV_SIGNATURE);
    v.push(FLV_VERSION);
    v.push(FLV_TYPE_FLAGS_AUDIO_VIDEO);
    v.extend_from_slice(&FLV_HEADER_SIZE.to_be_bytes());
    v.extend_from_slice(&0u32.to_be_bytes());
    v
}

/// Build one FLV tag (Annex E.4.1): `TagType` + `DataSize` + `Timestamp` +
/// `TimestampExtended` + `StreamID`(always 0) + `Data` + `PreviousTagSize`.
///
/// # Errors
/// [`RtmpError::Unsupported`] if `payload` is too large for the tag's
/// 24-bit `DataSize` field.
fn flv_tag(tag_type: u8, timestamp: u32, payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > FLV_MAX_DATA_SIZE {
        return Err(RtmpError::Unsupported {
            what: "flv tag payload exceeds the 24-bit DataSize field",
        });
    }
    let data_size = payload.len() as u32;

    let mut v = Vec::with_capacity(FLV_TAG_HEADER_LEN + payload.len() + FLV_PREV_TAG_SIZE_LEN);
    v.push(tag_type);
    v.push((data_size >> 16) as u8);
    v.push((data_size >> 8) as u8);
    v.push(data_size as u8);
    v.push((timestamp >> 16) as u8);
    v.push((timestamp >> 8) as u8);
    v.push(timestamp as u8);
    v.push((timestamp >> 24) as u8);
    v.extend_from_slice(&[0, 0, 0]); // StreamID, always 0.
    v.extend_from_slice(payload);
    let prev_tag_size = (FLV_TAG_HEADER_LEN + payload.len()) as u32;
    v.extend_from_slice(&prev_tag_size.to_be_bytes());
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handshake::{HANDSHAKE_PACKET_LEN, RTMP_VERSION};
    use crate::message::CONTROL_CHUNK_STREAM_ID as CTRL_CSID;
    use broadcast_common::Serialize;

    // ── Test-only wire builders (this crate's own encoders) ─────────────

    const CLIENT_CSID: u32 = 3;

    fn build_c0_c1() -> Vec<u8> {
        let mut v = vec![0u8; 1 + HANDSHAKE_PACKET_LEN];
        v[0] = RTMP_VERSION;
        v
    }

    fn build_c2() -> Vec<u8> {
        vec![0u8; HANDSHAKE_PACKET_LEN]
    }

    fn command_message(
        csid: u32,
        stream_id: u32,
        name: &str,
        txn: f64,
        args: Vec<Amf0Value>,
    ) -> Message {
        let body = Command {
            name: name.to_string(),
            transaction_id: txn,
            arguments: args,
        }
        .to_body();
        Message {
            chunk_stream_id: csid,
            timestamp: 0,
            message_type_id: msg_type::COMMAND_AMF0,
            message_stream_id: stream_id,
            payload: body,
        }
    }

    fn connect_bytes(app: &str) -> Vec<u8> {
        let args = vec![Amf0Value::Object(vec![
            ("app".to_string(), Amf0Value::String(app.to_string())),
            (
                "type".to_string(),
                Amf0Value::String("nonprivate".to_string()),
            ),
        ])];
        let msg = command_message(CLIENT_CSID, 0, "connect", 1.0, args);
        ChunkWriter::new().write(&msg)
    }

    fn connect_bytes_no_args() -> Vec<u8> {
        let msg = command_message(CLIENT_CSID, 0, "connect", 1.0, vec![]);
        ChunkWriter::new().write(&msg)
    }

    fn create_stream_bytes() -> Vec<u8> {
        let msg = command_message(CLIENT_CSID, 0, "createStream", 2.0, vec![Amf0Value::Null]);
        ChunkWriter::new().write(&msg)
    }

    fn publish_bytes(stream_id: u32, stream_key: &str) -> Vec<u8> {
        let args = vec![
            Amf0Value::Null,
            Amf0Value::String(stream_key.to_string()),
            Amf0Value::String("live".to_string()),
        ];
        let msg = command_message(CLIENT_CSID, stream_id, "publish", 3.0, args);
        ChunkWriter::new().write(&msg)
    }

    fn av_bytes(
        stream_id: u32,
        message_type_id: u8,
        csid: u32,
        timestamp: u32,
        payload: Vec<u8>,
    ) -> Vec<u8> {
        let msg = Message {
            chunk_stream_id: csid,
            timestamp,
            message_type_id,
            message_stream_id: stream_id,
            payload,
        };
        ChunkWriter::new().write(&msg)
    }

    /// Decode every reassembled [`Message`] out of a reply byte stream.
    /// Pre-sets a generous chunk size: this session's own `_result`/
    /// `onStatus` replies are always written with the writer's *current*
    /// chunk size (128 until `connect`'s `SetChunkSize` control message is
    /// sent, `config.chunk_size` after) but every individual message in
    /// these tests fits in a single chunk either way, so decoding under a
    /// single generous assumption reproduces the same framing without
    /// needing to replay `SetChunkSize` mid-decode.
    fn decode_messages(bytes: &[u8]) -> Vec<Message> {
        let mut assembler = ChunkAssembler::new();
        assembler.set_chunk_size(65536);
        assembler.push(bytes).expect("well-formed reply stream")
    }

    fn decode_commands(bytes: &[u8]) -> Vec<Command> {
        decode_messages(bytes)
            .iter()
            .filter(|m| m.message_type_id == msg_type::COMMAND_AMF0)
            .map(|m| Command::parse(&m.payload).expect("well-formed command reply"))
            .collect()
    }

    fn onstatus_code(cmd: &Command) -> Option<String> {
        cmd.arguments.iter().find_map(|v| match v {
            Amf0Value::Object(pairs) => pairs.iter().find_map(|(k, v)| {
                if k == "code" {
                    match v {
                        Amf0Value::String(s) => Some(s.clone()),
                        _ => None,
                    }
                } else {
                    None
                }
            }),
            _ => None,
        })
    }

    /// Drive a session through handshake → connect → createStream →
    /// publish, returning `(session, all reply bytes, all events)`.
    fn publish_flow(
        config: ServerConfig,
        stream_key: &str,
    ) -> (ServerSession, Vec<u8>, Vec<ServerEvent>) {
        let mut session = ServerSession::new(config);
        // `all_out` accumulates only *post-handshake* (chunk-encoded) reply
        // bytes: the handshake reply (S0+S1+S2) is a raw fixed-length
        // packet, not chunk-stream framing, so it must not be fed into a
        // `ChunkAssembler` alongside the chunk-encoded command replies.
        let mut all_out = Vec::new();
        let mut all_events = Vec::new();

        session.handle_data(&build_c0_c1()).unwrap();
        session.handle_data(&build_c2()).unwrap();

        let (out, events) = session.handle_data(&connect_bytes("live")).unwrap();
        all_out.extend(out);
        all_events.extend(events);

        let (out, events) = session.handle_data(&create_stream_bytes()).unwrap();
        all_out.extend(out);
        all_events.extend(events);

        // ServerSession always allocates stream ids starting at 1.
        let (out, events) = session.handle_data(&publish_bytes(1, stream_key)).unwrap();
        all_out.extend(out);
        all_events.extend(events);

        (session, all_out, all_events)
    }

    // ── Handshake ─────────────────────────────────────────────────────────

    #[test]
    fn handshake_completes_and_reply_contains_s0_s1_s2() {
        let mut session = ServerSession::with_defaults();
        let (out1, events1) = session.handle_data(&build_c0_c1()).unwrap();
        assert_eq!(
            out1.len(),
            1 + HANDSHAKE_PACKET_LEN + HANDSHAKE_PACKET_LEN,
            "S0+S1+S2 must be a single 3073-byte reply"
        );
        assert!(events1.is_empty());

        let (out2, events2) = session.handle_data(&build_c2()).unwrap();
        assert!(out2.is_empty(), "C2 receipt produces no reply bytes itself");
        assert!(events2.is_empty());
    }

    #[test]
    fn handshake_split_across_calls_still_completes() {
        let mut session = ServerSession::with_defaults();
        let c0c1 = build_c0_c1();
        let (out1, _) = session.handle_data(&c0c1[..500]).unwrap();
        assert!(out1.is_empty(), "partial C0+C1 produces no reply yet");
        let (out2, _) = session.handle_data(&c0c1[500..]).unwrap();
        assert_eq!(out2.len(), 1 + 2 * HANDSHAKE_PACKET_LEN);
        let (_out3, _) = session.handle_data(&build_c2()).unwrap();
    }

    #[test]
    fn c2_pipelined_with_connect_chunk_in_one_call_still_parses_connect() {
        // Real clients commonly send C2 back-to-back with the very next
        // chunk-encoded message (e.g. `connect`) in the same TCP segment,
        // so both arrive together in a single `handle_data` call. The
        // post-handshake leftover bytes from that call must be handed to
        // the chunk assembler within the SAME call, not merely buffered
        // for a subsequent one.
        let mut session = ServerSession::with_defaults();
        session.handle_data(&build_c0_c1()).unwrap();

        let mut pipelined = build_c2();
        pipelined.extend_from_slice(&connect_bytes("live"));

        let (_out, events) = session.handle_data(&pipelined).unwrap();
        assert_eq!(
            events,
            vec![ServerEvent::Connected {
                app: "live".to_string()
            }],
            "C2 pipelined with the connect chunk in one handle_data call must \
             still yield Connected from that call (leftover bytes must not be dropped)"
        );
    }

    // ── connect ───────────────────────────────────────────────────────────

    #[test]
    fn connect_emits_connected_event_and_result_reply() {
        let mut session = ServerSession::with_defaults();
        session.handle_data(&build_c0_c1()).unwrap();
        session.handle_data(&build_c2()).unwrap();

        let (out, events) = session.handle_data(&connect_bytes("live")).unwrap();
        assert_eq!(
            events,
            vec![ServerEvent::Connected {
                app: "live".to_string()
            }]
        );

        let commands = decode_commands(&out);
        assert!(
            commands.iter().any(|c| c.name == "_result"),
            "connect reply must contain a _result command"
        );
    }

    #[test]
    fn connect_without_command_object_is_malformed() {
        let mut session = ServerSession::with_defaults();
        session.handle_data(&build_c0_c1()).unwrap();
        session.handle_data(&build_c2()).unwrap();

        // No arguments at all: arg0 (the command object) is missing.
        let err = session.handle_data(&connect_bytes_no_args()).unwrap_err();
        assert!(
            matches!(err, RtmpError::Malformed { .. }),
            "connect with no command-object argument must error, not default app to \"\""
        );
    }

    // ── createStream ──────────────────────────────────────────────────────

    #[test]
    fn create_stream_replies_result_with_stream_id() {
        let mut session = ServerSession::with_defaults();
        session.handle_data(&build_c0_c1()).unwrap();
        session.handle_data(&build_c2()).unwrap();
        session.handle_data(&connect_bytes("live")).unwrap();

        let (out, _events) = session.handle_data(&create_stream_bytes()).unwrap();
        let commands = decode_commands(&out);
        let result = commands
            .iter()
            .find(|c| c.name == "_result")
            .expect("createStream _result reply");
        assert_eq!(
            result.arguments.get(1),
            Some(&Amf0Value::Number(1.0)),
            "first allocated stream id must be 1"
        );
    }

    #[test]
    fn create_stream_before_connect_is_unexpected_state() {
        let mut session = ServerSession::with_defaults();
        session.handle_data(&build_c0_c1()).unwrap();
        session.handle_data(&build_c2()).unwrap();

        let err = session.handle_data(&create_stream_bytes()).unwrap_err();
        assert!(matches!(err, RtmpError::UnexpectedState { .. }));
    }

    // ── publish ───────────────────────────────────────────────────────────

    #[test]
    fn publish_reaches_publishing_emits_event_and_stream_begin_plus_onstatus() {
        let (_session, out, events) = publish_flow(ServerConfig::default(), "testkey");

        assert!(events.contains(&ServerEvent::Publish {
            app: "live".to_string(),
            stream_key: "testkey".to_string(),
            stream_id: 1,
        }));

        let messages = decode_messages(&out);
        let has_stream_begin = messages.iter().any(|m| {
            m.message_type_id == msg_type::USER_CONTROL
                && matches!(
                    UserControl::parse(&m.payload),
                    Ok(UserControl::StreamBegin(1))
                )
        });
        assert!(
            has_stream_begin,
            "publish reply must include StreamBegin(1)"
        );

        let commands = decode_commands(&out);
        let on_status = commands
            .iter()
            .find(|c| c.name == "onStatus")
            .expect("onStatus reply to publish");
        assert_eq!(
            onstatus_code(on_status).as_deref(),
            Some("NetStream.Publish.Start")
        );
    }

    #[test]
    fn publish_before_connect_is_unexpected_state() {
        let mut session = ServerSession::with_defaults();
        session.handle_data(&build_c0_c1()).unwrap();
        session.handle_data(&build_c2()).unwrap();

        let err = session
            .handle_data(&publish_bytes(1, "testkey"))
            .unwrap_err();
        assert!(matches!(err, RtmpError::UnexpectedState { .. }));
    }

    #[test]
    fn publish_without_create_stream_is_unexpected_state() {
        let mut session = ServerSession::with_defaults();
        session.handle_data(&build_c0_c1()).unwrap();
        session.handle_data(&build_c2()).unwrap();
        session.handle_data(&connect_bytes("live")).unwrap();

        // createStream never called: publish must not succeed on app-only.
        let err = session
            .handle_data(&publish_bytes(1, "testkey"))
            .unwrap_err();
        assert!(matches!(err, RtmpError::UnexpectedState { .. }));
    }

    #[test]
    fn create_stream_after_closed_is_unexpected_state() {
        let mut session = ServerSession::with_defaults();
        session.handle_data(&build_c0_c1()).unwrap();
        session.handle_data(&build_c2()).unwrap();
        session.handle_data(&connect_bytes("live")).unwrap();
        session.handle_data(&create_stream_bytes()).unwrap();
        session.handle_data(&publish_bytes(1, "testkey")).unwrap();

        let delete_stream = command_message(
            CLIENT_CSID,
            1,
            "deleteStream",
            4.0,
            vec![Amf0Value::Null, Amf0Value::Number(1.0)],
        );
        let (_out, events) = session
            .handle_data(&ChunkWriter::new().write(&delete_stream))
            .unwrap();
        assert_eq!(events, vec![ServerEvent::Eof]);

        let err = session.handle_data(&create_stream_bytes()).unwrap_err();
        assert!(
            matches!(err, RtmpError::UnexpectedState { .. }),
            "createStream after State::Closed must be rejected, not silently re-allowed"
        );
    }

    // ── Audio/Video → Media / FLV ─────────────────────────────────────────

    #[test]
    fn audio_and_video_emit_media_first_carries_flv_file_header() {
        let (mut session, _out, _events) = publish_flow(ServerConfig::default(), "testkey");

        let (_out1, events1) = session
            .handle_data(&av_bytes(
                1,
                msg_type::AUDIO,
                4,
                0,
                vec![0xAF, 0x01, 0xDE, 0xAD],
            ))
            .unwrap();
        assert_eq!(events1.len(), 1);
        let ServerEvent::Media { flv } = &events1[0] else {
            panic!("expected Media event");
        };
        assert!(
            flv.starts_with(b"FLV"),
            "the first Media event must carry the FLV file header"
        );

        let (_out2, events2) = session
            .handle_data(&av_bytes(
                1,
                msg_type::VIDEO,
                6,
                40,
                vec![0x17, 0x01, 0x00, 0x00, 0x00, 0xDE, 0xAD, 0xBE, 0xEF],
            ))
            .unwrap();
        assert_eq!(events2.len(), 1);
        let ServerEvent::Media { flv } = &events2[0] else {
            panic!("expected Media event");
        };
        assert!(
            !flv.starts_with(b"FLV"),
            "only the first Media event carries the file header"
        );
    }

    #[test]
    fn concatenated_media_forms_structurally_valid_flv() {
        let (mut session, _out, _events) = publish_flow(ServerConfig::default(), "testkey");

        let mut flv_stream = Vec::new();
        let (_out1, events1) = session
            .handle_data(&av_bytes(
                1,
                msg_type::AUDIO,
                4,
                0,
                vec![0xAF, 0x01, 1, 2, 3],
            ))
            .unwrap();
        let (_out2, events2) = session
            .handle_data(&av_bytes(
                1,
                msg_type::VIDEO,
                6,
                33,
                vec![0x17, 0x01, 0, 0, 0, 4, 5, 6],
            ))
            .unwrap();
        for e in events1.into_iter().chain(events2) {
            if let ServerEvent::Media { flv } = e {
                flv_stream.extend(flv);
            }
        }

        // File header (13 bytes): signature/version/flags/data-offset/prevTagSize0.
        assert_eq!(&flv_stream[0..3], b"FLV");
        assert_eq!(flv_stream[3], 1, "FLV version");
        assert_eq!(flv_stream[4], 0b0000_0101, "audio+video TypeFlags");
        assert_eq!(
            u32::from_be_bytes(flv_stream[5..9].try_into().unwrap()),
            9,
            "DataOffset (header size)"
        );
        assert_eq!(
            u32::from_be_bytes(flv_stream[9..13].try_into().unwrap()),
            0,
            "PreviousTagSize0"
        );

        // First tag (audio): TagType=8, DataSize=5.
        let tag1 = &flv_stream[13..];
        assert_eq!(tag1[0], msg_type::AUDIO);
        let data_size1 =
            (u32::from(tag1[1]) << 16) | (u32::from(tag1[2]) << 8) | u32::from(tag1[3]);
        assert_eq!(data_size1, 5);
        let tag1_total = FLV_TAG_HEADER_LEN + 5 + FLV_PREV_TAG_SIZE_LEN;
        let prev_tag_size1 = u32::from_be_bytes(
            flv_stream[13 + tag1_total - 4..13 + tag1_total]
                .try_into()
                .unwrap(),
        );
        assert_eq!(prev_tag_size1 as usize, FLV_TAG_HEADER_LEN + 5);

        // Second tag (video) immediately follows.
        let tag2 = &flv_stream[13 + tag1_total..];
        assert_eq!(tag2[0], msg_type::VIDEO);
        let data_size2 =
            (u32::from(tag2[1]) << 16) | (u32::from(tag2[2]) << 8) | u32::from(tag2[3]);
        assert_eq!(data_size2, 8);
        assert_eq!(
            flv_stream.len(),
            13 + tag1_total + FLV_TAG_HEADER_LEN + 8 + FLV_PREV_TAG_SIZE_LEN
        );
    }

    #[test]
    fn data_amf0_onmetadata_emits_media_with_script_tag_type() {
        let (mut session, _out, _events) = publish_flow(ServerConfig::default(), "testkey");

        // A representative onMetadata Data-AMF0 payload: handler-name
        // string followed by a properties object (§7.1's Data Message
        // shape), same as e.g. width/height/framerate metadata a real
        // encoder sends.
        let mut payload = Amf0Value::String("onMetaData".to_string()).to_bytes();
        payload.extend(
            Amf0Value::Object(vec![
                ("width".to_string(), Amf0Value::Number(1920.0)),
                ("height".to_string(), Amf0Value::Number(1080.0)),
            ])
            .to_bytes(),
        );

        let (_out, events) = session
            .handle_data(&av_bytes(1, msg_type::DATA_AMF0, 4, 0, payload))
            .unwrap();

        assert_eq!(events.len(), 1);
        let ServerEvent::Media { flv } = &events[0] else {
            panic!("expected Media event");
        };
        assert!(
            flv.starts_with(b"FLV"),
            "the first Media event must carry the FLV file header"
        );
        let tag_type = flv[FLV_HEADER_SIZE as usize + FLV_PREV_TAG_SIZE_LEN];
        assert_eq!(
            tag_type,
            msg_type::DATA_AMF0,
            "Data-AMF0 message must produce a script(18) FLV tag"
        );
    }

    #[test]
    fn media_before_publishing_is_silently_dropped() {
        let mut session = ServerSession::with_defaults();
        session.handle_data(&build_c0_c1()).unwrap();
        session.handle_data(&build_c2()).unwrap();
        session.handle_data(&connect_bytes("live")).unwrap();
        session.handle_data(&create_stream_bytes()).unwrap();

        // publish never called: state is Connected, not Publishing.
        let (_out, events) = session
            .handle_data(&av_bytes(1, msg_type::AUDIO, 4, 0, vec![0xAF, 0x01]))
            .unwrap();
        assert!(events.is_empty());
    }

    // ── expected_stream_key mismatch ──────────────────────────────────────

    #[test]
    fn stream_key_mismatch_suppresses_publish_and_media_events() {
        let config = ServerConfig {
            expected_stream_key: Some("rightkey".to_string()),
            ..ServerConfig::default()
        };
        let (mut session, out, events) = publish_flow(config, "wrongkey");

        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ServerEvent::Publish { .. })),
            "mismatched stream key must not emit Publish"
        );

        let commands = decode_commands(&out);
        let on_status = commands
            .iter()
            .find(|c| c.name == "onStatus")
            .expect("onStatus reply on mismatch");
        assert_eq!(
            onstatus_code(on_status).as_deref(),
            Some("NetStream.Publish.BadName")
        );

        // Session never entered Publishing: subsequent A/V produces no Media.
        let (_out2, events2) = session
            .handle_data(&av_bytes(1, msg_type::AUDIO, 4, 0, vec![0xAF, 0x01]))
            .unwrap();
        assert!(
            events2.is_empty(),
            "no Media may be emitted after a rejected publish"
        );
    }

    // ── Ack accounting ────────────────────────────────────────────────────

    #[test]
    fn ack_written_once_window_ack_size_is_crossed() {
        let config = ServerConfig {
            window_ack_size: 32,
            ..ServerConfig::default()
        };
        let mut session = ServerSession::new(config);
        session.handle_data(&build_c0_c1()).unwrap();
        session.handle_data(&build_c2()).unwrap();

        // connect's chunk-encoded bytes comfortably exceed 32 bytes.
        let (out, _events) = session.handle_data(&connect_bytes("live")).unwrap();
        let messages = decode_messages(&out);
        let has_ack = messages.iter().any(|m| {
            matches!(
                ProtocolControl::from_message(m),
                Ok(Some(ProtocolControl::Acknowledgement(_)))
            )
        });
        assert!(
            has_ack,
            "crossing window_ack_size must produce an Acknowledgement"
        );
    }

    #[test]
    fn no_ack_below_window_ack_size() {
        let config = ServerConfig {
            window_ack_size: 10_000_000,
            ..ServerConfig::default()
        };
        let mut session = ServerSession::new(config);
        session.handle_data(&build_c0_c1()).unwrap();
        let (out, _events) = session.handle_data(&build_c2()).unwrap();
        let messages = decode_messages(&out);
        assert!(
            !messages.iter().any(|m| matches!(
                ProtocolControl::from_message(m),
                Ok(Some(ProtocolControl::Acknowledgement(_)))
            )),
            "no Acknowledgement should be due yet"
        );
    }

    // ── Garbage / truncated input never panics ────────────────────────────

    #[test]
    fn garbage_command_payload_after_handshake_is_error_not_panic() {
        let mut session = ServerSession::with_defaults();
        session.handle_data(&build_c0_c1()).unwrap();
        session.handle_data(&build_c2()).unwrap();

        // A structurally valid chunk envelope (Type0 header, Command-AMF0
        // type id) whose payload is not valid AMF0 (0xFF is not a defined
        // AMF0 marker) — Command::parse must reject this, not panic.
        let bogus = Message {
            chunk_stream_id: CLIENT_CSID,
            timestamp: 0,
            message_type_id: msg_type::COMMAND_AMF0,
            message_stream_id: 0,
            payload: vec![0xFF, 0xFF, 0xFF, 0xFF],
        };
        let bytes = ChunkWriter::new().write(&bogus);
        let err = session.handle_data(&bytes).unwrap_err();
        assert!(matches!(
            err,
            RtmpError::Unsupported { .. }
                | RtmpError::Malformed { .. }
                | RtmpError::BufferTooShort { .. }
        ));
    }

    #[test]
    fn truncated_post_handshake_bytes_do_not_panic() {
        let mut session = ServerSession::with_defaults();
        session.handle_data(&build_c0_c1()).unwrap();
        session.handle_data(&build_c2()).unwrap();

        // A handful of arbitrary bytes with no complete chunk in them.
        let (out, events) = session.handle_data(&[0x03, 0x01, 0x02]).unwrap();
        assert!(out.is_empty());
        assert!(events.is_empty());
    }

    // ── Mutation-check sentinels ──────────────────────────────────────────

    #[test]
    fn mutation_check_publish_event_must_echo_actual_stream_key() {
        let (_session, _out, events) = publish_flow(ServerConfig::default(), "specific-key-xyz");
        let publish_event = events
            .iter()
            .find_map(|e| match e {
                ServerEvent::Publish { stream_key, .. } => Some(stream_key.clone()),
                _ => None,
            })
            .expect("Publish event");
        assert_eq!(
            publish_event, "specific-key-xyz",
            "a hardcoded/ignored stream_key would fail this"
        );
    }

    #[test]
    fn mutation_check_flv_file_header_bytes_are_exact() {
        let header = flv_file_header();
        assert_eq!(
            header,
            vec![
                b'F',
                b'L',
                b'V',        // Signature
                1,           // Version
                0b0000_0101, // TypeFlags: audio + video
                0,
                0,
                0,
                9, // DataOffset = 9
                0,
                0,
                0,
                0, // PreviousTagSize0 = 0
            ]
        );
    }

    // ── Regression: client Set Chunk Size mid-buffer (#738 Task 8) ────────

    #[test]
    fn client_set_chunk_size_takes_effect_before_next_message_in_same_call() {
        // Reproduces the exact bug a real `ffmpeg` publish surfaced: the
        // client sends its own SetChunkSize (as ffmpeg does, right after
        // `connect`) and, in the very same TCP segment / `handle_data` call,
        // the next message is already framed at the *new* chunk size. If
        // `ServerSession` collected a whole batch of messages from one
        // `ChunkAssembler::push` before dispatching any of them (applying
        // SetChunkSize's effect only afterwards), that next message would
        // be misparsed under the *old* chunk size.
        let (mut session, _out, _events) = publish_flow(ServerConfig::default(), "testkey");

        const NEW_CHUNK_SIZE: u32 = 4096;
        let set_chunk_size_bytes =
            ChunkWriter::new().write(&ProtocolControl::SetChunkSize(NEW_CHUNK_SIZE).to_message());

        // A video payload bigger than the *default* 128-byte chunk size but
        // written by a client-side writer already using the new size, so it
        // lands as a single physical chunk — the shape a real client
        // produces immediately after raising its chunk size.
        let big_payload = vec![0x17u8; 300];
        let mut client_writer = ChunkWriter::new();
        client_writer.set_chunk_size(NEW_CHUNK_SIZE);
        let video_bytes = client_writer.write(&Message {
            chunk_stream_id: 6,
            timestamp: 0,
            message_type_id: msg_type::VIDEO,
            message_stream_id: 1,
            payload: big_payload.clone(),
        });

        let mut combined = set_chunk_size_bytes;
        combined.extend_from_slice(&video_bytes);

        // Both messages arrive in a single `handle_data` call: this is the
        // exact shape that broke before the incremental-dispatch fix.
        let (_out, events) = session.handle_data(&combined).expect(
            "SetChunkSize must take effect before parsing the message that follows it \
                     in the same handle_data call, not only on a subsequent call",
        );

        let media = events
            .iter()
            .find_map(|e| match e {
                ServerEvent::Media { flv } => Some(flv.clone()),
                _ => None,
            })
            .expect("the video message must still be parsed into a Media event");
        assert!(
            media
                .windows(big_payload.len())
                .any(|w| w == big_payload.as_slice()),
            "the video payload must survive intact through the chunk-size change"
        );
    }

    #[test]
    fn control_chunk_stream_id_constant_matches_message_module() {
        // Sanity: our reply csid choice for command messages must not
        // collide with the reserved control/user-control csid.
        assert_ne!(COMMAND_CHUNK_STREAM_ID, CTRL_CSID);
    }

    #[test]
    fn next_stream_id_saturates_instead_of_overflowing() {
        // Mutation check: with `next_stream_id` already at `u32::MAX`, a
        // bare `+= 1` panics (debug-mode overflow check) or wraps to 0
        // (release mode) — either way, the wrong behaviour. `saturating_add`
        // must instead keep it pinned at `u32::MAX`.
        let mut session = ServerSession::with_defaults();
        session.handle_data(&build_c0_c1()).unwrap();
        session.handle_data(&build_c2()).unwrap();
        session.handle_data(&connect_bytes("live")).unwrap();

        session.next_stream_id = u32::MAX;
        let (out, _events) = session
            .handle_data(&create_stream_bytes())
            .expect("createStream must not panic when next_stream_id is already u32::MAX");

        let commands = decode_commands(&out);
        let result = commands
            .iter()
            .find(|c| c.name == "_result")
            .expect("createStream _result reply");
        assert_eq!(
            result.arguments.get(1),
            Some(&Amf0Value::Number(f64::from(u32::MAX))),
            "the stream id allocated at the u32::MAX boundary must still be u32::MAX"
        );
        assert_eq!(
            session.next_stream_id,
            u32::MAX,
            "next_stream_id must saturate at u32::MAX, not wrap to 0"
        );
    }

    // ── serde (feature "serde") ────────────────────────────────────────────

    #[cfg(feature = "serde")]
    #[test]
    fn server_config_and_server_event_serde_round_trip() {
        let config = ServerConfig::default()
            .with_chunk_size(8192)
            .with_expected_stream_key(Some("k".to_string()));
        let json = serde_json::to_string(&config).expect("serialize ServerConfig");
        let back: ServerConfig = serde_json::from_str(&json).expect("deserialize ServerConfig");
        assert_eq!(back.chunk_size, config.chunk_size);
        assert_eq!(back.expected_stream_key, config.expected_stream_key);

        let event = ServerEvent::Publish {
            app: "live".to_string(),
            stream_key: "testkey".to_string(),
            stream_id: 1,
        };
        let json = serde_json::to_string(&event).expect("serialize ServerEvent");
        let back: ServerEvent = serde_json::from_str(&json).expect("deserialize ServerEvent");
        assert_eq!(back, event);
    }
}
