//! RTMP client session engine — `connect` → `createStream` → `publish`
//! (Adobe RTMP 1.0 §7.2, `NetConnection`/`NetStream` commands).
//!
//! [`ClientSession`] mirrors [`crate::server::ServerSession`]: a sans-IO
//! state machine for the **client publish** role. Feed inbound bytes via
//! [`ClientSession::handle_data`], get back outbound bytes + typed
//! [`ClientEvent`]s. Auto-advances: a successful `connect` `_result`
//! automatically emits `createStream`, and a successful `createStream`
//! `_result` emits `publish`.

use broadcast_common::error::BufferTooShort;
use broadcast_common::len::need_in;
use broadcast_common::{Parse, Serialize};

use crate::RtmpError;
use crate::amf0::{Amf0Value, Command};
use crate::chunk::{ChunkAssembler, ChunkWriter, DEFAULT_MAX_IN_PROGRESS_BYTES, Message};
use crate::flv::{FlvTagWriter, unpack_aggregate};
use crate::handshake::{
    EchoPacket, HANDSHAKE_PACKET_LEN, HandshakePacket, RANDOM_LEN, RTMP_VERSION, Version,
    default_random_fill,
};
use crate::ids::{ChunkStreamId, MessageStreamId, SequenceNumber, Timestamp};
use crate::message::{ChunkSize, MessageTypeId, ProtocolControl, UserControl};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

type Result<T> = core::result::Result<T, RtmpError>;

const COMMAND_CHUNK_STREAM_ID: u32 = 3;
/// Chunk stream audio and data messages are sent on.
const PAYLOAD_CHUNK_STREAM_ID: u32 = 4;
/// Chunk stream video messages are sent on, kept apart from audio so the two
/// never interleave on one chunk stream.
const VIDEO_CHUNK_STREAM_ID: u32 = 5;
/// Default `chunk_size` the client announces.
const DEFAULT_CLIENT_CHUNK_SIZE: u32 = 4096;
/// Default `window_ack_size` the client announces (RTMP §5.4.4).
const DEFAULT_WINDOW_ACK_SIZE: u32 = 2_500_000;
/// Tolerance comparing an echoed AMF0 transaction id with the one sent: AMP0
/// carries them as doubles.
const TRANSACTION_ID_TOLERANCE: f64 = 0.5;
/// Mask of the 32-bit sequence number an acknowledgement carries (§5.4.3).
const SEQUENCE_NUMBER_MASK: u64 = u32::MAX as u64;
const VERSION_LEN: usize = 1;
/// `flashVer` announced in `connect`: the Flash Media Live Encoder
/// identity that ingest services key their publisher-compatibility
/// handling on (what ffmpeg/OBS announce; #1108/RTMP-W9).
const FLASH_VER: &str = "FMLE/3.0 (compatible; rtmp-runtime)";
/// FMLE-style publish preamble commands sent before `createStream`
/// (Adobe FMLE publish sequence, as ffmpeg's `rtmpproto` emits it).
const CMD_RELEASE_STREAM: &str = "releaseStream";
const CMD_FC_PUBLISH: &str = "FCPublish";
/// `play` Start argument (§7.2.2.1): `-2` plays the live stream of that name, else a recorded
/// one, else waits for a live one.
const PLAY_START_LIVE_OR_RECORDED: f64 = -2.0;
/// Default playback buffer length announced with SetBufferLength before `play` (§6.7).
pub const DEFAULT_PLAY_BUFFER: core::time::Duration = core::time::Duration::from_secs(3);
const STATUS_PLAY_START: &str = "NetStream.Play.Start";
const STATUS_PLAY_STOP: &str = "NetStream.Play.Stop";
const STATUS_PLAY_COMPLETE: &str = "NetStream.Play.Complete";
const STATUS_PLAY_STREAM_NOT_FOUND: &str = "NetStream.Play.StreamNotFound";
const STATUS_PLAY_FAILED: &str = "NetStream.Play.Failed";
const STATUS_LEVEL_ERROR: &str = "error";

/// What a [`ClientSession`] does after `connect` and `createStream`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClientMode {
    /// `publish` a stream (§7.2.2.6): the session sends audio, video and metadata.
    #[default]
    Publish,
    /// `play` a stream (§7.2.2.1): the session receives audio, video and data.
    Play,
}

impl ClientMode {
    /// The NetStream command this mode ends in.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Publish => "publish",
            Self::Play => "play",
        }
    }
}

broadcast_common::impl_spec_display!(ClientMode);

/// Configuration for a [`ClientSession`].
#[non_exhaustive]
#[derive(Clone)]
pub struct RtmpClientConfig {
    /// Publish (default) or play.
    pub mode: ClientMode,
    /// Playback buffer length announced to the server before `play` (§6.7 SetBufferLength),
    /// sent as whole milliseconds on the wire. Play mode only.
    pub play_buffer: core::time::Duration,
    /// Outbound chunk size (§5.4.1).
    pub chunk_size: u32,
    /// Window Acknowledgement Size (§5.4.4).
    pub window_ack_size: u32,
    /// RTMP `app` name for the `connect` command.
    pub app: String,
    /// Publishing name (stream key) for the `publish` command, or the stream name `play` asks
    /// for in play mode.
    pub stream_key: String,
    /// `tcUrl` for the `connect` command (e.g. `rtmp://host/app`).
    pub tc_url: Option<String>,
    /// Ceiling on the total payload bytes this session's
    /// [`ChunkAssembler`] may buffer across **all** in-progress messages
    /// at once (#1108/RTMP-W1); see
    /// [`crate::chunk::DEFAULT_MAX_IN_PROGRESS_BYTES`] for the default.
    pub max_in_progress_bytes: usize,
}

/// Manual `Debug`: the stream key is a publish secret and never renders
/// (issue #1142).
impl core::fmt::Debug for RtmpClientConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RtmpClientConfig")
            .field("mode", &self.mode)
            .field("play_buffer", &self.play_buffer)
            .field("chunk_size", &self.chunk_size)
            .field("window_ack_size", &self.window_ack_size)
            .field("app", &self.app)
            .field("stream_key", &"<redacted>")
            .field("tc_url", &self.tc_url)
            .field("max_in_progress_bytes", &self.max_in_progress_bytes)
            .finish()
    }
}

impl Default for RtmpClientConfig {
    fn default() -> Self {
        Self {
            mode: ClientMode::Publish,
            play_buffer: DEFAULT_PLAY_BUFFER,
            chunk_size: DEFAULT_CLIENT_CHUNK_SIZE,
            window_ack_size: DEFAULT_WINDOW_ACK_SIZE,
            app: String::new(),
            stream_key: String::new(),
            tc_url: None,
            max_in_progress_bytes: DEFAULT_MAX_IN_PROGRESS_BYTES,
        }
    }
}

/// Typed events [`ClientSession::handle_data`] surfaces.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    /// Server accepted `connect`.
    Connected {
        /// Properties from `_result`.
        properties: Vec<(String, Amf0Value)>,
    },
    /// Server allocated a stream id.
    StreamCreated {
        /// The allocated message stream id.
        stream_id: u32,
    },
    /// Server accepted `publish` — session is now publishing.
    Publishing,
    /// Server accepted `play` (`NetStream.Play.Start`): media follows. Play mode only.
    Playing {
        /// The server announced the stream as recorded (User Control StreamIsRecorded, §6.7)
        /// before starting it, so its end is final; a live stream's end is not.
        recorded: bool,
    },
    /// One FLV chunk of played media: the payload of a single Audio(8)/Video(9)/Data-AMF0(18)
    /// message as an FLV tag (+ `PreviousTagSize`), the very first chunk prefixed with the FLV
    /// file header. Concatenating every chunk in order yields a valid FLV stream. Play mode only.
    Media {
        /// FLV bytes for this tag.
        flv: Vec<u8>,
    },
    /// The played stream ended: User Control Stream EOF (§6.7) or `NetStream.Play.Stop` /
    /// `NetStream.Play.Complete`. Play mode only.
    StreamEnded,
    /// Server returned an error.
    Error {
        /// Status code.
        code: String,
        /// Description.
        description: String,
    },
    /// Session closed.
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ClientState {
    Init,
    HandshakeDone,
    ConnectSent { txn_id: f64 },
    Connected,
    CreateStreamSent { txn_id: f64 },
    StreamCreated,
    PublishSent,
    Publishing,
    PlaySent,
    Playing,
}

/// Client-side handshake: generate C0+C1, consume S0+S1+S2, produce C2.
#[derive(Debug)]
struct ClientHandshake {
    state: ClientHandshakeState,
    local_time: u32,
    local_random: [u8; RANDOM_LEN],
}

#[derive(Debug)]
enum ClientHandshakeState {
    SendC0C1,
    WaitS0S1S2,
    Done,
}

impl ClientHandshake {
    fn new() -> Self {
        Self {
            state: ClientHandshakeState::SendC0C1,
            local_time: 0,
            local_random: default_random_fill(),
        }
    }

    fn start(&mut self) -> Vec<u8> {
        let c0 = Version(RTMP_VERSION);
        let c1 = HandshakePacket {
            time: self.local_time,
            zero: 0,
            random: self.local_random,
        };
        let mut out = vec![0u8; VERSION_LEN + HANDSHAKE_PACKET_LEN];
        out[..VERSION_LEN].copy_from_slice(&c0.to_bytes());
        out[VERSION_LEN..].copy_from_slice(&c1.to_bytes());
        self.state = ClientHandshakeState::WaitS0S1S2;
        out
    }

    fn read(&mut self, input: &[u8]) -> Result<(Vec<u8>, usize, bool)> {
        match &self.state {
            ClientHandshakeState::SendC0C1 => Ok((Vec::new(), 0, false)),
            ClientHandshakeState::WaitS0S1S2 => {
                let need = VERSION_LEN + HANDSHAKE_PACKET_LEN + HANDSHAKE_PACKET_LEN;
                need_in(input, need, "S0+S1+S2")?;
                let s0 = Version::parse(&input[..VERSION_LEN])?;
                // #1108/RTMP-W11: see handshake.rs's identical C0 check.
                if s0.0 != RTMP_VERSION {
                    return Err(RtmpError::HandshakeVersion {
                        value: s0.0,
                        expected: RTMP_VERSION,
                    });
                }
                let s1 = HandshakePacket::parse(
                    &input[VERSION_LEN..VERSION_LEN + HANDSHAKE_PACKET_LEN],
                )?;
                let _s2 = EchoPacket::parse(&input[VERSION_LEN + HANDSHAKE_PACKET_LEN..need])?;

                let c2 = EchoPacket {
                    time: s1.time,
                    time2: 0,
                    random_echo: s1.random,
                };
                let mut reply = vec![0u8; HANDSHAKE_PACKET_LEN];
                c2.serialize_into(&mut reply)?;

                self.state = ClientHandshakeState::Done;
                Ok((reply, need, true))
            }
            ClientHandshakeState::Done => Ok((Vec::new(), 0, true)),
        }
    }

    fn is_done(&self) -> bool {
        matches!(self.state, ClientHandshakeState::Done)
    }
}

/// Sans-IO RTMP client publish session.
#[derive(Debug)]
pub struct ClientSession {
    config: RtmpClientConfig,
    handshake: ClientHandshake,
    handshake_buf: Vec<u8>,
    assembler: ChunkAssembler,
    writer: ChunkWriter,
    state: ClientState,
    stream_id: Option<MessageStreamId>,
    next_txn_id: f64,
    ack_threshold: u32,
    bytes_received: u64,
    bytes_acked: u64,
    /// The peer's declared limit on OUR outbound bandwidth (§5.4.5's
    /// `SetPeerBandwidth`), tracked separately from `ack_threshold`
    /// (#1108/RTMP-W7): it constrains how much *we send*, not how often
    /// *we acknowledge received bytes* (that's `WindowAckSize`, §5.4.4).
    /// Not otherwise enforced by this sans-IO engine (it doesn't do its
    /// own outbound rate limiting), but kept and exposed rather than
    /// silently discarded or conflated with `ack_threshold`.
    peer_bandwidth: Option<u32>,
    /// The window size we last advertised to the peer via our own
    /// outbound `WindowAckSize` (distinct from `ack_threshold`, which
    /// tracks the PEER's advertised window for OUR acking of THEIR
    /// data — #1108/RTMP-W7).
    advertised_window_ack_size: u32,
    flv: FlvTagWriter,
    stream_is_recorded: bool,
}

impl ClientSession {
    /// Create a new client session.
    #[must_use]
    pub fn new(config: RtmpClientConfig) -> Self {
        let ack_threshold = config.window_ack_size;
        let advertised_window_ack_size = config.window_ack_size;
        let max_in_progress_bytes = config.max_in_progress_bytes;
        Self {
            config,
            handshake: ClientHandshake::new(),
            handshake_buf: Vec::new(),
            assembler: ChunkAssembler::new().with_max_in_progress_bytes(max_in_progress_bytes),
            writer: ChunkWriter::new(),
            state: ClientState::Init,
            stream_id: None,
            next_txn_id: 1.0,
            ack_threshold,
            bytes_received: 0,
            bytes_acked: 0,
            peer_bandwidth: None,
            advertised_window_ack_size,
            flv: FlvTagWriter::default(),
            stream_is_recorded: false,
        }
    }

    /// Produce C0+C1 handshake bytes. Call once at session start.
    pub fn start(&mut self) -> Vec<u8> {
        self.handshake.start()
    }

    /// Feed inbound bytes. Returns `(outbound bytes, events)`.
    pub fn handle_data(&mut self, input: &[u8]) -> Result<(Vec<u8>, Vec<ClientEvent>)> {
        let mut out = Vec::new();
        let mut events = Vec::new();

        let chunk_input = match self.drive_handshake(input, &mut out, &mut events)? {
            Some(bytes) => bytes,
            None => return Ok((out, events)),
        };

        self.bytes_received = self.bytes_received.saturating_add(chunk_input.len() as u64);

        self.assembler.feed(chunk_input);
        while let Some(msg) = self.assembler.next_message()? {
            self.dispatch_message(&msg, &mut out, &mut events)?;
        }

        self.maybe_ack(&mut out)?;
        Ok((out, events))
    }

    /// Encode an audio message (type 8). Only valid in `Publishing` state.
    ///
    /// `timestamp` is the media timestamp on the publisher's own clock
    /// (§5.1.3), carried on the wire as whole milliseconds.
    pub fn send_audio(&mut self, timestamp: Timestamp, data: &[u8]) -> Result<Vec<u8>> {
        if self.state != ClientState::Publishing {
            return Err(RtmpError::UnexpectedState {
                what: "send_audio requires the Publishing state",
            });
        }
        let msg = Message {
            chunk_stream_id: ChunkStreamId::new(PAYLOAD_CHUNK_STREAM_ID),
            timestamp,
            message_type_id: MessageTypeId::Audio,
            message_stream_id: self.stream_id.unwrap_or(MessageStreamId::new(1)),
            payload: data.to_vec(),
        };
        self.writer.write(&msg)
    }

    /// Encode a video message (type 9). Only valid in `Publishing` state.
    ///
    /// `timestamp` is the media timestamp on the publisher's own clock
    /// (§5.1.3), carried on the wire as whole milliseconds.
    pub fn send_video(&mut self, timestamp: Timestamp, data: &[u8]) -> Result<Vec<u8>> {
        if self.state != ClientState::Publishing {
            return Err(RtmpError::UnexpectedState {
                what: "send_video requires the Publishing state",
            });
        }
        let msg = Message {
            chunk_stream_id: ChunkStreamId::new(VIDEO_CHUNK_STREAM_ID),
            timestamp,
            message_type_id: MessageTypeId::Video,
            message_stream_id: self.stream_id.unwrap_or(MessageStreamId::new(1)),
            payload: data.to_vec(),
        };
        self.writer.write(&msg)
    }

    /// Encode a metadata message (`@setDataFrame`/`onMetaData`, type 18).
    pub fn send_metadata(&mut self, metadata: &[(String, Amf0Value)]) -> Result<Vec<u8>> {
        if self.state != ClientState::Publishing {
            return Err(RtmpError::UnexpectedState {
                what: "send_metadata requires the Publishing state",
            });
        }
        let mut payload = Amf0Value::String("@setDataFrame".to_string()).try_to_bytes()?;
        payload.extend(Amf0Value::String("onMetaData".to_string()).try_to_bytes()?);
        let obj = Amf0Value::Object(metadata.to_vec());
        payload.extend(obj.try_to_bytes()?);
        let msg = Message {
            chunk_stream_id: ChunkStreamId::new(PAYLOAD_CHUNK_STREAM_ID),
            timestamp: Timestamp::ZERO,
            message_type_id: MessageTypeId::DataAmf0,
            message_stream_id: self.stream_id.unwrap_or(MessageStreamId::new(1)),
            payload,
        };
        self.writer.write(&msg)
    }

    /// Whether the session has reached `Publishing` state.
    #[must_use]
    pub fn is_publishing(&self) -> bool {
        self.state == ClientState::Publishing
    }

    fn drive_handshake<'a>(
        &mut self,
        input: &'a [u8],
        out: &mut Vec<u8>,
        events: &mut Vec<ClientEvent>,
    ) -> Result<Option<&'a [u8]>> {
        if self.handshake.is_done() && self.state != ClientState::Init {
            return Ok(Some(input));
        }

        self.handshake_buf.extend_from_slice(input);

        if !self.handshake.is_done() {
            match self.handshake.read(&self.handshake_buf) {
                Ok((reply, consumed, done)) => {
                    out.extend_from_slice(&reply);
                    self.handshake_buf.drain(..consumed);
                    if done {
                        self.state = ClientState::HandshakeDone;
                        out.extend_from_slice(&self.build_connect()?);
                    }
                }
                Err(RtmpError::BufferTooShort(..)) => {
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }
        }

        if self.handshake.is_done() && !self.handshake_buf.is_empty() {
            let remaining = core::mem::take(&mut self.handshake_buf);
            self.bytes_received = self.bytes_received.saturating_add(remaining.len() as u64);
            self.assembler.feed(&remaining);
            while let Some(msg) = self.assembler.next_message()? {
                self.dispatch_message(&msg, out, events)?;
            }
        }

        Ok(None)
    }

    fn build_connect(&mut self) -> Result<Vec<u8>> {
        let txn_id = self.next_txn_id;
        self.next_txn_id += 1.0;
        self.state = ClientState::ConnectSent { txn_id };

        let tc_url = self
            .config
            .tc_url
            .clone()
            .unwrap_or_else(|| format!("rtmp://localhost/{}", self.config.app));

        let cmd = Command {
            name: "connect".to_string(),
            transaction_id: txn_id,
            arguments: vec![Amf0Value::Object(vec![
                (
                    "app".to_string(),
                    Amf0Value::String(self.config.app.clone()),
                ),
                (
                    "flashVer".to_string(),
                    Amf0Value::String(FLASH_VER.to_string()),
                ),
                ("tcUrl".to_string(), Amf0Value::String(tc_url)),
                (
                    "type".to_string(),
                    Amf0Value::String("nonprivate".to_string()),
                ),
            ])],
        };

        let mut out = Vec::new();

        let chunk_size = ChunkSize::try_new(self.config.chunk_size)?;
        let set_chunk_size = ProtocolControl::SetChunkSize(chunk_size);
        out.extend_from_slice(&self.writer.write(&set_chunk_size.to_message())?);
        self.writer.set_chunk_size(chunk_size);

        let window_ack = ProtocolControl::WindowAckSize(self.config.window_ack_size);
        out.extend_from_slice(&self.writer.write(&window_ack.to_message())?);

        let msg = Message {
            chunk_stream_id: ChunkStreamId::new(COMMAND_CHUNK_STREAM_ID),
            timestamp: Timestamp::ZERO,
            message_type_id: MessageTypeId::CommandAmf0,
            message_stream_id: MessageStreamId::new(0),
            payload: cmd.to_body()?,
        };
        out.extend_from_slice(&self.writer.write(&msg)?);
        Ok(out)
    }

    /// `releaseStream` + `FCPublish` (each `(null, stream_key)`, fire-and-
    /// forget: their `_result`s carry transaction ids the state machine
    /// never waits on) then `createStream` — the ffmpeg/OBS/FMLE sequence
    /// some CDN ingest front ends require (#1108/RTMP-W9).
    fn build_publish_preamble_and_create_stream(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for name in [CMD_RELEASE_STREAM, CMD_FC_PUBLISH] {
            let txn_id = self.next_txn_id;
            self.next_txn_id += 1.0;
            let cmd = Command {
                name: name.to_string(),
                transaction_id: txn_id,
                arguments: vec![
                    Amf0Value::Null,
                    Amf0Value::String(self.config.stream_key.clone()),
                ],
            };
            let msg = Message {
                chunk_stream_id: ChunkStreamId::new(COMMAND_CHUNK_STREAM_ID),
                timestamp: Timestamp::ZERO,
                message_type_id: MessageTypeId::CommandAmf0,
                message_stream_id: MessageStreamId::new(0),
                payload: cmd.to_body()?,
            };
            out.extend_from_slice(&self.writer.write(&msg)?);
        }
        out.extend_from_slice(&self.build_create_stream()?);
        Ok(out)
    }

    fn build_create_stream(&mut self) -> Result<Vec<u8>> {
        let txn_id = self.next_txn_id;
        self.next_txn_id += 1.0;
        self.state = ClientState::CreateStreamSent { txn_id };

        let cmd = Command {
            name: "createStream".to_string(),
            transaction_id: txn_id,
            arguments: vec![Amf0Value::Null],
        };
        let msg = Message {
            chunk_stream_id: ChunkStreamId::new(COMMAND_CHUNK_STREAM_ID),
            timestamp: Timestamp::ZERO,
            message_type_id: MessageTypeId::CommandAmf0,
            message_stream_id: MessageStreamId::new(0),
            payload: cmd.to_body()?,
        };
        self.writer.write(&msg)
    }

    fn build_publish(&mut self) -> Result<Vec<u8>> {
        self.state = ClientState::PublishSent;

        let cmd = Command {
            name: "publish".to_string(),
            transaction_id: 0.0,
            arguments: vec![
                Amf0Value::Null,
                Amf0Value::String(self.config.stream_key.clone()),
                Amf0Value::String("live".to_string()),
            ],
        };
        let msg = Message {
            chunk_stream_id: ChunkStreamId::new(COMMAND_CHUNK_STREAM_ID),
            timestamp: Timestamp::ZERO,
            message_type_id: MessageTypeId::CommandAmf0,
            message_stream_id: self.stream_id.unwrap_or(MessageStreamId::new(1)),
            payload: cmd.to_body()?,
        };
        self.writer.write(&msg)
    }

    /// `SetBufferLength` (§6.7) then `play` (§7.2.2.1) on the created stream.
    fn build_play(&mut self) -> Result<Vec<u8>> {
        self.state = ClientState::PlaySent;
        let stream_id = self.stream_id.unwrap_or(MessageStreamId::new(1));
        let mut out = Vec::new();
        let buffer = UserControl::SetBufferLength {
            stream_id: stream_id.get(),
            buffer: self.config.play_buffer,
        };
        out.extend_from_slice(&self.writer.write(&buffer.to_message()?)?);
        let cmd = Command {
            name: "play".to_string(),
            transaction_id: 0.0,
            arguments: vec![
                Amf0Value::Null,
                Amf0Value::String(self.config.stream_key.clone()),
                Amf0Value::Number(PLAY_START_LIVE_OR_RECORDED),
            ],
        };
        let msg = Message {
            chunk_stream_id: ChunkStreamId::new(COMMAND_CHUNK_STREAM_ID),
            timestamp: Timestamp::ZERO,
            message_type_id: MessageTypeId::CommandAmf0,
            message_stream_id: stream_id,
            payload: cmd.to_body()?,
        };
        out.extend_from_slice(&self.writer.write(&msg)?);
        Ok(out)
    }

    fn receiving_media(&self) -> bool {
        matches!(self.state, ClientState::PlaySent | ClientState::Playing)
    }

    fn emit_media(
        &mut self,
        tag_type: MessageTypeId,
        timestamp: u32,
        payload: &[u8],
        events: &mut Vec<ClientEvent>,
    ) -> Result<()> {
        let flv = self.flv.tag(tag_type, timestamp, payload)?;
        events.push(ClientEvent::Media { flv });
        Ok(())
    }

    fn dispatch_message(
        &mut self,
        msg: &Message,
        out: &mut Vec<u8>,
        events: &mut Vec<ClientEvent>,
    ) -> Result<()> {
        if let Some(pc) = ProtocolControl::from_message(msg)? {
            match pc {
                ProtocolControl::SetChunkSize(size) => {
                    self.assembler.set_chunk_size(size);
                }
                ProtocolControl::WindowAckSize(size) => {
                    self.ack_threshold = size;
                }
                ProtocolControl::SetPeerBandwidth {
                    ack_window_size, ..
                } => {
                    // #1108/RTMP-W7: §5.4.5 limits the RECEIVER's (i.e.
                    // our) *output* bandwidth — it says nothing about how
                    // often we acknowledge inbound bytes, which is
                    // `WindowAckSize`'s job (§5.4.4) and already handled by
                    // the arm above. This used to overwrite `ack_threshold`
                    // with the peer's bandwidth-limit value, and echo that
                    // same value back as if it were our own advertised
                    // window. Track it separately; reply with our OWN
                    // configured window size, and only when it actually
                    // changed from what we last advertised (private/specs-verified/rtmp/rtmp-runtime/rtmp.md
                    // §5.4.5: "Receiver SHOULD reply with Window Ack Size
                    // if its window differs from the last one it
                    // advertised").
                    self.peer_bandwidth = Some(ack_window_size);
                    if self.advertised_window_ack_size != self.config.window_ack_size {
                        self.advertised_window_ack_size = self.config.window_ack_size;
                        let ack = ProtocolControl::WindowAckSize(self.advertised_window_ack_size);
                        out.extend_from_slice(&self.writer.write(&ack.to_message())?);
                    }
                }
                // #1108/RTMP-W2: was ignored — see `ChunkAssembler::abort`'s doc.
                ProtocolControl::Abort { chunk_stream_id } => {
                    self.assembler.abort(ChunkStreamId::new(chunk_stream_id));
                }
                _ => {}
            }
            return Ok(());
        }
        if msg.message_type_id == MessageTypeId::CommandAmf0 {
            let cmd = Command::parse(&msg.payload)?;
            self.handle_command(&cmd, out, events)?;
        }
        if msg.message_type_id == MessageTypeId::CommandAmf3 {
            let body = msg
                .payload
                .get(1..)
                .ok_or(RtmpError::BufferTooShort(BufferTooShort {
                    need: 1,
                    have: msg.payload.len(),
                    what: "AMF3 command encoding marker byte",
                }))?;
            let cmd = Command::parse(body)?;
            self.handle_command(&cmd, out, events)?;
        }
        if self.receiving_media() {
            match msg.message_type_id {
                MessageTypeId::Audio | MessageTypeId::Video | MessageTypeId::DataAmf0 => {
                    self.emit_media(
                        msg.message_type_id,
                        msg.timestamp.millis(),
                        &msg.payload,
                        events,
                    )?;
                }
                MessageTypeId::Aggregate => {
                    unpack_aggregate(msg, |tag_type, timestamp, payload| {
                        self.emit_media(tag_type, timestamp, payload, events)
                    })?;
                }
                _ => {}
            }
        }
        if msg.message_type_id == MessageTypeId::UserControl {
            // #1108/RTMP-W8: servers that probe liveness with User Control
            // PingRequest (event 6 — FMS/Wowza) got no PingResponse and may
            // have dropped the publisher; malformed/unrecognised event
            // types are tolerated, same as the server side's inbound
            // handling.
            match UserControl::parse(&msg.payload) {
                Ok(UserControl::PingRequest(ts)) => {
                    let reply = UserControl::PingResponse(ts);
                    out.extend_from_slice(&self.writer.write(&reply.to_message()?)?);
                }
                Ok(UserControl::StreamIsRecorded(_)) if self.receiving_media() => {
                    self.stream_is_recorded = true;
                }
                Ok(UserControl::StreamEof(_)) if self.receiving_media() => {
                    events.push(ClientEvent::StreamEnded);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_command(
        &mut self,
        cmd: &Command,
        out: &mut Vec<u8>,
        events: &mut Vec<ClientEvent>,
    ) -> Result<()> {
        match cmd.name.as_str() {
            "_result" => self.handle_result(cmd, out, events),
            "_error" => {
                let (code, desc) = extract_status(&cmd.arguments);
                events.push(ClientEvent::Error {
                    code,
                    description: desc,
                });
                Ok(())
            }
            "onStatus" => self.handle_on_status(cmd, events),
            _ => Ok(()),
        }
    }

    fn handle_result(
        &mut self,
        cmd: &Command,
        out: &mut Vec<u8>,
        events: &mut Vec<ClientEvent>,
    ) -> Result<()> {
        match self.state {
            ClientState::ConnectSent { txn_id }
                if (cmd.transaction_id - txn_id).abs() < TRANSACTION_ID_TOLERANCE =>
            {
                let props = match cmd.arguments.first() {
                    Some(Amf0Value::Object(pairs)) => pairs.clone(),
                    _ => Vec::new(),
                };
                events.push(ClientEvent::Connected { properties: props });
                self.state = ClientState::Connected;
                match self.config.mode {
                    ClientMode::Play => out.extend_from_slice(&self.build_create_stream()?),
                    _ => out.extend_from_slice(&self.build_publish_preamble_and_create_stream()?),
                }
            }
            ClientState::CreateStreamSent { txn_id }
                if (cmd.transaction_id - txn_id).abs() < TRANSACTION_ID_TOLERANCE =>
            {
                let sid = MessageStreamId::new(match cmd.arguments.last() {
                    Some(Amf0Value::Number(n)) => *n as u32,
                    _ => 1,
                });
                self.stream_id = Some(sid);
                events.push(ClientEvent::StreamCreated {
                    stream_id: sid.get(),
                });
                self.state = ClientState::StreamCreated;
                match self.config.mode {
                    ClientMode::Play => out.extend_from_slice(&self.build_play()?),
                    _ => out.extend_from_slice(&self.build_publish()?),
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_on_status(&mut self, cmd: &Command, events: &mut Vec<ClientEvent>) -> Result<()> {
        let (code, desc) = extract_status(&cmd.arguments);
        if self.config.mode == ClientMode::Play {
            let is_error = extract_status_level(&cmd.arguments) == STATUS_LEVEL_ERROR
                || code == STATUS_PLAY_STREAM_NOT_FOUND
                || code == STATUS_PLAY_FAILED;
            if is_error {
                events.push(ClientEvent::Error {
                    code,
                    description: desc,
                });
            } else if code == STATUS_PLAY_START {
                self.state = ClientState::Playing;
                events.push(ClientEvent::Playing {
                    recorded: self.stream_is_recorded,
                });
            } else if code == STATUS_PLAY_STOP || code == STATUS_PLAY_COMPLETE {
                events.push(ClientEvent::StreamEnded);
            }
            return Ok(());
        }
        if code == "NetStream.Publish.Start" {
            self.state = ClientState::Publishing;
            events.push(ClientEvent::Publishing);
        } else {
            events.push(ClientEvent::Error {
                code,
                description: desc,
            });
        }
        Ok(())
    }

    fn maybe_ack(&mut self, out: &mut Vec<u8>) -> Result<()> {
        if self.ack_threshold == 0 {
            return Ok(());
        }
        if self.bytes_received.wrapping_sub(self.bytes_acked) >= u64::from(self.ack_threshold) {
            let seq = SequenceNumber::new((self.bytes_received & SEQUENCE_NUMBER_MASK) as u32);
            let ack = ProtocolControl::Acknowledgement(seq);
            out.extend_from_slice(&self.writer.write(&ack.to_message())?);
            self.bytes_acked = self.bytes_received;
        }
        Ok(())
    }
}

fn extract_status_level(args: &[Amf0Value]) -> String {
    args.iter()
        .find_map(|arg| match arg {
            Amf0Value::Object(pairs) => pairs.iter().find_map(|(k, v)| match (k.as_str(), v) {
                ("level", Amf0Value::String(s)) => Some(s.clone()),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_default()
}

fn extract_status(args: &[Amf0Value]) -> (String, String) {
    for arg in args {
        if let Amf0Value::Object(pairs) = arg {
            let code = pairs
                .iter()
                .find(|(k, _)| k == "code")
                .and_then(|(_, v)| match v {
                    Amf0Value::String(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let desc = pairs
                .iter()
                .find(|(k, _)| k == "description")
                .and_then(|(_, v)| match v {
                    Amf0Value::String(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            if !code.is_empty() {
                return (code, desc);
            }
        }
    }
    (String::new(), String::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handshake;
    use crate::server::{RtmpServerConfig, ServerSession};

    #[test]
    fn client_handshake_round_trip() {
        let mut client_hs = ClientHandshake::new();
        let mut server_hs = handshake::Handshake::new();

        let c0_c1 = client_hs.start();
        assert!(!client_hs.is_done());

        let (s0_s1_s2, consumed, _done) = server_hs.read(&c0_c1).unwrap();
        assert_eq!(consumed, c0_c1.len());

        let (c2, consumed2, done2) = client_hs.read(&s0_s1_s2).unwrap();
        assert_eq!(consumed2, s0_s1_s2.len());
        assert!(done2);
        assert!(client_hs.is_done());
        assert_eq!(c2.len(), HANDSHAKE_PACKET_LEN);

        let (_reply, consumed3, done3) = server_hs.read(&c2).unwrap();
        assert_eq!(consumed3, HANDSHAKE_PACKET_LEN);
        assert!(done3);
    }

    /// RTMP-W11 (#1108): S0's version byte used to be parsed and discarded.
    #[test]
    fn client_handshake_rejects_bad_s0_version() {
        let mut client_hs = ClientHandshake::new();
        let mut server_hs = handshake::Handshake::new();
        let c0_c1 = client_hs.start();
        let (mut s0_s1_s2, _consumed, _done) = server_hs.read(&c0_c1).unwrap();
        s0_s1_s2[0] = 6; // RTMPE, not plain RTMP
        let err = client_hs.read(&s0_s1_s2).unwrap_err();
        assert!(matches!(
            err,
            RtmpError::HandshakeVersion {
                value: 6,
                expected: RTMP_VERSION
            }
        ));
    }

    /// RTMP-W9 (#1108): after `connect`'s `_result` the client must send
    /// `releaseStream` -> `FCPublish` -> `createStream` (ffmpeg/OBS/FMLE
    /// order), and `connect` must carry `flashVer`. Fails without the fix
    /// (the client sent a bare `createStream`).
    #[test]
    fn client_publish_sequence_mirrors_ffmpeg() {
        let config = RtmpClientConfig {
            app: "live".to_string(),
            stream_key: "test_key".to_string(),
            ..RtmpClientConfig::default()
        };
        let mut client = ClientSession::new(config);
        let mut server = ServerSession::new(
            RtmpServerConfig::default().with_expected_stream_key(Some("test_key".to_string())),
        );
        let c0_c1 = client.start();
        let (s0_s1_s2, _) = server.handle_data(&c0_c1).unwrap();
        let (mut to_server, _) = client.handle_data(&s0_s1_s2).unwrap();
        let mut names = Vec::new();
        let mut connect_has_flash_ver = false;
        let mut sniff = ChunkAssembler::new();
        for round in 0..4 {
            // Round 0's output starts with C2, which is not chunk data.
            let skip = if round == 0 { HANDSHAKE_PACKET_LEN } else { 0 };
            sniff.set_chunk_size(
                crate::message::ChunkSize::new(client.config.chunk_size)
                    .expect("the client's own configured size is validated before connect"),
            );
            for m in sniff.push(&to_server[skip..]).unwrap() {
                if m.message_type_id == MessageTypeId::CommandAmf0 {
                    let c = Command::parse(&m.payload).unwrap();
                    if c.name == "connect" {
                        connect_has_flash_ver = matches!(
                            c.arguments.first(),
                            Some(Amf0Value::Object(p)) if p.iter().any(|(k, _)| k == "flashVer")
                        );
                    }
                    names.push(c.name);
                }
            }
            let (to_client, _) = server.handle_data(&to_server).unwrap();
            let (next, _) = client.handle_data(&to_client).unwrap();
            to_server = next;
        }
        assert!(connect_has_flash_ver);
        assert_eq!(
            names,
            [
                "connect",
                "releaseStream",
                "FCPublish",
                "createStream",
                "publish"
            ]
        );
        assert!(client.is_publishing());
    }

    /// RTMP-W8 (#1108): the client never answered a User Control
    /// PingRequest (event 6 — FMS/Wowza liveness probe), so a server that
    /// probes with it got no PingResponse and may drop the publisher.
    #[test]
    fn client_replies_to_ping_request_with_ping_response() {
        let config = RtmpClientConfig {
            app: "live".to_string(),
            stream_key: "test_key".to_string(),
            ..RtmpClientConfig::default()
        };
        let mut client = ClientSession::new(config);
        let c0_c1 = client.start();
        let mut server = ServerSession::new(
            RtmpServerConfig::default().with_expected_stream_key(Some("test_key".to_string())),
        );
        let (s0_s1_s2, _) = server.handle_data(&c0_c1).unwrap();
        let (_client_out, _) = client.handle_data(&s0_s1_s2).unwrap();

        let ping = UserControl::PingRequest(0xDEAD_BEEF);
        let bytes = ChunkWriter::new()
            .write(&ping.to_message().unwrap())
            .unwrap();

        let (out, _events) = client.handle_data(&bytes).unwrap();
        let mut assembler = ChunkAssembler::new();
        let messages = assembler.push(&out).unwrap();
        let reply = messages
            .iter()
            .find(|m| m.message_type_id == MessageTypeId::UserControl)
            .map(|m| UserControl::parse(&m.payload).unwrap());
        assert_eq!(reply, Some(UserControl::PingResponse(0xDEAD_BEEF)));
    }

    /// RTMP-W7 (#1108): `SetPeerBandwidth` used to overwrite `ack_threshold`
    /// with the peer's declared OUTBOUND bandwidth limit — a value with no
    /// relation to how often we should acknowledge inbound bytes
    /// (`WindowAckSize`'s job, §5.4.4). It's now tracked separately in
    /// `peer_bandwidth`, and `ack_threshold` is untouched.
    #[test]
    fn set_peer_bandwidth_does_not_corrupt_ack_threshold() {
        let config = RtmpClientConfig {
            app: "live".to_string(),
            stream_key: "test_key".to_string(),
            ..RtmpClientConfig::default()
        };
        let original_ack_threshold_config = config.window_ack_size;
        let mut client = ClientSession::new(config.clone());
        assert_eq!(client.ack_threshold, original_ack_threshold_config);

        // Complete the handshake so `handle_data` dispatches chunk messages
        // (rather than still buffering handshake bytes).
        let c0_c1 = client.start();
        let mut server = ServerSession::new(
            RtmpServerConfig::default().with_expected_stream_key(Some("test_key".to_string())),
        );
        let (s0_s1_s2, _) = server.handle_data(&c0_c1).unwrap();
        client.handle_data(&s0_s1_s2).unwrap();

        let spb = crate::message::ProtocolControl::SetPeerBandwidth {
            ack_window_size: 999_999,
            limit_type: crate::message::LimitType::Hard,
        };
        let bytes = ChunkWriter::new().write(&spb.to_message()).unwrap();
        let (_out, _events) = client.handle_data(&bytes).unwrap();

        assert_eq!(
            client.ack_threshold, original_ack_threshold_config,
            "ack_threshold must not be overwritten by SetPeerBandwidth"
        );
        assert_eq!(client.peer_bandwidth, Some(999_999));
    }

    /// The invalid-chunk-size rejection on the wire: a client configured
    /// with a size §5.4.1 forbids fails its `connect` with the typed
    /// `RtmpError::Malformed`, before any `SetChunkSize` message is built,
    /// and emits nothing. The former `try_to_message` fold-panic is
    /// unreachable now that `ChunkSize` validates at construction.
    #[test]
    fn client_connect_with_invalid_chunk_size_errors_before_any_message() {
        let config = RtmpClientConfig {
            app: "live".to_string(),
            stream_key: "test_key".to_string(),
            chunk_size: 0x8000_0000,
            ..RtmpClientConfig::default()
        };
        let mut client = ClientSession::new(config);
        let mut server = ServerSession::new(RtmpServerConfig::default());
        let c0_c1 = client.start();
        let (s0_s1_s2, _) = server.handle_data(&c0_c1).unwrap();
        let err = client.handle_data(&s0_s1_s2).unwrap_err();
        assert!(
            matches!(err, RtmpError::ChunkSizeOutOfRange { value: 0x8000_0000 }),
            "{err:?}"
        );
        assert!(!client.is_publishing());
    }

    #[test]
    fn client_connect_flow() {
        let config = RtmpClientConfig {
            app: "live".to_string(),
            stream_key: "test_key".to_string(),
            ..RtmpClientConfig::default()
        };
        let mut client = ClientSession::new(config);
        let c0_c1 = client.start();

        let mut server = ServerSession::new(
            RtmpServerConfig::default().with_expected_stream_key(Some("test_key".to_string())),
        );

        let (s0_s1_s2, server_events) = server.handle_data(&c0_c1).unwrap();
        assert!(server_events.is_empty());

        let (client_out, _client_events) = client.handle_data(&s0_s1_s2).unwrap();
        assert!(
            !client_out.is_empty(),
            "client should emit C2 + connect + protocol control"
        );

        let (server_reply, server_events) = server.handle_data(&client_out).unwrap();
        assert!(
            server_events
                .iter()
                .any(|e| matches!(e, crate::server::ServerEvent::Connected { .. })),
            "server should see connect: {server_events:?}"
        );

        let (client_out2, client_events2) = client.handle_data(&server_reply).unwrap();
        assert!(
            client_events2
                .iter()
                .any(|e| matches!(e, ClientEvent::Connected { .. })),
            "client should see Connected: {client_events2:?}"
        );
        // `client_out2` alone is not a reliable auto-advance signal: the
        // server's connect reply also carries a `SetPeerBandwidth` protocol
        // control message, which the client acks unconditionally — so bytes
        // come back even if `createStream` itself never fires. Check the
        // actual state transition instead (issue #933 finding M2).
        let _ = client_out2;
        assert!(
            matches!(client.state, ClientState::CreateStreamSent { .. }),
            "client should auto-advance from Connected to CreateStreamSent: {:?}",
            client.state
        );
    }

    /// Drives the full client auto-advance —
    /// `connect` -> `createStream` -> `publish` — end to end against a real
    /// [`ServerSession`], and asserts every state transition actually
    /// happens (issue #933 finding M2: the prior version of this test
    /// buried its post-`Connected` assertions inside
    /// `if !client_out2.is_empty() { .. }` guards, so a broken auto-advance
    /// that emitted no bytes would make the test vacuously pass instead of
    /// fail).
    #[test]
    fn client_full_publish_flow() {
        let config = RtmpClientConfig {
            app: "live".to_string(),
            stream_key: "test_key".to_string(),
            ..RtmpClientConfig::default()
        };
        let mut client = ClientSession::new(config);
        let c0_c1 = client.start();

        let mut server = ServerSession::new(
            RtmpServerConfig::default().with_expected_stream_key(Some("test_key".to_string())),
        );

        // Handshake.
        let (s0_s1_s2, server_events) = server.handle_data(&c0_c1).unwrap();
        assert!(server_events.is_empty());

        // Client completes the handshake and sends C2 + connect.
        let (client_out, client_events) = client.handle_data(&s0_s1_s2).unwrap();
        assert!(
            !client_out.is_empty(),
            "client should emit C2 + connect + protocol control"
        );
        assert!(
            client_events.is_empty(),
            "no client events expected before the server replies: {client_events:?}"
        );
        assert_eq!(client.state, ClientState::ConnectSent { txn_id: 1.0 });

        // Server completes the handshake and accepts connect.
        let (server_reply, server_events) = server.handle_data(&client_out).unwrap();
        assert!(
            server_events
                .iter()
                .any(|e| matches!(e, crate::server::ServerEvent::Connected { .. })),
            "server should see connect: {server_events:?}"
        );

        // Client sees Connected and auto-advances: emits createStream.
        let (client_out2, client_events2) = client.handle_data(&server_reply).unwrap();
        assert!(
            client_events2
                .iter()
                .any(|e| matches!(e, ClientEvent::Connected { .. })),
            "client should see Connected: {client_events2:?}"
        );
        assert!(
            !client_out2.is_empty(),
            "client should auto-advance from Connected and emit createStream"
        );
        assert!(matches!(client.state, ClientState::CreateStreamSent { .. }));

        // Server replies to createStream with an allocated stream id.
        let (server_reply2, _server_events2) = server.handle_data(&client_out2).unwrap();
        assert!(
            !server_reply2.is_empty(),
            "server should reply to createStream"
        );

        // Client sees StreamCreated and auto-advances: emits publish.
        let (client_out3, client_events3) = client.handle_data(&server_reply2).unwrap();
        assert!(
            client_events3
                .iter()
                .any(|e| matches!(e, ClientEvent::StreamCreated { .. })),
            "client should see StreamCreated: {client_events3:?}"
        );
        assert!(
            !client_out3.is_empty(),
            "client should auto-advance from StreamCreated and emit publish"
        );
        assert_eq!(client.state, ClientState::PublishSent);

        // Server accepts publish.
        let (server_reply3, server_events3) = server.handle_data(&client_out3).unwrap();
        assert!(
            server_events3
                .iter()
                .any(|e| matches!(e, crate::server::ServerEvent::Publish { .. })),
            "server should see publish: {server_events3:?}"
        );
        assert!(
            !server_reply3.is_empty(),
            "server should reply onStatus for publish"
        );

        // Client sees Publishing — the auto-advance has run to completion.
        let (_client_out4, client_events4) = client.handle_data(&server_reply3).unwrap();
        assert!(
            client_events4
                .iter()
                .any(|e| matches!(e, ClientEvent::Publishing)),
            "client should see Publishing: {client_events4:?}"
        );
        assert_eq!(client.state, ClientState::Publishing);
        assert!(client.is_publishing());
    }

    #[test]
    fn send_audio_video_while_publishing() {
        let config = RtmpClientConfig {
            app: "live".to_string(),
            stream_key: "test".to_string(),
            ..RtmpClientConfig::default()
        };
        let mut client = ClientSession::new(config);
        client.state = ClientState::Publishing;
        client.stream_id = Some(MessageStreamId::new(1));

        let audio = client
            .send_audio(Timestamp::from_millis(100), &[0xAA, 0xBB])
            .unwrap();
        assert!(!audio.is_empty());

        let video = client
            .send_video(Timestamp::from_millis(200), &[0xCC, 0xDD, 0xEE])
            .unwrap();
        assert!(!video.is_empty());

        let meta = client
            .send_metadata(&[
                ("width".to_string(), Amf0Value::Number(1920.0)),
                ("height".to_string(), Amf0Value::Number(1080.0)),
            ])
            .unwrap();
        assert!(!meta.is_empty());
    }

    /// T15: an Amf0 String over the u16 length cap inside the metadata
    /// object must surface as `Err`, not a `to_bytes` panic.
    #[test]
    fn send_metadata_over_range_string_errors_not_panics() {
        let mut client = ClientSession::new(RtmpClientConfig {
            app: "live".to_string(),
            stream_key: "test".to_string(),
            ..RtmpClientConfig::default()
        });
        client.state = ClientState::Publishing;
        client.stream_id = Some(MessageStreamId::new(1));
        let huge = "x".repeat(usize::from(u16::MAX) + 1);
        let r = client.send_metadata(&[("title".to_string(), Amf0Value::String(huge))]);
        assert!(
            matches!(r, Err(RtmpError::Amf0StringTooLong { .. })),
            "{r:?}"
        );
    }

    #[test]
    fn send_audio_before_publishing_fails() {
        let config = RtmpClientConfig {
            app: "live".to_string(),
            stream_key: "test".to_string(),
            ..RtmpClientConfig::default()
        };
        let mut client = ClientSession::new(config);
        assert!(matches!(
            client.send_audio(Timestamp::from_millis(0), &[0x00]),
            Err(crate::error::RtmpError::UnexpectedState { .. })
        ));
    }

    #[test]
    fn connect_error_produces_event() {
        let config = RtmpClientConfig {
            app: "live".to_string(),
            stream_key: "test".to_string(),
            ..RtmpClientConfig::default()
        };
        let mut client = ClientSession::new(config);
        client.handshake.state = ClientHandshakeState::Done;
        client.state = ClientState::ConnectSent { txn_id: 1.0 };

        let error_cmd = Command {
            name: "_error".to_string(),
            transaction_id: 1.0,
            arguments: vec![
                Amf0Value::Null,
                Amf0Value::Object(vec![
                    (
                        "code".to_string(),
                        Amf0Value::String("NetConnection.Connect.Rejected".to_string()),
                    ),
                    (
                        "description".to_string(),
                        Amf0Value::String("Connection refused".to_string()),
                    ),
                ]),
            ],
        };
        let msg = Message {
            chunk_stream_id: ChunkStreamId::new(COMMAND_CHUNK_STREAM_ID),
            timestamp: Timestamp::ZERO,
            message_type_id: MessageTypeId::CommandAmf0,
            message_stream_id: MessageStreamId::new(0),
            payload: error_cmd.to_body().unwrap(),
        };
        let bytes = ChunkWriter::new().write(&msg).unwrap();

        let (_, events) = client.handle_data(&bytes).unwrap();
        assert!(
            events.iter().any(|e| matches!(
                e,
                ClientEvent::Error { code, .. } if code == "NetConnection.Connect.Rejected"
            )),
            "expected Error event: {events:?}"
        );
    }

    // ── Play mode (audit 22, finding 1) ───────────────────────────────────

    fn play_config() -> RtmpClientConfig {
        RtmpClientConfig {
            mode: ClientMode::Play,
            app: "live".to_string(),
            stream_key: "stream_key".to_string(),
            ..RtmpClientConfig::default()
        }
    }

    fn server_properties() -> Vec<(String, Amf0Value)> {
        vec![
            (
                "fmsVer".to_string(),
                Amf0Value::String("FMS/3,5,3,888".to_string()),
            ),
            ("capabilities".to_string(), Amf0Value::Number(31.0)),
        ]
    }

    fn connect_result(transaction_id: f64) -> Command {
        Command {
            name: "_result".to_string(),
            transaction_id,
            arguments: vec![Amf0Value::Object(server_properties())],
        }
    }

    fn create_stream_result(transaction_id: f64, stream_id: f64) -> Command {
        Command {
            name: "_result".to_string(),
            transaction_id,
            arguments: vec![Amf0Value::Null, Amf0Value::Number(stream_id)],
        }
    }

    fn on_status(code: &str, level: &str, description: &str) -> Command {
        Command {
            name: "onStatus".to_string(),
            transaction_id: 0.0,
            arguments: vec![
                Amf0Value::Null,
                Amf0Value::Object(vec![
                    ("level".to_string(), Amf0Value::String(level.to_string())),
                    ("code".to_string(), Amf0Value::String(code.to_string())),
                    (
                        "description".to_string(),
                        Amf0Value::String(description.to_string()),
                    ),
                ]),
            ],
        }
    }

    fn command_bytes(cmd: &Command) -> Vec<u8> {
        let msg = Message {
            chunk_stream_id: ChunkStreamId::new(COMMAND_CHUNK_STREAM_ID),
            timestamp: Timestamp::ZERO,
            message_type_id: MessageTypeId::CommandAmf0,
            message_stream_id: MessageStreamId::new(0),
            payload: cmd.to_body().unwrap(),
        };
        ChunkWriter::new().write(&msg).unwrap()
    }

    fn user_control_bytes(event: UserControl) -> Vec<u8> {
        ChunkWriter::new()
            .write(&event.to_message().unwrap())
            .unwrap()
    }

    fn media_bytes(
        tag_type: MessageTypeId,
        timestamp: u32,
        payload: &[u8],
        chunk_stream_id: u32,
    ) -> Vec<u8> {
        let msg = Message {
            chunk_stream_id: ChunkStreamId::new(chunk_stream_id),
            timestamp: Timestamp::from_millis(timestamp),
            message_type_id: tag_type,
            message_stream_id: MessageStreamId::new(1),
            payload: payload.to_vec(),
        };
        ChunkWriter::new().write(&msg).unwrap()
    }

    /// Client-outbound messages are all framed at the client's own configured
    /// chunk size, which `build_connect` adopts before anything else.
    fn decode_client_messages(bytes: &[u8]) -> Vec<Message> {
        let mut assembler = ChunkAssembler::new();
        assembler.set_chunk_size(ChunkSize::new(4096).unwrap());
        assembler.push(bytes).unwrap()
    }

    fn commands(bytes: &[u8]) -> Vec<Command> {
        decode_client_messages(bytes)
            .iter()
            .filter(|m| m.message_type_id == MessageTypeId::CommandAmf0)
            .map(|m| Command::parse(&m.payload).unwrap())
            .collect()
    }

    fn ack_in(out: &[u8]) -> Option<u32> {
        decode_client_messages(out).into_iter().find_map(|m| {
            match ProtocolControl::from_message(&m).unwrap() {
                Some(ProtocolControl::Acknowledgement(seq)) => Some(seq.get()),
                _ => None,
            }
        })
    }

    /// The FLV tag Adobe FLV v10.1 Annex E.4.1 defines for these values,
    /// written out from the spec layout (not through the writer under test).
    fn expected_flv_tag(
        file_header: bool,
        tag_type: u8,
        timestamp: u32,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        if file_header {
            out.extend_from_slice(b"FLV");
            out.extend_from_slice(&[0x01, 0x05]);
            out.extend_from_slice(&[0x00, 0x00, 0x00, 0x09]);
            out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        }
        let data_size = payload.len() as u32;
        out.push(tag_type);
        out.extend_from_slice(&[
            (data_size >> 16) as u8,
            (data_size >> 8) as u8,
            data_size as u8,
        ]);
        out.extend_from_slice(&[
            (timestamp >> 16) as u8,
            (timestamp >> 8) as u8,
            timestamp as u8,
        ]);
        out.push((timestamp >> 24) as u8);
        out.extend_from_slice(&[0x00, 0x00, 0x00]);
        out.extend_from_slice(payload);
        out.extend_from_slice(&(11u32 + data_size).to_be_bytes());
        out
    }

    fn amf0_string(s: &str) -> Vec<u8> {
        let mut out = vec![0x02];
        out.extend_from_slice(&(s.len() as u16).to_be_bytes());
        out.extend_from_slice(s.as_bytes());
        out
    }

    fn aggregate_sub_tag(tag_type: u8, timestamp: u32, payload: &[u8]) -> Vec<u8> {
        let data_size = payload.len() as u32;
        let mut out = vec![tag_type];
        out.extend_from_slice(&[
            (data_size >> 16) as u8,
            (data_size >> 8) as u8,
            data_size as u8,
        ]);
        out.extend_from_slice(&[
            (timestamp >> 16) as u8,
            (timestamp >> 8) as u8,
            timestamp as u8,
        ]);
        out.push((timestamp >> 24) as u8);
        out.extend_from_slice(&[0x00, 0x00, 0x00]);
        out.extend_from_slice(payload);
        out.extend_from_slice(&(11u32 + data_size).to_be_bytes());
        out
    }

    fn aggregate_bytes(timestamp: u32, sub_tags: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = Vec::new();
        for sub_tag in sub_tags {
            payload.extend_from_slice(sub_tag);
        }
        let msg = Message {
            chunk_stream_id: ChunkStreamId::new(4),
            timestamp: Timestamp::from_millis(timestamp),
            message_type_id: MessageTypeId::Aggregate,
            message_stream_id: MessageStreamId::new(1),
            payload,
        };
        ChunkWriter::new().write(&msg).unwrap()
    }

    /// Client that has completed the handshake and sent `connect` (state
    /// `ConnectSent { txn_id: 1.0 }`).
    fn started_play_client() -> ClientSession {
        let mut client = ClientSession::new(play_config());
        let c0_c1 = client.start();
        let mut server_hs = handshake::Handshake::new();
        let (s0_s1_s2, consumed, _) = server_hs.read(&c0_c1).unwrap();
        assert_eq!(consumed, c0_c1.len());
        let (out, events) = client.handle_data(&s0_s1_s2).unwrap();
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(client.state, ClientState::ConnectSent { txn_id: 1.0 });
        let sent = commands(&out[HANDSHAKE_PACKET_LEN..]);
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0].name, "connect");
        assert_eq!(sent[0].transaction_id, 1.0);
        client
    }

    fn play_client_at_play_sent() -> ClientSession {
        let mut client = started_play_client();
        client
            .handle_data(&command_bytes(&connect_result(1.0)))
            .unwrap();
        client
            .handle_data(&command_bytes(&create_stream_result(2.0, 1.0)))
            .unwrap();
        assert_eq!(client.state, ClientState::PlaySent);
        client
    }

    #[test]
    fn client_mode_names_are_the_netstream_commands() {
        assert_eq!(ClientMode::Publish.name(), "publish");
        assert_eq!(ClientMode::Play.name(), "play");
        assert_eq!(ClientMode::Publish.to_string(), "publish");
        assert_eq!(ClientMode::Play.to_string(), "play");
    }

    #[test]
    fn play_mode_runs_create_stream_then_set_buffer_and_play() {
        let mut client = started_play_client();
        let (out, events) = client
            .handle_data(&command_bytes(&connect_result(1.0)))
            .unwrap();
        assert_eq!(
            events,
            vec![ClientEvent::Connected {
                properties: server_properties(),
            }]
        );
        assert_eq!(client.state, ClientState::CreateStreamSent { txn_id: 2.0 });
        assert_eq!(
            commands(&out),
            vec![Command {
                name: "createStream".to_string(),
                transaction_id: 2.0,
                arguments: vec![Amf0Value::Null],
            }]
        );

        let (out, events) = client
            .handle_data(&command_bytes(&create_stream_result(2.0, 3.0)))
            .unwrap();
        assert_eq!(events, vec![ClientEvent::StreamCreated { stream_id: 3 }]);
        assert_eq!(client.state, ClientState::PlaySent);
        assert_eq!(client.next_txn_id, 3.0);

        let messages = decode_client_messages(&out);
        assert_eq!(
            messages
                .iter()
                .find_map(|m| UserControl::parse(&m.payload).ok()),
            Some(UserControl::SetBufferLength {
                stream_id: 3,
                buffer: core::time::Duration::from_secs(3),
            })
        );
        let play = commands(&out);
        assert_eq!(play.len(), 1, "{play:?}");
        assert_eq!(play[0].name, "play");
        assert_eq!(play[0].transaction_id, 0.0);
        assert_eq!(
            play[0].arguments,
            vec![
                Amf0Value::Null,
                Amf0Value::String("stream_key".to_string()),
                Amf0Value::Number(-2.0),
            ]
        );
        let play_message = messages
            .iter()
            .find(|m| m.message_type_id == MessageTypeId::CommandAmf0)
            .unwrap();
        assert_eq!(play_message.message_stream_id, MessageStreamId::new(3));
    }

    #[test]
    fn result_transaction_ids_must_match_the_pending_request() {
        let mut client = started_play_client();
        let (out, events) = client
            .handle_data(&command_bytes(&connect_result(7.0)))
            .unwrap();
        assert!(events.is_empty(), "{events:?}");
        assert!(out.is_empty());
        assert_eq!(client.state, ClientState::ConnectSent { txn_id: 1.0 });

        let (out, events) = client
            .handle_data(&command_bytes(&connect_result(1.0)))
            .unwrap();
        assert_eq!(
            events,
            vec![ClientEvent::Connected {
                properties: server_properties(),
            }]
        );
        assert_eq!(client.state, ClientState::CreateStreamSent { txn_id: 2.0 });
        assert_eq!(commands(&out)[0].name, "createStream");

        let (out, events) = client
            .handle_data(&command_bytes(&create_stream_result(9.0, 5.0)))
            .unwrap();
        assert!(events.is_empty(), "{events:?}");
        assert!(out.is_empty());
        assert_eq!(client.state, ClientState::CreateStreamSent { txn_id: 2.0 });

        let (out, events) = client
            .handle_data(&command_bytes(&create_stream_result(2.0, 5.0)))
            .unwrap();
        assert_eq!(events, vec![ClientEvent::StreamCreated { stream_id: 5 }]);
        let play_message = decode_client_messages(&out)
            .into_iter()
            .find(|m| m.message_type_id == MessageTypeId::CommandAmf0)
            .unwrap();
        assert_eq!(play_message.message_stream_id, MessageStreamId::new(5));
    }

    #[test]
    fn play_mode_delivers_audio_video_and_data_as_flv_tags() {
        let mut client = started_play_client();
        let audio = media_bytes(MessageTypeId::Audio, 7, &[0xAF, 0x00], 4);
        let (out, events) = client.handle_data(&audio).unwrap();
        assert!(
            events.is_empty(),
            "state Connected receives no media: {events:?}"
        );
        assert!(out.is_empty());

        client
            .handle_data(&command_bytes(&connect_result(1.0)))
            .unwrap();
        client
            .handle_data(&command_bytes(&create_stream_result(2.0, 1.0)))
            .unwrap();
        assert_eq!(client.state, ClientState::PlaySent);

        let (_, events) = client.handle_data(&audio).unwrap();
        assert_eq!(
            events,
            vec![ClientEvent::Media {
                flv: expected_flv_tag(true, 8, 7, &[0xAF, 0x00]),
            }]
        );
        let (_, events) = client
            .handle_data(&media_bytes(
                MessageTypeId::Video,
                40,
                &[0x17, 0x01, 0xAA],
                5,
            ))
            .unwrap();
        assert_eq!(
            events,
            vec![ClientEvent::Media {
                flv: expected_flv_tag(false, 9, 40, &[0x17, 0x01, 0xAA]),
            }]
        );
        let meta = amf0_string("onMetaData");
        let (_, events) = client
            .handle_data(&media_bytes(MessageTypeId::DataAmf0, 0, &meta, 4))
            .unwrap();
        assert_eq!(
            events,
            vec![ClientEvent::Media {
                flv: expected_flv_tag(false, 18, 0, &meta),
            }]
        );
    }

    #[test]
    fn play_mode_unpacks_an_aggregate_into_its_sub_tags() {
        let mut client = play_client_at_play_sent();
        let sub_audio = aggregate_sub_tag(8, 0, &[0xAF, 0x01]);
        let sub_video = aggregate_sub_tag(9, 40, &[0x17, 0x01, 0xBB]);
        let (_, events) = client
            .handle_data(&aggregate_bytes(0, &[sub_audio, sub_video]))
            .unwrap();
        assert_eq!(
            events,
            vec![
                ClientEvent::Media {
                    flv: expected_flv_tag(true, 8, 0, &[0xAF, 0x01]),
                },
                ClientEvent::Media {
                    flv: expected_flv_tag(false, 9, 40, &[0x17, 0x01, 0xBB]),
                },
            ]
        );
    }

    #[test]
    fn play_mode_reports_recorded_only_announced_while_receiving() {
        let mut client = play_client_at_play_sent();
        client
            .handle_data(&user_control_bytes(UserControl::StreamIsRecorded(1)))
            .unwrap();
        let (_, events) = client
            .handle_data(&command_bytes(&on_status(
                "NetStream.Play.Start",
                "status",
                "started",
            )))
            .unwrap();
        assert_eq!(events, vec![ClientEvent::Playing { recorded: true }]);
        assert_eq!(client.state, ClientState::Playing);

        let mut client = started_play_client();
        client
            .handle_data(&user_control_bytes(UserControl::StreamIsRecorded(1)))
            .unwrap();
        client
            .handle_data(&command_bytes(&connect_result(1.0)))
            .unwrap();
        client
            .handle_data(&command_bytes(&create_stream_result(2.0, 1.0)))
            .unwrap();
        let (_, events) = client
            .handle_data(&command_bytes(&on_status(
                "NetStream.Play.Start",
                "status",
                "started",
            )))
            .unwrap();
        assert_eq!(events, vec![ClientEvent::Playing { recorded: false }]);
    }

    #[test]
    fn play_mode_ends_on_eof_and_on_the_play_status_codes() {
        for code in ["NetStream.Play.Stop", "NetStream.Play.Complete"] {
            let mut client = play_client_at_play_sent();
            let (_, events) = client
                .handle_data(&command_bytes(&on_status(code, "status", "stopped")))
                .unwrap();
            assert_eq!(events, vec![ClientEvent::StreamEnded], "{code}");
        }

        let mut client = play_client_at_play_sent();
        let (_, events) = client
            .handle_data(&user_control_bytes(UserControl::StreamEof(1)))
            .unwrap();
        assert_eq!(events, vec![ClientEvent::StreamEnded]);

        let mut client = started_play_client();
        let (_, events) = client
            .handle_data(&user_control_bytes(UserControl::StreamEof(1)))
            .unwrap();
        assert!(events.is_empty(), "{events:?}");
    }

    #[test]
    fn play_mode_ignores_a_status_that_is_neither_error_nor_lifecycle() {
        let mut client = play_client_at_play_sent();
        let (_, events) = client
            .handle_data(&command_bytes(&on_status(
                "NetStream.Play.Reset",
                "status",
                "reset",
            )))
            .unwrap();
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(client.state, ClientState::PlaySent);
    }

    #[test]
    fn play_mode_maps_error_statuses_and_error_level_to_error_events() {
        for (code, level, description) in [
            (
                "NetStream.Play.StreamNotFound",
                "error",
                "Failed to play live",
            ),
            ("NetStream.Play.Failed", "error", "Failed"),
            (
                "NetStream.Play.InsufficientBW",
                "error",
                "Insufficient bandwidth",
            ),
            ("NetStream.Play.StreamNotFound", "status", "No such stream"),
            ("NetStream.Play.Failed", "status", "Play failed"),
        ] {
            let mut client = play_client_at_play_sent();
            let (_, events) = client
                .handle_data(&command_bytes(&on_status(code, level, description)))
                .unwrap();
            assert_eq!(
                events,
                vec![ClientEvent::Error {
                    code: code.to_string(),
                    description: description.to_string(),
                }],
                "{code}"
            );
        }
    }

    // ── Coalesced S2 (audit 22, finding 2) ────────────────────────────────

    fn coalesced_s2_batch(client: &mut ClientSession) -> Vec<u8> {
        let c0_c1 = client.start();
        let mut server_hs = handshake::Handshake::new();
        let (s0_s1_s2, _, _) = server_hs.read(&c0_c1).unwrap();
        let mut batch = s0_s1_s2;
        batch.extend_from_slice(
            &ChunkWriter::new()
                .write(&ProtocolControl::WindowAckSize(2_500_000).to_message())
                .unwrap(),
        );
        batch.extend_from_slice(&command_bytes(&connect_result(1.0)));
        batch
    }

    #[test]
    fn s2_coalesced_with_control_and_connect_result_is_dispatched() {
        let mut client = ClientSession::new(play_config());
        let batch = coalesced_s2_batch(&mut client);
        let (out, events) = client.handle_data(&batch).unwrap();
        assert_eq!(
            events,
            vec![ClientEvent::Connected {
                properties: server_properties(),
            }]
        );
        assert_eq!(client.state, ClientState::CreateStreamSent { txn_id: 2.0 });
        let sent = commands(&out[HANDSHAKE_PACKET_LEN..]);
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert_eq!(sent[0].name, "connect");
        assert_eq!(sent[1].name, "createStream");
    }

    #[test]
    fn the_coalesced_s2_batch_is_byte_split_independent() {
        let mut client = ClientSession::new(play_config());
        let batch = coalesced_s2_batch(&mut client);
        let expected = vec![ClientEvent::Connected {
            properties: server_properties(),
        }];

        for split in 1..batch.len() {
            let mut client = ClientSession::new(play_config());
            client.start();
            let (out_a, events_a) = client.handle_data(&batch[..split]).unwrap();
            let (out_b, events_b) = client.handle_data(&batch[split..]).unwrap();
            let events: Vec<ClientEvent> = events_a.into_iter().chain(events_b).collect();
            assert_eq!(events, expected, "split at {split}");
            assert_eq!(
                client.state,
                ClientState::CreateStreamSent { txn_id: 2.0 },
                "split at {split}"
            );
            let mut out = Vec::new();
            if out_a.is_empty() {
                out.extend_from_slice(&out_b[HANDSHAKE_PACKET_LEN..]);
            } else {
                out.extend_from_slice(&out_a[HANDSHAKE_PACKET_LEN..]);
                out.extend_from_slice(&out_b);
            }
            let names: Vec<String> = commands(&out).into_iter().map(|c| c.name).collect();
            assert_eq!(
                names,
                vec!["connect".to_string(), "createStream".to_string()],
                "split at {split}"
            );
        }
    }

    // ── Acknowledgement and Abort (audit 22, finding 4) ───────────────────

    #[test]
    fn acknowledgement_follows_the_servers_advertised_window() {
        let mut client = ClientSession::new(play_config());
        let c0_c1 = client.start();
        let mut server_hs = handshake::Handshake::new();
        let (s0_s1_s2, _, _) = server_hs.read(&c0_c1).unwrap();
        let (out, _) = client.handle_data(&s0_s1_s2).unwrap();
        assert!(ack_in(&out).is_none());
        assert_eq!(client.ack_threshold, 2_500_000);

        let connect = command_bytes(&connect_result(1.0));
        let create = command_bytes(&create_stream_result(2.0, 1.0));
        client.handle_data(&connect).unwrap();
        client.handle_data(&create).unwrap();
        let mut fed = (connect.len() + create.len()) as u64;

        let ping = user_control_bytes(UserControl::PingRequest(7));
        fed += ping.len() as u64;
        let (out, _) = client.handle_data(&ping).unwrap();
        assert_eq!(ack_in(&out), None, "no ack below the window");

        let window = ChunkWriter::new()
            .write(&ProtocolControl::WindowAckSize(100).to_message())
            .unwrap();
        fed += window.len() as u64;
        let (out, _) = client.handle_data(&window).unwrap();
        assert_eq!(client.ack_threshold, 100);
        assert_eq!(ack_in(&out), Some(fed as u32));

        let small = user_control_bytes(UserControl::PingRequest(9));
        fed += small.len() as u64;
        let (out, _) = client.handle_data(&small).unwrap();
        assert_eq!(ack_in(&out), None, "still below the window");

        let big_payload = vec![0xAF; 1_200];
        let big = media_bytes(MessageTypeId::Audio, 0, &big_payload, 4);
        fed += big.len() as u64;
        let (out, events) = client.handle_data(&big).unwrap();
        assert_eq!(
            events,
            vec![ClientEvent::Media {
                flv: expected_flv_tag(true, 8, 0, &big_payload),
            }]
        );
        assert_eq!(ack_in(&out), Some(fed as u32));
    }

    #[test]
    fn acknowledgement_sequence_is_the_low_32_bits_of_the_byte_count() {
        let mut client = play_client_at_play_sent();
        let ping = user_control_bytes(UserControl::PingRequest(7));
        client.ack_threshold = 1;
        client.bytes_acked = 0;
        client.bytes_received = 0x1_0000_0000 - ping.len() as u64 + 0x10;
        let (out, _) = client.handle_data(&ping).unwrap();
        assert_eq!(ack_in(&out), Some(0x10));
    }

    #[test]
    fn abort_discards_the_in_progress_message_on_that_chunk_stream() {
        let mut client = play_client_at_play_sent();
        let aborted = media_bytes(MessageTypeId::Audio, 0, &[0xAA; 300], 12);
        let (_, events) = client.handle_data(&aborted[..1 + 11 + 128]).unwrap();
        assert!(events.is_empty(), "{events:?}");

        let abort = ChunkWriter::new()
            .write(
                &ProtocolControl::Abort {
                    chunk_stream_id: 12,
                }
                .to_message(),
            )
            .unwrap();
        let (_, events) = client.handle_data(&abort).unwrap();
        assert!(events.is_empty(), "{events:?}");

        let mut fresh = Vec::new();
        for chunk_len in [128usize, 128, 44] {
            fresh.push(0xCC);
            fresh.extend(core::iter::repeat_n(0xEEu8, chunk_len));
        }
        let (_, events) = client.handle_data(&fresh).unwrap();
        assert_eq!(
            events,
            vec![ClientEvent::Media {
                flv: expected_flv_tag(true, 8, 0, &[0xEE; 300]),
            }]
        );
    }
}
