//! The shared origin-level resource route: `init-*.mp4` / `seg-*.m4s` /
//! `part-*.m4s` byte serving, mounted **once per stream** by
//! [`crate::origin::router`] rather than per-`Output` — LL-HLS and DASH are
//! both fMP4/CMAF and reference the exact same bytes (resolved through the
//! route's shared [`hls_runtime::server::HlsOrigin`], the `ServedEgress`
//! every output's bytes resolve through — see [`crate::http`]), so serving
//! them per-output would duplicate the route (and previously caused an axum
//! panic: two `Output`s both mounting a `/:file` catch-all under the same
//! `/{stream}` nest — issue #663 P4's "multi-output nest collision" fix).
//!
//! Each [`crate::output::Output`] contributes only its manifest route(s)
//! (`master.m3u8`/`media.m3u8` for LL-HLS, `manifest.mpd` for DASH); this
//! module is the one thing every output shares.
//!
//! # Chunked-transfer whole-segment serving (issue #721)
//!
//! [`crate::output::ll_dash`]'s true low-latency DASH design addresses whole
//! segments (`seg-{track}-{seq}.m4s`, the same filenames
//! [`crate::output::dash`]'s regular MPD uses) but needs a segment's bytes to
//! start flowing *before* it closes. [`dynamic_file`] implements this: a
//! `seg-*.m4s` request that doesn't (yet) resolve to a closed segment falls
//! through to [`stream_in_progress_segment`], which re-fetches that
//! segment's `part-{track}-{seq}.{idx}.m4s` entries in order — the exact
//! bytes the route's `HlsOrigin` already produces for LL-HLS's own
//! preload-hint requests — and streams them as one HTTP
//! chunked-transfer-encoded response body, ending once a part index resolves
//! [`media_plane::egress::EgressResponse::NotFound`] (which only happens once
//! the segment has actually closed without that part, i.e. exactly the
//! segment's end). A genuinely future segment (nothing produced yet) blocks
//! the same bounded [`crate::http::BLOCKING_RELOAD_TIMEOUT`] on its first
//! part before giving up (404), mirroring the plain closed-segment/part
//! lookups below. LL-HLS itself never triggers this path: its playlist never
//! advertises an in-progress segment's whole-segment URI (RFC 8216bis
//! §4.4.4.9), so a well-behaved client never requests one.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use futures_util::stream;
use hls_runtime::server::{HlsBody, HlsRequest};
use media_plane::trunk::{EventAnchor, Trunk};
use transmux::{EmsgBox, PresentationTime};

/// The DASH `emsg` `schemeIdUri` for a binary SCTE-35 section — the value
/// `crate::output::dash` also declares with `<InbandEventStream>`
/// (ANSI/SCTE 214-3).
const EMSG_SCHEME_ID_URI: &str = "urn:scte:scte35:2013:bin";

/// The `emsg` `timescale` field: the trunk event ring's 90 kHz clock, which
/// is also `presentation_time`'s clock (ANSI/SCTE 214-3).
const EMSG_TIMESCALE: u32 = timed_metadata::PTS_HZ as u32;

use crate::http::{self, BLOCKING_RELOAD_TIMEOUT};
use crate::route::{ProgramServing, RouteHandle};

pub(crate) const MP4_CONTENT_TYPE: &str = "video/mp4";

/// `Content-Type` for a whole MPEG-2 Transport Stream segment (a `ts_hls`
/// route's `seg-{track}-{seq}.ts`). RFC 3555 §4.3 registers `video/mp2t`
/// for MPEG-2 Transport Stream *data*, and a TS segment is exactly that —
/// serving it as `video/mp4` is a lie a strict player (or Apple's
/// `mediastreamvalidator`) rejects (audit run 7, W12).
pub(crate) const TS_CONTENT_TYPE: &str = "video/mp2t";

/// The `Content-Type` a resource body must be served with, chosen from the
/// route's container — fMP4/CMAF segments (`.m4s`) are `video/mp4`, whole
/// MPEG-2 TS segments (`.ts`) are `video/mp2t` (audit run 7, W12).
///
/// `HlsBody::Resource` covers init segments too (`init-{track}.mp4`), but
/// a `MpegTs` container serves no init resource at all, so a resource on an
/// `MpegTs` route is a `.ts` segment in practice. Anything else falls back
/// to `video/mp4` for forward compatibility with a future container.
fn resource_content_type(container: hls_runtime::server::Container) -> &'static str {
    match container {
        hls_runtime::server::Container::MpegTs => TS_CONTENT_TYPE,
        _ => MP4_CONTENT_TYPE,
    }
}

/// Abuse bound for [`stream_in_progress_segment`]'s whole-segment number,
/// mirroring `hls_runtime::server::engine`'s own `ABUSE_MSN_FUTURE_BOUND`
/// (RFC 8216bis §6.2.5.2's abuse-prevention SHOULD, applied here to the
/// DASH-facing whole-segment lookup): a legitimate LL-DASH client only ever
/// requests the segment right after the one it already has, so a segment
/// number more than a few ahead of the current live edge is either a broken
/// client or abuse — reject it immediately (404) rather than tying up a
/// blocking-wait task and a connection slot for the full
/// [`BLOCKING_RELOAD_TIMEOUT`].
const SEGMENT_ABUSE_FUTURE_BOUND: u32 = 4;

/// RAII guard bumping/dropping [`crate::prometheus::ACTIVE_BLOCKING_REQUESTS`]
/// for the lifetime of a genuine blocking wait ([`crate::http::resolve_blocking`]'s
/// `on_enter_wait`, both here and in `output::llhls`) — incremented on
/// construction, decremented on drop, so the gauge stays accurate even if the
/// awaited future is itself dropped (e.g. the client disconnects mid-wait),
/// not just on a normal return.
pub(crate) struct BlockingRequestGuard;

impl BlockingRequestGuard {
    pub(crate) fn new() -> Self {
        metrics::gauge!(crate::prometheus::ACTIVE_BLOCKING_REQUESTS).increment(1.0);
        BlockingRequestGuard
    }
}

impl Drop for BlockingRequestGuard {
    fn drop(&mut self) {
        metrics::gauge!(crate::prometheus::ACTIVE_BLOCKING_REQUESTS).decrement(1.0);
    }
}

/// Build the shared resource router for one stream: `GET /:file`, serving
/// `init-{track}.mp4` / `seg-{track}-{seq}.m4s` / `part-{track}-{seq}.{idx}.m4s`
/// via `route`'s `HlsOrigin`. Mounted once per stream by
/// [`crate::origin::router`], merged alongside every configured `Output`'s
/// manifest routes before the whole per-stream router is `.nest`ed — see
/// this module's docs.
///
/// `Cache-Control`/CORS headers are applied by the origin's shared
/// `add_response_headers` middleware (wrapping the *merged* per-stream
/// router, not this one alone), so every output's responses get the same
/// policy uniformly.
pub(crate) fn router(route: Arc<RouteHandle>) -> Router {
    Router::new()
        .route("/:file", get(dynamic_file).options(cors_preflight))
        .with_state(route)
}

/// `OPTIONS` preflight handler shared by every route this origin serves
/// (manifest and resource alike): browsers (hls.js/dash.js) send a CORS
/// preflight before the real `GET` for cross-origin requests with custom
/// headers. Returns `204 No Content` with no body; the origin's
/// `add_response_headers` middleware adds the actual
/// `Access-Control-Allow-*` headers to this response the same as every other
/// response.
pub(crate) async fn cors_preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

/// `GET /:file` — catch-all for the dynamic init/segment/part filenames
/// `route`'s `HlsOrigin` names (and the same filenames a DASH
/// `SegmentTemplate` references — see `crate::output::dash`).
///
/// A single catch-all (rather than three routes with per-filename literals)
/// because axum 0.7's `matchit`-based router cannot mix multiple params with
/// literal text in one path segment (e.g. `seg-:track-:seq.m4s`) — only one
/// param per segment is supported, capturing the whole segment. Parsing
/// `file` into a segment/part/init lookup — including the "block until a
/// preload-hinted part is produced" behaviour (RFC 8216bis §6.2.2, §6.3.1) —
/// is [`hls_runtime::server::HlsOrigin::resolve`]'s job; this handler
/// only drives the wait ([`http::resolve_blocking`]) and maps the outcome to
/// an HTTP response ([`http::into_response`]).
async fn dynamic_file(State(route): State<Arc<RouteHandle>>, Path(file): Path<String>) -> Response {
    let serving = match http::resolve_route_program(&route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let trunk = serving.trunk();
    let ll_hls = serving.ll_hls();
    let resp = http::resolve_blocking(
        &trunk,
        ll_hls.as_ref(),
        HlsRequest::Resource { name: file.clone() },
        BLOCKING_RELOAD_TIMEOUT,
        BlockingRequestGuard::new,
    )
    .await;
    match http::into_response(resp, StatusCode::NOT_FOUND, |body| {
        let body = inject_segment_events(&trunk, &file, body);
        resource_body_response(body, route.container())
    }) {
        // A resource that resolved NotFound might still be a whole-segment
        // filename the chunked-transfer path (issue #721) can serve while
        // its segment is still in progress -- try that before giving up.
        resp if resp.status() == StatusCode::NOT_FOUND => {
            if let Some((track, seq)) = parse_segment_filename(&file)
                && let Some(resp) = stream_in_progress_segment(serving, track, seq).await
            {
                return resp;
            }
            StatusCode::NOT_FOUND.into_response()
        }
        resp => resp,
    }
}

fn resource_body_response(
    body: hls_runtime::server::HlsBody,
    container: hls_runtime::server::Container,
) -> Response {
    match body {
        hls_runtime::server::HlsBody::Resource(bytes) => (
            [(header::CONTENT_TYPE, resource_content_type(container))],
            bytes,
        )
            .into_response(),
        // A resource request never resolves to a rendered playlist body --
        // defensive, not reachable via `dynamic_file`'s own `HlsRequest::Resource`.
        hls_runtime::server::HlsBody::Playlist(_) => StatusCode::NOT_FOUND.into_response(),
        // `HlsBody` is `#[non_exhaustive]`; treat any future body variant
        // the same as the playlist case above -- not a resource body.
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Parse a whole-segment dynamic filename (`seg-{track}-{seq}.m4s`) into
/// `(track, seq)`. Mirrors `hls_runtime::server`'s own (private)
/// `seg-`/`part-` filename parsing, but keeps `track` as a borrowed `&str`
/// (rather than discarding it once validated) so [`stream_in_progress_segment`]
/// can reuse it verbatim to build this segment's `part-{track}-{seq}.{idx}.m4s`
/// filenames -- `{track}` is otherwise unused (the origin holds a single
/// track's data regardless of the number a client's `$RepresentationID$`
/// substitution produces, exactly like `HlsOrigin::resolve` itself).
fn parse_segment_filename(file: &str) -> Option<(&str, u32)> {
    let rest = file.strip_prefix("seg-")?.strip_suffix(".m4s")?;
    let (track, seq) = rest.split_once('-')?;
    track.parse::<u32>().ok()?;
    Some((track, seq.parse().ok()?))
}

/// A `(scheme, value, id)`-unique `emsg` id for one inband SCTE-35 event
/// (issue #1083, W9).
///
/// A SCTE-35 `splice_insert` carries a `splice_event_id` (`Some`) — the
/// operator's own identity, kept **verbatim**. Only a `None` event (a
/// `time_signal` has no id at all) gets a derived id — the pre-fix
/// `unwrap_or(0)` gave every such event id 0, and DASH clients de-duplicate
/// by `(scheme, value, id)`, so all but the first were dropped. The derived
/// id walks upward past a collision with a value already emitted in this
/// segment's boxes, so uniqueness holds across every box spliced into one
/// segment.
fn unique_emsg_id(
    declared: Option<u32>,
    media_time_90k: u64,
    used: &mut std::collections::HashSet<u32>,
) -> u32 {
    // A declared `splice_event_id` is kept verbatim — it is the operator's
    // own identity, and a client keyed on it must see the same value. If it
    // collides with an already-emitted id in this segment (two events
    // genuinely sharing a `splice_event_id`), that is the upstream's data,
    // not something to silently rewrite.
    if let Some(id) = declared {
        used.insert(id);
        return id;
    }
    // No declared id: derive one from the presentation time (90 kHz ticks
    // already), truncating into the id space, and walk upward past any
    // collision with a value emitted earlier in this segment.
    let base = u32::try_from(media_time_90k).unwrap_or_else(|_| {
        // The 90 kHz clock is 33-bit; anything wider cannot occur, but if it
        // ever did, fold it rather than panic.
        u32::try_from(media_time_90k % u64::from(u32::MAX)).unwrap_or(u32::MAX)
    });
    let mut id = base;
    while !used.insert(id) {
        // Wrapping is fine (the derived id is opaque); the walk terminates
        // because `used` is finite and smaller than the u32 space.
        id = id.wrapping_add(1);
    }
    id
}

/// Inject DASH `emsg` boxes for a segment's resolved SCTE-35 events into
/// served fMP4 segment bytes (issue #969) — the inband counterpart to
/// `crate::output::dash`'s `<InbandEventStream>` MPD declaration.
///
/// Per CMAF, event boxes sit between `styp` and `moof`:
/// `[styp][emsg*][moof][mdat]`. So this splices the serialized `emsg` boxes
/// in immediately after `styp`. Only **resolved** events (already on this
/// trunk's 90 kHz absolute clock, [`EventAnchor::Media`]) and only
/// SCTE-35-sourced events are injected; non-segment resources (init, parts),
/// segments with no (relevant) events, and non-SCTE-35 events all pass
/// through unchanged.
/// Inject DASH `emsg` boxes for a segment's resolved SCTE-35 events into
/// served fMP4 segment bytes (issue #969) — the inband counterpart to
/// `crate::output::dash`'s `<InbandEventStream>` MPD declaration.
///
/// Per CMAF, event boxes sit between `styp` and `moof`:
/// `[styp][emsg*][moof][mdat]`. So this splices the serialized `emsg` boxes
/// in immediately after `styp`. Only **resolved** events (already on this
/// trunk's 90 kHz absolute clock, [`EventAnchor::Media`]) and only
/// SCTE-35-sourced events are injected; non-segment resources (init, parts),
/// segments with no (relevant) events, and non-SCTE-35 events all pass
/// through unchanged.
fn inject_segment_events(trunk: &Trunk, file: &str, body: HlsBody) -> HlsBody {
    let HlsBody::Resource(bytes) = &body else {
        return body;
    };
    let Some((_track, seq)) = parse_segment_filename(file) else {
        return body;
    };

    let events = trunk.events_in_segment(seq);
    if events.is_empty() {
        return body;
    }

    // Build a serialized emsg box per SCTE-35 event. Skip everything else
    // silently (a non-SCTE-35 event, or an unresolved [EventAnchor] that
    // can't yet produce a presentation-time).
    // The raw splice bytes live verbatim in `SourcePayload::Scte35`; we build
    // the box directly from the raw bytes rather than via
    // `timed_metadata::Timeline::to_emsg` (which needs an `EmsgConfig`).
    let mut emsg_bytes = Vec::new();
    // DASH clients de-duplicate `emsg` boxes by `(scheme_id_uri, value, id)`
    // (ISO/IEC 23009-1 §5.10.3.3): two events sharing an id means the client
    // silently drops the later one. A SCTE-35 `time_signal` has no
    // `splice_event_id` at all (`TimedEvent::id == None`), so the pre-fix
    // `id.unwrap_or(0)` made every id-less event collide on id 0 (issue
    // #1083, W9). Derive a stable, presentation-time-based id for those, and
    // guarantee uniqueness across everything actually emitted below — a real
    // `splice_event_id` is kept as-is (it is the operator-visible identity),
    // but a collision is resolved by nudging the derived id rather than
    // dropping an event.
    let mut used_ids: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for entry in &events {
        let timed_metadata::event::SourcePayload::Scte35 { raw } = &entry.event.source else {
            continue;
        };
        let EventAnchor::Media(media_time) = entry.anchor else {
            continue;
        };
        let id = unique_emsg_id(entry.event.id, media_time.0, &mut used_ids);
        let emsg = EmsgBox {
            scheme_id_uri: EMSG_SCHEME_ID_URI,
            value: "",
            timescale: EMSG_TIMESCALE, // Trunk's 90 kHz event clock
            presentation_time: PresentationTime::Absolute(media_time.0),
            // `event_duration` is a u32; saturate an over-long duration rather
            // than truncating -- a client reading the box wants to know the
            // break spans the whole segment, not a wrapped-around short value.
            event_duration: entry
                .event
                .duration
                .map(|d| u32::try_from(d.0).unwrap_or(u32::MAX))
                .unwrap_or(0),
            id,
            message_data: raw,
        };
        if let Ok(box_bytes) = emsg.to_vec() {
            emsg_bytes.extend_from_slice(&box_bytes);
        }
    }

    if emsg_bytes.is_empty() {
        return body;
    }

    // Splice the emsg boxes after the styp box, before moof. styp box layout:
    // first 4 bytes = big-endian u32 size, next 4 bytes = "styp".
    let segment = bytes.as_ref();
    let spliced = if segment.len() >= 8 && &segment[4..8] == b"styp" {
        let styp_size =
            u32::from_be_bytes([segment[0], segment[1], segment[2], segment[3]]) as usize;
        if styp_size <= segment.len() {
            let mut out = Vec::with_capacity(segment.len() + emsg_bytes.len());
            out.extend_from_slice(&segment[..styp_size]);
            out.extend_from_slice(&emsg_bytes);
            out.extend_from_slice(&segment[styp_size..]);
            out
        } else {
            // Malformed styp size -- can't identify the split point; prepend
            // the boxes rather than corrupting the file.
            let mut out = Vec::with_capacity(segment.len() + emsg_bytes.len());
            out.extend_from_slice(&emsg_bytes);
            out.extend_from_slice(segment);
            out
        }
    } else {
        // No styp box (unexpected for CMAF, but be defensive): prepend.
        let mut out = Vec::with_capacity(segment.len() + emsg_bytes.len());
        out.extend_from_slice(&emsg_bytes);
        out.extend_from_slice(segment);
        out
    };

    HlsBody::Resource(Bytes::from(spliced))
}

/// Serve a not-yet-closed whole-segment filename (`seg-{track}-{seq}.m4s`)
/// over **HTTP chunked transfer-encoding**, streaming `seq`'s
/// `part-{track}-{seq}.{idx}.m4s` bytes in order as they are produced
/// (issue #721 -- see this module's docs and `crate::output::ll_dash`).
///
/// `None` (caller 404s) if the segment's very first part never arrives
/// within [`BLOCKING_RELOAD_TIMEOUT`] — a genuinely future segment whose
/// ingest hasn't reached it yet (or a stalled/dead route), mirroring the
/// plain closed-segment lookup's own bound. Once the first part is ready,
/// `Some` commits to a `200 OK` streamed response that keeps pulling
/// subsequent parts (each wait bounded the same way) until a part index
/// resolves [`media_plane::egress::EgressResponse::NotFound`] — which only
/// happens once the segment has actually closed (or been evicted) without
/// that part, i.e. exactly the segment's end — at which point the stream
/// ends normally (the response completes; axum/hyper terminate the
/// chunked-transfer encoding on drop).
async fn stream_in_progress_segment(
    serving: Arc<ProgramServing>,
    track: &str,
    seq: u32,
) -> Option<Response> {
    // Abuse/malformed-request bound (see `SEGMENT_ABUSE_FUTURE_BOUND`) --
    // checked before ever registering a blocking wait.
    let (in_progress_seg_seq, _) = serving.latest_progress();
    if seq > in_progress_seg_seq.saturating_add(SEGMENT_ABUSE_FUTURE_BOUND) {
        return None;
    }

    let track = track.to_string();
    let first = fetch_part(&serving, &track, seq, 0).await;
    let first_bytes = first?;

    let cursor = PartCursor {
        serving,
        track,
        seq,
        next_index: 1,
        pending_first: Some(first_bytes),
    };
    let body_stream = stream::unfold(cursor, |mut cursor| async move {
        if let Some(bytes) = cursor.pending_first.take() {
            return Some((Ok::<_, std::io::Error>(bytes), cursor));
        }
        match fetch_part(
            &cursor.serving,
            &cursor.track,
            cursor.seq,
            cursor.next_index,
        )
        .await
        {
            Some(bytes) => {
                cursor.next_index += 1;
                Some((Ok(bytes), cursor))
            }
            None => None,
        }
    });

    let mut response = Response::new(Body::from_stream(body_stream));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(MP4_CONTENT_TYPE),
    );
    Some(response)
}

/// Fetch one part's bytes, blocking (bounded) if it is a preload-hinted part
/// not yet produced — `None` once it can no longer appear (its segment
/// closed without it).
async fn fetch_part(
    serving: &Arc<ProgramServing>,
    track: &str,
    seq: u32,
    idx: u32,
) -> Option<bytes::Bytes> {
    let trunk = serving.trunk();
    let ll_hls = serving.ll_hls();
    let name = format!("part-{track}-{seq}.{idx}.m4s");
    let resp = http::resolve_blocking(
        &trunk,
        ll_hls.as_ref(),
        HlsRequest::Resource { name },
        BLOCKING_RELOAD_TIMEOUT,
        BlockingRequestGuard::new,
    )
    .await;
    match resp {
        media_plane::egress::EgressResponse::Ready {
            body: hls_runtime::server::HlsBody::Resource(bytes),
            ..
        } => Some(bytes),
        _ => None,
    }
}

/// Streaming state for [`stream_in_progress_segment`]'s `futures_util::stream::unfold`.
struct PartCursor {
    serving: Arc<ProgramServing>,
    track: String,
    seq: u32,
    /// The 0-based index of the next part to fetch once `pending_first` is
    /// drained.
    next_index: u32,
    /// Part 0's bytes, already fetched by the caller to decide whether to
    /// commit to a streamed response at all -- yielded first so it isn't
    /// fetched twice.
    pending_first: Option<bytes::Bytes>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::RouteHandle;
    use hls_runtime::server::Container;
    use media_plane::trunk::TrunkConfig;
    use timed_metadata::{MediaTime, TimeAnchor};
    use transmux::ll_hls::{PartInfo, SegmentInfo};

    fn part(seq: u32, idx: u32) -> PartInfo {
        PartInfo {
            bytes: vec![0x10 + idx as u8; 4],
            duration: 0.5,
            independent: idx == 0,
            segment_seq: seq,
            part_index: idx,
        }
    }

    fn seg(seq: u32) -> SegmentInfo {
        SegmentInfo {
            bytes: vec![0x20 + seq as u8; 8],
            duration: 4.0,
            segment_seq: seq,
            part_count: 2,
        }
    }

    /// A populated route: a closed segment 1, plus two live parts of
    /// in-progress segment 2 -- so `latest_progress()` treats it as `(2, 2)`.
    /// Publishes `SPTS_PROGRAM_ID` into the registry first
    /// (`publish_new_program`, issue #805 tasks 3/6) so
    /// `dynamic_file`/`fetch_part`'s `resolve_route_program` lookup sees
    /// `Found`, and so there is a `ProgramServing` bundle to write into.
    fn make_route() -> Arc<RouteHandle> {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 8]);
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1))
            .expect("add_segment");
        route.add_part(crate::route::SPTS_PROGRAM_ID, part(2, 0));
        route.add_part(crate::route::SPTS_PROGRAM_ID, part(2, 1));
        route
    }

    async fn body_bytes(resp: Response) -> Vec<u8> {
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    #[tokio::test]
    async fn dynamic_file_init_present() {
        let route = make_route();
        let resp = dynamic_file(State(route), Path("init-1.mp4".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, vec![0xAA; 8]);
    }

    #[tokio::test]
    async fn dynamic_file_segment_present_and_absent() {
        let route = make_route();
        let ok = dynamic_file(State(route.clone()), Path("seg-1-1.m4s".to_string())).await;
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(content_type(&ok), "video/mp4");
        assert_eq!(body_bytes(ok).await, vec![0x21; 8]);

        let missing = dynamic_file(State(route), Path("seg-1-99.m4s".to_string())).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    /// Audit run 7, W12: a `ts_hls` route (`Container::MpegTs`) serves
    /// whole MPEG-2 TS segments, so the resource route must answer
    /// `video/mp2t`, not the `video/mp4` it hard-coded before — a strict
    /// player (and Apple's `mediastreamvalidator`) rejects the mismatch.
    #[tokio::test]
    async fn ts_route_segment_is_served_as_video_mp2t() {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4).with_container(Container::MpegTs));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, ts_seg(1))
            .expect("add_segment");

        let resp = dynamic_file(State(route), Path("seg-1-1.ts".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            content_type(&resp),
            "video/mp2t",
            "a TS segment must not be advertised as video/mp4"
        );
        assert_eq!(body_bytes(resp).await, vec![0x47; 188]);
    }

    fn content_type(resp: &Response) -> String {
        resp.headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    }

    /// A minimal but real MPEG-2 TS segment: one 188-byte packet whose
    /// sync byte is `0x47` — the same byte ffprobe/mediastreamvalidator key
    /// their container detection on.
    fn ts_seg(seq: u32) -> SegmentInfo {
        SegmentInfo {
            bytes: vec![0x47; 188],
            duration: 4.0,
            segment_seq: seq,
            part_count: 0,
        }
    }

    #[tokio::test]
    async fn dynamic_file_part_present() {
        let route = make_route();
        let resp = dynamic_file(State(route), Path("part-1-2.0.m4s".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        // W12: an fMP4 part is served as `video/mp4` (the fMP4 container),
        // not the TS type.
        assert_eq!(content_type(&resp), "video/mp4");
        assert_eq!(body_bytes(resp).await, vec![0x10; 4]);
    }

    /// W12: the resource content type follows the **container**, not the
    /// track type — an audio-only fMP4 part is still `video/mp4` (CMAF), not
    /// anything video-specific. Exercises the part path on a route with no
    /// video track at all.
    #[tokio::test]
    async fn audio_only_fmp4_part_is_video_mp4() {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 8]);
        route.add_part(crate::route::SPTS_PROGRAM_ID, part(1, 0));
        let resp = dynamic_file(State(route), Path("part-1-1.0.m4s".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            content_type(&resp),
            "video/mp4",
            "an fMP4 part is video/mp4 regardless of the track's media type"
        );
    }

    #[tokio::test]
    async fn dynamic_file_part_blocks_until_available_then_serves() {
        // part-1-2.2 is the preload-hinted next part of in-progress segment 2
        // (which currently has parts .0 and .1). The request must BLOCK until
        // the part is produced, not 404 immediately. Produce it after a short
        // delay from another task, then assert the handler returned its bytes.
        let route = make_route();
        let route_for_task = route.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            route_for_task.add_part(crate::route::SPTS_PROGRAM_ID, part(2, 2));
        });
        let resp = dynamic_file(State(route), Path("part-1-2.2.m4s".to_string())).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "part request must block until the part is produced, not 404"
        );
        assert_eq!(body_bytes(resp).await, vec![0x12; 4]); // 0x10 + idx(2)
    }

    #[tokio::test]
    async fn dynamic_file_part_404_promptly_when_segment_closes_without_it() {
        // part-1-2.9 will never be produced. When segment 2 closes (advancing
        // the in-progress segment), the handler must 404 promptly — not hang
        // until the blocking timeout.
        let route = make_route();
        let route_for_task = route.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            route_for_task
                .add_segment(crate::route::SPTS_PROGRAM_ID, seg(2))
                .expect("add_segment"); // closes segment 2
        });
        let started = std::time::Instant::now();
        let resp = dynamic_file(State(route), Path("part-1-2.9.m4s".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(
            started.elapsed() < BLOCKING_RELOAD_TIMEOUT,
            "must 404 promptly on segment close, not wait out the timeout"
        );
    }

    #[tokio::test]
    async fn dynamic_file_part_served_after_close() {
        // Segment 2 has live parts .0 and .1; close it. Its final part must
        // still be served -- the in-flight preload-hint request that races
        // the segment close must not 404. Both behaviours below shipped as
        // multimux 0.2.1/0.2.2 bug fixes and are now asserted directly
        // against the live camera's own regression shape (see
        // `hls-runtime/src/server/engine.rs`'s own tests for the same
        // property at the `ServedEgress` layer; this test proves the axum
        // adapter preserves it end to end).
        let route = make_route();
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(2))
            .expect("add_segment"); // close segment 2
        let resp = dynamic_file(State(route), Path("part-1-2.1.m4s".to_string())).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a just-closed segment's part must still be served, not 404"
        );
        assert_eq!(body_bytes(resp).await, vec![0x11; 4]); // part(2,1): 0x10 + idx(1)
    }

    #[tokio::test]
    async fn dynamic_file_part_of_old_closed_segment_404() {
        // Segment 1 closed in make_route() with no parts recorded, so a
        // request for one of its parts 404s without blocking (it will never
        // be produced and isn't individually addressable anymore).
        let route = make_route();
        let resp = dynamic_file(State(route), Path("part-1-1.0.m4s".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dynamic_file_unmatched_filename_404() {
        let route = make_route();
        let resp = dynamic_file(State(route), Path("not-a-thing.txt".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // --- issue #721: chunked-transfer whole-segment serving ---

    #[tokio::test]
    async fn dynamic_file_in_progress_segment_streams_concatenated_parts_and_completes_on_close() {
        // Only part 0 exists when the request is made -- part 1 doesn't
        // land, and the segment doesn't close, until *after* the handler
        // must already have committed to a streamed response (it can only
        // ever see part 0 at call time). This proves genuine incremental
        // streaming, not "wait for everything, then answer once": if the
        // handler eagerly required the whole segment up front, this request
        // would have nothing to serve yet and would 404/block differently.
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 8]);
        route.add_part(crate::route::SPTS_PROGRAM_ID, part(2, 0));

        let route_for_task = route.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            route_for_task.add_part(crate::route::SPTS_PROGRAM_ID, part(2, 1));
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            route_for_task
                .add_segment(crate::route::SPTS_PROGRAM_ID, seg(2))
                .expect("add_segment");
        });

        let resp = dynamic_file(State(route), Path("seg-1-2.m4s".to_string())).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "an in-progress whole-segment request must stream, not 404"
        );
        assert_eq!(
            body_bytes(resp).await,
            [vec![0x10; 4], vec![0x11; 4]].concat(),
            "streamed body must be part 0 + part 1 concatenated in order, \
             including the part that only landed after the response started"
        );
    }

    #[tokio::test]
    async fn dynamic_file_future_segment_within_bound_blocks_then_streams_once_started() {
        // Segment 3 hasn't started at all (latest_progress() == (2, 2), so 3
        // is the very next segment -- within SEGMENT_ABUSE_FUTURE_BOUND).
        // The request must block (not immediately 404) until the segment's
        // first part lands, then stream from it.
        let route = make_route();
        let route_for_start = route.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            route_for_start.add_part(
                crate::route::SPTS_PROGRAM_ID,
                PartInfo {
                    bytes: vec![0x77; 4],
                    duration: 0.5,
                    independent: true,
                    segment_seq: 3,
                    part_index: 0,
                },
            );
        });

        let started = std::time::Instant::now();
        let resp = dynamic_file(State(route.clone()), Path("seg-1-3.m4s".to_string())).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a near-future segment must be waited for, not rejected"
        );
        assert!(
            started.elapsed() < BLOCKING_RELOAD_TIMEOUT,
            "must resolve once the part lands, not idle out the full timeout"
        );
        // Only one part exists so far; the response completes once segment 3
        // eventually closes. Close it now so the body finishes.
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(3))
            .expect("add_segment");
        assert_eq!(body_bytes(resp).await, vec![0x77; 4]);
    }

    #[tokio::test]
    async fn dynamic_file_far_future_segment_beyond_abuse_bound_404_promptly() {
        // latest_progress() == (2, 2); segment 99 is far beyond
        // SEGMENT_ABUSE_FUTURE_BOUND ahead of the live edge -- must reject
        // immediately (no blocking wait at all), unlike a legitimate
        // near-future segment.
        let route = make_route();
        let started = std::time::Instant::now();
        let resp = dynamic_file(State(route), Path("seg-1-99.m4s".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "an abusive far-future segment number must 404 promptly, not block: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn dynamic_file_closed_segment_still_served_whole_not_streamed() {
        // Regression: a segment that is ALREADY closed must still take the
        // plain, non-streaming fast path (whole bytes, never falling through
        // to `stream_in_progress_segment`) -- proven by the exact byte match
        // ([0x21; 8] is `seg`'s literal whole-segment fixture bytes, not a
        // concatenation of any parts).
        let route = make_route();
        let resp = dynamic_file(State(route), Path("seg-1-1.m4s".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, vec![0x21; 8]);
    }

    /// MUTATION VERIFIED (issue #805 task 4): a route with no program
    /// announced yet (a bare `RouteHandle::new`, never
    /// `publish_new_program`/`publish_program`d) must answer `503`, not
    /// `404` -- see `output::llhls`'s identical test for the mutation this
    /// guards (`http::resolve_route_program`'s `NotYetAnnounced` arm).
    #[tokio::test]
    async fn dynamic_file_not_yet_announced_is_503_not_404() {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        let resp = dynamic_file(State(route), Path("init-1.mp4".to_string())).await;
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "a route with no program announced yet must be 503 (not ready), not 404 (gone)"
        );
    }

    /// The workspace's existing `make_route` builds a `RouteHandle`, whose
    /// `Trunk` is reachable through `ProgramServing::trunk()` — but a plain
    /// closed-segment route carries no events in its event log, so for the
    /// `inject_segment_events` unit tests below we build a bare `Trunk`
    /// directly and publish a SCTE-35 event into it.
    fn event_trunk() -> Arc<Trunk> {
        let nz = |n: usize| std::num::NonZeroUsize::new(n).unwrap();
        let config = TrunkConfig::new(nz(8), nz(8), nz(8), nz(8), nz(8));
        let trunk = Trunk::new(config);
        let writer = trunk.writer().expect("writer");
        let seg_writer = trunk.segment_writer().expect("segment writer");

        seg_writer.set_time_anchor(TimeAnchor {
            pts_90k: 0,
            utc_epoch_ms: 1_000_000_000_000,
        });

        // Parse a real SCTE-35 splice (a program-splice, so `at` is None) and
        // publish it as a Media-anchored event at the start of segment 1.
        let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
        let splice_bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let mut timeline = timed_metadata::Timeline::new();
        let ev = timeline.push_scte35(&splice_bytes).unwrap();
        writer.publish_event(
            ev.clone(),
            EventAnchor::Media(ev.at.unwrap_or(MediaTime(0))),
        );

        // Segment boundaries: segment 1 spans [0, 4_000_000), segment 2 from
        // 4_000_000 on. The event at pts 0 falls inside segment 1.
        seg_writer.note_segment_start(1, MediaTime(0));
        seg_writer.note_segment_start(2, MediaTime(4_000_000));
        trunk
    }

    #[test]
    fn inject_segment_events_prepends_emsg_after_styp() {
        // issue #969: a segment filename with a resolved SCTE-35 event must
        // come back with a `[styp][emsg][moof]` shape -- emsg after styp.
        let trunk = event_trunk();

        // Minimal fake segment: styp box (12 bytes: size + "styp" + brand) + a
        // moof placeholder.
        let styp: [u8; 12] = [
            0, 0, 0, 12, // size = 12
            b's', b't', b'y', b'p', //
            b'm', b's', b'd', b'h', // major brand
        ];
        let moof = b"moof_placeholder";
        let mut seg_bytes = Vec::new();
        seg_bytes.extend_from_slice(&styp);
        seg_bytes.extend_from_slice(moof);

        let body = HlsBody::Resource(Bytes::from(seg_bytes));
        let result = inject_segment_events(&trunk, "seg-1-1.m4s", body);

        let HlsBody::Resource(result_bytes) = result else {
            panic!("expected Resource body");
        };

        // The result is larger (an emsg box was injected after styp).
        assert!(
            result_bytes.len() > 12 + moof.len(),
            "emsg should have been injected"
        );
        // styp is still the leading box.
        assert_eq!(&result_bytes[4..8], b"styp");
        // The emsg box begins right after styp's 12-byte box: bytes [12..15].
        assert_eq!(
            &result_bytes[16..20],
            b"emsg",
            "emsg box should be immediately after styp"
        );
    }

    /// Two SCTE-35 `time_signal` events (no `splice_event_id`, so
    /// `TimedEvent::id == None`) in one segment must get DISTINCT `emsg` ids
    /// — DASH clients de-duplicate by `(scheme, value, id)` (ISO/IEC
    /// 23009-1 §5.10.3.3) and would drop the second pre-fix, when both
    /// resolved to id 0.
    ///
    /// Biting test: restore `id: entry.event.id.unwrap_or(0)` and both boxes
    /// carry id 0, failing the distinctness assertion.
    #[test]
    fn inject_segment_events_gives_id_less_events_distinct_ids() {
        let nz = |n: usize| std::num::NonZeroUsize::new(n).unwrap();
        let trunk = Trunk::new(TrunkConfig::new(nz(8), nz(8), nz(8), nz(8), nz(8)));
        let writer = trunk.writer().expect("writer");
        let seg_writer = trunk.segment_writer().expect("segment writer");
        seg_writer.set_time_anchor(TimeAnchor {
            pts_90k: 0,
            utc_epoch_ms: 1_000_000_000_000,
        });

        // Build one id-less event from the real SCTE-35 fixture the crate's
        // own tests use, then publish it twice at different anchors — the
        // id-less shape (`TimedEvent::id == None`) that a `time_signal`
        // produces.
        let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
        let sb: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let mut timeline = timed_metadata::Timeline::new();
        let mut ev = timeline.push_scte35(&sb).unwrap();
        ev.id = None; // force the id-less shape under test
        assert_eq!(ev.id, None, "this test needs an id-less event");

        let mut a = ev.clone();
        a.id = None;
        let mut b = ev.clone();
        b.id = None;
        writer.publish_event(a, EventAnchor::Media(MediaTime(0)));
        writer.publish_event(b, EventAnchor::Media(MediaTime(1_000_000)));
        seg_writer.note_segment_start(1, MediaTime(0));
        seg_writer.note_segment_start(2, MediaTime(4_000_000));

        let styp: [u8; 12] = [0, 0, 0, 12, b's', b't', b'y', b'p', b'm', b's', b'd', b'h'];
        let mut seg_bytes = Vec::new();
        seg_bytes.extend_from_slice(&styp);
        seg_bytes.extend_from_slice(b"moof_placeholder");
        let result = inject_segment_events(
            &trunk,
            "seg-1-1.m4s",
            HlsBody::Resource(Bytes::from(seg_bytes)),
        );
        let HlsBody::Resource(out) = result else {
            panic!("expected Resource");
        };
        // Collect every emsg id in the output.
        let parsed: Vec<EmsgBox<'_>> = parse_emsg_boxes(&out);
        assert_eq!(parsed.len(), 2, "two emsg boxes must parse");
        assert_ne!(
            parsed[0].id, parsed[1].id,
            "id-less events must not collide on id 0 — DASH clients drop the later one"
        );
    }

    /// Parse every `emsg` box in `bytes` (a small `#[cfg(test)]` helper so the
    /// ids can be asserted literally without re-decoding by hand).
    fn parse_emsg_boxes(bytes: &[u8]) -> Vec<EmsgBox<'_>> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i + 8 <= bytes.len() {
            let size =
                u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
            if size < 8 || i + size > bytes.len() {
                break;
            }
            if &bytes[i + 4..i + 8] == b"emsg"
                && let Ok(e) = EmsgBox::parse(&bytes[i..i + size])
            {
                out.push(e);
            }
            i += size;
        }
        out
    }

    #[test]
    fn inject_segment_events_passthrough_for_init_and_eventless_segment() {
        // Non-segment resources (init) and segments with no events pass
        // through byte-identical.
        let trunk = event_trunk();

        let init_body = HlsBody::Resource(Bytes::from(vec![0xAA; 8]));
        let init_result = inject_segment_events(&trunk, "init-1.mp4", init_body);
        let HlsBody::Resource(init_bytes) = init_result else {
            panic!("init expected Resource body");
        };
        assert_eq!(&init_bytes[..], &[0xAA; 8]);

        // Segment 2 contains no events (the only event is in segment 1).
        let seg_body = HlsBody::Resource(Bytes::from(vec![
            0x00, 0x00, 0x00, 0x0C, b's', b't', b'y', b'p', 0, 0, 0, 0,
        ]));
        let seg_result = inject_segment_events(&trunk, "seg-1-2.m4s", seg_body);
        let HlsBody::Resource(seg_bytes) = seg_result else {
            panic!("segment expected Resource body");
        };
        assert_eq!(seg_bytes.len(), 12);
    }
}
