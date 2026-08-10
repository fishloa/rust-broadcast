# multimux 0.9.0

**Release date:** 2026-08-10

The largest change set of this release wave: DVR catch-up/VOD-from-live
serving, EIT-aligned programme-boundary DVR rolling, WHIP ingest and WHEP
egress, push-egress convergence onto the sans-IO `PushEgress` shape, several
correctness fixes (an RTSP auth-failure retry loop, an admin-API lock
poisoning that could permanently disable the admin API, a DVR index-rebuild
panic, an unbounded allocation from a corrupt DVR sidecar, an ingest
takedown from a single bad datagram, and RTMP push framing/URL-parsing
bugs), and two source-breaking signature changes. Four transitive dependency
floors move to close understated-requirement gaps found by an audit, not to
add features. Carries the workspace-wide MSRV bump.

## Breaking changes

- **`source::ts_udp::recv_and_feed` now returns `Result<usize>`** (was
  `Result<()>`) — the number of bytes read, so a caller can also feed the
  same slice to the new `RouteHandle::feed_si_ts` EIT tracker without a
  second read.

  ```rust
  // Before (0.8.x)
  pub async fn recv_and_feed(...) -> Result<()>;

  // After (0.9.0)
  pub async fn recv_and_feed(...) -> Result<usize>;
  ```

  Source-breaking for any direct caller of this `pub` async function; a
  caller that ignored the `Ok` value is unaffected.

- **`source::srt::StreamStatus::Fed` and `source::ts_http::StreamStatus::Fed`
  now carry the payload** (`Fed(Vec<u8>)`, were the unit variant `Fed`), for
  the same EIT-tracker reason.

  ```rust
  // Before (0.8.x)
  pub enum StreamStatus {
      Fed,
      Ended,
  }

  // After (0.9.0)
  pub enum StreamStatus {
      Fed(Vec<u8>),
      Ended,
  }
  ```

  Both `StreamStatus` enums are `pub`; a direct `match ... { StreamStatus::Fed => ... }`
  no longer compiles without a binding on `Fed`.

- **Not breaking, despite two earlier pre-release audits claiming otherwise:**
  `InputSpec::Custom` and `OutputKind::Catchup` are additive variants on
  enums that were already `#[non_exhaustive]` as of the `multimux-v0.8.0`
  tag. No external exhaustive `match` over either enum compiled at 0.8.0,
  so nothing breaks by adding a variant now. If you were told a previous
  draft of this release described a breaking `#[non_exhaustive]` addition,
  that was incorrect and is superseded by this note.

## What's new

- **Catch-up / time-shift / VOD-from-live serving over the DVR archive**
  (issue #900): a new `"catchup"` output (`OutputKind::Catchup`) serves
  `GET /catchup.m3u8` (optionally `?window_secs=N`), `GET /vod/p{N}.m3u8`
  (a finished archived period rendered as a complete VOD asset), and
  `GET /catchup/seg-{seq}.{ext}`. Requires `routes.dvr.enabled: true`.
  `catchup.m3u8` merges the DVR archive's segments with only the live
  `Trunk`'s still-unarchived tail (`crate::catchup::merge_segments`) into
  one continuous, gap-free playlist across the archive/live boundary,
  reading the archive fresh from disk per request and the live tail through
  hls-runtime's new `HlsOrigin::closed_segments()` — no second in-memory
  cache of live data.
  - `crate::dvr::IndexEntry` gained `duration_ns`/`discontinuous` fields
    (`#[serde(default)]`, so an existing `pN.idx` still parses).
- **Programme-aligned DVR rolling** (issue #903): a DVR route can opt in to
  rolling its archive period on the DVB EIT present/following transition
  (ETSI EN 300 468 §5.2.4) instead of only on the clock. `DvrConfig::dvb_service_id`
  names the service to track; `DvrRecorder::feed_si` (fed by the new
  `RouteHandle::feed_si_ts`) reassembles the EIT p/f actual section and
  rolls the period when the present `event_id` changes, writing a
  `pN.event.json` sidecar alongside `pN.<ext>`/`pN.idx`. The clock-based
  `period_duration_secs` stays active as both fallback and hard cap.
- **WHEP egress** (`OutputKind::Whep`, config token `"whep"`, issue #743):
  accepts a viewer's `POST`ed SDP offer, negotiates ICE + DTLS-SRTP, and
  pushes the route's `Trunk` samples out as SRTP RTP. Video (H.264) only, no
  trickle ICE, no `PATCH`. Behind the new `whep` feature.
- **WHIP ingest** (`InputSpec::Whip`, config token `"whip"`, issues
  #740/#743): an inbound WHIP publisher (RFC 9725). Video-only (H.264); no
  RTP/Opus depayloader exists in this workspace. Behind the new `whip`
  feature.
- **Push egress converged onto the same sans-IO `PushEgress` shape as
  WHIP/WHEP** (issue #942): `PushTransport` gained `encode_media` and
  `write_message`, so `PushTransportEgress<T>` implements `PushEgress`.
  `drive_push` now also detects a mid-stream track-set change
  (`Trunk::track_generation`) and renegotiates.

## Fixed

- **One authenticated `POST /admin/routes` request could permanently
  disable the admin API.** `RouteRegistry::add_route` held the registry's
  write lock across `spawn_route`; a route whose outputs included a `whep`
  entry (accepted by `validate_standalone`, which never rejected it)
  reached an `unreachable!()` in `build_output`, and the resulting panic
  unwound while the lock was held, poisoning it — every later admin
  operation and router rebuild calls `.expect()` on that same lock, so one
  bad request took the admin API down for the rest of the process
  lifetime. `spawn_route` now filters `whep` outputs the same way the
  startup path already did. Requires the `whep` feature.
- **A wrong RTSP password retried forever instead of failing the route**
  (issue #957). `origin::supervisor::supervise_driver` now treats
  `MultimuxError::Auth` (and a `404` specifically on RTSP `DESCRIBE`) as
  non-transient after a bounded number of consecutive attempts
  (`MAX_AUTH_ATTEMPTS_BEFORE_PERMANENT`, 5), marking the route
  `HealthState::Failed` instead of retrying indefinitely; a camera still
  booting can transiently answer `401` and is tolerated within that bound.
- **`catchup::read_archived_bytes` allocated whatever a corrupt `pN.idx`
  sidecar claimed.** `byte_len` is now bound against the period file's real
  size (plus a `checked_add` on `offset + len`) before allocating.
- **One unauthenticated datagram could tear down a live WHIP ingest.** An
  SRTP authentication failure on the post-handshake media socket (which
  accepts datagrams from any source) is now logged at debug and skipped
  instead of reaping the session, matching `output::whep`'s existing
  handling.
- **Off-by-four panic in DVR index rebuild on a truncated period file.**
  `rebuild_index` guarded `mdat_offset + 4 <= data.len()` but then read
  `&data[mdat_offset + 4..mdat_offset + 8]`; a period file truncated
  between a `moof` and the `mdat` four-CC (the routine power-loss/full-disk
  shape `rebuild_index` exists to recover) now recovers instead of
  panicking.
- `routes.dvr` was validated but never wired into the `RouteHandle` built
  for a config-driven or admin-API-added route, so DVR recording configured
  via `Config`/JSON silently never ran. Both route-creation paths now call
  `.with_dvr(route.dvr.clone())`.
- RTMP push shipped raw MPEG-2 TS as an RTMP `send_video` payload, which no
  RTMP server can decode. `PushTransport` gained `send_media`; `RtmpTransport`
  overrides it to split each batch into FLV-framed payloads dispatched
  through `send_video`/`send_audio`, sending `onMetaData` plus AVC/AAC
  sequence headers first. Verified against a real, independent RTMP server
  over a real TCP loopback connection.
- RTMP push's `app`/`stream_key` were derived wrong from the push URL (`app`
  took the whole path; `stream_key` was always empty). The last path segment
  is now the stream key, everything before it the app.

## Dependency floor corrections

Four transitive floors move because the *published* previous version's
declared requirement was looser than what the code actually calls — not
because of new features in this release:

- `broadcast-common` `"9"` -> `"9.3"` (`output::smooth`'s
  `broadcast_common::hex::hex_encode` does not exist at 9.0.0/9.1.0).
- `transmux` `"0.23"` -> `"0.24"` (`push::rtmp`'s
  `flv_sequence_header_payloads`/`flv_frame_payloads` do not exist at
  transmux-v0.23.0).
- `hls-runtime` `"0.5"` -> `"0.6"` (`catchup.rs` uses
  `hls_runtime::server::ClosedSegment`/`HlsOrigin::closed_segments()`, new
  in hls-runtime 0.6.0).
- `broadcast-auth` `"0.2"` -> `"0.3"` (`config.rs` uses
  `Verifier::signed_url`/`SignedUrlKeySet`, added in 0.2.1).

## What changed

- MSRV raised to **1.95.0** (issue #949) — the workspace-wide move that
  removes the separate MSRV lane `webrtc-runtime`'s optional `media` feature
  previously required.

## Migration

- Update any direct caller of `source::ts_udp::recv_and_feed` to use the
  returned byte count (or discard it) instead of `()`.
- Update any `match` on `source::srt::StreamStatus` or
  `source::ts_http::StreamStatus` to bind the payload on `Fed(bytes)`.
- Bump the four dependency floors above if you pin them explicitly.
- No action needed for `InputSpec`/`OutputKind` — both were already
  `#[non_exhaustive]`.
