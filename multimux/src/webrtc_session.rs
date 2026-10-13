//! Shared WHIP/WHEP session limits and the RAII capacity slot.
//!
//! The bounded HTTP request *reader* that used to live beside these moved out
//! with the WHIP/WHEP listeners onto axum `Router`s (W2a SP2.1); what remains
//! here is the part that is not HTTP at all: the per-listener session cap and
//! the RAII slot that hands a reservation off to a long-lived session.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Bound on concurrent in-flight signalling requests for one WHIP/WHEP
/// listener (a shared `GlobalConcurrencyLimitLayer` request-concurrency pool,
/// not a TCP connection cap — see `serve_hyper_util`'s `MAX_CONNECTIONS` for
/// that). A flood of requests that never complete cannot spawn an unbounded
/// number of concurrent handler tasks.
pub(crate) const MAX_PENDING_HTTP_CONNECTIONS: usize = 256;

/// Body cap for a WHIP/WHEP signalling request (SDP offer/answer). A request
/// claiming a larger body is rejected `413` before any body byte is read.
pub(crate) const MAX_HTTP_BODY_BYTES: usize = 64 * 1024;

/// Bounds an entire signalling request (headers + body) — a peer that stops
/// sending mid-request is closed rather than held open indefinitely.
pub(crate) const HTTP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Minimum floor between two consecutive [`MediaTransport::handle_timeout`]
/// calls (finding N3): a transport whose `poll_timeout` keeps returning an
/// instant at or before now (an ICE/DTLS error state) would otherwise fire
/// `handle_timeout` back-to-back and spin a core. An overdue deadline still
/// fires immediately *on the first* call; only the *next* is floored.
pub(crate) const MIN_TIMER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1);

/// Absolute instant at which a transport timer with `deadline` should fire:
/// `max(deadline, last_fire + MIN_TIMER_INTERVAL)` (just `deadline` when the
/// timer has never fired). `None` means never.
/// Absolute (not a duration measured from `last_fire`), so rebuilding the
/// sleep on every loop iteration (each inbound datagram) cannot push the fire
/// time back — provided `last_fire` is the REAL previous fire, carried across
/// iterations (issue #1150), not "now".
pub(crate) fn timer_fire_at(
    deadline: Option<std::time::Instant>,
    last_fire: Option<std::time::Instant>,
) -> Option<tokio::time::Instant> {
    deadline.map(|d| {
        let at = last_fire.map_or(d, |l| d.max(l + MIN_TIMER_INTERVAL));
        tokio::time::Instant::from_std(at)
    })
}

/// Whether the transport timer is due AT `now`: its deadline has passed and
/// the [`MIN_TIMER_INTERVAL`] floor from the previous fire has elapsed. A loop
/// checks this before reading another datagram, so a datagram flood cannot
/// postpone an overdue timer (issue #1150).
#[cfg(feature = "whip")]
pub(crate) fn timer_due(
    deadline: Option<std::time::Instant>,
    last_fire: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    timer_fire_at(deadline, last_fire).is_some_and(|at| at <= tokio::time::Instant::from_std(now))
}

/// Sleep until [`timer_fire_at`], or forever when there is no deadline.
pub(crate) fn sleep_until_timer(
    deadline: Option<std::time::Instant>,
    last_fire: Option<std::time::Instant>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    match timer_fire_at(deadline, last_fire) {
        None => Box::pin(std::future::pending()),
        Some(at) => Box::pin(tokio::time::sleep_until(at)),
    }
}

/// RAII capacity slot for a WHIP/WHEP session (issue r07-C11 follow-up):
/// [`Self::acquire`] does the `fetch_add`/cap-check/`fetch_sub`-on-refusal
/// dance once, in one place, and — critically — releases the slot on
/// `Drop` if it is ever dropped still armed. Without this, any `?` between
/// a successful `acquire` and the point a session is finally handed off to
/// its long-lived owner (an admitted-session channel send, in both WHIP and
/// WHEP) leaked the slot forever on that error path; `max_sessions` such
/// leaks with no real live session left disabled the route.
pub(crate) struct SessionSlot<'a> {
    counter: &'a AtomicUsize,
    armed: bool,
}

impl<'a> SessionSlot<'a> {
    /// Reserves one slot against `counter`, refusing (and leaving `counter`
    /// unchanged) once `max` are already held.
    pub(crate) fn acquire(counter: &'a AtomicUsize, max: usize) -> Option<Self> {
        let prev = counter.fetch_add(1, Ordering::SeqCst);
        if prev >= max {
            counter.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(SessionSlot {
            counter,
            armed: true,
        })
    }

    /// Hands this slot's lifetime off to whatever the caller is about to
    /// admit the session into — from this point the *new* owner is
    /// responsible for decrementing `counter` (e.g. on session reap), and
    /// this guard's own `Drop` becomes a no-op so the slot is never
    /// double-released.
    pub(crate) fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for SessionSlot<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.counter.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Length of a session resource id (the last path segment of the `Location`
/// the `201` returns). RFC 9725 §8 asks for unguessable session URLs; this is
/// 32 characters of a 62-symbol alphabet (~190 bits) drawn from the OS CSPRNG
/// (`crate::secure_random`), the same source ICE credentials use.
const RESOURCE_ID_LEN: usize = 32;

/// Ceiling on remote candidates queued for the media task between two of its
/// loop iterations; matches the transport's own cap so a PATCH flood cannot
/// buffer more than the transport would ever admit.
const MAX_PENDING_CANDIDATES: usize = webrtc_runtime::media::MAX_REMOTE_CANDIDATES;

/// Length of a locally generated ICE `ufrag` (RFC 8445 section 5.4 asks for
/// at least 24 bits; matches the offer-time credentials).
pub(crate) const ICE_UFRAG_LEN: usize = 8;
/// Length of a locally generated ICE `pwd` (at least 128 bits required).
pub(crate) const ICE_PWD_LEN: usize = 24;
/// Length of a generated `ETag` opaque tag.
pub(crate) const ETAG_TOKEN_LEN: usize = 16;

/// A fresh unguessable resource id for a WHIP/WHEP session URL.
pub(crate) fn new_resource_id() -> String {
    crate::secure_random::token(RESOURCE_ID_LEN)
}

/// The channel from the HTTP resource handlers (`PATCH`/`DELETE`) to the one
/// task that owns the session's `MediaTransport`: that task takes the
/// transport by value (SP6.3), so a handler never touches it — it leaves a
/// message here and wakes the task.
///
/// `terminated` is sticky (a `DELETE` is never lost, even if it lands before
/// the media task has started reading); queued candidates are drained by
/// [`Self::take`].
#[derive(Debug, Default)]
pub(crate) struct SessionControl {
    wake: tokio::sync::Notify,
    inner: parking_lot::Mutex<ControlInner>,
}

#[derive(Debug, Default)]
struct ControlInner {
    terminated: bool,
    ops: Vec<IceOp>,
}

/// One ICE change a `PATCH` asks the media task to make on its transport.
/// Kept in arrival order: a restart's own candidates and any trickle sent
/// after it must apply after the new credentials, never before.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum IceOp {
    /// Add a remote candidate (Trickle ICE).
    Candidate(String),
    /// Restart ICE under new credentials (RFC 9725 section 4.3.2).
    Restart(IceRestart),
}

/// The four credentials of an ICE restart.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct IceRestart {
    pub local_ufrag: String,
    pub local_pwd: String,
    pub remote_ufrag: String,
    pub remote_pwd: String,
}

/// What a media task finds when it drains its [`SessionControl`].
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ControlTake {
    /// A `DELETE` ended the session resource.
    pub terminated: bool,
    /// ICE changes a `PATCH` queued, in arrival order.
    pub ops: Vec<IceOp>,
}

impl SessionControl {
    pub(crate) fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::default())
    }

    /// Ends the session: sticky, and wakes the media task.
    pub(crate) fn terminate(&self) {
        self.inner.lock().terminated = true;
        self.wake.notify_one();
    }

    /// Queues trickled remote candidates. `false` (nothing queued) when they
    /// would push the pending queue past [`MAX_PENDING_CANDIDATES`].
    pub(crate) fn push_candidates(&self, candidates: Vec<String>) -> bool {
        self.push_ops(candidates.into_iter().map(IceOp::Candidate).collect())
    }

    /// Queues an ICE restart followed by the candidates the same `PATCH`
    /// carried. `false` (nothing queued) when the queue would overflow.
    pub(crate) fn push_restart(&self, restart: IceRestart, candidates: Vec<String>) -> bool {
        let mut ops = vec![IceOp::Restart(restart)];
        ops.extend(candidates.into_iter().map(IceOp::Candidate));
        self.push_ops(ops)
    }

    fn push_ops(&self, ops: Vec<IceOp>) -> bool {
        {
            let mut inner = self.inner.lock();
            if inner.ops.len() + ops.len() > MAX_PENDING_CANDIDATES {
                return false;
            }
            inner.ops.extend(ops);
        }
        self.wake.notify_one();
        true
    }

    /// Drains the queued ICE changes and reports the sticky terminated flag.
    pub(crate) fn take(&self) -> ControlTake {
        let mut inner = self.inner.lock();
        ControlTake {
            terminated: inner.terminated,
            ops: std::mem::take(&mut inner.ops),
        }
    }

    /// Resolves on the next [`Self::terminate`] / [`Self::push_candidates`]
    /// (a notification that arrived with no waiter is kept, so none is lost).
    pub(crate) fn notified(&self) -> tokio::sync::futures::Notified<'_> {
        self.wake.notified()
    }
}

/// One live WHIP/WHEP session resource (`M` is the protocol's sans-IO state
/// machine, `webrtc_runtime::whip::server::WhipSession` or
/// `webrtc_runtime::whep::server::WhepSession`).
pub(crate) struct Resource<M> {
    /// The RFC 9725 / draft-ietf-wish-whep resource state machine: owns the
    /// `ETag` and decides which `PATCH` is a trickle update and which an ICE
    /// restart.
    pub machine: parking_lot::Mutex<M>,
    /// How a handler reaches the media task.
    pub control: std::sync::Arc<SessionControl>,
    /// The peer's current ICE `ufrag` (the offer's, then each restart's): a
    /// trickle fragment naming another one belongs to a different ICE
    /// generation.
    pub remote_ufrag: parking_lot::Mutex<String>,
    /// This side's local candidates, as `candidate` attribute bodies: the
    /// restart answer lists them again (the socket does not change).
    pub local_candidates: Vec<String>,
}

/// The live session resources of one WHIP/WHEP listener, keyed by the id in
/// the `Location` URL. An entry exists exactly while its session does: it is
/// removed by `DELETE`, or by the [`ResourceGuard`] the session owns when it
/// ends for any other reason (idle reap, transport error, shutdown) — so a
/// resource that no longer has a session answers `404`, never a stale `200`.
pub(crate) struct ResourceTable<M> {
    map: parking_lot::Mutex<std::collections::HashMap<String, std::sync::Arc<Resource<M>>>>,
}

impl<M> ResourceTable<M> {
    pub(crate) fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            map: parking_lot::Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// Registers `resource` under `id`; the returned guard unregisters it on drop.
    pub(crate) fn insert(
        self: &std::sync::Arc<Self>,
        id: String,
        resource: Resource<M>,
    ) -> ResourceGuard<M> {
        self.map
            .lock()
            .insert(id.clone(), std::sync::Arc::new(resource));
        ResourceGuard {
            table: std::sync::Arc::clone(self),
            id,
        }
    }

    pub(crate) fn get(&self, id: &str) -> Option<std::sync::Arc<Resource<M>>> {
        self.map.lock().get(id).cloned()
    }

    pub(crate) fn remove(&self, id: &str) -> Option<std::sync::Arc<Resource<M>>> {
        self.map.lock().remove(id)
    }

    /// Number of live resources (observable through the `#[doc(hidden)]`
    /// test handles).
    pub(crate) fn len(&self) -> usize {
        self.map.lock().len()
    }
}

/// Unregisters a session's resource when the session is dropped.
pub(crate) struct ResourceGuard<M> {
    table: std::sync::Arc<ResourceTable<M>>,
    id: String,
}

impl<M> Drop for ResourceGuard<M> {
    fn drop(&mut self) {
        self.table.remove(&self.id);
    }
}

/// True when `headers` carries exactly one `Content-Type` whose media type
/// (parameters ignored, RFC 9110 §8.3.1) is `expected`.
pub(crate) fn content_type_is(headers: &axum::http::HeaderMap, expected: &str) -> bool {
    use headers::HeaderMapExt as _;
    headers
        .get_all(axum::http::header::CONTENT_TYPE)
        .iter()
        .count()
        == 1
        && headers
            .typed_get::<headers::ContentType>()
            .map(headers::Mime::from)
            .is_some_and(|m| m.essence_str().eq_ignore_ascii_case(expected))
}

/// The ICE information of a Trickle ICE SDP fragment (RFC 8840 §4.4; RFC 9725
/// §4.3): the candidates to add and the generation (`ufrag`) they belong to.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct IceFragment {
    pub ufrag: Option<String>,
    pub pwd: Option<String>,
    pub candidates: Vec<String>,
    pub end_of_candidates: bool,
}

/// Parses a Trickle ICE SDP fragment with `sdp-types`: the fragment has no
/// session-header lines, so it is appended to a typed, `Session::write`-
/// rendered prelude and parsed as one session. Candidates are taken from the
/// session level and every media section.
pub(crate) fn parse_ice_fragment(body: &[u8]) -> Result<IceFragment, String> {
    use sdp_types::{Origin, Session};
    let text = std::str::from_utf8(body).map_err(|_| "fragment is not UTF-8".to_string())?;
    let mut doc = Vec::new();
    Session::new(
        Origin::with_ip_addr("0", 0, std::net::Ipv4Addr::UNSPECIFIED),
        "-",
    )
    .write(&mut doc)
    .map_err(|e| format!("render prelude: {e}"))?;
    for line in text.lines().filter(|l| !l.is_empty()) {
        doc.extend_from_slice(line.as_bytes());
        doc.extend_from_slice(b"\r\n");
    }
    let session = Session::parse(&doc).map_err(|e| format!("parse fragment: {e}"))?;
    let mut out = IceFragment::default();
    let attrs = session
        .attributes
        .iter()
        .chain(session.medias.iter().flat_map(|m| m.attributes.iter()));
    for a in attrs {
        match (a.attribute.as_str(), a.value.as_deref()) {
            ("candidate", Some(v)) => out.candidates.push(v.to_string()),
            ("ice-ufrag", Some(v)) if out.ufrag.is_none() => out.ufrag = Some(v.to_string()),
            ("ice-pwd", Some(v)) if out.pwd.is_none() => out.pwd = Some(v.to_string()),
            ("end-of-candidates", _) => out.end_of_candidates = true,
            _ => {}
        }
    }
    Ok(out)
}

/// Whether a parsed fragment is an acceptable trickle update for a session
/// whose ICE generation is `remote_ufrag`.
pub(crate) fn validate_trickle(fragment: &IceFragment, remote_ufrag: &str) -> Result<(), String> {
    if let Some(u) = &fragment.ufrag
        && u != remote_ufrag
    {
        return Err(format!(
            "fragment ice-ufrag {u:?} is not this session's ICE generation"
        ));
    }
    if fragment.candidates.is_empty() && !fragment.end_of_candidates {
        return Err("fragment carries no candidate and no end-of-candidates".to_string());
    }
    Ok(())
}

/// The credentials an ICE-restart fragment (RFC 9725 section 4.3.2) must
/// carry: the peer's new `ice-ufrag` and `ice-pwd`, which must differ from its
/// current ufrag (otherwise nothing restarted).
pub(crate) fn validate_restart(
    fragment: &IceFragment,
    current_remote_ufrag: &str,
) -> Result<(String, String), String> {
    match (&fragment.ufrag, &fragment.pwd) {
        (Some(u), Some(p)) if u != current_remote_ufrag => Ok((u.clone(), p.clone())),
        (Some(_), Some(_)) => Err("restart fragment reuses the current ice-ufrag".to_string()),
        _ => Err("restart fragment needs both ice-ufrag and ice-pwd".to_string()),
    }
}

/// Renders the ICE-restart answer fragment (RFC 9725 section 4.3.2: this
/// side's new credentials and its candidates, as an RFC 8840 section 4.4
/// `application/trickle-ice-sdpfrag` body). Built with `sdp-types` and cut
/// down to the fragment lines (the session header lines are not part of a
/// fragment).
pub(crate) fn render_restart_fragment(
    ufrag: &str,
    pwd: &str,
    candidates: &[String],
) -> Result<Vec<u8>, String> {
    use sdp_types::{Media, Origin, Session};
    let mut session = Session::new(
        Origin::with_ip_addr("0", 0, std::net::Ipv4Addr::UNSPECIFIED),
        "-",
    );
    session.add_attribute_with_value("ice-ufrag", ufrag);
    session.add_attribute_with_value("ice-pwd", pwd);
    let mut media = Media {
        media: "video".to_string(),
        port: 9,
        num_ports: None,
        proto: "UDP/TLS/RTP/SAVPF".to_string(),
        fmt: "0".to_string(),
        media_title: None,
        connections: Vec::new(),
        bandwidths: Vec::new(),
        key: None,
        attributes: Vec::new(),
    };
    for c in candidates {
        media.add_attribute_with_value("candidate", c);
    }
    session.medias.push(media);
    let mut full = Vec::new();
    session
        .write(&mut full)
        .map_err(|e| format!("render restart fragment: {e}"))?;
    // Keep the attribute and media lines; drop the header lines.
    let text = String::from_utf8(full).map_err(|e| e.to_string())?;
    let mut out = String::new();
    for line in text.lines() {
        if matches!(line.as_bytes().first(), Some(b'a' | b'm')) {
            out.push_str(line);
            out.push_str("\r\n");
        }
    }
    Ok(out.into_bytes())
}

/// An ICE restart worked out from a `PATCH`, not yet applied.
pub(crate) struct RestartPlan {
    restart: IceRestart,
    candidates: Vec<String>,
    /// The `200` body: this side's new credentials and candidates.
    pub answer: Vec<u8>,
    /// The resource's new `ETag` (RFC 9725 section 4.3.2).
    pub etag: String,
}

/// Plans an ICE restart from the client's fragment: validates it, draws this
/// side's new credentials and `ETag` from `token` (a random-token source of
/// the given length) and renders the answer fragment.
pub(crate) fn plan_restart(
    fragment: &[u8],
    current_remote_ufrag: &str,
    local_candidates: &[String],
    token: fn(usize) -> String,
) -> Result<RestartPlan, String> {
    let parsed = parse_ice_fragment(fragment)?;
    let (remote_ufrag, remote_pwd) = validate_restart(&parsed, current_remote_ufrag)?;
    let local_ufrag = token(ICE_UFRAG_LEN);
    let local_pwd = token(ICE_PWD_LEN);
    let answer = render_restart_fragment(&local_ufrag, &local_pwd, local_candidates)?;
    Ok(RestartPlan {
        restart: IceRestart {
            local_ufrag,
            local_pwd,
            remote_ufrag,
            remote_pwd,
        },
        candidates: parsed.candidates,
        answer,
        etag: token(ETAG_TOKEN_LEN),
    })
}

/// Queues `plan` for the media task and records the peer's new `ufrag` as the
/// resource's current ICE generation. `false` (nothing changed) when the
/// task's pending queue is full.
pub(crate) fn commit_restart<M>(resource: &Resource<M>, plan: RestartPlan) -> bool {
    let new_ufrag = plan.restart.remote_ufrag.clone();
    if !resource.control.push_restart(plan.restart, plan.candidates) {
        return false;
    }
    *resource.remote_ufrag.lock() = new_ufrag;
    true
}

/// What [`apply_ice_ops`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct AppliedOps {
    pub candidates: u64,
    pub restarts: u64,
}

/// Applies the queued ICE changes to the transport the calling task owns, in
/// order. A change the transport refuses (an unparsable or over-cap candidate,
/// bad restart credentials) is logged and skipped, like the offer's own
/// candidates.
pub(crate) fn apply_ice_ops(
    media: &mut webrtc_runtime::media::MediaTransport,
    ops: Vec<IceOp>,
    now: std::time::Instant,
) -> AppliedOps {
    let mut done = AppliedOps::default();
    for op in ops {
        match op {
            IceOp::Candidate(c) => match media.add_remote_candidate(&c) {
                Ok(()) => done.candidates += 1,
                Err(e) => tracing::debug!(error = %e, "webrtc: trickled candidate rejected"),
            },
            IceOp::Restart(r) => match media.restart_ice(
                now,
                &r.local_ufrag,
                &r.local_pwd,
                &r.remote_ufrag,
                &r.remote_pwd,
            ) {
                Ok(()) => done.restarts += 1,
                Err(e) => tracing::warn!(error = %e, "webrtc: ICE restart refused"),
            },
        }
    }
    done
}

/// The HTTP status for a state-machine error raised by a `PATCH`.
pub(crate) fn patch_error_status(err: &webrtc_runtime::Error) -> axum::http::StatusCode {
    use axum::http::StatusCode;
    use webrtc_runtime::Error;
    match err {
        Error::ETagMismatch { .. } => StatusCode::PRECONDITION_FAILED,
        Error::WrongState { .. } => StatusCode::CONFLICT,
        _ => StatusCode::BAD_REQUEST,
    }
}

/// An `HttpResponse` from `webrtc-runtime` as an axum response.
pub(crate) fn to_axum(
    resp: webrtc_runtime::whip::server::HttpResponse,
) -> axum::response::Response {
    let mut out = axum::response::Response::new(axum::body::Body::from(resp.body));
    *out.status_mut() = resp.status;
    *out.headers_mut() = resp.headers;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn session_slot_releases_on_drop() {
        let counter = AtomicUsize::new(0);
        {
            let _slot = SessionSlot::acquire(&counter, 1).expect("first acquire succeeds");
            assert_eq!(counter.load(Ordering::SeqCst), 1);
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "a slot dropped without being disarmed must release itself"
        );
    }

    #[test]
    fn session_slot_refuses_over_capacity_without_changing_the_counter() {
        let counter = AtomicUsize::new(1);
        assert!(SessionSlot::acquire(&counter, 1).is_none());
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "a refused acquire must leave the counter exactly as it found it"
        );
    }

    #[test]
    fn session_slot_disarm_prevents_double_release() {
        let counter = AtomicUsize::new(0);
        let slot = SessionSlot::acquire(&counter, 1).unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        slot.disarm();
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "disarm must not itself release the slot — the new owner does, later"
        );
        // Simulate the new owner's own teardown decrementing the same
        // counter exactly once.
        counter.fetch_sub(1, Ordering::SeqCst);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_trickle_fragment_yields_its_candidates_generation_and_end_marker() {
        let frag = b"a=ice-ufrag:EsAw\r\na=ice-pwd:P2uYro0UCOQ4zxjKXaWCBui1\r\n\
            m=audio 9 RTP/AVP 0\r\na=mid:0\r\n\
            a=candidate:1 1 udp 2130706431 192.0.2.1 5000 typ host\r\n\
            a=candidate:2 1 udp 1694498815 198.51.100.7 6000 typ srflx raddr 192.0.2.1 rport 5000\r\n\
            a=end-of-candidates\r\n";
        let f = parse_ice_fragment(frag).expect("RFC 8840 section 4.4 fragment");
        assert_eq!(f.ufrag.as_deref(), Some("EsAw"));
        assert_eq!(f.candidates.len(), 2);
        assert!(f.candidates[0].starts_with("1 1 udp 2130706431 192.0.2.1 5000"));
        assert!(f.end_of_candidates);
        assert!(validate_trickle(&f, "EsAw").is_ok());
        assert!(
            validate_trickle(&f, "other").is_err(),
            "another ICE generation"
        );
        let empty = parse_ice_fragment(b"a=ice-pwd:x\r\n").unwrap();
        assert!(validate_trickle(&empty, "x").is_err(), "nothing to add");
        assert!(parse_ice_fragment(&[0xff, 0xfe]).is_err(), "not UTF-8");
    }

    #[test]
    fn control_termination_is_sticky_and_candidates_drain_once() {
        let c = SessionControl::new();
        assert!(c.push_candidates(vec!["a".into(), "b".into()]));
        c.terminate();
        let first = c.take();
        assert!(first.terminated);
        assert_eq!(
            first.ops,
            [IceOp::Candidate("a".into()), IceOp::Candidate("b".into())]
        );
        let second = c.take();
        assert!(second.terminated, "DELETE is never forgotten");
        assert!(second.ops.is_empty(), "ops drain exactly once");
        let over = vec![String::new(); MAX_PENDING_CANDIDATES + 1];
        assert!(
            !SessionControl::new().push_candidates(over),
            "bounded queue"
        );
    }

    #[test]
    fn a_resource_unregisters_when_its_guard_drops() {
        let table = ResourceTable::<u8>::new();
        let guard = table.insert(
            "id".into(),
            Resource {
                machine: parking_lot::Mutex::new(0),
                control: SessionControl::new(),
                remote_ufrag: parking_lot::Mutex::new("u".into()),
                local_candidates: Vec::new(),
            },
        );
        assert!(table.get("id").is_some());
        drop(guard);
        assert!(table.get("id").is_none());
        assert_eq!(table.len(), 0);
    }

    use std::time::Duration;

    fn std_now() -> std::time::Instant {
        tokio::time::Instant::now().into_std()
    }

    /// K1: the sleep is absolute. A deadline 500 ms out with the previous fire
    /// 10 s ago fires at +500 ms, not +10.5 s.
    #[tokio::test(start_paused = true)]
    async fn a_real_deadline_fires_on_time_regardless_of_the_last_fire() {
        let start = tokio::time::Instant::now();
        let last_fire = Some(std_now() - Duration::from_secs(10));
        sleep_until_timer(Some(std_now() + Duration::from_millis(500)), last_fire).await;
        assert_eq!(start.elapsed(), Duration::from_millis(500));
    }

    /// K1: the sleep is rebuilt every loop iteration (once per inbound
    /// datagram); a datagram every 10 ms must not delay a timer due at +500 ms.
    #[tokio::test(start_paused = true)]
    async fn steady_inbound_datagrams_do_not_starve_the_transport_timer() {
        let start = tokio::time::Instant::now();
        let last_fire = Some(std_now() - Duration::from_secs(10));
        let deadline = std_now() + Duration::from_millis(500);
        let mut datagrams = tokio::time::interval(Duration::from_millis(10));
        datagrams.tick().await;
        loop {
            tokio::select! {
                _ = datagrams.tick() => {}
                () = sleep_until_timer(Some(deadline), last_fire) => break,
            }
            assert!(start.elapsed() < Duration::from_secs(2), "timer starved");
        }
        assert_eq!(start.elapsed(), Duration::from_millis(500));
    }

    /// An overdue deadline is floored to `last_fire + MIN_TIMER_INTERVAL`.
    #[tokio::test(start_paused = true)]
    async fn an_overdue_deadline_is_floored_from_the_last_fire() {
        let start = tokio::time::Instant::now();
        let now = std_now();
        sleep_until_timer(Some(now - Duration::from_secs(1)), Some(now)).await;
        assert_eq!(start.elapsed(), MIN_TIMER_INTERVAL);
    }
}
