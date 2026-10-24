#![no_main]

use std::collections::BTreeSet;
use std::sync::OnceLock;

use broadcast_client::{Action, Config, Credentials, Event, Session};
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
const TS_FIXTURE: &[u8] = include_bytes!("../../fixtures/ts/h264_aac.ts");
const MAX_PUMP_ACTIONS: usize = 4096;
const TS_PACKET: usize = 188;
const TS_SEED_PACKETS: usize = 160;
const STEP_US: u64 = 700_000;
const BACKWARD_US: u64 = 5 * STEP_US;
const TIMER_JUMP_US: u64 = 20_000_000;
const RTSP_URLS: [&str; 2] = ["rtsp://example/live/", "rtsps://example:8322/live/"];
const RTSP_SDP: &str = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 packetization-mode=1; sprop-parameter-sets=Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==\r\na=control:trackID=1\r\n";
const RTSP_REPLY_STATUSES: [u16; 6] = [200, 200, 200, 401, 404, 461];
const INTERLEAVED_MAGIC: u8 = 0x24;
const RTSP_CHANNELS: u8 = 4;
const COOKIE_NAMES: [&str; 6] = ["sid", "a", "t", "__Host-h", "__Secure-s", "p"];
const COOKIE_PREFIXES: [&str; 5] = ["", "Secure; ", "Path=/; ", "Secure; Path=/; ", "Path=/x; "];
const STATUS_CODES: [u16; 7] = [200, 200, 404, 302, 401, 500, 307];
const LOCATIONS: [&str; 4] = [
    "http://example/next.ts",
    "http://other.example/x.ts",
    "/rel/next.m3u8",
    "https://cdn.other/seg.ts",
];
const STALE_CHALLENGE: &str = "Digest realm=\"f\", nonce=\"n\", qop=\"auth\", stale=true";
const AUDIT_HEADERS: [&str; 3] = ["authorization", "cookie", "x-fuzz"];

fn ts_seed(offset: usize, packets: usize) -> &'static [u8] {
    let total = TS_FIXTURE.len() / TS_PACKET;
    let first = offset % total;
    let last = (first + packets).min(total);
    &TS_FIXTURE[first * TS_PACKET..last * TS_PACKET]
}

fn encrypted_segment() -> &'static [u8] {
    static SEGMENT: OnceLock<Vec<u8>> = OnceLock::new();
    SEGMENT.get_or_init(|| {
        transmux::sample_aes::aes128_encrypt_segment(&KEY, &IV, ts_seed(0, TS_SEED_PACKETS))
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
        while let Some(action) = session.poll(now) {
            steps += 1;
            if steps > MAX_PUMP_ACTIONS {
                assert!(now == u64::MAX, "runaway action loop at {now}");
                break;
            }
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
                Action::WakeAt(_) => assert!(!self.closed, "WakeAt after close"),
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
            usize::from(chunk.get(1).copied().unwrap_or(0)),
            TS_SEED_PACKETS,
        )
        .to_vec()
    }
}

fn serve(session: &mut Session, watch: &mut Watch, now: u64, encrypted: bool, chunk: &[u8]) {
    for (conn, url) in watch.actions(session, now) {
        session.on_response(conn, 200, &[]);
        let body = body_for(&url, encrypted, chunk);
        session.on_data(conn, &body, now);
        session.on_data(conn, chunk, now);
        session.on_closed(conn, None, now);
        watch.host_closed(conn);
    }
}

fn drive(url: &str, data: &[u8]) {
    let config = Config {
        preferred_audio_language: Some("de".to_string()),
        initial_cookies: vec![("seed".to_string(), "1".to_string())],
        extra_headers: vec![("X-Fuzz".to_string(), "1".to_string())],
        max_key_failures: 3,
        ..Config::default()
    };
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
        match pick % 10 {
            0 => serve(&mut session, &mut watch, now, false, chunk),
            1 => {
                if let Some(conn) = conn {
                    let slice = if second % 2 == 0 {
                        ts_seed(second, 1 + second % 8)
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
                    session.on_response(conn, status, &headers);
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
        head.push_str(&format!(
            "Content-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{RTSP_SDP}",
            RTSP_SDP.len()
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
    while let Some(action) = session.poll(now) {
        steps += 1;
        if steps > MAX_PUMP_ACTIONS {
            assert!(now == u64::MAX, "runaway action loop at {now}");
            break;
        }
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
    let mut session = match Session::open(url, Config::default(), 0) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut watch = Watch::default();
    let mut now = 0u64;
    let mut conns: Vec<u32> = Vec::new();
    for chunk in data.chunks(CHUNK_BYTES) {
        now = next_time(now, chunk);
        let pick = chunk.first().copied().unwrap_or(0);
        let second = usize::from(chunk.get(1).copied().unwrap_or(0));
        let status = RTSP_REPLY_STATUSES[second % RTSP_REPLY_STATUSES.len()];
        let conn = conns.get(second % conns.len().max(1)).copied();
        match pick % 8 {
            0 | 1 => rtsp_pump(&mut session, &mut watch, now, &mut conns, status, &[]),
            2 => {
                if let Some(conn) = conn {
                    let mut frame = vec![
                        INTERLEAVED_MAGIC,
                        chunk.get(1).copied().unwrap_or(0) % RTSP_CHANNELS,
                    ];
                    let payload = chunk.get(2..).unwrap_or(&[]);
                    frame.extend_from_slice(
                        &u16::try_from(payload.len()).unwrap_or(0).to_be_bytes(),
                    );
                    frame.extend_from_slice(payload);
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
            _ => {
                now = now.saturating_add(TIMER_JUMP_US);
                let _ = session.poll(now);
            }
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
}

fuzz_target!(|data: &[u8]| {
    for url in URLS {
        drive(url, data);
    }
    for url in RTSP_URLS {
        drive_rtsp(url, data);
    }
});
