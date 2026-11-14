//! A sans-IO RTMP play server for tests: the peer a `ClientSession` in play mode talks to,
//! built from this crate's own public handshake, chunk, AMF0 and message codecs.
//!
//! `ServerSession` is a publish-ingest engine and has no `play` role, so this peer answers
//! `connect`, `createStream` and `play` by hand (Adobe RTMP 1.0 §7.2.1.1, §7.2.1.3, §7.2.2.1)
//! and then streams whatever media the test hands it. Shared with `broadcast-client`'s wire
//! tests through a `#[path]` include.

use broadcast_common::Parse;
use rtmp_runtime::RtmpError;
use rtmp_runtime::amf0::{Amf0Value, Command};
use rtmp_runtime::chunk::{ChunkAssembler, ChunkWriter, Message};
use rtmp_runtime::handshake::Handshake;
use rtmp_runtime::message::{ChunkSize, LimitType, ProtocolControl, UserControl, msg_type};

pub const COMMAND_CSID: u32 = 3;
pub const AUDIO_CSID: u32 = 4;
pub const VIDEO_CSID: u32 = 5;
pub const DATA_CSID: u32 = 6;
pub const PLAY_STREAM_ID: u32 = 1;
const CONNECT_RESULT_CODE: &str = "NetConnection.Connect.Success";
const PLAY_START_CODE: &str = "NetStream.Play.Start";
const PLAY_RESET_CODE: &str = "NetStream.Play.Reset";
const PLAY_NOT_FOUND_CODE: &str = "NetStream.Play.StreamNotFound";

#[derive(Debug, Clone)]
pub struct PeerConfig {
    pub stream_name: String,
    pub window_ack_size: u32,
    pub chunk_size_after_connect: Option<u32>,
    pub client_chunk_size: u32,
    pub recorded: bool,
}

impl Default for PeerConfig {
    fn default() -> Self {
        Self {
            stream_name: "live".to_string(),
            window_ack_size: 2_500_000,
            chunk_size_after_connect: None,
            client_chunk_size: 4096,
            recorded: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Seen {
    Command {
        name: String,
        args: Vec<Amf0Value>,
        stream_id: u32,
    },
    UserControl(UserControl),
    Acknowledgement(u32),
    WindowAckSize(u32),
    SetChunkSize(u32),
}

pub struct PlayPeer {
    config: PeerConfig,
    handshake: Handshake,
    hs_buf: Vec<u8>,
    assembler: ChunkAssembler,
    writer: ChunkWriter,
    pub seen: Vec<Seen>,
    pub chunk_bytes_received: u64,
    pub bytes_sent_after_handshake: u64,
    next_stream_id: u32,
    playing: bool,
}

impl PlayPeer {
    pub fn new(config: PeerConfig) -> Self {
        let assembler = ChunkAssembler::new().with_chunk_size(config.client_chunk_size);
        Self {
            config,
            handshake: Handshake::new(),
            hs_buf: Vec::new(),
            assembler,
            writer: ChunkWriter::new(),
            seen: Vec::new(),
            chunk_bytes_received: 0,
            bytes_sent_after_handshake: 0,
            next_stream_id: PLAY_STREAM_ID,
            playing: false,
        }
    }

    pub fn commands(&self) -> Vec<&str> {
        self.seen
            .iter()
            .filter_map(|s| match s {
                Seen::Command { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn acks(&self) -> Vec<u32> {
        self.seen
            .iter()
            .filter_map(|s| match s {
                Seen::Acknowledgement(n) => Some(*n),
                _ => None,
            })
            .collect()
    }

    pub fn command_args(&self, name: &str) -> Option<&Vec<Amf0Value>> {
        self.seen.iter().find_map(|s| match s {
            Seen::Command { name: n, args, .. } if n == name => Some(args),
            _ => None,
        })
    }

    pub fn on_bytes(&mut self, input: &[u8]) -> Result<Vec<u8>, RtmpError> {
        let mut out = Vec::new();
        let mut rest: Vec<u8> = Vec::new();
        if !self.handshake.is_done() {
            self.hs_buf.extend_from_slice(input);
            loop {
                match self.handshake.read(&self.hs_buf) {
                    Ok((reply, consumed, done)) => {
                        out.extend_from_slice(&reply);
                        self.hs_buf.drain(..consumed);
                        if done {
                            break;
                        }
                    }
                    Err(RtmpError::BufferTooShort(..)) => break,
                    Err(e) => return Err(e),
                }
            }
            if self.handshake.is_done() {
                rest = core::mem::take(&mut self.hs_buf);
            } else {
                return Ok(out);
            }
        } else {
            rest.extend_from_slice(input);
        }
        self.chunk_bytes_received += rest.len() as u64;
        let handshake_len = out.len();
        for msg in self.assembler.push(&rest)? {
            self.on_message(&msg, &mut out)?;
        }
        self.bytes_sent_after_handshake += (out.len() - handshake_len) as u64;
        Ok(out)
    }

    fn on_message(&mut self, msg: &Message, out: &mut Vec<u8>) -> Result<(), RtmpError> {
        if let Some(pc) = ProtocolControl::from_message(msg)? {
            match pc {
                ProtocolControl::SetChunkSize(n) => {
                    self.assembler.set_chunk_size(n.value());
                    self.seen.push(Seen::SetChunkSize(n.value()));
                }
                ProtocolControl::Acknowledgement(n) => self.seen.push(Seen::Acknowledgement(n)),
                ProtocolControl::WindowAckSize(n) => self.seen.push(Seen::WindowAckSize(n)),
                _ => {}
            }
            return Ok(());
        }
        if msg.message_type_id == msg_type::USER_CONTROL {
            if let Ok(uc) = UserControl::parse(&msg.payload) {
                self.seen.push(Seen::UserControl(uc));
            }
            return Ok(());
        }
        if msg.message_type_id == msg_type::COMMAND_AMF0 {
            let cmd = Command::parse(&msg.payload)?;
            self.seen.push(Seen::Command {
                name: cmd.name.clone(),
                args: cmd.arguments.clone(),
                stream_id: msg.message_stream_id,
            });
            self.on_command(&cmd, out)?;
        }
        Ok(())
    }

    fn on_command(&mut self, cmd: &Command, out: &mut Vec<u8>) -> Result<(), RtmpError> {
        match cmd.name.as_str() {
            "connect" => {
                self.send_control(
                    ProtocolControl::WindowAckSize(self.config.window_ack_size).to_message(),
                    out,
                )?;
                self.send_control(
                    ProtocolControl::SetPeerBandwidth {
                        ack_window_size: self.config.window_ack_size,
                        limit_type: LimitType::Dynamic,
                    }
                    .to_message(),
                    out,
                )?;
                if let Some(n) = self.config.chunk_size_after_connect {
                    out.extend(self.set_chunk_size(n)?);
                }
                self.send_control(UserControl::StreamBegin(0).to_message(), out)?;
                let props = Amf0Value::Object(vec![(
                    "fmsVer".to_string(),
                    Amf0Value::String("FMS/3,0,1,123".to_string()),
                )]);
                let info = Amf0Value::Object(vec![
                    ("level".to_string(), Amf0Value::String("status".to_string())),
                    (
                        "code".to_string(),
                        Amf0Value::String(CONNECT_RESULT_CODE.to_string()),
                    ),
                ]);
                self.send_command("_result", cmd.transaction_id, vec![props, info], 0, out)
            }
            "createStream" => {
                let id = self.next_stream_id;
                self.next_stream_id += 1;
                self.send_command(
                    "_result",
                    cmd.transaction_id,
                    vec![Amf0Value::Null, Amf0Value::Number(f64::from(id))],
                    0,
                    out,
                )
            }
            "play" => {
                let name = match cmd.arguments.get(1) {
                    Some(Amf0Value::String(s)) => s.clone(),
                    _ => String::new(),
                };
                if name != self.config.stream_name {
                    return self.send_on_status("error", PLAY_NOT_FOUND_CODE, out);
                }
                if self.config.recorded {
                    self.send_control(
                        UserControl::StreamIsRecorded(PLAY_STREAM_ID).to_message(),
                        out,
                    )?;
                }
                self.send_control(UserControl::StreamBegin(PLAY_STREAM_ID).to_message(), out)?;
                self.send_on_status("status", PLAY_RESET_CODE, out)?;
                self.send_on_status("status", PLAY_START_CODE, out)?;
                self.playing = true;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    fn send_control(&mut self, msg: Message, out: &mut Vec<u8>) -> Result<(), RtmpError> {
        out.extend(self.writer.write(&msg)?);
        Ok(())
    }

    fn send_command(
        &mut self,
        name: &str,
        txn: f64,
        args: Vec<Amf0Value>,
        stream_id: u32,
        out: &mut Vec<u8>,
    ) -> Result<(), RtmpError> {
        let cmd = Command {
            name: name.to_string(),
            transaction_id: txn,
            arguments: args,
        };
        let msg = Message {
            chunk_stream_id: COMMAND_CSID,
            timestamp: 0,
            message_type_id: msg_type::COMMAND_AMF0,
            message_stream_id: stream_id,
            payload: cmd.to_body()?,
        };
        out.extend(self.writer.write(&msg)?);
        Ok(())
    }

    fn counted(&mut self, bytes: Vec<u8>) -> Vec<u8> {
        self.bytes_sent_after_handshake += bytes.len() as u64;
        bytes
    }

    pub fn on_status(&mut self, level: &str, code: &str) -> Result<Vec<u8>, RtmpError> {
        let mut out = Vec::new();
        self.send_on_status(level, code, &mut out)?;
        Ok(self.counted(out))
    }

    fn send_on_status(
        &mut self,
        level: &str,
        code: &str,
        out: &mut Vec<u8>,
    ) -> Result<(), RtmpError> {
        let info = Amf0Value::Object(vec![
            ("level".to_string(), Amf0Value::String(level.to_string())),
            ("code".to_string(), Amf0Value::String(code.to_string())),
            (
                "description".to_string(),
                Amf0Value::String(format!("{code} on {}", self.config.stream_name)),
            ),
        ]);
        self.send_command(
            "onStatus",
            0.0,
            vec![Amf0Value::Null, info],
            PLAY_STREAM_ID,
            out,
        )
    }

    pub fn set_chunk_size(&mut self, n: u32) -> Result<Vec<u8>, RtmpError> {
        let bytes = self
            .writer
            .write(&ProtocolControl::SetChunkSize(ChunkSize::new(n).unwrap()).to_message())?;
        self.writer.set_chunk_size(n);
        Ok(self.counted(bytes))
    }

    pub fn ping(&mut self, ts: u32) -> Result<Vec<u8>, RtmpError> {
        let bytes = self
            .writer
            .write(&UserControl::PingRequest(ts).to_message())?;
        Ok(self.counted(bytes))
    }

    pub fn stream_eof(&mut self) -> Result<Vec<u8>, RtmpError> {
        let bytes = self
            .writer
            .write(&UserControl::StreamEof(PLAY_STREAM_ID).to_message())?;
        Ok(self.counted(bytes))
    }

    pub fn media(
        &mut self,
        tag_type: u8,
        timestamp: u32,
        payload: &[u8],
    ) -> Result<Vec<u8>, RtmpError> {
        let csid = match tag_type {
            msg_type::AUDIO => AUDIO_CSID,
            msg_type::VIDEO => VIDEO_CSID,
            _ => DATA_CSID,
        };
        let bytes = self.writer.write(&Message {
            chunk_stream_id: csid,
            timestamp,
            message_type_id: tag_type,
            message_stream_id: PLAY_STREAM_ID,
            payload: payload.to_vec(),
        })?;
        Ok(self.counted(bytes))
    }

    pub fn raw_message(&mut self, msg: &Message) -> Result<Vec<u8>, RtmpError> {
        let bytes = self.writer.write(msg)?;
        Ok(self.counted(bytes))
    }
}
