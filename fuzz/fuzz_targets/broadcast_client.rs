#![no_main]

use std::collections::BTreeSet;
use std::sync::OnceLock;

use broadcast_client::{
    Action, Config, Credentials, DashClockPolicy, Event, Hint, Session, TrackId,
};
use broadcast_common::{Package, Unpackage};
use libfuzzer_sys::fuzz_target;

const URLS: [&str; 4] = [
    "http://example/live.ts",
    "http://example/live.m3u8",
    "http://example/master.m3u8",
    "http://example/encrypted.m3u8",
];
const ORIGIN_PREFIX: &str = "http://example/";
const CHUNK_BYTES: usize = 32;
const HLS_MASTER_SEED: &[u8] = b"#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"En\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"audio/en.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"De\",LANGUAGE=\"de\",AUTOSELECT=YES,URI=\"http://cdn/audio/de.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Mux\",DEFAULT=NO\n#EXT-X-STREAM-INF:BANDWIDTH=800000,AUDIO=\"a\"\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=900000,AUDIO=\"a\"\nhigh.m3u8\n";
const HLS_SEED: &[u8] = b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nseg0.ts\n#EXTINF:2.0,\nseg1.ts\n";
const HLS_ENC_SEED: &[u8] = b"#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x000102030405060708090A0B0C0D0E0F\n#EXTINF:2.0,\nseg0.ts\n#EXTINF:2.0,\nseg1.ts\n";
const HLS_CROSS_ORIGIN_KEY_SEED: &[u8] = b"#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-KEY:METHOD=AES-128,URI=\"http://keys.other/key.bin\",IV=0x000102030405060708090A0B0C0D0E0F\n#EXTINF:2.0,\nseg0.ts\n";
const KEY: [u8; 16] = [0x42; 16];
const IV: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
];
const TS_SOURCES: [&[u8]; 4] = [
    include_bytes!("../../fixtures/ts/h264_aac.ts"),
    include_bytes!("../../fixtures/ts/legacy/mpeg2_mp2.ts"),
    include_bytes!("../../fixtures/ts/hevc/main.ts"),
    include_bytes!("../../fixtures/ts/aac-5_1-640k.ts"),
];
const PMT_FLIP_SOURCE: usize = TS_SOURCES.len();
const TS_SOURCE_COUNT: usize = TS_SOURCES.len() + 1;
const PAT_PID: u16 = 0;
const PUSI: u8 = 0x40;
const PID_HIGH_MASK: u8 = 0x1f;
const SECTION_HEADER: usize = 3;
const CRC_LEN: usize = 4;
const PAT_PROGRAM_OFFSET: usize = 8;
const PAT_ENTRY: usize = 4;
const PMT_FIXED: usize = 12;
const PMT_ES_FIXED: usize = 5;
const PMT_VERSION_BYTE: usize = 5;
const PMT_VERSION_BASE: u8 = 0xC1;
const PMT_VERSION_MASK: u8 = 0x1f;
const PMT_DROP_FROM: usize = 3;
const PMT_DROP_UNTIL: usize = 6;
const PMT_DIVISIONS: usize = 9;
const CONFIG_HEADER: usize = 8;
const MAX_PUMP_ACTIONS: usize = 4096;
const TS_PACKET: usize = 188;
const TS_SEED_PACKETS: usize = 160;
const STEP_US: u64 = 700_000;
const BACKWARD_US: u64 = 5 * STEP_US;
const TIMER_JUMP_US: u64 = 20_000_000;
const RTSP_URLS: [&str; 2] = ["rtsp://example/live/", "rtsps://example:8322/live/"];
const RTSP_REPLY_STATUSES: [u16; 6] = [200, 200, 200, 401, 404, 461];
const INTERLEAVED_MAGIC: u8 = 0x24;
const MTU: usize = 1400;
const RTP_SSRC: u32 = 0x1234_5678;
const RTP_TRACKS: usize = 2;
const CHANNEL_STEP: u8 = 2;
const MAX_PACKET_RUN: usize = 64;
const POSITION_UNIT: u64 = 2_700_000;
const STORM_ROUNDS: usize = 24;
const STORM_FIRST_STATUSES: [u16; 5] = [302, 401, 404, 307, 301];
const STORM_THEN_STATUSES: [u16; 8] = [401, 401, 404, 500, 407, 403, 410, 302];
const BASIC_CHALLENGE: &str = "Basic realm=\"f\"";
const PLAYER_HINTS: [Hint; 3] = [Hint::SourceEnded, Hint::SourceError, Hint::PcrDiscontinuity];
const RENDERED_IDS: [u32; 7] = [1, 2, 3, 1_001, 1_002, 999, u32::MAX];
const HOLD_WALL_US: [u64; 4] = [0, 1_000, 1_000_000, u64::MAX];
const START_WAIT_US: [u64; 4] = [0, 500_000, 1_500_000, u64::MAX];
const DASH_US: [u64; 4] = [1, 1_000, 20_000_000, u64::MAX];
const DASH_ZERO_OK_US: [u64; 4] = [0, 1_000, 20_000_000, u64::MAX];
const DASH_LIVE_DELAY_US: [Option<u64>; 4] = [None, Some(0), Some(6_000_000), Some(u64::MAX)];
const DASH_POLICIES: [DashClockPolicy; 4] = [
    DashClockPolicy::Auto,
    DashClockPolicy::PreferHost,
    DashClockPolicy::Auto,
    DashClockPolicy::PreferHost,
];
const DASH_FLAGS: [(bool, bool); 4] = [(false, true), (true, true), (false, false), (true, false)];
const HOLD_MEDIA_US: [u64; 4] = [0, 1_000, 1_000_000, u64::MAX];
const HOLD_QUIET_BYTES: [usize; 4] = [1, TS_PACKET, 512 * 1024, 1 << 30];
const HOLD_MAX_BYTES: [usize; 4] = [0, TS_PACKET, 4 * 1024 * 1024, usize::MAX];
const QUEUE_CAPS: [usize; 4] = [1, 16, 100_000, usize::MAX];
const JUMP_US: [u64; 4] = [1, 1_000_000, 5_000_000, u64::MAX];
const LEAD_US: [u64; 4] = [0, 1_000_000, 6_000_000, u64::MAX];
const KEY_FAILURES: [u32; 4] = [0, 1, 3, u32::MAX];
const SEGMENT_FAILURES: [u32; 4] = [0, 1, 3, u32::MAX];
const REDIRECTS: [u32; 4] = [0, 1, 5, u32::MAX];
const INFLIGHT: [usize; 4] = [0, 1, 4, usize::MAX];
const BYTE_LIMITS: [usize; 4] = [1, 4096, 4 * 1024 * 1024, usize::MAX];
const TIMEOUTS_US: [u64; 4] = [1, 1_000, 20_000_000, u64::MAX];
const COOKIE_NAMES: [&str; 6] = ["sid", "a", "t", "__Host-h", "__Secure-s", "p"];
const COOKIE_PREFIXES: [&str; 8] = [
    "",
    "Secure; ",
    "Path=/; ",
    "Secure; Path=/; ",
    "Path=/x; ",
    "Domain=example; ",
    "Domain=.EXAMPLE; Path=/; ",
    "Domain=other.example; ",
];
const STATUS_CODES: [u16; 9] = [200, 200, 404, 302, 401, 500, 307, 429, 503];
const RETRY_AFTER_VALUES: [&str; 8] = [
    "30",
    "0",
    "99999999999999999999",
    "Mon, 26 Oct 2026 08:39:34 GMT",
    "Sunday, 06-Nov-94 08:49:37 GMT",
    "Sun Nov  6 08:49:37 1994",
    "",
    "soon",
];
const RESPONSE_DATES: [&str; 4] = [
    "Mon, 26 Oct 2026 08:39:04 GMT",
    "Thu, 01 Jan 1970 00:00:00 GMT",
    "Thu, 30 Dec 9999 00:00:00 GMT",
    "yesterday",
];
const WALL_UNIX_US: [u64; 4] = [0, 1, 1_793_003_944_000_000, u64::MAX];
const WALL_CLOCK_BYTE: usize = 7;
const WALL_CLOCK_PERIOD: u8 = 8;
const LOCATIONS: [&str; 4] = [
    "http://example/next.ts",
    "http://other.example/x.ts",
    "/rel/next.m3u8",
    "https://cdn.other/seg.ts",
];
const STALE_CHALLENGE: &str = "Digest realm=\"f\", nonce=\"n\", qop=\"auth\", stale=true";
const AUDIT_HEADERS: [&str; 3] = ["authorization", "cookie", "x-fuzz"];

fn pid_of(p: &[u8]) -> u16 {
    (u16::from(p[1] & PID_HIGH_MASK) << 8) | u16::from(p[2])
}

fn section_of(p: &[u8]) -> &[u8] {
    &p[5 + usize::from(p[4])..]
}

fn section_len(s: &[u8]) -> usize {
    SECTION_HEADER + ((usize::from(s[1] & 0xf) << 8) | usize::from(s[2]))
}

fn pmt_pid_of(ts: &[u8]) -> Option<u16> {
    let pat = ts
        .chunks_exact(TS_PACKET)
        .find(|p| pid_of(p) == PAT_PID && p[1] & PUSI != 0)?;
    let s = section_of(pat);
    let end = section_len(s).checked_sub(CRC_LEN)?;
    let mut i = PAT_PROGRAM_OFFSET;
    while i + PAT_ENTRY <= end {
        if u16::from_be_bytes([s[i], s[i + 1]]) != 0 {
            return Some((u16::from(s[i + 2] & PID_HIGH_MASK) << 8) | u16::from(s[i + 3]));
        }
        i += PAT_ENTRY;
    }
    None
}

fn rebuilt_pmt(packet: &[u8], keep_every_es: bool, version: u8) -> Option<Vec<u8>> {
    let s = section_of(packet);
    let len = section_len(s);
    if 5 + usize::from(packet[4]) + len > TS_PACKET || len < PMT_FIXED + CRC_LEN {
        return None;
    }
    let section = &s[..len];
    let info_len = (usize::from(section[10] & 0xf) << 8) | usize::from(section[11]);
    let mut out = section[..PMT_FIXED + info_len].to_vec();
    out[PMT_VERSION_BYTE] = PMT_VERSION_BASE | ((version & PMT_VERSION_MASK) << 1);
    let mut j = PMT_FIXED + info_len;
    let mut entries: Vec<&[u8]> = Vec::new();
    while j + PMT_ES_FIXED <= len - CRC_LEN {
        let il = (usize::from(section[j + 3] & 0xf) << 8) | usize::from(section[j + 4]);
        entries.push(&section[j..(j + PMT_ES_FIXED + il).min(len - CRC_LEN)]);
        j += PMT_ES_FIXED + il;
    }
    if !keep_every_es {
        entries.pop();
    }
    for e in entries {
        out.extend_from_slice(e);
    }
    let new_len = out.len() - SECTION_HEADER + CRC_LEN;
    out[1] = (out[1] & 0xf0) | (new_len >> 8) as u8;
    out[2] = new_len as u8;
    let crc = broadcast_common::crc32_mpeg2::compute(&out);
    out.extend_from_slice(&crc.to_be_bytes());
    let mut p = vec![
        0x47,
        PUSI | (packet[1] & PID_HIGH_MASK),
        packet[2],
        packet[3],
        0,
    ];
    p.extend_from_slice(&out);
    p.resize(TS_PACKET, 0xff);
    Some(p)
}

fn pmt_flipped(ts: &[u8]) -> Vec<u8> {
    let Some(pmt_pid) = pmt_pid_of(ts) else {
        return ts.to_vec();
    };
    let total = ts.len() / TS_PACKET;
    let mut out = Vec::with_capacity(ts.len());
    for (i, p) in ts.chunks_exact(TS_PACKET).enumerate() {
        if pid_of(p) == pmt_pid && p[1] & PUSI != 0 {
            let phase = i * PMT_DIVISIONS / total.max(1);
            let rebuilt = if (PMT_DROP_FROM..PMT_DROP_UNTIL).contains(&phase) {
                rebuilt_pmt(p, false, 1)
            } else if phase >= PMT_DROP_UNTIL {
                rebuilt_pmt(p, true, 2)
            } else {
                None
            };
            if let Some(r) = rebuilt {
                out.extend_from_slice(&r);
                continue;
            }
        }
        out.extend_from_slice(p);
    }
    out
}

fn ts_source(index: usize) -> &'static [u8] {
    static FLIPPED: OnceLock<Vec<u8>> = OnceLock::new();
    match index % TS_SOURCE_COUNT {
        PMT_FLIP_SOURCE => FLIPPED.get_or_init(|| pmt_flipped(TS_SOURCES[0])),
        n => TS_SOURCES[n],
    }
}

fn ts_seed(source: usize, offset: usize, packets: usize) -> &'static [u8] {
    let ts = ts_source(source);
    let total = (ts.len() / TS_PACKET).max(1);
    let first = offset % total;
    let last = (first + packets).min(total);
    &ts[first * TS_PACKET..last * TS_PACKET]
}

struct RtpSeed {
    sdp: String,
    packets: Vec<Vec<Vec<u8>>>,
}

fn rtp_seed() -> &'static RtpSeed {
    static SEED: OnceLock<RtpSeed> = OnceLock::new();
    SEED.get_or_init(|| {
        let media = transmux::TsDemux::new()
            .unpackage(TS_SOURCES[0])
            .expect("demux fixture");
        let mut packetiser = transmux::RtpPacketiser {
            mtu: MTU,
            ssrc: RTP_SSRC,
            stap_a_parameter_sets: false,
            ..transmux::RtpPacketiser::default()
        };
        let out = packetiser.package(&media).expect("packetise fixture");
        let mut sdp = String::new();
        let mut media_sections = 0usize;
        for line in out.sdp.lines() {
            sdp.push_str(line);
            sdp.push_str("\r\n");
            if line.starts_with("m=") {
                media_sections += 1;
                sdp.push_str(&format!("a=control:trackID={media_sections}\r\n"));
            }
        }
        let packets = out
            .streams
            .iter()
            .map(|stream| {
                stream
                    .packets
                    .iter()
                    .map(|p| p.as_contiguous().to_vec())
                    .collect()
            })
            .collect();
        RtpSeed { sdp, packets }
    })
}

fn interleaved_frame(channel: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![INTERLEAVED_MAGIC, channel];
    frame.extend_from_slice(&u16::try_from(payload.len()).unwrap_or(0).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn seed_frame(cursors: &mut [usize; RTP_TRACKS], chunk: &[u8]) -> Vec<u8> {
    let seed = rtp_seed();
    let byte = |i: usize| chunk.get(i).copied().unwrap_or(0);
    let track = usize::from(byte(1)) % seed.packets.len().max(1);
    let packets = &seed.packets[track];
    let cursor = &mut cursors[track % RTP_TRACKS];
    let mut packet = packets[*cursor % packets.len().max(1)].clone();
    *cursor = cursor.wrapping_add(1 + usize::from(byte(2) % 3 == 0));
    if byte(2) % 4 == 0 && !packet.is_empty() {
        let at = usize::from(byte(3)) % packet.len();
        packet[at] ^= byte(4).max(1);
    }
    let channel = u8::try_from(track).unwrap_or(0) * CHANNEL_STEP + u8::from(byte(5) % 8 == 0);
    interleaved_frame(channel, &packet)
}

fn player(session: &mut Session, now: u64, chunk: &[u8]) {
    let byte = |i: usize| chunk.get(i).copied().unwrap_or(0);
    match byte(2) % 5 {
        0 => {
            let position = match byte(3) % 8 {
                0 => u64::MAX,
                _ => u64::from(byte(4)) * POSITION_UNIT,
            };
            session.on_playback_position(now, Some(position));
        }
        1 => session.on_playback_position(now, None),
        2 => {
            let ids: Vec<TrackId> = RENDERED_IDS
                .iter()
                .enumerate()
                .filter(|(n, _)| byte(3) >> (n % 8) & 1 == 1)
                .map(|(_, id)| TrackId::new(*id))
                .collect();
            session.set_rendered_tracks(&ids);
        }
        3 => session.rejoin(),
        _ => session.hint(PLAYER_HINTS[usize::from(byte(3)) % PLAYER_HINTS.len()]),
    }
}

fn storm(session: &mut Session, watch: &mut Watch, now: u64, chunk: &[u8]) {
    let byte = |i: usize| usize::from(chunk.get(i).copied().unwrap_or(0));
    if byte(3) % 3 != 0 {
        let _ = session.set_credentials(Credentials::new("user", "pass"));
    }
    let first = STORM_FIRST_STATUSES[byte(1) % STORM_FIRST_STATUSES.len()];
    let then = STORM_THEN_STATUSES[byte(4) % STORM_THEN_STATUSES.len()];
    let mut answered: Vec<(u32, u16)> = Vec::new();
    let mut rounds = 0usize;
    loop {
        let requests = watch.actions(session, now);
        if requests.is_empty() {
            break;
        }
        rounds += 1;
        if now == u64::MAX && rounds > STORM_ROUNDS {
            break;
        }
        assert!(
            rounds <= STORM_ROUNDS,
            "{rounds} rounds of undelayed requests answered {first}/{then} at {now}"
        );
        let status = if rounds == 1 { first } else { then };
        for (conn, _) in requests {
            let mut headers = vec![
                (
                    "Location".to_string(),
                    LOCATIONS[(byte(2) + rounds) % LOCATIONS.len()].to_string(),
                ),
                ("WWW-Authenticate".to_string(), BASIC_CHALLENGE.to_string()),
            ];
            if byte(5) % 2 == 0 {
                headers.push(("Set-Cookie".to_string(), "storm=1".to_string()));
            }
            answered.push((conn, status));
            session.on_response(conn, status, &headers, now);
        }
    }
    if now != u64::MAX {
        for (conn, status) in answered {
            assert!(
                !watch.live.contains(&conn),
                "conn {conn} answered {status} is still open"
            );
        }
    }
}

fn finish(session: &mut Session, watch: &mut Watch, now: u64) {
    if !watch.closed {
        session.close();
        watch.closed = true;
    }
    watch.actions(session, now);
    watch.events(session);
    assert!(
        watch.live.is_empty(),
        "connections never closed after close(): {:?}",
        watch.live
    );
    assert!(session.poll(now).is_none(), "action after drained close()");
}

fn encrypted_segment() -> &'static [u8] {
    static SEGMENT: OnceLock<Vec<u8>> = OnceLock::new();
    SEGMENT.get_or_init(|| {
        transmux::sample_aes::aes128_encrypt_segment(&KEY, &IV, ts_seed(0, 0, TS_SEED_PACKETS))
    })
}

#[derive(Default)]
struct Watch {
    live: BTreeSet<u32>,
    fatal: bool,
    closed: bool,
}

impl Watch {
    fn actions(&mut self, session: &mut Session, now: u64) -> Vec<(u32, String)> {
        let mut requests = Vec::new();
        let mut steps = 0usize;
        loop {
            if steps >= MAX_PUMP_ACTIONS {
                assert!(now == u64::MAX, "runaway action loop at {now}");
                break;
            }
            let Some(action) = session.poll(now) else {
                break;
            };
            steps += 1;
            match action {
                Action::HttpRequest {
                    conn, url, headers, ..
                } => {
                    assert!(!self.closed, "HttpRequest after close");
                    assert!(self.live.insert(conn), "live conn id {conn} issued twice");
                    if !url.starts_with(ORIGIN_PREFIX) {
                        for (name, _) in &headers {
                            assert!(
                                !AUDIT_HEADERS.iter().any(|h| name.eq_ignore_ascii_case(h)),
                                "{name} sent to {url}"
                            );
                        }
                    }
                    requests.push((conn, url));
                }
                Action::TcpConnect { conn, .. } => {
                    assert!(!self.closed, "TcpConnect after close");
                    assert!(self.live.insert(conn), "live conn id {conn} issued twice");
                }
                Action::Close { conn } => {
                    self.live.remove(&conn);
                }
                Action::WakeAt(at) => {
                    assert!(!self.closed, "WakeAt after close");
                    assert!(at > now, "WakeAt({at}) is not after now {now}");
                }
                _ => {}
            }
        }
        requests
    }

    fn events(&mut self, session: &mut Session) {
        while let Some(event) = session.next_event() {
            assert!(!self.closed, "event after close: {event:?}");
            assert!(!self.fatal, "event after a fatal error: {event:?}");
            if matches!(event, Event::Error { fatal: true, .. }) {
                self.fatal = true;
            }
        }
    }

    fn host_closed(&mut self, conn: u32) {
        self.live.remove(&conn);
    }
}

fn next_time(now: u64, chunk: &[u8]) -> u64 {
    match chunk.get(2).copied().unwrap_or(0) % 24 {
        0 => now.saturating_add(TIMER_JUMP_US),
        1 => now.saturating_sub(BACKWARD_US),
        2 => u64::MAX,
        3 => STEP_US,
        _ => now.saturating_add(STEP_US),
    }
}

fn body_for(url: &str, encrypted: bool, chunk: &[u8]) -> Vec<u8> {
    if url.contains("keys.other") || url.contains("key") {
        return KEY.to_vec();
    }
    if url.contains("master") {
        return HLS_MASTER_SEED.to_vec();
    }
    if url.ends_with(".m3u8") {
        return match (encrypted, chunk.get(1).copied().unwrap_or(0) % 5) {
            (true, 0) => HLS_CROSS_ORIGIN_KEY_SEED.to_vec(),
            (true, _) => HLS_ENC_SEED.to_vec(),
            (false, _) => HLS_SEED.to_vec(),
        };
    }
    if encrypted {
        encrypted_segment().to_vec()
    } else {
        ts_seed(
            usize::from(chunk.get(6).copied().unwrap_or(0)),
            usize::from(chunk.get(1).copied().unwrap_or(0)),
            TS_SEED_PACKETS,
        )
        .to_vec()
    }
}

fn serve(session: &mut Session, watch: &mut Watch, now: u64, encrypted: bool, chunk: &[u8]) {
    for (conn, url) in watch.actions(session, now) {
        session.on_response(conn, 200, &[], now);
        let body = body_for(&url, encrypted, chunk);
        session.on_data(conn, &body, now);
        session.on_data(conn, chunk, now);
        session.on_closed(conn, None, now);
        watch.host_closed(conn);
    }
}

const KNOB_BITS: usize = 2;
const KNOB_MASK: u8 = 3;
const KNOBS_PER_BYTE: usize = 8 / KNOB_BITS;

fn knob<T: Copy>(header: &[u8], field: usize, table: &[T; 4]) -> T {
    let byte = header.get(field / KNOBS_PER_BYTE).copied().unwrap_or(0);
    let index = (byte >> ((field % KNOBS_PER_BYTE) * KNOB_BITS)) & KNOB_MASK;
    table[usize::from(index)]
}

fn config_from(header: &[u8]) -> Config {
    let mut config = Config::default();
    config.preferred_audio_language = Some("de".to_string());
    config.initial_cookies = vec![("seed".to_string(), "1".to_string())];
    config.extra_headers = vec![("X-Fuzz".to_string(), "1".to_string())];
    config.max_key_failures = knob(header, 0, &KEY_FAILURES);
    config.max_redirects = knob(header, 1, &REDIRECTS);
    config.max_inflight_segments = knob(header, 2, &INFLIGHT);
    config.max_queued_events = knob(header, 3, &QUEUE_CAPS);
    config.max_manifest_bytes = knob(header, 4, &BYTE_LIMITS);
    config.max_segment_bytes = knob(header, 5, &BYTE_LIMITS);
    config.request_timeout_us = knob(header, 6, &TIMEOUTS_US);
    config.segment_timeout_us = knob(header, 7, &TIMEOUTS_US);
    config.tracks_hold_max_wall_us = knob(header, 8, &HOLD_WALL_US);
    config.tracks_hold_max_media_us = knob(header, 9, &HOLD_MEDIA_US);
    let quiet = knob(header, 10, &HOLD_QUIET_BYTES);
    config.tracks_hold_quiet_input_bytes = quiet;
    config.tracks_hold_max_input_bytes = knob(header, 11, &HOLD_MAX_BYTES).max(quiet);
    config.timeline_jump = match knob(header, 12, &[0u8, 1, 2, 3]) {
        0 => None,
        _ => Some(broadcast_client::TimelineJumpConfig {
            backward_us: knob(header, 13, &JUMP_US),
            forward_slack_us: knob(header, 14, &JUMP_US),
        }),
    };
    config.playout.vod_start_lead_us = knob(header, 15, &LEAD_US);
    config.playout.live_start_lead_us = knob(header, 16, &LEAD_US);
    config.playout.pause_lead_us = knob(header, 17, &LEAD_US);
    config.start_pending_track_wait_us = knob(header, 18, &START_WAIT_US);
    config.max_segment_failures = knob(header, 19, &SEGMENT_FAILURES);
    config.dash.clock_policy = knob(header, 20, &DASH_POLICIES);
    let utc_timeout = knob(header, 21, &DASH_US);
    config.dash.utc_timeout_us = utc_timeout;
    config.dash.utc_wait_us = knob(header, 22, &DASH_US).max(utc_timeout);
    config.dash.utc_max_rtt_us = knob(header, 23, &DASH_US);
    config.dash.utc_resync_us = knob(header, 24, &DASH_US);
    config.dash.host_clock_uncertainty_us = knob(header, 25, &DASH_ZERO_OK_US);
    config.dash.live_delay_us = knob(header, 26, &DASH_LIVE_DELAY_US);
    config.dash.availability_margin_us = knob(header, 27, &DASH_ZERO_OK_US);
    config.dash.early_retry_us = knob(header, 28, &DASH_US);
    config.dash.early_grace_us = knob(header, 29, &DASH_ZERO_OK_US);
    config.dash.min_mpd_refresh_us = knob(header, 30, &DASH_US);
    let (offset, cross_origin) = knob(header, 31, &DASH_FLAGS);
    config.dash.use_availability_time_offset = offset;
    config.dash.cross_origin_urls = cross_origin;
    config
}

fn wall_clock(session: &mut Session, now: u64, chunk: &[u8]) {
    let byte = |i: usize| usize::from(chunk.get(i).copied().unwrap_or(0));
    if chunk.get(WALL_CLOCK_BYTE).copied().unwrap_or(1) % WALL_CLOCK_PERIOD != 0 {
        return;
    }
    let mono = match byte(3) % 4 {
        0 => 0,
        1 => u64::MAX,
        _ => now,
    };
    session.set_wall_clock(mono, WALL_UNIX_US[byte(4) % WALL_UNIX_US.len()]);
}

fn split_header(data: &[u8]) -> (&[u8], &[u8]) {
    data.split_at(CONFIG_HEADER.min(data.len()))
}

fn drive(url: &str, data: &[u8]) {
    let (header, data) = split_header(data);
    let config = config_from(header);
    let mut session = match Session::open(url, config, 0) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut watch = Watch::default();
    let mut now = 0u64;
    let mut conn_ids: Vec<u32> = Vec::new();
    for chunk in data.chunks(CHUNK_BYTES) {
        now = next_time(now, chunk);
        let pick = chunk.first().copied().unwrap_or(0);
        let second = usize::from(chunk.get(1).copied().unwrap_or(0));
        let conn = conn_ids.get(second % conn_ids.len().max(1)).copied();
        wall_clock(&mut session, now, chunk);
        match pick % 12 {
            0 => serve(&mut session, &mut watch, now, false, chunk),
            1 => {
                if let Some(conn) = conn {
                    let slice = if second % 2 == 0 {
                        ts_seed(second, second, 1 + second % 8)
                    } else {
                        chunk
                    };
                    session.on_data(conn, slice, now);
                }
            }
            2 => {
                if let Some(conn) = conn {
                    session.on_closed(conn, None, now);
                    watch.host_closed(conn);
                }
            }
            3 => {
                let creds = if second % 2 == 0 {
                    Credentials::bearer("token")
                } else {
                    Credentials::new("user", "pass")
                };
                let _ = session.set_credentials(creds);
            }
            4 => session.reconnect(now),
            5 => {
                if let Some(conn) = conn {
                    let status = STATUS_CODES[second % STATUS_CODES.len()];
                    let location = LOCATIONS[second % LOCATIONS.len()];
                    let mut headers = vec![("Location".to_string(), location.to_string())];
                    let attrs = String::from_utf8_lossy(chunk.get(2..).unwrap_or(&[])).into_owned();
                    let name = COOKIE_NAMES[second % COOKIE_NAMES.len()];
                    let prefix = COOKIE_PREFIXES[second % COOKIE_PREFIXES.len()];
                    headers.push((
                        "Set-Cookie".to_string(),
                        format!("{name}=v; {prefix}{attrs}"),
                    ));
                    headers.push(("Set-Cookie".to_string(), attrs));
                    if status == 401 && second % 3 == 0 {
                        headers.push(("WWW-Authenticate".to_string(), STALE_CHALLENGE.to_string()));
                    }
                    let retry_after = match second % (RETRY_AFTER_VALUES.len() + 1) {
                        n if n < RETRY_AFTER_VALUES.len() => RETRY_AFTER_VALUES[n].to_string(),
                        _ => String::from_utf8_lossy(chunk.get(8..).unwrap_or(&[])).into_owned(),
                    };
                    headers.push(("Retry-After".to_string(), retry_after));
                    if second % 2 == 1 {
                        headers.push((
                            "Date".to_string(),
                            RESPONSE_DATES[second % RESPONSE_DATES.len()].to_string(),
                        ));
                    }
                    session.on_response(conn, status, &headers, now);
                }
            }
            6 => {
                if let Some(conn) = conn {
                    session.on_closed(conn, Some("reset".to_string()), now);
                    watch.host_closed(conn);
                }
            }
            7 => {
                let _ = session.select_variant(second);
            }
            8 => serve(&mut session, &mut watch, now, true, chunk),
            10 => player(&mut session, now, chunk),
            11 => storm(&mut session, &mut watch, now, chunk),
            _ => {
                let _ = session.select_audio_rendition(
                    chunk
                        .get(1)
                        .copied()
                        .filter(|b| b % 4 != 0)
                        .map(|b| usize::from(b % 4)),
                );
            }
        }
        if pick == u8::MAX {
            session.close();
            watch.closed = true;
        }
        for (conn, _) in watch.actions(&mut session, now) {
            conn_ids.push(conn);
        }
        let _ = session.cookies();
        watch.events(&mut session);
    }
    finish(&mut session, &mut watch, now);
}

fn rtsp_cseq(request: &[u8]) -> u32 {
    String::from_utf8_lossy(request)
        .lines()
        .find_map(|l| l.strip_prefix("CSeq:").and_then(|v| v.trim().parse().ok()))
        .unwrap_or(0)
}

fn rtsp_reply(request: &[u8], status: u16) -> Vec<u8> {
    let cseq = rtsp_cseq(request);
    let text = String::from_utf8_lossy(request);
    let is_describe = text.starts_with("DESCRIBE");
    let is_setup = text.starts_with("SETUP");
    let transport = text
        .lines()
        .find_map(|l| l.strip_prefix("Transport:"))
        .map(|v| v.trim().to_string())
        .unwrap_or_default();
    let mut head = format!("RTSP/1.0 {status} X\r\nCSeq: {cseq}\r\n");
    if status == 401 {
        head.push_str(&format!("WWW-Authenticate: {STALE_CHALLENGE}\r\n"));
    }
    if is_setup {
        head.push_str(&format!(
            "Session: 1;timeout=10\r\nTransport: {transport}\r\n"
        ));
    }
    if is_describe && status == 200 {
        let sdp = &rtp_seed().sdp;
        head.push_str(&format!(
            "Content-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{sdp}",
            sdp.len()
        ));
    } else {
        head.push_str("\r\n");
    }
    head.into_bytes()
}

fn rtsp_pump(
    session: &mut Session,
    watch: &mut Watch,
    now: u64,
    conns: &mut Vec<u32>,
    status: u16,
    tail: &[u8],
) {
    let mut steps = 0usize;
    loop {
        if steps >= MAX_PUMP_ACTIONS {
            assert!(now == u64::MAX, "runaway action loop at {now}");
            break;
        }
        let Some(action) = session.poll(now) else {
            break;
        };
        steps += 1;
        match action {
            Action::TcpConnect { conn, .. } => {
                assert!(!watch.closed, "TcpConnect after close");
                assert!(watch.live.insert(conn), "live conn id {conn} issued twice");
                conns.push(conn);
                session.on_connected(conn);
            }
            Action::Send { conn, bytes } => {
                let reply = rtsp_reply(&bytes, status);
                session.on_data(conn, &reply, now);
                if !tail.is_empty() {
                    session.on_data(conn, tail, now);
                }
            }
            Action::Close { conn } => {
                watch.live.remove(&conn);
            }
            Action::WakeAt(_) => assert!(!watch.closed, "WakeAt after close"),
            _ => {}
        }
    }
}

fn drive_rtsp(url: &str, data: &[u8]) {
    let (header, data) = split_header(data);
    let mut session = match Session::open(url, config_from(header), 0) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut watch = Watch::default();
    let mut now = 0u64;
    let mut conns: Vec<u32> = Vec::new();
    let mut cursors = [0usize; RTP_TRACKS];
    for chunk in data.chunks(CHUNK_BYTES) {
        now = next_time(now, chunk);
        let pick = chunk.first().copied().unwrap_or(0);
        let second = usize::from(chunk.get(1).copied().unwrap_or(0));
        let status = RTSP_REPLY_STATUSES[second % RTSP_REPLY_STATUSES.len()];
        let conn = conns.get(second % conns.len().max(1)).copied();
        wall_clock(&mut session, now, chunk);
        match pick % 10 {
            0 | 1 => rtsp_pump(&mut session, &mut watch, now, &mut conns, status, &[]),
            2 => {
                if let Some(conn) = conn {
                    let frame = seed_frame(&mut cursors, chunk);
                    session.on_data(conn, &frame, now);
                }
            }
            3 => {
                if let Some(conn) = conn {
                    session.on_data(conn, chunk, now);
                }
            }
            4 => {
                if let Some(conn) = conn {
                    let error = (second % 2 == 0).then(|| "reset".to_string());
                    session.on_closed(conn, error, now);
                    watch.live.remove(&conn);
                }
            }
            5 => {
                let _ = session.set_credentials(Credentials::new("u", "p"));
            }
            6 => session.reconnect(now),
            7 => now = now.saturating_add(TIMER_JUMP_US),
            8 => {
                if let Some(conn) = conn {
                    for _ in 0..=second % MAX_PACKET_RUN {
                        let frame = seed_frame(&mut cursors, &[chunk[0], 0, 1, 0, 0, 1]);
                        session.on_data(conn, &frame, now);
                        let frame = seed_frame(&mut cursors, &[chunk[0], 1, 1, 0, 0, 1]);
                        session.on_data(conn, &frame, now);
                    }
                }
            }
            _ => player(&mut session, now, chunk),
        }
        if pick == u8::MAX {
            session.close();
            watch.closed = true;
        }
        rtsp_pump(&mut session, &mut watch, now, &mut conns, 200, chunk);
        let _ = session.rtp_loss_count();
        let _ = session.rtp_malformed_count();
        watch.events(&mut session);
    }
    finish(&mut session, &mut watch, now);
}

fuzz_target!(|data: &[u8]| {
    for url in URLS {
        drive(url, data);
    }
    for url in RTSP_URLS {
        drive_rtsp(url, data);
    }
});
