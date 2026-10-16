---
status: DRAFT
---
# multimux 0.13.0

_Released 2026-10-16._

**Breaking release.** The origin gets real WHIP/WHEP session resources (per-session `Location`, `DELETE`, Trickle-ICE `PATCH`, ICE restart), a secure-by-default ingest policy (`Config::allow_open_ingest`), origin-bound pull credentials, a corrected DASH `availabilityStartTime`/`SegmentTimeline` model, CSPRNG-drawn ICE credentials, and typed ids throughout. Several of these change behaviour for existing deployments: read the migration list.

## Why this version

- **Proposed version:** 0.12.0 -> **0.13.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (Config::allow_open_ingest, WHIP/WHEP Location resources, typed ids).
- **cargo-semver-checks vs live 0.12.0 (all features):** MAJOR (2 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-auth, broadcast-common, broadcast-hls, container-probe, dvb-conformance, dvb-si, hls-runtime, media-doctor, media-plane, mpeg-ts, rtmp-runtime, rtp-packet, rtsp-runtime, scte35-splice, srt-runtime, timed-metadata, transmux, webrtc-runtime.
- **Caret epoch:** 0.12 -> 0.13; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-auth = "0.4"
+broadcast-auth = "0.5"
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-broadcast-hls = "0.3"
+broadcast-hls = "0.4"
-container-probe = "0.1"
+container-probe = "0.2"
-dvb-si = "11.1"
+dvb-si = "12.0"
-hls-runtime = "0.8"
+hls-runtime = "0.9"
-media-plane = "0.6"
+media-plane = "0.7"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
-rtmp-runtime = "0.7"
+rtmp-runtime = "0.8"
-rtp-packet = "0.4"
+rtp-packet = "0.5"
-rtsp-runtime = "0.7"
+rtsp-runtime = "0.8"
-srt-runtime = "0.5"
+srt-runtime = "0.6"
-timed-metadata = "0.6"
+timed-metadata = "0.7"
-transmux = "0.26"
+transmux = "0.27"
-webrtc-runtime = "0.3"
+webrtc-runtime = "0.4"
-dvb-conformance = "11.1" # dev-dependency
+dvb-conformance = "12.0" # dev-dependency
-media-doctor = "0.10" # dev-dependency
+media-doctor = "0.11" # dev-dependency
-rtp-packet = "0.4" # dev-dependency
+rtp-packet = "0.5" # dev-dependency
-scte35-splice = "3.0" # dev-dependency
+scte35-splice = "4.0" # dev-dependency
-webrtc-runtime = "0.3" # dev-dependency
+webrtc-runtime = "0.4" # dev-dependency
```

## Breaking changes and migration

### 1. WHIP/WHEP `Location` is a per-session resource (#1149)

`201 Created` now returns `/whip/session/<id>` / `/whep/session/<id>` (a 32-character unguessable id, RFC 9725 section 8) instead of the constant `/whip/session`; the id-less paths no longer exist (`404`). `DELETE` on the resource ends the session (`200`); a repeat is `404`. `PATCH` carries Trickle ICE (`application/trickle-ice-sdpfrag`, optional `If-Match`; `415` wrong type, `412` wrong entity-tag, `400` bad fragment); `If-Match: *` is an ICE restart answered `200` with a new `ETag`. **Any client that hard-coded `/whip/session` must follow the `Location` header.** The WHEP `201` now carries the `ETag` too.

### 2. Open ingest must be opted into (#1142)

```json
// before: an RTMP route without stream_key, a WHIP route or a listener-mode SRT route on 0.0.0.0 was accepted
// after: Config::validate refuses it unless the config says so
{ "allow_open_ingest": true }
```

Loopback listeners, RTMP routes with a `stream_key` and caller-mode SRT are unaffected. In Rust, `Config` literals must add the field (or use `..Default::default()`).

### 3. Pull credentials are bound to the source origin

HLS/DASH/Smooth pull and TS-over-HTTP credentials are sent only to the origin (scheme, host, port) of the configured source URL; an absolute playlist URL on another host or a redirect target gets no `Authorization`, and a Digest challenge from a redirect target is not answered.

### 4. DASH `availabilityStartTime` and live timeline (#1187)

`MPD@availabilityStartTime` is the program's presentation origin (the instant its `Trunk` was minted, `RouteHandle::presentation_origin(program)`), no longer the route's construction time; before a program is published the MPD is `503`, not a made-up AST. `render_mpd_at`/`render_ll_dash_mpd_at` take the origin explicitly. DASH pull extends a `$Time$` SegmentTimeline refresh by time, and resolves a negative `S@r` via `transmux::SegmentTimeline::resolve`. `output::dash::xs_duration_for_test` is removed (use `transmux::dash::xs_duration`).

### 5. Typed ids

`route_channel` returns `Option<TrackId>`, `TrackInit::track_id` is `TrackId`, `DashWindowSegment::segment_seq` is a `SegmentSeq`, `TsProgramTracker` keys programs by `ProgramNumber`. The on-disk DVR index (`IndexEntry::seq`) stays a `u32` with an unchanged JSON format. `rtsp-runtime`'s caret epoch moves 0.7 -> 0.8 (see its note).

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed (BREAKING)
- `output::dash::xs_duration_for_test` (a `#[doc(hidden)]` test seam) is removed; the `xs:duration` spelling is `transmux::dash::xs_duration`, the one shared implementation. For a duration beyond `i64::MAX` seconds (~292 billion years) the manifest value is now the saturated maximum instead of `PT{secs}S` (#1216).

#### Fixed (DASH timeline + availabilityStartTime, #1187)

- **BREAKING (behaviour):** the DASH/LL-DASH output's `MPD@availabilityStartTime` is the program's presentation origin (the instant its `Trunk` was minted, new `RouteHandle::presentation_origin(program)`), no longer the route's construction time. The route is built before ingest connects and does not move when a reconnect mints a fresh `Trunk` that restarts at segment 1, so the old value broke `AST + (N - startNumber) * duration` (ISO/IEC 23009-1 §5.3.1.2, §5.3.9.5.3). Before a program is published the MPD is not served (503) instead of carrying a made-up AST. `render_mpd_at`/`render_ll_dash_mpd_at` take the origin explicitly.
- DASH pull: a live `SegmentTimeline` (`$Time$`) MPD refresh is extended by **time** (`S@t` is absolute; the window slides and the enumerated numbers restart), not by `$Number$`; before, a refreshed `$Time$` MPD with a constant `@startNumber` never added another segment and the route stalled. A negative `S@r` is resolved to the next `@t`/Period end (via `transmux::SegmentTimeline::resolve`) instead of one segment. Tests use real DASH-IF livesim2 MPDs (`fixtures/dash/livesim2/`, BSD-3-Clause).

#### Security (secrets and identity handling, #1142)

- **BREAKING:** `Config::allow_open_ingest` (default `false`). `Config::validate` now refuses an ingest listener that admits a publisher with no authentication (an RTMP route without `stream_key`, a WHIP route, a listener-mode SRT route) on a non-loopback address unless the config opts in with `"allow_open_ingest": true`. Loopback listeners, RTMP routes with a `stream_key` and caller-mode SRT are unaffected. `Config` struct literals must add the field (or use `..Default::default()`).
- **BREAKING (behaviour):** HLS/DASH/Smooth pull and TS-over-HTTP credentials are bound to the origin (scheme, host, port) of the configured source URL. A request to any other origin (an absolute playlist/manifest URL on another host, a redirect target) is sent without `Authorization`, and a Digest challenge from a redirect target is not answered.
- ICE `ufrag`/`pwd`, WHIP/WHEP session resource ids, `ETag` tokens and RTP SSRCs are drawn from the OS CSPRNG (`getrandom`) instead of `RandomState` hashing.
- `Debug` of `RtmpTransportConfig` (stream key), `RtspTransportConfig` (credentials) and the internal ICE restart/fragment types no longer prints the secret. New `tests/secret_debug_guard.rs` and `tests/secret_hygiene.rs`.

#### Changed

- **BREAKING (#1188):** follows `media_plane::ids::{SegmentSeq, PartIndex}`: `DashWindowSegment::segment_seq` is a `SegmentSeq`, and the DVR archive reader/recorder (`archive_seq_floor`, `needs_poll`) and catch-up internals are typed. `latest_progress` no longer overflows (`last_closed + 1` panicked in debug at `u32::MAX`; now `saturating_next`). The on-disk DVR index (`IndexEntry::seq`) stays a plain `u32`, its JSON format unchanged.

- **BREAKING (#1159):** follows `transmux::TrackId` (`route_channel` returns `Option<TrackId>`, `TrackInit::track_id`, the per-source track maps) and `hls_runtime::server::Generation`; `TsProgramTracker` keys programs by `ProgramNumber`.
#### Changed (BREAKING)
- **WHIP/WHEP session resources are real.** The `201 Created` `Location` is now a per-session resource, `/whip/session/<id>` / `/whep/session/<id>` (a 32-character unguessable id, RFC 9725 section 8), instead of the constant `/whip/session` / `/whep/session`; the id-less paths no longer exist (`404`). `DELETE` on the resource ends the session (`200`, RFC 9725 section 4.2 / draft-ietf-wish-whep section 5): the media task drops the transport (WHIP also releases the published program), the `Trunk` cursor (WHEP) and the capacity slot, and a repeated `DELETE` (or one for a session that already ended by idle reap / error) is `404`. `PATCH` carries Trickle ICE (RFC 9725 section 4.3): `application/trickle-ice-sdpfrag` with an optional `If-Match` adds the fragment's candidates to the live transport (`204`); a wrong `Content-Type` is `415`, an `If-Match` naming another entity-tag `412`, a fragment for another ICE generation / with nothing to add / unparsable `400`, an unknown resource `404`,. `If-Match: *` is an **ICE restart** (RFC 9725 section 4.3.2): the PATCH carries the client's new `ice-ufrag`/`ice-pwd` (and candidates); the server restarts ICE in place under new credentials of its own (DTLS/SRTP untouched, media keeps flowing), and answers `200` with a new `ETag` and an `application/trickle-ice-sdpfrag` body holding its new credentials and candidates. A restart fragment that keeps the current ufrag or lacks a pwd is `400`; after a restart the old `ETag` is stale (`412`). Previously both methods answered `405` (#1149). Any client that hard-coded `/whip/session` instead of following `Location` must follow the header. The WHEP `201` now also carries the `ETag` the state machine owns (as WHIP's always did).

#### Fixed
- Push (RTMP/SRT/RTSP) no longer starts a (re)connected stream mid-GOP (audit r07-W17, #1083): `PushTransportEgress` drops every sample until the first video sync sample after the connect (no gating when the selection has no video track).
- WHIP: an overdue transport (ICE/DTLS) timer could be postponed indefinitely by a datagram flood with sub-millisecond gaps, because the floor was re-measured from "now" on every datagram. The per-session last fire is now carried across reads and an already-overdue deadline is serviced before the next datagram is read (at most one datagram is handled per loop turn). Pinned by a paused-clock test with a 2 kHz flood (#1150).

#### Added
- `whip_session_resource` / `whep_session_resource` integration tests (real HTTP against the real routes, including a real ICE + DTLS loopback peer for WHIP), and `#[doc(hidden)]` test seams for them.

#### Fixed
- An RTSP interleaved frame whose RTP payload does not parse no longer ends the ingest session (audit T16, #1144): it is skipped, logged and counted (`RtspIngestSession::malformed_media_frames`), and the frames after it are still depayloaded. Events decoded before a terminal RTSP protocol error are now handled before the error is raised (rtsp-runtime `HandleOutcome`).

#### Changed
- Adapted to rtsp-runtime's breaking `ClientSession::handle_data` return type (`HandleOutcome`); caret epoch of `rtsp-runtime` moves 0.7 -> 0.8.

## Merged changes since `multimux-v0.12.0`

- tests: one fixture resolver (public -> private/fixtures -> .test-streams); move local-only captures to the private repo (#1259) (#1260)
- transmux: public dash::xs_duration; multimux uses it, copy and test seam removed (#1216) (#1217)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- media-plane: SegmentSeq/PartIndex newtypes across media-plane, hls-runtime, multimux (BREAKING) (#1194)
- transmux: subtitle sample entry re-mux, E-AC-3 dependent substreams across PES, PPS A->B->A (#1186) (#1192)
- DASH: SegmentTimeline absolute t / negative r, AST = presentation origin (#1191)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
- Audit systemic wave 2: T16 resync, largesize, ttml drop, WHIP/WHEP DELETE/PATCH/ICE restart, mpeg-ts/pes reserved+stuffing preservation, real-capture guards (NO release) (#1175)
