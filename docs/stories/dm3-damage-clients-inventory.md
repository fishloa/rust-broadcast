# Damage report inventory — the pull clients, the origin and what feeds them

Part A of #4191 (lane dm3). Every place a reader tolerates, defaults, skips or
drops content without publishing a `broadcast_core::damage::Damage`, with what
the caller is told today. Sections marked **convert** become a `Damage`;
sections marked **terminal** stay an error, with the reason playback cannot
continue.

The report channel is each crate's existing output enum, gaining one
`Damage`-carrying variant, plus a caller `DamagePolicy` on its configuration.
No callback is stored in a reader.

## hls-runtime (client)

Channel today: `Output` (`src/client/output.rs:217`) plus a bounded
`next_error` queue; `Action` (`src/client/action.rs:162`) carries requests.

**convert**

- `src/client/engine.rs:2317` — every `DemuxEvent::InputDegraded` while the
  init segment is read is dropped; `:2420-2425` counts only
  `MalformedUnit`. The demuxer's own `DemuxEvent::Damage` is discarded by the
  `_ => {}` at `:2424` and `:2589-2595` (classic TS path). No report today.
- `src/client/engine.rs:976-1004` — a closed segment skipped for four reasons
  (below the lag floor, unusable `EXT-X-MAP`, per-segment key-format error,
  unsupported encryption) is marked delivered and lost; only `first_error`
  survives, so N skips become one error and no count. The lag-floor skip
  (`:978-981`) is fully silent.
- `src/client/engine.rs:2909-2957` — no media-sequence-gap detection: a reload
  jumping from `msn=5..7` to `msn=20..22` loses 13 segments with no report.
- `src/client/engine.rs:2004`, `:2649-2656` — a discontinuity inferred from a
  URI/duration/byte-range/PDT/sequence mismatch is a bare
  `Output::Discontinuity`, with no reason and no extent. `:1224` does the same
  for a caller-abandoned resource.
- `src/client/engine.rs:2429` — samples whose track is not in the spec set
  (CENC tracks) are dropped; `TrackSkipped` fires once per track per init.
- `src/client/engine.rs:1476-1481` — bytes for a pruned init slot are accepted
  and dropped; `on_resource` returns success.
- `src/client/engine.rs:187-208` — an `#EXT-X-MAP`/`#EXT-X-KEY` probe that
  fails to parse is skipped by `if let Ok(..)`; the previous key stays in
  effect.
- `src/client/engine.rs:1042` — a preload-hint part whose URI does not resolve
  is dropped with no error and no report.
- `src/client/engine.rs:1984-1990` — `merge_delta` keeps only carried
  `EXT-X-KEY` tags of an elided prefix; other segment-scoped tags are lost.
- `broadcast-hls` diagnostics (`ParseDiagnostic`: `DateRangeWithoutStartDate`,
  `DanglingSegmentTags`, `InvalidProgramDateTime`) are parsed and never read;
  a missing PDT blinds join timing.
- `src/client/engine.rs:2016-2050` — `#EXT-X-GAP` is never read: the segment
  is fetched, gets `NotMedia`/`Demux`, and blocks `EndOfStream`.

**terminal**

- `src/client/error.rs:15` `PlaylistNotUtf8`, `:20` `PlaylistParse` — the
  playlist is the only thing that names the media; nothing can be played.
- `src/client/error.rs:27` `PlaylistTooLarge` — a caller limit.
- `src/client/error.rs:161` `UnsupportedDefine` — the URI template cannot be
  resolved, so segment addresses are unknown.
- `src/client/error.rs:227` `UnsupportedEncryption` for the whole playlist's
  key set — nothing decryptable.

## dash-runtime

Channel today: `Output` (`src/client/output.rs:166`), `Notice`
(`:112`), `DropReason`/`DropCause`, `FailureClass`.

**convert**

- `src/client/demux.rs:63-72` — the `_ => {}` swallows `DemuxEvent::Damage`,
  `DamagedSpan`, `Discontinuity`, `GaplessSignalConflict`, `TrackRemoved`;
  `:66-70` ignores `TruncatedUnit` so a truncated segment reports nothing.
- `src/client/engine.rs:1684-1698` — a 404/410 on a short last segment is
  `Skipped { marker: false }`: no `Discontinuity`, no `Output`.
- `src/client/engine.rs:1741-1755` / `src/client/engine/live.rs:1198-1266` —
  a skipped or expired segment is a marker or an unidentified `Notice`;
  `SegmentLost` is the only identified skip.
- `src/client/engine/live.rs:1636-1656`, `:900-913` — a presentation reset or
  an abandoned provisional Period discards queued requests and inits with no
  `Output`; `:1400-1426` `remap_periods` failure escalates to a reset
  silently.
- `src/client/engine/live.rs:1602-1634` — a failed replan is discarded and the
  stale plan kept.
- `src/client/engine.rs:1286-1305` — a failed init resolution becomes an
  empty reference the client fetches as the whole BaseURL.
- `src/client/engine.rs:1312-1327` — a malformed byte range silently becomes a
  whole-body fetch.
- `src/address/timeline.rs:143-151` — an `S@t`/`S@n` gap is accepted as a run
  boundary with no report; `:125-133` silently shortens a cut run's tail.
- `src/address/plan.rs:367-373`, `src/address/list.rs:57-64` — the last
  segment's declared duration is silently clamped to the Period end.
- `src/client/engine.rs:589-599` — a refresh MPD that fails to parse leaves the
  old one in force; the caller gets an error but no `Output`.
- `src/client/engine.rs:1089-1098` — `Notice::UnsupportedEncryption` and
  `Notice::XlinkSkipped` per Period; `:1078-1088` one `TrackDropped` per
  dropped AdaptationSet.
- `src/client/engine.rs:1026-1039` — an unresolvable/cross-origin URL is
  **fatal for the whole presentation**; another Representation could play.
  Convert to `TrackDropped`/`Resync` and continue, fatal only when nothing is
  left.

**terminal**

- `src/client/engine.rs:606-611` `no_period`, `:1793-1797`
  `no_playable_representation` — nothing to play.
- `src/client/engine/live.rs:793-797` `ClockUnavailable` — a dynamic MPD's
  availability window is undecidable without a clock.
- `src/client/engine.rs:1378-1389` `SegmentFetchFailed` on a static MPD after
  `max_segment_failures` — the caller's retry budget, on a resource that
  cannot be skipped in VOD.

## smooth-runtime

Channel today: `Output` (`src/client/output.rs:81`), `Dropped`
(`:56`), `ManifestFault` (`:25`).

**convert** (`Dropped` variants become the report's typed detail)

- `src/client/demux.rs:119-124` — only `MalformedUnit`/`TruncatedUnit` set
  `damaged`; every other `InputDegradation` kind and every other
  `DemuxEvent` (`Damage`, `DamagedSpan`, `Discontinuity`, `TrackAbandoned`) is
  discarded by `_ => {}`.
- `src/client/stream.rs:114-116` — one damaged unit rejects the whole
  fragment's samples.
- `src/client/stream.rs:117` — a fragment with no `tfxd` silently falls back to
  the manifest chunk start (a timeline repair).
- `src/client/stream.rs:132-141` — a sample with no timing is shifted by zero.
- `src/client/stream.rs:81-97` — "gap at the join" and "plan length forced the
  oldest chunks out" collapse to one bare `Output::Discontinuity`.
- `src/client/engine.rs:508-518` — a stream that is neither Video nor Audio, or
  has no QualityLevel, is skipped with no report.
- `src/client/engine.rs:592-594`, `:664-688` — `Dropped::StreamLost` when a
  stream misses `max_refresh_misses` manifests or the hold budget.
- `src/client/engine.rs:420-431` — an unresolvable fragment URL is
  `Dropped::FragmentUrl`.
- `broadcast-smooth/src/manifest.rs:855-873` — absent `TimeScale`/`IsLive`
  defaulted, unknown children and a second `<Protection>` dropped, all
  unreported (an extension or a defect the session can continue past).

**terminal**

- `src/client/engine.rs:265-267` `<Protection>` — protected content this crate
  does not decrypt.
- `src/client/engine.rs:501` `NoUsableStream` — no video or audio stream with a
  quality level.

## rtsp-runtime (play path, interleaved framing)

Channel today: `ClientEvent` (`src/client.rs:104`), `ServerEvent`
(`src/server.rs:38`), header warnings in `session_warnings()`
(`src/client.rs:323`). No `broadcast-core` dependency today.

**convert**

- `src/interleaved.rs:107-110`, `:136-149` — one non-`$` byte between frames is
  `FrameError::BadMagic` and ends the pull; scanning forward to the next `0x24`
  is a resync a player can survive.
- `src/interleaved.rs:113-116` — a frame's declared length is trusted; a wrong
  one over/under-consumes and surfaces later as `BadMagic`.
- `src/client.rs:660-672` — an interleaved payload on any channel is surfaced
  with no check against the negotiated pairs.
- `src/io.rs:369-382` — one non-2xx `SETUP` aborts `start_play` although
  `PlayPlan::skipped` exists for exactly that shape.
- `src/play.rs:172-174` — a section rejected by the `select` closure appears
  nowhere in the plan.
- `src/play.rs:176-186` — a cross-authority or over-limit media section is in
  `skipped`/`over_limit` (a string list, no `Damage`).
- `src/session_header.rs:100-119` — a bad `timeout` silently becomes 60 s;
  unknown `Session` params are kept and never surfaced.
- `src/client.rs:809-825` — a malformed/absent `RTP-Info` becomes an empty
  list, indistinguishable from none.
- `src/client.rs:723-749` — a response with a missing or unknown `CSeq` is
  dropped with nothing told.
- `src/client.rs:873-877` — a server-to-client `Message::Request` is ignored.

**terminal**

- `src/framing.rs:34-42` — the inbound head cap: framing is lost.
- `src/client.rs:842`/`src/state.rs:99-104` — an RTSP state-table violation:
  the request sequence can no longer be correlated.

## rtmp-runtime (session, chunk stream)

Channel today: `ServerEvent` (`src/server.rs:219`), `ClientEvent`
(`src/client.rs:149`), `ResyncWalk`/`ResyncDecision` (`src/flv.rs:746`).

**convert**

- `src/chunk.rs:1405-1413` — a Type 0 header arriving mid-message clears the
  in-progress payload with no report.
- `src/chunk.rs:806-815` — `Abort` clears a partial message silently.
- `src/chunk.rs:1273-1284` — a Type 3 extended timestamp's value is consumed
  and ignored.
- `src/amf0.rs:322-331` — an ECMA-Array associative count is read and
  discarded; `:264-266` surplus trailing bytes are ignored.
- `src/codec.rs:4-5` — a message cut short at EOF reads as a clean EOF.
- `src/server.rs:911-913` — Audio/Video/Data before `publishing` is dropped
  with no event (a test pins today's silence); `src/client.rs:716-721` drops
  the same inbound media outside play mode.
- `src/server.rs:575-580` — unknown message types are `ServerEvent::Unsupported`
  (a report, but not a `Damage`).
- `src/server.rs:529-532`, `src/client.rs:737-741` — an unknown User Control
  event is tolerated silently.
- `src/flv.rs:79-140` — a malformed aggregate sub-message after good ones ends
  the session after the good ones were delivered.

**terminal**

- `src/chunk.rs:1196-1264` `ChunkStreamState` — the chunk stream is
  desynchronised; later chunks cannot be framed.
- `src/message.rs:260-284` `ChunkSizeOutOfRange` — framing parameters are
  outside the protocol.

## srt-runtime (delivery)

Channel today: `SocketStats` counters (`src/io.rs:475`), `TickOutcome`
(`src/tsbpd.rs:101`), `recv() -> Result<Option<Bytes>>` (`src/io.rs:1067`).
No event enum.

**convert** (coalesced per contiguous sequence run)

- `src/tsbpd.rs:389-404`, `:497-511` — a too-late packet is dropped; the
  caller sees only `too_late_dropped`.
- `src/tsbpd.rs:534-577` — TLPKTDROP walks each missing sequence one at a time
  into `dropped`; a gap wider than `MAX_TLPKT_SKIP_SPAN` (`:553-577`) silently
  waits.
- `src/tsbpd.rs:580-608` — a DROPREQ range discards buffered packets and jumps
  the cursor; a malformed range (`:603-608`) is ignored.
- `src/tsbpd.rs:615-624` — a DROPREQ range beyond `MAX_SKIPS` is dropped from
  `pending_skips`.
- `src/tsbpd.rs:266` — a play time before the epoch is clamped to zero.
- `src/io.rs:1900-1903` — a packet outside the flow window is returned and
  discarded with no counter at all.
- `src/io.rs:1845-1866` — an encrypted DATA packet with no installed SEK, or an
  AES-CTR failure, is dropped with no counter.
- `src/io.rs:1954` — a peer's `PeerError` control packet is discarded.
- `src/tsbpd.rs:215` — a negotiated delay below 120 ms is silently raised.
- `src/arq/receiver.rs:452-461` — Full ACK carries fabricated zero rate
  fields.

**terminal**

- handshake rejections (`src/caller.rs:296-340`) — the peer cannot be
  connected to.
- `src/io.rs:735-801` — `HandshakeTimedOut` after the retry budget.

## media-plane (ingest and trunk)

Channel today: `SessionEvent` (`src/ingress.rs:410`) with `Damage` at `:542`
already carried from the TS demux; `IngestDriver::drain` (`:1107`) counts it
and throws the payload away (`:1158-1162`).

**convert**

- `src/ingress.rs:1158-1162` — `SessionEvent::Damage` is counted only; the
  driver keeps no payload and gives the host no way to observe it per report.
- `src/ingress.rs:1143-1155` — a sample or `TracksChanged` for an unannounced
  program is dropped with no counter and no report.
- `src/trunk.rs:3253` — `TracksLagged` means unread samples were dropped; told
  only to the cursor, never through the session channel.
- `src/trunk.rs:3177-3201`, `:3383`, `:3500` — a lagged cursor is in-band to
  the cursor, but the ingest driver and `multimux` see only a counter.
- `src/segmenter/program.rs:1385-1389` — the segmenter logs a lagged timed ring
  and carries on.
- `src/file_source.rs:680-688`, `src/file_stream.rs:663` — `DemuxEvent::Damage`
  is discarded by `_ => {}` in both file paths; `file_stream.rs:691-697` drops
  samples with no timing.
- `src/publish_queue.rs:108` — a non-monotonic sequence number is logged and
  dropped.
- `src/ts_session.rs:332-347` — a pre-resolution backlog overflow drops the
  oldest sample, told only through an optional observer
  (`ts_session.rs:165`).

**terminal** (unchanged)

- `src/trunk.rs:2721` `NonMonotonicSequenceNumber` — the segment index would be
  corrupted.

## multimux (thin)

- A route's metrics and logs see no `Damage`: `report_driver_progress`
  (`src/source/mod.rs:272`) reads only programs and trunks; the existing
  `TS_BACKLOG_DROPPED_TOTAL` (`src/prometheus.rs:151`) counts backlog drops
  alone.
- Per-route policy: `Route` (`src/config.rs:1666`) has no `DamagePolicy`; the
  input kind token set is `input_kind_name` (`src/origin/admin.rs:1151`).

## broadcast-client (every source and the session)

- `Event::Damage` exists (`src/events.rs:993`) and only the TS-over-HTTP source
  constructs it (`src/source/ts_http.rs:487`); HLS, DASH, Smooth, RTSP, RTMP
  and SRT publish nothing.
- The per-source adapters each collapse a crate-level report to one
  `ErrorKind` (`src/source/hls.rs:1078`, `src/source/dash.rs:1278`), so the
  location, count and cause are lost.
- `Config` (`src/events.rs:1187`) carries `damage_policy` only inside
  `tshttp::Config` (`src/tshttp/config.rs:54`), so a caller cannot set one
  policy for every source.
