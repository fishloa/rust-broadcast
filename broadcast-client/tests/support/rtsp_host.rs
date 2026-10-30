use std::cell::{Cell, RefCell};

use broadcast_auth::{AuthResult, RequestContext, Verifier};
use broadcast_client::{Action, Config, Event, Session};
use broadcast_common::{Package, Serialize, Unpackage};
use rtsp_runtime::transport::{Transport, TransportSpec};
use rtsp_types::{Message, StatusCode, Version, headers};
use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;
use transmux::{RtpPacketiser, Sample, SenderReport, TsDemux};

pub const MS: u64 = 1_000;
pub const SEC: u64 = 1_000_000;
pub const BASE_URL: &str = "rtsp://cam.test/live/";
pub const DESCRIBE: &str = "DESCRIBE";
pub const SETUP: &str = "SETUP";
pub const PLAY: &str = "PLAY";
pub const OPTIONS: &str = "OPTIONS";
pub const TEARDOWN: &str = "TEARDOWN";
const FIXTURE: &str = "ts/h264_aac.ts";
const MTU: usize = 1400;
const SSRC: u32 = 0x1234_5678;
const SESSION_ID: &str = "A1B2C3D4";
const INTERLEAVED_MAGIC: u8 = 0x24;
const CHANNEL_STEP: u8 = 2;
const RUNAWAY_LIMIT: usize = 2_000;
const UDP_TRANSPORT: &str = "RTP/AVP;unicast;client_port=5000-5001;server_port=6000-6001";
pub const RTCP_PT_SR: u8 = 200;

pub struct TrackFixture {
    pub kind: RtpMediaKind,
    pub packets: Vec<Vec<u8>>,
    pub samples: Vec<Sample>,
}

pub struct MediaFixture {
    pub sdp: String,
    pub tracks: Vec<TrackFixture>,
}

impl MediaFixture {
    pub fn load() -> Self {
        Self::from_media(&Self::demux())
    }

    pub fn demux() -> transmux::Media {
        let data = test_fixtures::read(FIXTURE);
        TsDemux::new().unpackage(&data[..]).expect("demux fixture")
    }

    pub fn from_media(media: &transmux::Media) -> Self {
        let mut packetiser = RtpPacketiser {
            mtu: MTU,
            ssrc: SSRC,
            stap_a_parameter_sets: false,
            ..RtpPacketiser::default()
        };
        let out = packetiser.package(media).expect("packetise fixture");
        let mut session = sdp_types::Session::parse(out.sdp.as_bytes()).expect("parse sdp");
        for (n, m) in session.medias.iter_mut().enumerate() {
            m.add_attribute_with_value("control", format!("trackID={}", n + 1));
        }
        let mut sdp = Vec::new();
        session.write(&mut sdp).expect("write sdp");
        let tracks = out
            .streams
            .iter()
            .map(|stream| {
                let original = media
                    .tracks
                    .iter()
                    .find(|t| {
                        let is_video = matches!(t.spec.config, CodecConfig::Avc { .. });
                        is_video == (stream.kind == RtpMediaKind::H264)
                    })
                    .expect("matching original track");
                TrackFixture {
                    kind: stream.kind,
                    packets: stream
                        .packets
                        .iter()
                        .map(|p| p.as_contiguous().to_vec())
                        .collect(),
                    samples: original.samples.clone(),
                }
            })
            .collect();
        Self {
            sdp: String::from_utf8(sdp).expect("utf8 sdp"),
            tracks,
        }
    }

    pub fn frames(&self) -> Vec<Vec<u8>> {
        let longest = self
            .tracks
            .iter()
            .map(|t| t.packets.len())
            .max()
            .unwrap_or(0);
        let mut frames = Vec::new();
        for i in 0..longest {
            for (n, track) in self.tracks.iter().enumerate() {
                if let Some(packet) = track.packets.get(i) {
                    frames.push(interleaved(channel_of(n), packet));
                }
            }
        }
        frames
    }
}

pub fn channel_of(track_index: usize) -> u8 {
    u8::try_from(track_index).expect("small index") * CHANNEL_STEP
}

pub fn interleaved(channel: u8, payload: &[u8]) -> Vec<u8> {
    let len = u16::try_from(payload.len()).expect("frame fits u16");
    let mut out = vec![INTERLEAVED_MAGIC, channel];
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn sender_report_frame(channel: u8, ssrc: u32, rtp_timestamp: u32) -> Vec<u8> {
    let sr = SenderReport::new(ssrc, 3_900_000_000, 0, rtp_timestamp, 10, 1000, Vec::new());
    let mut buf = vec![0u8; sr.serialized_len()];
    let n = sr.serialize_into(&mut buf).expect("serialize sr");
    buf.truncate(n);
    assert_eq!(buf[1], RTCP_PT_SR);
    interleaved(channel, &buf)
}

const NTP_FRACTION: f64 = 4_294_967_296.0;

pub fn sender_report_at(channel: u8, ssrc: u32, ntp_seconds: f64, rtp_timestamp: u32) -> Vec<u8> {
    let whole = ntp_seconds.trunc();
    let msw = whole as u32;
    let lsw = ((ntp_seconds - whole) * NTP_FRACTION) as u32;
    let sr = SenderReport::new(ssrc, msw, lsw, rtp_timestamp, 10, 1000, Vec::new());
    let mut buf = vec![0u8; sr.serialized_len()];
    let n = sr.serialize_into(&mut buf).expect("serialize sr");
    buf.truncate(n);
    interleaved(channel, &buf)
}

#[derive(Debug, Clone)]
pub struct ReqRec {
    pub method: String,
    pub uri: String,
    pub cseq: u32,
    pub headers: Vec<(String, String)>,
    pub authorized: Option<bool>,
}

impl ReqRec {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn interleaved(&self) -> Option<(u8, u8)> {
        let value = self.header("transport")?;
        Transport::parse(value).ok()?.first()?.interleaved
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SetupReply {
    Echo,
    NoTransport,
    Fixed(u8, u8),
}

pub struct Server {
    inbound: Vec<u8>,
    pub content_base: Option<String>,
    pub setup_reply: Option<Box<dyn Fn(usize) -> SetupReply>>,
    pub rotate_options_nonce: bool,
    nonce_counter: Cell<u32>,
    pending_nonce: RefCell<Option<String>>,
    pub log: Vec<ReqRec>,
    pub sdp: Vec<u8>,
    pub session_timeout: Option<u32>,
    pub auth: Option<Verifier>,
    pub describe_status: Option<u16>,
    pub describe_location: Option<String>,
    pub setup_status: Option<u16>,
    pub play_status: Option<u16>,
    pub options_status: u16,
    pub udp_transport: bool,
    pub rtp_info: Option<String>,
}

impl Server {
    pub fn new(sdp: &str) -> Self {
        Self {
            inbound: Vec::new(),
            content_base: None,
            setup_reply: None,
            rotate_options_nonce: false,
            nonce_counter: Cell::new(0),
            pending_nonce: RefCell::new(None),
            log: Vec::new(),
            sdp: sdp.as_bytes().to_vec(),
            session_timeout: None,
            auth: None,
            describe_status: None,
            describe_location: None,
            setup_status: None,
            play_status: None,
            options_status: 200,
            udp_transport: false,
            rtp_info: None,
        }
    }

    pub fn count(&self, method: &str) -> usize {
        self.log.iter().filter(|r| r.method == method).count()
    }

    pub fn methods(&self) -> Vec<&str> {
        self.log.iter().map(|r| r.method.as_str()).collect()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.inbound.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let parsed = Message::<Vec<u8>>::parse(&self.inbound);
            let Ok((message, consumed)) = parsed else {
                break;
            };
            self.inbound.drain(..consumed);
            if let Message::Request(req) = message {
                let method = <&str>::from(req.method()).to_string();
                let uri = req
                    .request_uri()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                let cseq = req
                    .header(&headers::CSEQ)
                    .and_then(|v| v.as_str().trim().parse().ok())
                    .expect("request carries CSeq");
                let hdrs: Vec<(String, String)> = req
                    .headers()
                    .map(|(n, v)| (n.to_string(), v.as_str().to_string()))
                    .collect();
                let mut rec = ReqRec {
                    method,
                    uri,
                    cseq,
                    headers: hdrs,
                    authorized: None,
                };
                let response = self.answer(&mut rec);
                self.log.push(rec);
                out.push(response);
            }
        }
        out
    }

    fn answer(&self, rec: &mut ReqRec) -> Vec<u8> {
        if let Some(verifier) = &self.auth {
            let header = rec.header("authorization").map(str::to_string);
            let pairs: Vec<(&str, &str)> = header
                .iter()
                .map(|h| ("authorization", h.as_str()))
                .collect();
            let ctx = RequestContext::new(&rec.method, &rec.uri).with_headers(&pairs);
            let ok = verifier.verify(&ctx) == AuthResult::Ok;
            rec.authorized = Some(ok);
            if !ok {
                return respond(
                    rec.cseq,
                    401,
                    vec![(headers::WWW_AUTHENTICATE, verifier.challenge_for(&ctx))],
                    Vec::new(),
                );
            }
        }
        match rec.method.as_str() {
            DESCRIBE => match self.describe_status {
                Some(status) => {
                    let extra = self
                        .describe_location
                        .iter()
                        .map(|l| (headers::LOCATION, l.clone()))
                        .collect();
                    respond(rec.cseq, status, extra, Vec::new())
                }
                None => {
                    let mut extra = vec![(headers::CONTENT_TYPE, "application/sdp".to_string())];
                    if let Some(base) = &self.content_base {
                        extra.push((headers::CONTENT_BASE, base.clone()));
                    }
                    respond(rec.cseq, 200, extra, self.sdp.clone())
                }
            },
            SETUP => match self.setup_status {
                Some(status) => respond(rec.cseq, status, Vec::new(), Vec::new()),
                None => {
                    let reply = self
                        .setup_reply
                        .as_ref()
                        .map_or(SetupReply::Echo, |f| f(self.count(SETUP)));
                    let interleaved = |lo, hi| {
                        Transport::single(TransportSpec::rtp_avp_tcp_interleaved(lo, hi))
                            .to_header_value()
                            .expect("transport header")
                    };
                    let transport = if self.udp_transport {
                        Some(UDP_TRANSPORT.to_string())
                    } else {
                        match reply {
                            SetupReply::Echo => {
                                let (lo, hi) = rec.interleaved().expect("interleaved SETUP");
                                Some(interleaved(lo, hi))
                            }
                            SetupReply::NoTransport => None,
                            SetupReply::Fixed(lo, hi) => Some(interleaved(lo, hi)),
                        }
                    };
                    let mut extra = Vec::new();
                    if let Some(transport) = transport {
                        extra.push((headers::TRANSPORT, transport));
                    }
                    extra.push((headers::SESSION, self.session_value()));
                    respond(rec.cseq, 200, extra, Vec::new())
                }
            },
            PLAY => match self.play_status {
                Some(status) => respond(rec.cseq, status, Vec::new(), Vec::new()),
                None => {
                    let mut extra = vec![(headers::SESSION, SESSION_ID.to_string())];
                    if let Some(info) = &self.rtp_info {
                        extra.push((headers::RTP_INFO, info.clone()));
                    }
                    respond(rec.cseq, 200, extra, Vec::new())
                }
            },
            OPTIONS if self.rotate_options_nonce => self.answer_options_rotating(rec),
            OPTIONS => respond(rec.cseq, self.options_status, Vec::new(), Vec::new()),
            _ => respond(rec.cseq, 200, Vec::new(), Vec::new()),
        }
    }

    fn answer_options_rotating(&self, rec: &ReqRec) -> Vec<u8> {
        let presented = rec.header("authorization").and_then(|h| {
            let (_, rest) = h.split_once("nonce=\"")?;
            rest.split_once('"').map(|(n, _)| n.to_string())
        });
        let mut pending = self.pending_nonce.borrow_mut();
        if presented.is_some() && presented == *pending {
            *pending = None;
            return respond(rec.cseq, 200, Vec::new(), Vec::new());
        }
        let n = self.nonce_counter.get() + 1;
        self.nonce_counter.set(n);
        let nonce = format!("nonce{n}");
        let challenge = format!("Digest realm=\"cam\", nonce=\"{nonce}\", qop=\"auth\"");
        *pending = Some(nonce);
        respond(
            rec.cseq,
            401,
            vec![(headers::WWW_AUTHENTICATE, challenge)],
            Vec::new(),
        )
    }

    fn session_value(&self) -> String {
        match self.session_timeout {
            Some(t) => format!("{SESSION_ID};timeout={t}"),
            None => SESSION_ID.to_string(),
        }
    }
}

pub fn session_id() -> &'static str {
    SESSION_ID
}

fn respond(
    cseq: u32,
    status: u16,
    extra: Vec<(rtsp_types::HeaderName, String)>,
    body: Vec<u8>,
) -> Vec<u8> {
    let mut builder = rtsp_types::Response::builder(Version::V1_0, StatusCode::from(status))
        .header(headers::CSEQ, cseq.to_string());
    for (name, value) in extra {
        builder = builder.header(name, value);
    }
    let response = builder.build(body);
    let mut out = Vec::new();
    Message::from(response)
        .write(&mut out)
        .expect("write response");
    out
}

#[derive(Clone)]
pub enum Chunking {
    Whole,
    Cycle(Vec<usize>),
}

pub struct Host {
    pub session: Session,
    pub server: Server,
    pub now: u64,
    pub conn: Option<u32>,
    pub actions: Vec<Action>,
    pub history: Vec<Event>,
    pub chunking: Chunking,
    pub silent: bool,
    pub recorded: Vec<u8>,
    cycle_pos: usize,
}

impl Host {
    pub fn new(url: &str, config: Config, server: Server) -> Self {
        let session = Session::open(url, config, 0).expect("open rtsp session");
        Self {
            session,
            server,
            now: 0,
            conn: None,
            actions: Vec::new(),
            history: Vec::new(),
            chunking: Chunking::Whole,
            silent: false,
            recorded: Vec::new(),
            cycle_pos: 0,
        }
    }

    pub fn drain(&mut self) {
        let mut steps = 0usize;
        while let Some(action) = self.session.poll(self.now) {
            steps += 1;
            assert!(
                steps < RUNAWAY_LIMIT,
                "runaway action loop: the session keeps producing actions"
            );
            self.actions.push(action.clone());
            match action {
                Action::TcpConnect { conn, .. } => self.conn = Some(conn),
                Action::Send { conn, bytes } if self.conn == Some(conn) && !self.silent => {
                    for response in self.server.feed(&bytes) {
                        self.deliver(&response);
                    }
                }
                Action::Close { conn } if self.conn == Some(conn) => self.conn = None,
                _ => {}
            }
        }
    }

    pub fn connect(&mut self) {
        self.drain();
        let conn = self.conn.expect("a TcpConnect was requested");
        self.session.on_connected(conn);
        self.drain();
    }

    pub fn advance(&mut self, delta_us: u64) {
        self.now += delta_us;
        self.drain();
    }

    pub fn advance_to(&mut self, now_us: u64) {
        self.now = now_us;
        self.drain();
    }

    pub fn deliver(&mut self, bytes: &[u8]) {
        self.recorded.extend_from_slice(bytes);
        let conn = self.conn.expect("a live connection");
        match self.chunking.clone() {
            Chunking::Whole => self.session.on_data(conn, bytes, self.now),
            Chunking::Cycle(sizes) => {
                let mut rest = bytes;
                while !rest.is_empty() {
                    let size = sizes[self.cycle_pos % sizes.len()].max(1).min(rest.len());
                    self.cycle_pos += 1;
                    let (head, tail) = rest.split_at(size);
                    self.session.on_data(conn, head, self.now);
                    rest = tail;
                }
            }
        }
    }

    pub fn deliver_all(&mut self, frames: &[Vec<u8>]) {
        for frame in frames {
            self.deliver(frame);
            self.drain();
        }
    }

    pub fn events(&mut self) -> Vec<Event> {
        let mut fresh = Vec::new();
        while let Some(e) = self.session.next_event() {
            fresh.push(e);
        }
        self.history.extend(fresh.iter().cloned());
        fresh
    }

    pub fn sent_methods(&self) -> Vec<String> {
        self.actions
            .iter()
            .filter_map(|a| match a {
                Action::Send { bytes, .. } => {
                    let line = String::from_utf8_lossy(bytes);
                    line.split_whitespace().next().map(str::to_string)
                }
                _ => None,
            })
            .collect()
    }

    pub fn count_actions(&self, pred: impl Fn(&Action) -> bool) -> usize {
        self.actions.iter().filter(|a| pred(a)).count()
    }

    pub fn clear_actions(&mut self) {
        self.actions.clear();
    }
}

pub fn connects(actions: &[Action]) -> Vec<&Action> {
    actions
        .iter()
        .filter(|a| matches!(a, Action::TcpConnect { .. }))
        .collect()
}

pub fn wakes(actions: &[Action]) -> Vec<u64> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::WakeAt(t) => Some(*t),
            _ => None,
        })
        .collect()
}

pub fn samples_of(events: &[Event]) -> Vec<(u32, Sample)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, sample, .. } => Some((track.value(), sample.clone())),
            _ => None,
        })
        .collect()
}

pub fn quiet_config() -> Config {
    let mut config = Config::default();
    config.ts_http.start_timeout_ms = 24 * 3_600_000;
    config.ts_http.stall_notice_ms = 24 * 3_600_000;
    config.ts_http.stall_reconnect_ms = 48 * 3_600_000;
    config
}
