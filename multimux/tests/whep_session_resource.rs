#![cfg(all(feature = "whep", feature = "test-hooks"))]
//! Issue #1149 (WHEP): the `Location` of the `201` is a live session resource
//! (draft-ietf-wish-whep sections 4/5, RFC 9725 sections 4.2/4.3) — `DELETE`
//! ends the viewer session, drops its `Trunk` subscription and frees its
//! capacity slot; `PATCH` trickles ICE candidates. Real HTTP against the real
//! route (`run_whep_on_probed`, a real media socket per session).
//!
//! The browser-driven WHEP playback test (`whep_egress.rs`) is separate and
//! skips without Node/Chromium; nothing in this file needs either.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const TRICKLE: &str = "application/trickle-ice-sdpfrag";
const CANDIDATE: &str = "2 1 udp 2130706431 10.0.0.9 5555 typ host";

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

async fn http(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Resp {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut buf)).await;
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("no status line in {text:?}"));
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Resp {
        status,
        headers,
        body: body.to_string(),
    }
}

async fn post_offer(addr: std::net::SocketAddr) -> Resp {
    http(
        addr,
        "POST",
        "/whep",
        &[("Content-Type", "application/sdp")],
        multimux::output::whep::WHEP_TEST_OFFER,
    )
    .await
}

fn resource_path(resp: &Resp) -> String {
    assert_eq!(resp.status, 201, "{}", resp.body);
    let loc = resp.header("location").expect("201 carries a Location");
    let id = loc
        .strip_prefix("/whep/session/")
        .unwrap_or_else(|| panic!("Location must be a per-session resource: {loc}"));
    assert_eq!(id.len(), 32, "an unguessable 32-char resource id: {loc}");
    loc.to_string()
}

fn etag_of(resp: &Resp) -> String {
    resp.header("etag")
        .expect("201 carries an ETag")
        .to_string()
}

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn fragment(ufrag: &str, candidate: &str) -> String {
    format!(
        "a=ice-ufrag:{ufrag}\r\na=ice-pwd:whatever-is-ignored-here\r\n\
         m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=mid:0\r\na=candidate:{candidate}\r\n"
    )
}

/// The WHEP test offer's own ICE ufrag.
const OFFER_UFRAG: &str = "abcd";

#[tokio::test]
async fn delete_ends_the_viewer_drops_its_trunk_subscription_and_frees_the_slot() {
    let h = multimux::output::whep::serve_whep_run_probed_for_test(1).await;
    // Everything the route itself holds of the trunk, before any viewer: taken
    // once the route answers (its state, which clones the trunk, exists by then).
    assert_eq!(http(h.addr, "OPTIONS", "/whep", &[], "").await.status, 204);
    let baseline = Arc::strong_count(&h.trunk);

    let created = post_offer(h.addr).await;
    let loc = resource_path(&created);
    assert!(
        created
            .header("content-type")
            .unwrap()
            .starts_with("application/sdp"),
        "{:?}",
        created.headers
    );
    etag_of(&created);
    wait_until("viewer admitted", || {
        h.probes.active_sessions.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(h.probes.live_resources(), 1);
    // The session task holds a trunk clone and a cursor (another clone).
    wait_until("viewer subscribed to the trunk", || {
        Arc::strong_count(&h.trunk) > baseline
    })
    .await;

    // At capacity (cap 1).
    assert_eq!(post_offer(h.addr).await.status, 503);

    let del = http(h.addr, "DELETE", &loc, &[], "").await;
    assert_eq!(del.status, 200, "{}", del.body);

    wait_until("capacity slot released", || {
        h.probes.active_sessions.load(Ordering::SeqCst) == 0
    })
    .await;
    wait_until("trunk subscription dropped", || {
        Arc::strong_count(&h.trunk) == baseline
    })
    .await;
    assert_eq!(h.probes.live_resources(), 0);

    // Second DELETE and a PATCH on the deleted resource: gone.
    assert_eq!(http(h.addr, "DELETE", &loc, &[], "").await.status, 404);
    assert_eq!(
        http(
            h.addr,
            "PATCH",
            &loc,
            &[("Content-Type", TRICKLE)],
            &fragment(OFFER_UFRAG, CANDIDATE)
        )
        .await
        .status,
        404
    );

    // The slot is reusable.
    let again = post_offer(h.addr).await;
    resource_path(&again);
    wait_until("slot reused", || {
        h.probes.active_sessions.load(Ordering::SeqCst) == 1
    })
    .await;

    h.cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(10), h.handle).await;
}

#[tokio::test]
async fn unknown_resources_are_404_for_patch_and_delete() {
    let h = multimux::output::whep::serve_whep_run_probed_for_test(4).await;
    let bogus = "/whep/session/does-not-exist";
    assert_eq!(http(h.addr, "DELETE", bogus, &[], "").await.status, 404);
    assert_eq!(
        http(
            h.addr,
            "PATCH",
            bogus,
            &[("Content-Type", TRICKLE)],
            &fragment(OFFER_UFRAG, CANDIDATE)
        )
        .await
        .status,
        404
    );
    assert_eq!(
        http(h.addr, "DELETE", "/whep/session", &[], "")
            .await
            .status,
        404
    );
    h.cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(10), h.handle).await;
}

#[tokio::test]
async fn patch_trickles_a_candidate_and_rejections_follow_the_rfc() {
    let h = multimux::output::whep::serve_whep_run_probed_for_test(4).await;
    let created = post_offer(h.addr).await;
    let loc = resource_path(&created);
    let etag = etag_of(&created);
    wait_until("viewer admitted", || {
        h.probes.active_sessions.load(Ordering::SeqCst) == 1
    })
    .await;
    let good = fragment(OFFER_UFRAG, CANDIDATE);

    // 204 + the candidate reaches the session's transport.
    let ok = http(
        h.addr,
        "PATCH",
        &loc,
        &[("Content-Type", TRICKLE), ("If-Match", &etag)],
        &good,
    )
    .await;
    assert_eq!(ok.status, 204, "{}", ok.body);
    wait_until("candidate handed to the transport", || {
        h.probes.trickled_candidates.load(Ordering::Relaxed) == 1
    })
    .await;

    // 415 for a wrong or missing Content-Type.
    for ct in ["application/sdp", "text/plain"] {
        assert_eq!(
            http(h.addr, "PATCH", &loc, &[("Content-Type", ct)], &good)
                .await
                .status,
            415,
            "{ct}"
        );
    }
    assert_eq!(http(h.addr, "PATCH", &loc, &[], &good).await.status, 415);
    // 412: If-Match naming another tag.
    assert_eq!(
        http(
            h.addr,
            "PATCH",
            &loc,
            &[("Content-Type", TRICKLE), ("If-Match", "\"other\"")],
            &good
        )
        .await
        .status,
        412
    );
    // 422: ICE restart is unsupported by this transport.
    assert_eq!(
        http(
            h.addr,
            "PATCH",
            &loc,
            &[("Content-Type", TRICKLE), ("If-Match", "*")],
            &good
        )
        .await
        .status,
        422
    );
    // 400: other ICE generation, empty fragment, garbage.
    for body in [
        fragment("zzzz", CANDIDATE).as_str(),
        "a=ice-pwd:x\r\n",
        "this is not sdp",
    ] {
        assert_eq!(
            http(
                h.addr,
                "PATCH",
                &loc,
                &[("Content-Type", TRICKLE), ("If-Match", &etag)],
                body
            )
            .await
            .status,
            400,
            "{body:?}"
        );
    }
    // Only the one good candidate ever reached the transport.
    assert_eq!(h.probes.trickled_candidates.load(Ordering::Relaxed), 1);
    assert_eq!(http(h.addr, "DELETE", &loc, &[], "").await.status, 200);

    h.cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(10), h.handle).await;
}
