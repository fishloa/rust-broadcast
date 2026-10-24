#![no_main]

use broadcast_client::{Action, Config, Credentials, Session};
use libfuzzer_sys::fuzz_target;

const URLS: [&str; 3] = [
    "http://example/live.ts",
    "http://example/live.m3u8",
    "http://example/master.m3u8",
];
const HLS_MASTER_SEED: &[u8] = b"#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"En\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"audio/en.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"De\",LANGUAGE=\"de\",AUTOSELECT=YES,URI=\"http://cdn/audio/de.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Mux\",DEFAULT=NO\n#EXT-X-STREAM-INF:BANDWIDTH=800000,AUDIO=\"a\"\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=900000,AUDIO=\"a\"\nhigh.m3u8\n";
const HLS_SEED: &[u8] = b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nseg0.ts\n#EXTINF:2.0,\nseg1.ts\n";
const HLS_ENC_SEED: &[u8] = b"#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x000102030405060708090A0B0C0D0E0F\n#EXTINF:2.0,\nseg0.ts\n#EXTINF:2.0,\nseg1.ts\n";
const STEP_US: u64 = 700_000;
const RTSP_URLS: [&str; 2] = ["rtsp://example/live/", "rtsps://example:8322/live/"];
const RTSP_SDP: &str = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 packetization-mode=1; sprop-parameter-sets=Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==\r\na=control:trackID=1\r\n";
const RTSP_REPLY_STATUSES: [u16; 6] = [200, 200, 200, 401, 404, 461];
const INTERLEAVED_MAGIC: u8 = 0x24;
const RTSP_CHANNELS: u8 = 4;
const TIMER_JUMP_US: u64 = 20_000_000;
const COOKIE_NAMES: [&str; 3] = ["sid", "a", "t"];
const STATUS_CODES: [u16; 6] = [200, 200, 404, 302, 401, 500];

fn drive(url: &str, data: &[u8]) {
    let config = Config {
        preferred_audio_language: Some("de".to_string()),
        initial_cookies: vec![("seed".to_string(), "1".to_string())],
        extra_headers: vec![("X-Fuzz".to_string(), "1".to_string())],
        ..Config::default()
    };
    let mut session = match Session::open(url, config, 0) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut now = 0u64;
    let mut conn_ids: Vec<u32> = Vec::new();
    for chunk in data.chunks(32) {
        now = now.saturating_add(STEP_US);
        let pick = chunk.first().copied().unwrap_or(0);
        let conn = conn_ids.get(usize::from(pick) % conn_ids.len().max(1)).copied();
        match pick % 10 {
            0 => {
                while let Some(action) = session.poll(now) {
                    if let Action::HttpRequest { conn, url, .. } = action {
                        conn_ids.push(conn);
                        session.on_response(conn, 200, &[]);
                        let body = if url.contains("master") {
                            HLS_MASTER_SEED
                        } else {
                            HLS_SEED
                        };
                        session.on_data(conn, body, now);
                        session.on_data(conn, chunk, now);
                    }
                }
            }
            1 => {
                if let Some(conn) = conn {
                    session.on_data(conn, chunk, now);
                }
            }
            2 => {
                if let Some(conn) = conn {
                    session.on_closed(conn, None, now);
                }
            }
            3 => {
                session.set_credentials(Credentials::bearer("token"));
            }
            4 => {
                session.reconnect(now);
            }
            5 => {
                if let Some(conn) = conn {
                    let status = STATUS_CODES[usize::from(chunk.get(1).copied().unwrap_or(0)) % STATUS_CODES.len()];
                    let mut headers = vec![("Location".to_string(), "http://example/next.ts".to_string())];
                    let attrs = String::from_utf8_lossy(chunk.get(2..).unwrap_or(&[])).into_owned();
                    let name = COOKIE_NAMES[usize::from(chunk.get(1).copied().unwrap_or(0)) % COOKIE_NAMES.len()];
                    headers.push(("Set-Cookie".to_string(), format!("{name}=v; {attrs}")));
                    headers.push(("Set-Cookie".to_string(), attrs));
                    session.on_response(conn, status, &headers);
                }
            }
            6 => {
                if let Some(conn) = conn {
                    session.on_closed(conn, Some("reset".to_string()), now);
                }
            }
            7 => {
                let _ = session.select_variant(usize::from(chunk.get(1).copied().unwrap_or(0)));
            }
            8 => {
                while let Some(action) = session.poll(now) {
                    if let Action::HttpRequest { conn, url, .. } = action {
                        conn_ids.push(conn);
                        session.on_response(conn, 200, &[]);
                        if url.contains("key") {
                            let len = usize::from(chunk.get(1).copied().unwrap_or(0)) % (chunk.len() + 1);
                            session.on_data(conn, &chunk[..len.min(chunk.len())], now);
                        } else {
                            session.on_data(conn, HLS_ENC_SEED, now);
                            session.on_data(conn, chunk, now);
                        }
                        session.on_closed(conn, None, now);
                    }
                }
            }
            9 => {
                let index = chunk.get(1).copied().filter(|b| b % 4 != 0);
                let _ = session.select_audio_rendition(index.map(|b| usize::from(b % 4)));
            }
            _ => {
                let _ = session.poll(now);
            }
        }
        if pick == u8::MAX {
            session.close();
        }
        while let Some(action) = session.poll(now) {
            if let Action::HttpRequest { conn, .. } = action {
                conn_ids.push(conn);
            }
        }
        let _ = session.cookies();
        while session.next_event().is_some() {}
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
        head.push_str("WWW-Authenticate: Basic realm=\"f\"\r\n");
    }
    if is_setup {
        head.push_str(&format!("Session: 1;timeout=10\r\nTransport: {transport}\r\n"));
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

fn rtsp_pump(session: &mut Session, now: u64, conns: &mut Vec<u32>, status: u16, tail: &[u8]) {
    while let Some(action) = session.poll(now) {
        match action {
            Action::TcpConnect { conn, .. } => {
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
            _ => {}
        }
    }
}

fn drive_rtsp(url: &str, data: &[u8]) {
    let mut session = match Session::open(url, Config::default(), 0) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut now = 0u64;
    let mut conns: Vec<u32> = Vec::new();
    for chunk in data.chunks(32) {
        now = now.saturating_add(STEP_US);
        let pick = chunk.first().copied().unwrap_or(0);
        let second = usize::from(chunk.get(1).copied().unwrap_or(0));
        let status = RTSP_REPLY_STATUSES[second % RTSP_REPLY_STATUSES.len()];
        let conn = conns.get(second % conns.len().max(1)).copied();
        match pick % 8 {
            0 | 1 => rtsp_pump(&mut session, now, &mut conns, status, &[]),
            2 => {
                if let Some(conn) = conn {
                    let mut frame = vec![INTERLEAVED_MAGIC, chunk.get(1).copied().unwrap_or(0) % RTSP_CHANNELS];
                    let payload = chunk.get(2..).unwrap_or(&[]);
                    frame.extend_from_slice(&u16::try_from(payload.len()).unwrap_or(0).to_be_bytes());
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
                }
            }
            5 => session.set_credentials(Credentials::new("u", "p")),
            6 => session.reconnect(now),
            _ => {
                now = now.saturating_add(TIMER_JUMP_US);
                let _ = session.poll(now);
            }
        }
        if pick == u8::MAX {
            session.close();
        }
        rtsp_pump(&mut session, now, &mut conns, 200, chunk);
        let _ = session.rtp_loss_count();
        let _ = session.rtp_malformed_count();
        while session.next_event().is_some() {}
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
