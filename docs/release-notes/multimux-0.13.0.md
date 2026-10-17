---
status: DRAFT
---
# multimux 0.13.0

_Released 2026-10-16._

**Breaking release: several behaviour changes of its own, plus the `broadcast-common` 10 epoch cascade.** The origin gets real WHIP/WHEP session resources (per-session `Location`, `DELETE`, Trickle-ICE `PATCH` that now requires `If-Match`, ICE restart, CORS for the session resource), a secure-by-default ingest policy (`Config::allow_open_ingest`), origin-bound pull credentials, a corrected DASH `availabilityStartTime`/`SegmentTimeline` model, CSPRNG-drawn ICE credentials and session ids, and typed ids throughout. It also fixes three ingest data-loss bugs that predate this release: the first access units of every track at TS stream start (replay backlog now held until the programme is announced; overflow is counted in `multimux_ts_backlog_dropped_total`), the last access unit of every track at end of input (TS/FLV file reader, `ts_udp`/`ts_http`/`srt`, and the RTP depacketisers behind `rtp_udp`/`rtsp`/`whip`), and unstable `ProgramId`s on multi-program transport streams (now assigned in ascending `program_number` order). Read the migration list: several of these change behaviour for existing deployments.

## Why this version

- **Proposed version:** 0.12.0 -> **0.13.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (`Config::allow_open_ingest`, WHIP/WHEP `Location` resources and mandatory `If-Match` on trickle `PATCH`, origin-bound pull credentials, DASH `availabilityStartTime`, stable `ProgramId`s, typed ids).
- **cargo-semver-checks vs live 0.12.0 (all features):** MAJOR (2 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** in addition to its own breaks, it builds against new caret epochs of 18 workspace siblings (`broadcast-common` 9 -> 10 and the rest listed below); under the epoch-purity rule that is major-class on its own. Siblings it depends on that move epoch in this wave: broadcast-auth, broadcast-common, broadcast-hls, container-probe, dvb-conformance, dvb-si, hls-runtime, media-doctor, media-plane, mpeg-ts, rtmp-runtime, rtp-packet, rtsp-runtime, scte35-splice, srt-runtime, timed-metadata, transmux, webrtc-runtime.
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

### 5. Trickle `PATCH` requires `If-Match`; session resource CORS (#1267)

A WHIP trickle `PATCH` without `If-Match` is now `428 Precondition Required` (it was `204`): the session supports ICE restarts, so it always issues an `ETag` and RFC 9725 sections 4.3.1/4.3.2 make entity-tags mandatory. Send the `ETag` of the `201` (or of the last restart `200`):

```
PATCH /whip/session/<id>
Content-Type: application/trickle-ice-sdpfrag
If-Match: "<etag from the 201 or the last restart 200>"
```

The session resource now answers a preflight `OPTIONS` with `204` (it was `405`) and `Access-Control-Expose-Headers` is `Location, ETag`, so a cross-origin browser can `DELETE` and trickle. An ICE-restart `PATCH` the transport cannot apply (an empty `a=ice-ufrag`/`a=ice-pwd`) is `400` with the existing session and `ETag` kept; a `POST` whose admit channel is closed (route shutting down) is `503` with no resource registered. Known trade-off, documented rather than changed: `WhipClient`/`WhepPlayer` do not send the bearer token to a `Location` on another origin (origin-scoped credentials, #1142), so a deployment whose session URLs live on another origin has to host them on the endpoint's origin; an explicit opt-in allow-list is left for a follow-up.

### 6. `ProgramId`s are stable on multi-program TS

`ProgramTracker` (the TS ingest session under `ts_udp`/`ts_http`/`srt`) minted `ProgramId`s by iterating a `HashMap`, so on a multi-program stream the id of each `program_number`, the `NewProgram` order and the mid-stream `TracksChanged` order were random per process (the same bug exists in 0.12.0). Programs resolved together now get `ProgramId(0..)` in ascending `program_number` order, `TracksChanged` is emitted in `ProgramId` order with tracks sorted by `track_id`, and a program's URL is stable across restarts; a program first seen mid-stream takes the next id in arrival order. Anything that hard-coded a `ProgramId` observed from an earlier run must re-read it. `RouteHandle`'s per-program map and `IngestDriver::programs()` also iterate in `ProgramId` order.

### 7. Typed ids

`route_channel` returns `Option<TrackId>`, `TrackInit::track_id` is `TrackId`, `DashWindowSegment::segment_seq` is a `SegmentSeq`, `TsProgramTracker` keys programs by `ProgramNumber`. The on-disk DVR index (`IndexEntry::seq`) stays a `u32` with an unchanged JSON format. `rtsp-runtime`'s caret epoch moves 0.7 -> 0.8 (see its note): `ClientSession::handle_data` now returns a `HandleOutcome`, and an RTSP interleaved frame whose RTP payload does not parse no longer ends the ingest session (it is skipped and counted in `RtspIngestSession::malformed_media_frames`).

```rust
// before
let seq: u32 = window_segment.segment_seq;    let t: u32 = init.track_id;
// after
let seq: SegmentSeq = window_segment.segment_seq;    let t: TrackId = init.track_id;
```

### No migration needed, but worth knowing

- TS ingest no longer drops the first access units of each track at stream start; they are held and flushed in arrival order right after `NewProgram` (bounded at 65,536 samples or 64 MiB, overflow counted in `multimux_ts_backlog_dropped_total`).
- End of input now flushes: `TsIngestSession::finish`, the file reader's TS and FLV paths, and the RTP depacketisers (`RtpUdpIngestSession`, `RtspIngestSession`, `WhipIngestSession`) emit the final access unit of every track and announce a programme whose declared PID never carried data. Previously the last frame of each track was lost.
- Push egress (RTMP/SRT/RTSP) no longer starts a (re)connected stream mid-GOP: samples are dropped until the first video sync sample after connect.
- DASH pull: a negative `S@r` with a non-zero `@presentationTimeOffset` now resolves (it gave zero segments), and a timeline refresh that restarts lower than the previous high-water mark is re-baselined instead of stalling the route.
- ICE `ufrag`/`pwd`, session ids, `ETag` tokens and RTP SSRCs come from the OS CSPRNG; `Debug` of RTMP/RTSP transport configs no longer prints stream keys or credentials.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `broadcast-auth` 0.4 -> 0.5, `broadcast-hls` 0.3 -> 0.4, `container-probe` 0.1 -> 0.2, `dvb-conformance` 11 -> 12, `dvb-si` 11 -> 12, `hls-runtime` 0.8 -> 0.9, `media-doctor` 0.10 -> 0.11, `media-plane` 0.6 -> 0.7, `mpeg-ts` 0.5 -> 0.6, `rtmp-runtime` 0.7 -> 0.8, `rtp-packet` 0.4 -> 0.5, `rtsp-runtime` 0.7 -> 0.8, `scte35-splice` 3 -> 4, `srt-runtime` 0.5 -> 0.6, `timed-metadata` 0.6 -> 0.7, `transmux` 0.26 -> 0.27, `webrtc-runtime` 0.3 -> 0.4). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Fixed (determinism of multi-program ingest)

- **Behaviour change (ProgramIds are now stable):** `ProgramTracker` (the TS ingest session under `ts_udp`/`ts_http`/`srt`) minted `ProgramId`s by iterating a `HashMap`, so on a multi-program transport stream the `ProgramId` of each `program_number`, the `NewProgram` announce order and (mid-stream) the `TracksChanged` order were random per process (the same bug exists in `multimux-v0.12.0`, not a later regression). Programs resolved together are now assigned `ProgramId(0..)` in ascending `program_number` order, and `TracksChanged` is emitted in `ProgramId` order with tracks sorted by `track_id`; a program's URL is stable across restarts. A program first seen mid-stream still takes the next id in arrival order. Any consumer that hard-coded a `ProgramId` observed from a previous random run must re-read it.
- `RouteHandle`'s per-program map and `media_plane::ingress::IngestDriver::programs()` iterate in `ProgramId` order (they were `HashMap` iteration), so per-program drive/DVR-drain order is reproducible; the config-reload diff reports removed routes in name order.

#### Fixed (end-of-input flush of RTP depacketisers)

- `RtpUdpIngestSession::finish`, `RtspIngestSession::finish` and `WhipIngestSession::finish` were no-ops, but `RtpStreamDepacketiser` always holds the last access unit (the one-AU-late `pending` sample, and any AU whose marker bit never arrived). They now call `RtpStreamDepacketiser::flush` per track at end of input (a pending AU is complete by definition at end of stream); the last frame of every track is no longer lost. Idempotent.

#### Fixed (end-of-input demux flush)

- `TsIngestSession::finish` (the session under `ts_udp`/`ts_http`/`srt`) was a no-op, so it never called `StreamingTsDemux::finish()`. On a capture whose declared PID never carries data (the private `france-tnt-m6-eac3-zenith.ts`: PID 0x8D has 0 PES) `TracksResolved` never fires, and nothing was announced at end of input: no `NewProgram`, no tracks, and every probing/parked track's backlog plus each live track's held-back final access unit were lost (present since at least multimux 0.12.0, which has the same no-op). `finish()` now flushes the demux and translates the events it emits, so the programme is announced and per-track samples equal the raw demux output after its own `finish()`. A second `finish()` queues nothing.
- The file reader's TS and FLV paths (`demux_streaming_ts`/`demux_streaming_flv`) fed the whole file but never called `finish()`: the last access unit of every track, and any track a silent declared PID kept unresolved, were dropped from the published timeline (and from each loop pass). Both now flush at end of input; the reader's "source's own sample set" test helpers include the flush too.

#### Fixed (stream-start data loss in TS ingest)

- The TS ingest session (`ts_udp`/`ts_http`/`srt`) no longer drops the first access units of every track at stream start. `transmux::StreamingTsDemux` replays a track's probe backlog as `Sample` events the moment the track is promoted, which is before `TracksResolved`, the signal the programme is announced on; the `track_program` guard dropped all of them (present since at least multimux 0.12.0 / transmux 0.26.0). Samples of an already-announced track are now held and flushed in arrival order immediately after `NewProgram`, ahead of any later sample (no loss, no duplicate, no reordering; a sample never precedes its programme's announcement). The hold is bounded at 65,536 samples or 64 MiB; on overflow (a head track that never resolves while others flow) the oldest held samples are dropped and counted in the new `multimux_ts_backlog_dropped_total` counter. Samples for a track the demux never announced are still dropped. Pinned by a synthetic stalled-head stream and the real France TNT capture (per-track bytes, timestamps and order equal to the raw demux output) plus unit tests for ordering and both bounds.

#### Fixed (adversarial review of the WHIP/WHEP/DASH-pull runtimes, #1267)

- **BREAKING (behaviour):** a WHIP trickle `PATCH` without `If-Match` is now `428 Precondition Required` (RFC 9725 sections 4.3.1/4.3.2: the session supports ICE restarts, so it always issues an `ETag` and entity-tags are mandatory for Trickle ICE); before it was `204`. Clients must send the `ETag` of the `201` (or of the last restart `200`).
- CORS: the session resource (`/whip/session/{id}`, `/whep/session/{id}`) now answers a preflight `OPTIONS` `204` (it was `405`, so a cross-origin browser could never `DELETE` or trickle), and `Access-Control-Expose-Headers` is `Location, ETag` (the `ETag` is not CORS-safelisted, so a browser could not read it).
- An ICE-restart `PATCH` the transport cannot apply (an empty `a=ice-ufrag`/`a=ice-pwd`) is a `400` with the existing ICE session and `ETag` kept (RFC 9725 section 4.3.3), instead of a `200` with new credentials the transport then refused (client and transport diverged). Applies to WHIP and WHEP.
- WHIP/WHEP `POST` whose admit channel is closed (route shutting down) is `503` with the resource unregistered, instead of a `201` + `Location` for a session nothing would ever run.
- DASH pull: a negative `S@r` (run to the Period end) with a non-zero `@presentationTimeOffset` resolved to zero segments because the Period end was not shifted by the offset while `S@t` is on the media timeline (ISO/IEC 23009-1 section 5.3.9.6; DVB-DASH MPDs carry a large PTO). A timeline refresh that restarts LOWER than the previous high-water mark (a new Period, an encoder restart) is re-baselined instead of being filtered to nothing, which stalled the route.
- Known tradeoff (documented, not changed): `WhipClient`/`WhepPlayer` (`webrtc-runtime`) do not send the bearer token to a `Location` on another origin (#1142, origin-scoped credentials: a hostile or compromised endpoint must not be able to redirect the token to a host of its choosing). RFC 9725 section 4.7.1 says the token goes to the session resource; a deployment whose session URLs live on another origin has to host them on the endpoint's origin. An explicit opt-in allow-list is left for a follow-up.

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

- fix(multimux): deterministic ProgramIds on MPTS; flush RTP depacketisers at end of input (#1332)
- fix(multimux): flush the TS/FLV demux at end of input (#1330)
- fix(multimux): stop dropping samples replayed before TracksResolved (#1327)
- fix(transmux): round-3 audit of StreamingTsDemux head-of-line, E-AC-3 hold-back and probe work (#1326)
- docs: clear per-crate documentation for cluster B (16 crates) (#1291)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- chore: pre-release hygiene (README coverage, snippet tool, docsrs, examples, CLAUDE.md) (#1278)
- Fix the confirmed WHIP/WHEP/DASH-pull review findings (supersedes #1267) (#1271)
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
