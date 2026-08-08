//! RTMP client session engine — `connect` → `createStream` → `publish`
//! (Adobe RTMP 1.0 §7.2, `NetConnection`/`NetStream` commands).
//!
//! [`ClientSession`] mirrors [`crate::server::ServerSession`]: a sans-IO
//! state machine for the **client publish** role. Feed inbound bytes via
//! [`ClientSession::handle_data`], get back outbound bytes + typed
//! [`ClientEvent`]s. Auto-advances: a successful `connect` `_result`
//! automatically emits `createStream`, and a successful `createStream`
//! `_result` emits `publish`.

use broadcast_common::{Parse, Serialize};

use crate::RtmpError;
use crate::amf0::{Amf0Value, Command};
use crate::chunk::{ChunkAssembler, ChunkWriter, Message};
use crate::handshake::{
    EchoPacket, HANDSHAKE_PACKET_LEN, HandshakePacket, RTMP_VERSION, Version, default_random_fill,
};
use crate::message::{ProtocolControl, msg_type};

type Result<T> = core::result::Result<T, RtmpError>;

const COMMAND_CHUNK_STREAM_ID: u32 = 3;
const VERSION_LEN: usize = 1;

/// Configuration for a [`ClientSession`].
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Outbound chunk size (§5.4.1).
    pub chunk_size: u32,
    /// Window Acknowledgement Size (§5.4.4).
    pub window_ack_size: u32,
    /// RTMP `app` name for the `connect` command.
    pub app: String,
    /// Publishing name (stream key) for the `publish` command.
    pub stream_key: String,
    /// `tcUrl` for the `connect` command (e.g. `rtmp://host/app`).
    pub tc_url: Option<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            chunk_size: 4096,
            window_ack_size: 2_500_000,
            app: String::new(),
            stream_key: String::new(),
            tc_url: None,
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
}

/// Client-side handshake: generate C0+C1, consume S0+S1+S2, produce C2.
#[derive(Debug)]
struct ClientHandshake {
    state: ClientHandshakeState,
    local_time: u32,
    local_random: [u8; 1528],
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
        c0.serialize_into(&mut out[..VERSION_LEN]).unwrap();
        c1.serialize_into(&mut out[VERSION_LEN..]).unwrap();
        self.state = ClientHandshakeState::WaitS0S1S2;
        out
    }

    fn read(&mut self, input: &[u8]) -> Result<(Vec<u8>, usize, bool)> {
        match &self.state {
            ClientHandshakeState::SendC0C1 => Ok((Vec::new(), 0, false)),
            ClientHandshakeState::WaitS0S1S2 => {
                let need = VERSION_LEN + HANDSHAKE_PACKET_LEN + HANDSHAKE_PACKET_LEN;
                if input.len() < need {
                    return Err(RtmpError::BufferTooShort {
                        need,
                        have: input.len(),
                        what: "S0+S1+S2",
                    });
                }
                let _s0 = Version::parse(&input[..VERSION_LEN])?;
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
    config: ClientConfig,
    handshake: ClientHandshake,
    handshake_buf: Vec<u8>,
    assembler: ChunkAssembler,
    writer: ChunkWriter,
    state: ClientState,
    stream_id: Option<u32>,
    next_txn_id: f64,
    ack_threshold: u32,
    bytes_received: u64,
    bytes_acked: u64,
}

impl ClientSession {
    /// Create a new client session.
    #[must_use]
    pub fn new(config: ClientConfig) -> Self {
        let ack_threshold = config.window_ack_size;
        Self {
            config,
            handshake: ClientHandshake::new(),
            handshake_buf: Vec::new(),
            assembler: ChunkAssembler::new(),
            writer: ChunkWriter::new(),
            state: ClientState::Init,
            stream_id: None,
            next_txn_id: 1.0,
            ack_threshold,
            bytes_received: 0,
            bytes_acked: 0,
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

        self.maybe_ack(&mut out);
        Ok((out, events))
    }

    /// Encode an audio message (type 8). Only valid in `Publishing` state.
    pub fn send_audio(&mut self, timestamp: u32, data: &[u8]) -> Result<Vec<u8>> {
        if self.state != ClientState::Publishing {
            return Err(RtmpError::Malformed {
                what: "send_audio: not in Publishing state",
            });
        }
        let msg = Message {
            chunk_stream_id: 4,
            timestamp,
            message_type_id: msg_type::AUDIO,
            message_stream_id: self.stream_id.unwrap_or(1),
            payload: data.to_vec(),
        };
        Ok(self.writer.write(&msg))
    }

    /// Encode a video message (type 9). Only valid in `Publishing` state.
    pub fn send_video(&mut self, timestamp: u32, data: &[u8]) -> Result<Vec<u8>> {
        if self.state != ClientState::Publishing {
            return Err(RtmpError::Malformed {
                what: "send_video: not in Publishing state",
            });
        }
        let msg = Message {
            chunk_stream_id: 5,
            timestamp,
            message_type_id: msg_type::VIDEO,
            message_stream_id: self.stream_id.unwrap_or(1),
            payload: data.to_vec(),
        };
        Ok(self.writer.write(&msg))
    }

    /// Encode a metadata message (`@setDataFrame`/`onMetaData`, type 18).
    pub fn send_metadata(&mut self, metadata: &[(String, Amf0Value)]) -> Result<Vec<u8>> {
        if self.state != ClientState::Publishing {
            return Err(RtmpError::Malformed {
                what: "send_metadata: not in Publishing state",
            });
        }
        let mut payload = Amf0Value::String("@setDataFrame".to_string()).to_bytes();
        payload.extend(Amf0Value::String("onMetaData".to_string()).to_bytes());
        let obj = Amf0Value::Object(metadata.to_vec());
        payload.extend(obj.to_bytes());
        let msg = Message {
            chunk_stream_id: 4,
            timestamp: 0,
            message_type_id: msg_type::DATA_AMF0,
            message_stream_id: self.stream_id.unwrap_or(1),
            payload,
        };
        Ok(self.writer.write(&msg))
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
                        out.extend_from_slice(&self.build_connect());
                    }
                }
                Err(RtmpError::BufferTooShort { .. }) => {
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }
        }

        if self.handshake.is_done() && !self.handshake_buf.is_empty() {
            let remaining = std::mem::take(&mut self.handshake_buf);
            self.bytes_received = self.bytes_received.saturating_add(remaining.len() as u64);
            self.assembler.feed(&remaining);
            while let Some(msg) = self.assembler.next_message()? {
                self.dispatch_message(&msg, out, events)?;
            }
        }

        Ok(None)
    }

    fn build_connect(&mut self) -> Vec<u8> {
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
                ("tcUrl".to_string(), Amf0Value::String(tc_url)),
                (
                    "type".to_string(),
                    Amf0Value::String("nonprivate".to_string()),
                ),
            ])],
        };

        let mut out = Vec::new();

        let set_chunk_size = ProtocolControl::SetChunkSize(self.config.chunk_size);
        out.extend_from_slice(&self.writer.write(&set_chunk_size.to_message()));
        self.writer.set_chunk_size(self.config.chunk_size);

        let window_ack = ProtocolControl::WindowAckSize(self.config.window_ack_size);
        out.extend_from_slice(&self.writer.write(&window_ack.to_message()));

        let msg = Message {
            chunk_stream_id: COMMAND_CHUNK_STREAM_ID,
            timestamp: 0,
            message_type_id: msg_type::COMMAND_AMF0,
            message_stream_id: 0,
            payload: cmd.to_body(),
        };
        out.extend_from_slice(&self.writer.write(&msg));
        out
    }

    fn build_create_stream(&mut self) -> Vec<u8> {
        let txn_id = self.next_txn_id;
        self.next_txn_id += 1.0;
        self.state = ClientState::CreateStreamSent { txn_id };

        let cmd = Command {
            name: "createStream".to_string(),
            transaction_id: txn_id,
            arguments: vec![Amf0Value::Null],
        };
        let msg = Message {
            chunk_stream_id: COMMAND_CHUNK_STREAM_ID,
            timestamp: 0,
            message_type_id: msg_type::COMMAND_AMF0,
            message_stream_id: 0,
            payload: cmd.to_body(),
        };
        self.writer.write(&msg)
    }

    fn build_publish(&mut self) -> Vec<u8> {
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
            chunk_stream_id: COMMAND_CHUNK_STREAM_ID,
            timestamp: 0,
            message_type_id: msg_type::COMMAND_AMF0,
            message_stream_id: self.stream_id.unwrap_or(1),
            payload: cmd.to_body(),
        };
        self.writer.write(&msg)
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
                    self.ack_threshold = ack_window_size;
                    let ack = ProtocolControl::WindowAckSize(ack_window_size);
                    out.extend_from_slice(&self.writer.write(&ack.to_message()));
                }
                _ => {}
            }
            return Ok(());
        }
        if msg.message_type_id == msg_type::COMMAND_AMF0 {
            let cmd = Command::parse(&msg.payload)?;
            self.handle_command(&cmd, out, events)?;
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
            ClientState::ConnectSent { txn_id } if (cmd.transaction_id - txn_id).abs() < 0.5 => {
                let props = match cmd.arguments.first() {
                    Some(Amf0Value::Object(pairs)) => pairs.clone(),
                    _ => Vec::new(),
                };
                events.push(ClientEvent::Connected { properties: props });
                self.state = ClientState::Connected;
                out.extend_from_slice(&self.build_create_stream());
            }
            ClientState::CreateStreamSent { txn_id }
                if (cmd.transaction_id - txn_id).abs() < 0.5 =>
            {
                let sid = match cmd.arguments.last() {
                    Some(Amf0Value::Number(n)) => *n as u32,
                    _ => 1,
                };
                self.stream_id = Some(sid);
                events.push(ClientEvent::StreamCreated { stream_id: sid });
                self.state = ClientState::StreamCreated;
                out.extend_from_slice(&self.build_publish());
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_on_status(&mut self, cmd: &Command, events: &mut Vec<ClientEvent>) -> Result<()> {
        let (code, desc) = extract_status(&cmd.arguments);
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

    fn maybe_ack(&mut self, out: &mut Vec<u8>) {
        if self.ack_threshold == 0 {
            return;
        }
        if self.bytes_received.wrapping_sub(self.bytes_acked) >= u64::from(self.ack_threshold) {
            let seq = (self.bytes_received & 0xFFFF_FFFF) as u32;
            let ack = ProtocolControl::Acknowledgement(seq);
            out.extend_from_slice(&self.writer.write(&ack.to_message()));
            self.bytes_acked = self.bytes_received;
        }
    }
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
    use crate::server::{ServerConfig, ServerSession};

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

    #[test]
    fn client_connect_flow() {
        let config = ClientConfig {
            app: "live".to_string(),
            stream_key: "test_key".to_string(),
            ..ClientConfig::default()
        };
        let mut client = ClientSession::new(config);
        let c0_c1 = client.start();

        let mut server = ServerSession::new(
            ServerConfig::default().with_expected_stream_key(Some("test_key".to_string())),
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
        let config = ClientConfig {
            app: "live".to_string(),
            stream_key: "test_key".to_string(),
            ..ClientConfig::default()
        };
        let mut client = ClientSession::new(config);
        let c0_c1 = client.start();

        let mut server = ServerSession::new(
            ServerConfig::default().with_expected_stream_key(Some("test_key".to_string())),
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
        let config = ClientConfig {
            app: "live".to_string(),
            stream_key: "test".to_string(),
            ..ClientConfig::default()
        };
        let mut client = ClientSession::new(config);
        client.state = ClientState::Publishing;
        client.stream_id = Some(1);

        let audio = client.send_audio(100, &[0xAA, 0xBB]).unwrap();
        assert!(!audio.is_empty());

        let video = client.send_video(200, &[0xCC, 0xDD, 0xEE]).unwrap();
        assert!(!video.is_empty());

        let meta = client
            .send_metadata(&[
                ("width".to_string(), Amf0Value::Number(1920.0)),
                ("height".to_string(), Amf0Value::Number(1080.0)),
            ])
            .unwrap();
        assert!(!meta.is_empty());
    }

    #[test]
    fn send_audio_before_publishing_fails() {
        let config = ClientConfig {
            app: "live".to_string(),
            stream_key: "test".to_string(),
            ..ClientConfig::default()
        };
        let mut client = ClientSession::new(config);
        assert!(client.send_audio(0, &[0x00]).is_err());
    }

    #[test]
    fn connect_error_produces_event() {
        let config = ClientConfig {
            app: "live".to_string(),
            stream_key: "test".to_string(),
            ..ClientConfig::default()
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
            chunk_stream_id: COMMAND_CHUNK_STREAM_ID,
            timestamp: 0,
            message_type_id: msg_type::COMMAND_AMF0,
            message_stream_id: 0,
            payload: error_cmd.to_body(),
        };
        let bytes = ChunkWriter::new().write(&msg);

        let (_, events) = client.handle_data(&bytes).unwrap();
        assert!(
            events.iter().any(|e| matches!(
                e,
                ClientEvent::Error { code, .. } if code == "NetConnection.Connect.Rejected"
            )),
            "expected Error event: {events:?}"
        );
    }
}
