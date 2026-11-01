//! Golden of every HTTP request/response the WHIP/WHEP state machines emit
//! (W1-R-low-b, spec §6). Rendered as `status`/`METHOD url`, then sorted
//! lower-cased `name: value` header lines (the old API's separate `content_type`
//! is rendered as a `content-type` header), then the body length. Task 9 renders
//! the typed `HeaderMap` the same way and must match byte-for-byte except for the
//! differences it lists in the CHANGELOG (none, in the end: see the report).

use http::StatusCode;
use webrtc_runtime::whep::{player::WhepPlayer, server::WhepSession};
use webrtc_runtime::whip::{client::WhipClient, server::WhipSession};

const URL: &str = "https://origin.example/whip/live";
const SESSION: &str = "https://origin.example/whip/live/s1";

fn render(headers: &http::HeaderMap) -> String {
    let mut lines: Vec<String> = headers
        .iter()
        .map(|(k, v)| format!("{}: {}", k.as_str(), v.to_str().unwrap()))
        .collect();
    lines.sort();
    lines.iter().map(|l| format!("  {l}\n")).collect()
}

macro_rules! resp {
    ($out:expr, $label:expr, $r:expr) => {{
        let r = $r;
        $out.push_str(&format!(
            "{} -> {}\n{}  body={}B\n",
            $label,
            r.status.as_u16(),
            render(&r.headers),
            r.body.len()
        ));
    }};
}

macro_rules! req {
    ($out:expr, $label:expr, $r:expr) => {{
        let r = $r;
        $out.push_str(&format!(
            "{} -> {} {}\n{}  body={}B\n",
            $label,
            r.method,
            r.url,
            render(&r.headers),
            r.body.len()
        ));
    }};
}

#[test]
fn whip_whep_http_output_is_byte_identical_to_golden() {
    let mut out = String::new();

    // WHIP server
    let mut s = WhipSession::new(SESSION.into());
    resp!(
        out,
        "whip accept",
        s.accept(b"sdp".to_vec(), "etag1".into())
    );
    resp!(out, "whip ack_trickle", s.ack_trickle());
    resp!(
        out,
        "whip ack_restart",
        s.ack_restart(b"frag".to_vec(), "etag2".into())
    );
    resp!(out, "whip ack_delete", s.ack_delete());

    // WHEP server
    let mut s = WhepSession::new(SESSION.into());
    resp!(out, "whep accept", s.accept(b"sdp".to_vec(), "e1".into()));
    let mut s = WhepSession::new(SESSION.into());
    resp!(
        out,
        "whep counter_offer(None)",
        s.counter_offer(b"o".to_vec(), None)
    );
    let mut s = WhepSession::new(SESSION.into());
    resp!(
        out,
        "whep counter_offer(valid-until)",
        s.counter_offer(b"o".to_vec(), Some("2030-01-01T00:00:00Z".into()))
    );
    resp!(out, "whep ack_answer", s.ack_answer("e2".into()));
    resp!(out, "whep ack_trickle", s.ack_trickle());
    resp!(
        out,
        "whep ack_restart",
        s.ack_restart(b"frag".to_vec(), "e3".into())
    );
    resp!(
        out,
        "whep no_publisher(Some(5))",
        WhepSession::no_publisher(Some(std::time::Duration::from_secs(5)))
    );
    resp!(
        out,
        "whep no_publisher(None)",
        WhepSession::no_publisher(None)
    );
    resp!(out, "whep ack_delete", s.ack_delete());

    // WHIP client, with and without a bearer token
    for token in [None, Some("tok3n".to_string())] {
        let tag = if token.is_some() { "bearer" } else { "anon" };
        let mut c = WhipClient::new(URL.into(), token);
        req!(
            out,
            &format!("whip client {tag} offer"),
            c.offer(b"o".to_vec()).unwrap()
        );
        c.on_response(
            webrtc_runtime::whip::client::HttpResponse::new(StatusCode::CREATED)
                .with_content_type("application/sdp")
                .unwrap()
                .with_location(SESSION)
                .unwrap()
                .with_etag("etag1")
                .unwrap()
                .with_body(b"a".to_vec()),
        )
        .unwrap();
        req!(
            out,
            &format!("whip client {tag} flush_candidates"),
            c.flush_candidates(b"f".to_vec()).unwrap()
        );
        c.on_response(webrtc_runtime::whip::client::HttpResponse::new(
            StatusCode::NO_CONTENT,
        ))
        .unwrap();
        req!(
            out,
            &format!("whip client {tag} ice_restart"),
            c.ice_restart(b"f".to_vec()).unwrap()
        );
        c.on_response(
            webrtc_runtime::whip::client::HttpResponse::new(StatusCode::OK)
                .with_etag("etag9")
                .unwrap()
                .with_body(b"x".to_vec()),
        )
        .unwrap();
        req!(
            out,
            &format!("whip client {tag} terminate"),
            c.terminate().unwrap()
        );
    }

    // WHEP player
    let mut p = WhepPlayer::new(URL.into(), Some("tok3n".into()));
    req!(out, "whep player offer", p.offer(b"o".to_vec()).unwrap());
    p.on_response(
        webrtc_runtime::whep::player::HttpResponse::new(StatusCode::CREATED)
            .with_content_type("application/sdp")
            .unwrap()
            .with_location(SESSION)
            .unwrap()
            .with_etag("e1")
            .unwrap()
            .with_body(b"a".to_vec()),
    )
    .unwrap();
    req!(
        out,
        "whep player trickle_ice",
        p.trickle_ice(b"f".to_vec()).unwrap()
    );
    p.on_response(webrtc_runtime::whep::player::HttpResponse::new(
        StatusCode::NO_CONTENT,
    ))
    .unwrap();
    req!(out, "whep player terminate", p.terminate().unwrap());

    let path = format!(
        "{}/tests/golden/whip_whep_http.golden",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var_os("GOLDEN_UPDATE").is_some() {
        std::fs::write(&path, &out).unwrap();
        return;
    }
    assert_eq!(
        out,
        std::fs::read_to_string(&path).expect("golden"),
        "http golden differs"
    );
}
