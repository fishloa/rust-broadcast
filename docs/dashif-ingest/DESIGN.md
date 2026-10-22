# DASH-IF Live Media Ingest — receiver design (Interface-1, CMAF ingest)

Status: design, 2026-10-22. Spec: DASH-IF Live Media Ingest Protocol 1.3 (transcribed: `private/specs-md/dashif_live_media_ingest_v1.3.md`; §5 common, §6 Interface-1). Scope of v1 is the **receiver** side of **Interface-1**; sender (push output) and Interface-2 (DASH/HLS ingest) are later increments.

## 1. What is being built

An ingest source — encoder or packager — pushes a CMAF presentation to us with HTTP/1.1 `POST`/`PUT` (§5.2.1). `multimux` gets a new input scheme `dashif_ingest` that is a **listener** (same family as `rtmp` and `whip`: the peer connects to us), feeding the existing `Trunk`, so every existing output (LL-HLS, DASH, Smooth, push, catch-up) works on ingested content unchanged.

Non-goals v1: TLS termination (§5.2.2/5.2.5-7: multimux is HTTP/1.1 plaintext today — terminate TLS at a reverse proxy; documented gap, not silently ignored), Interface-2, the sender, SCTE-35 splice conditioning checks (§6.7), failover orchestration across two ingest sources (§6.8/6.9: first-wins by sequence/`tfdt`, later), CENC (§6.3.1: not used on this hop).

## 2. Wire model (what a request is)

One request = one object (§6.2.3/§5.2.18). Four kinds, told apart by URL path + body, never by guessing:

| Object | Detected by | Handling |
|---|---|---|
| Empty probe | zero-length body (§6.2.1) | `200`/`204` if the publishing point is valid + auth ok, else the §5.3 error; no state change |
| Manifest | extension `.mpd` / `.m3u8` (§6.2.2, 16, 17) | parsed with the existing MPD parser (`transmux::dash`/`multimux::source::dash_pull`); only its `SegmentTemplate@media/@initialization` patterns and AdaptationSet/Representation grouping are used (switching sets, §6.4); a re-sent manifest MAY be ignored (§6.2.16g, 17e) — we ignore it after the first |
| CMAF header | path matches the `SegmentTemplate@initialization` pattern (or, with no MPD, the path's final segment-name pattern); body `ftyp`+`moov` | register/replace the track; identical re-POST after an `lmsg` restarts the track (§6.2.19); a header differing beyond `elng` mid-session is a new generation |
| CMAF segment / chunk | path matches `SegmentTemplate@media` with `$Number$` or `$Time$`; body `[styp][emsg*][prft][moof][mdat]` | demux against the track's header, push samples |

Path → track: `$RepresentationID$` in the template. With no MPD (grouping trivial, §6.2.2) the track id is taken from the path via the deprecated `Streams(name)` convention or the single path component preceding the file name — chosen once at route config (`path_style`), documented, never inferred per request.

**Chunked transfer** (§6.2.4, §5.2.21): a segment arrives as an HTTP chunked body. The receiver must feed `Fmp4Demux` incrementally (box-at-a-time: `moof`+`mdat` pairs = CMAF chunks) so low-latency parts can be published before the request finishes. `lmsg` brand on `styp` ⇒ track ended; `slat` ⇒ filler segment (tagged so a later real segment may replace it; v1 passes it through and counts it).

## 3. Architecture in this workspace

```
TCP accept (hyper http1)  ->  auth (broadcast-auth Verifier, Basic/Digest/Bearer, §5.2.5)
   -> per-request body stream (tower-http RequestDecompression for gzip, §5.2.13)
   -> classify (probe / manifest / header / segment)       [pure, sans-IO: new module in multimux]
   -> IngestSession (media_plane): Established gates on first header+segment, NewProgram/TracksChanged
   -> Fmp4Demux per track -> Samples -> the same segmenter+TrunkWriter path as dash_pull
```

* **Reuse, don't re-implement:** `source::dash_pull` already ingests per-Representation `init + segment` pairs into samples; the receiver's per-track ingestion is that code with the pull loop replaced by request handlers. The only new logic is request classification, the template matcher, and the error-code mapping.
* **Sans-IO core:** classification + template matching + the §5.3 status mapping live in a sans-IO module (testable with byte fixtures); axum/hyper glue is thin. Template matching uses a real parser for `$RepresentationID$`/`$Number$`/`$Time$` substitution with `%0Nd` widths (ISO/IEC 23009-1 §5.3.9.4.4) — the existing DASH template code in `transmux::dash`; **no regexes, no `split` heuristics** (project no-hand-roll rule).
* **Concurrency:** one TCP connection per track is the norm (§5.2.20, §6.2.8): the `Trunk` writer is single-writer, so per-track requests serialise through one `IngestSession` task per publishing point (a channel), not a lock per request (writer cost is O(cursor count), unrelated).
* **Routes/config:** `InputSpec::DashifIngest { listen, publishing_point, path_style, max_object_bytes, auth }`; one listener may host several publishing points (map keyed by path prefix); config validation follows `Whip`'s `listen` validation. Name collisions with output routes are rejected like `playlist_name`.

## 4. Error behaviour (§5.3 — table the receiver must implement exactly)

| Condition | Status |
|---|---|
| auth / permission / wrong path | `403` (§5.3.4) |
| publishing point not available | `404` (§5.3.5a) |
| media type / codec unsupported | `415` (§5.3.5b) |
| segment without a usable header | `412` (§5.3.5c) |
| receiver-side fault | `5xx` (§5.3.5d) |
| anything else malformed | `400` (§5.3.5e) |

After a TCP error the source reconnects and re-POSTs the **same POST_URL** (§5.3.3); the receiver must therefore accept a header and a repeated/overlapping segment idempotently (dedupe by `mfhd` sequence + `tfdt`; §6.2.10/6.2.11 say both SHOULD increase — a regression opens a discontinuity, it is not an error).

## 5. Increments (each: transcription check → brief → Beam → audit → PR)

1. **P1 sans-IO core** — request classifier, template matcher, status mapping, MPD grouping extract; tests on real CMAF (transmux-produced fixtures + a DASH-IF/`livesim2` capture if obtainable — see `dashif-test-assets`).
2. **P2 listener + wiring** — `InputSpec::DashifIngest`, accept loop, auth, gzip request bodies, publishing-point registry, `NewProgram`/`TracksChanged`; e2e: a test POST client (this is also the seed of the sender) streams a real CMAF presentation, outputs verified with the LL-HLS/DASH checkers and Apple `mediastreamvalidator`.
3. **P3 chunked/low-latency** — incremental chunk demux, parts published before request end; `lmsg`/`slat`.
4. **P4 robustness** — reconnect/duplicate/regression matrix, `prft` capture, limits (object size, slow-body timeout §5.2.9), metrics.
5. Later: sender (push output, §6 from the other side), Interface-2, TLS, redundant sources (§6.8-6.9).

## 6. Open points (must be resolved from the spec text before P1, never guessed)

* §6.4 switching-set signalling and §6.5/6.6 timed text/metadata track formats: transcribed; P1 only needs grouping, tracks of any kind are passed to the demuxer, but `emsg`/timed-metadata track handling must be checked against §6.6 before P3.
* CMAF (ISO/IEC 23000-19) box constraints are referenced, not restated, by the DASH-IF text; the receiver only requires what `Fmp4Demux` already parses. No claim of CMAF conformance validation beyond `transmux`'s existing fMP4/CMAF validator.
* `Streams(...)` keyword (deprecated, §6.2.9) exact syntax is not defined in the transcribed text; support it only if a real sender emits it (decision deferred to P1 with a fixture).
