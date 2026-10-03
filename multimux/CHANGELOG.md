# Changelog

## [Unreleased]

### Fixed
- **A non-finite `target_duration_secs` no longer panics the route.**
  `Config::validate` accepted `NaN` and `+inf` (it only checked `<= 0.0`), and
  `ProgramServing::new` then `expect`ed `HlsOriginBuilder::build` — which
  rejects a non-finite target — so publishing the first program aborted the
  ingest task. `validate` now requires a finite positive value, and
  `RouteHandle::publish_program` logs and declines (releasing the publish slot)
  instead of panicking if the origin cannot be built.
- **`master.m3u8` carries the measured peak `BANDWIDTH` and `CODECS`** instead
  of a fixed 5 Mb/s with no `CODECS`, via
  `hls_runtime::server::HlsOrigin::master_playlist`. `CODECS` comes from the
  init segment for fMP4 routes and from the TS demux's track specs (synced
  with the DASH specs) for TS-HLS routes (#1089).
- **A reconnect continues the HLS Media Sequence Number.** `publish_program`
  binding a fresh `Trunk` (which numbers segments from 1 again) over an
  already-served program now builds the replacement `HlsOrigin` with
  `media_sequence_offset` set from the previous origin's
  `next_media_sequence()`, so `EXT-X-MEDIA-SEQUENCE` never decreases (#1089).
- **An unvalidated in-process `Config` no longer panics on an extreme
  ingest timeout** (#1083). `IngestTimeouts::from` used
  `Duration::from_secs_f64`, which panics on a non-finite or overflowing
  value; it now falls back to the default (matching `HttpLimits::from`), so an
  embedder or the runtime admin API presenting `NaN`/`1e20` fails soft instead
  of aborting the process. (Config *validation* of these fields is a
  behaviour change — see "Changed (breaking)".)
- **CORS preflight now allows `Authorization`** (#1083). The origin sent
  `Access-Control-Allow-Headers: *`, but the Fetch spec's `*` wildcard never
  covers `Authorization` — so a browser player sending Basic or Bearer output
  auth cross-origin failed the preflight. The header is now an explicit list
  (`Authorization, Range, Content-Type`).
- **Push transports bound their connect/handshake, and the SRT URL's query
  string is no longer mistaken for the address** (#1083). The RTMP push's
  `TcpStream::connect` plus C0/C1/C2 handshake and `connect`/`publish` command
  exchange had no timeout, so a server that accepted the TCP connection but
  never finished the handshake wedged the push forever (`drive_push` only
  checks its cancel flag between iterations) — the whole exchange is now
  bounded (`RTMP_CONNECT_TIMEOUT`, exposed as
  `RtmpTransportConfig::connect_timeout`). The SRT push stripped only the
  `srt://` scheme, passing a URL's `?streamid=…&latency=…` through as part of
  the dial address; the address is now `host:port` alone, and `streamid`/
  `latency` are applied to the handshake configuration.
- **Pull sources cap a response body before buffering it** (#1083). Every
  DASH/HLS/Smooth fetch read its whole body into memory, so a misconfigured
  or hostile origin answering with a multi-gigabyte segment within the read
  timeout could OOM the process. Bodies are now bounded
  (`source::MAX_HTTP_BODY_BYTES`, 64 MiB) both from a declared
  `Content-Length` and from the streamed byte count, so a chunked response
  with no length (or a lying one) is bounded too.
  A hostile MPD's `$Number$` plan is also capped
  (`dash_pull::MAX_PLAN_ENTRIES`), and every pull-source HTTP client now uses
  a redirect policy (`source::redirect_policy`) limited to 3 hops that refuses
  an `https`→`http` downgrade.
- **The Smooth-pull "encrypted" check no longer false-positives on clear
  content** (#1083). It byte-scanned the *entire* fragment — including `mdat`
  payload — for `senc`/`saiz`/`saio`, so a random four-byte run in clear
  coded media tore the route down with a false `Encrypted` (roughly once
  every 45 minutes per stream at 2 s fragments). It now walks the box tree
  and inspects only the `moof` box's `traf` children, where
  `senc`/`saiz`/`saio` (and the PIFF sample-encryption `uuid` box) can
  legitimately appear (ISO/IEC 14496-12:2015 §8.8.7/§8.8.8).
- **Live pull sources no longer stall silently** (#1083). Four liveness
  defects in the DASH/HLS/Smooth pull sources, each fixed:
  - a **live `$Number$` DASH MPD produced nothing**: its plan was sized from
    `mediaPresentationDuration`, which a live MPD does not carry, so the plan
    was empty forever and the route went Live but emitted no samples — a
    dynamic MPD with no known total now enumerates a bounded look-ahead
    window that the refresh loop extends as the live edge advances;
  - a **tolerated `404` stalled the route forever** (DASH **and** Smooth):
    the segment/fragment was retried without a bound, so the Representation
    stayed in flight and the MPD/manifest was never refreshed (an encoder
    restart that resets numbering froze the route) — the retry is now bounded
    in both sources, after which the entry is abandoned (counted in
    `multimux_pull_fragment_abandoned_total`) and the manifest refreshed;
  - a **Smooth manifest refresh matched streams by `Type` alone**: with two
    audio `StreamIndex`es the second adopted the first's URL template and
    timeline, so it fetched the wrong fragments under a non-matching track id
    — streams are now matched by their full identity (`Type` + `Name` + `Url`
    template, MS-SSTR §2.2.3);
  - **one failed HLS resource fetch ended the session**: a single `404` from
    an eviction race (or one dropped connection) tore down and reconnected
    the whole route, disrupting every viewer — a resource fetch is now
    retried (bounded) before the session is failed; a failed playlist fetch
    remains terminal.
- **Smooth Streaming fragments are per-track, and no codec is advertised
  under a fabricated FourCC** (#1083). MS-SSTR §2.2.4 defines a fragment
  response as **one track's** samples (`moof`+`tfxd`+`mdat`), and each
  `StreamIndex` addresses its own track via
  `Fragments({type}={start time})` — but the fragment handler ignored the
  requested `TYPE` and returned the same muxed program segment for every
  StreamIndex, so an audio request was answered with the video (or muxed)
  fragment. The muxed segment is now demuxed and re-packaged into the
  requested StreamIndex's own track (via the crate's own `Fmp4Demux` +
  `SmoothPackager`) before it is served. Separately, a track whose codec
  Smooth cannot describe was advertised as `FourCC="H264"` with an empty
  `CodecPrivateData`, telling a client to initialise an H.264 decoder it
  could never use; such a track is now omitted from the manifest (a request
  for it 404s) and logged, never silently mislabelled.
- **A catch-up playlist that can lose leading segments no longer claims
  `EXT-X-PLAYLIST-TYPE`** (#1083). `GET /catchup.m3u8` rendered
  `PLAYLIST-TYPE:EVENT` unconditionally, but a `window_secs` bound
  (`apply_window`) or DVR retention (which evicts whole leading periods)
  both remove segments — RFC 8216 §6.2.2 is explicit that "A Live Playlist
  MUST NOT contain the EXT-X-PLAYLIST-TYPE tag, as no value of that tag
  allows Media Segments to be removed", and §6.2.1 forbids an `EVENT`
  playlist from changing or deleting any part. The tag is now omitted
  whenever either is active (retention is required by `DvrConfig::validate`,
  so in practice the live catch-up playlist never claims `EVENT`), and kept
  for the still-growing single-period `/vod/p{N}.m3u8`, whose own segment
  list only ever grows. An empty rendered playlist also declares
  `#EXT-X-MEDIA-SEQUENCE:0` now (RFC 8216 §4.3.3.2) instead of reusing the
  unrelated `#EXT-X-TARGETDURATION` floor.
- **A `ts_hls` route's `.ts` segments are served as `video/mp2t`, not
  `video/mp4`** (#1083). The shared resource route hard-coded `video/mp4`
  for every body, so a classic TS-HLS segment (a `seg-{track}-{seq}.ts`
  URI) was advertised with the wrong content type — Apple's
  `mediastreamvalidator` flags this and a strict player rejects it. The
  content type is now chosen from the route's container (`video/mp2t` for
  `Container::MpegTs`, `video/mp4` for fMP4/CMAF).
- WHIP ingest keeps an inbound RTP header extension (RFC 3550 §5.3.1, e.g. RFC 8285 `mid`/`rid`/CVO) when it rebuilds the wire packet for the depacketiser, now that `webrtc-runtime` reports it (#1090).
- The RTSP push transport (#1025) now targets the configured push URL instead of a hard-coded
  `rtsp://localhost/push`, checks every response status, writes the `AuthRetry` bytes rtsp-runtime
  computes on a 401 (previously dropped, so an authenticated push always failed even after
  rtsp-runtime's own retry-body fix), sends interleaved data as RTP (RFC 2250 §2, MP2T/90000)
  instead of raw unframed TS bytes, and advertises a spec-valid SDP (session-level `c=` line,
  matching `a=control`) for the single SETUP/interleaved channel it actually uses.
- `ProgramSegmenter` no longer calls the blocking `SegmentWriter::publish_segment`
  from `pump`/`flush`, which run synchronously on whatever thread the owning
  async ingest task is scheduled on (a shared tokio worker in production): a
  stalled `ArchiveOverrun::StallIngest` DVR pin could park that worker for
  seconds, starving every other task scheduled on it. Now uses the
  non-blocking `SegmentWriter::try_publish_segment`, queuing anything that
  cannot go through yet and retrying it (in order) on every `pump`/`flush`
  call (not only when something new is ready to publish), so a backlog
  keeps draining on an otherwise-idle tick and the tail is not lost at end
  of stream. The pending queue also holds live parts, not just segments, so
  a later segment's parts can never appear before an earlier segment still
  stuck in it (media-plane issue #1082, T7).
- The pending-publish queue above is now bounded: once its oldest entry has
  waited 30 seconds, or 8 SEGMENTS have accumulated behind a stuck one
  (whichever comes first — a hard cap so memory stays bounded even for a
  pin that never advances at all), the blocking `ArchiveOverrun::StallIngest`
  pin is force-expired (`SegmentWriter::expire_stalled_pins`, terminated the
  same way the blocking path's own bound already does) and draining
  retried immediately — a DVR disk error whose pin never advances no
  longer freezes live output past that bound (media-plane issue #1082,
  T6/T7). The cap counts queued segments only, never queued parts — a part
  always rides immediately behind its own segment, so counting parts
  toward the cap made an LL-HLS route (a new part roughly every
  `part_target_ms`, 200ms is a typical target) force-expire a `StallIngest`
  pin in well under 2 real seconds regardless of how briefly the pin was
  actually stuck, nowhere near the 30s deadline.
- `pending_since` (the "how long has the front of the pending-publish queue
  been stuck" clock) now resets whenever the front item actually publishes,
  not only once the whole queue drains empty — previously, a resolved
  stall's timestamp could leak onto a later, different item's brand-new
  stall (still queued behind it), making that item look like it had already
  waited past the 30s deadline the instant it got stuck, and force-expiring
  the pin for a stall that had not actually happened yet.
- `RouteHandle::add_segment` now uses `SegmentWriter::try_publish_segment`
  instead of the blocking `SegmentWriter::publish_segment` — this is a
  test/fallback direct-write path with no retry queue of its own, so a
  segment that cannot go through right now is logged and dropped rather
  than blocking the calling thread (media-plane issue #1082, T7).
- `ProgramSegmenter`'s fMP4 AND classic-TS (`Container::MpegTs`) paths now
  both seed their sequence numbering from `SegmentWriter::next_sequence_number`
  instead of starting at `1`/`0` — `publish_segment` now rejects a
  non-monotonic `sequence_number` (media-plane issue #1082), which a
  `Trunk` that already has segments from an earlier `SegmentWriter`
  (dropped and re-issued), or a TS segmenter rebuilt mid-stream after a
  track-set change (`ProgramSegmenter::apply_track_change`'s
  `Container::MpegTs` rebuild path — previously fMP4-only), would
  otherwise trigger, silently dropping every segment the reseeded/rebuilt
  segmenter cut until its own counter happened to catch back up. Needed a
  new `transmux::ts_hls::StreamingTsHlsSegmenter::with_start_sequence`
  seeded constructor, mirroring `LlHlsSegmenter::with_part_target_at`.
- `ProgramSegmenter::publish_ready`/`apply_track_change`'s fMP4 rebuild now
  queue a segmenter's ready parts and closed segments interleaved by
  sequence number (segment N's own parts, then its close, then segment
  N+1's parts, …) instead of every ready part followed by every ready
  segment. The old order could let a later segment's opening part reach
  the `Trunk`'s live-part log before an earlier, already-closed segment's
  own `SegmentEntry` — `Trunk::parts_in_segment` (see media-plane's own
  entry below) then lost that earlier segment's still-resident parts
  entirely, silently dropping its `#EXT-X-PART` tags from a served LL-HLS
  playlist.
- A `Stall`-policy `DvrRecorder` whose pin is force-expired by the
  non-blocking safety valve above no longer stops recording for good. The
  force-expiry produces the identical `SegmentCursorItem::Terminated`
  signal an `ArchiveOverrun::Terminate` pin's own intended stop does, and
  the recorder previously treated both the same way (permanently stopping,
  and logging "ArchiveOverrun::Terminate" even for a `Stall`-policy pin —
  the wrong policy name). It now re-arms with a fresh pinning cursor at
  the live edge, counts the segments lost in between as a gap (a new
  `multimux_dvr_pin_rearmed_total` counter, labelled `route`, plus a
  corrected log line naming the real policy), and keeps recording.

- **The runtime admin API could still leak runtimes under a concurrent
  mutation, and never started WHEP outputs** (#1083). `add_route`/`reload`/
  `remove_route` now serialise on an async mutation lock, so two concurrent
  adds/reloads of the same name can no longer overwrite one another (the
  loser's `RouteRuntime` was dropped with no `drain_route`, detaching its
  supervisor task and leaving every listen port bound). A reload's rollback
  now cancels each already-prepared route's push/WHEP tasks before aborting
  the supervisor, not just the supervisor (which left push tasks dialling
  their destinations forever). And `spawn_route` now calls
  `spawn_whep_outputs`, so a `whep` output on a route added through the
  admin API — which is every startup route when the admin API is enabled —
  is actually served instead of silently never started. Plugin (`Custom`)
  input/output factories no longer run while the registry lock is held, so a
  panicking third-party factory can no longer poison the registry into a
  permanent admin lockout.
- **A route name of `..` let the DVR/catch-up archive escape
  `archive_root`** (#1083). A name is both a URL path segment and an on-disk
  directory component, so it is now restricted to `[A-Za-z0-9._-]`, must not
  end in `.` or a space, must not be a Windows reserved device name (`CON`,
  `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`, with or without an extension), and
  `.`/`..` are rejected; a name containing `*` (which panicked axum's
  nest-wildcard handling at router build) and two outputs mounting one
  manifest path (an axum "overlapping method route" panic at build) are
  rejected as config errors too. `add_route` builds the route before inserting
  it, so a build panic can no longer leave a route in the registry that every
  later rebuild also panics on.
- **`max_concurrent_requests` was per endpoint, not global** (#1083).
  `Router::layer` clones a layer once per route, so the documented
  slow-loris bound was `bound × routes × methods × streams` and sat inside
  the request timeout, leaving queued requests with no timeout at all. It is
  now one server-wide semaphore (`origin::limit::GlobalLimit`), applied inside
  the public `router()` — so a library caller driving `router()` directly
  gets the bound too, not only a `serve*` entry point. Three budgets now:
  the ops endpoints (`/healthz`, `/readyz`, `/metrics`) are *exempt* (a `503`
  there would have an orchestrator restart the process during a load spike);
  LL-HLS blocking reloads (`_HLS_msn`/`_HLS_part`) draw from their own,
  smaller pool so a reload flood starves only other reloads; everything else
  shares the main pool. A request that cannot get a permit within the
  configurable queue timeout is answered `503 Service Unavailable` with
  `Retry-After`, and counted in `multimux_http_shed_total`. The permit is
  held until the inner service — which in this origin collects the whole
  response body — resolves, so it bounds the entire response, not just
  header time.
- **Every lock this crate holds now recovers from poisoning instead of
  cascading** (#1083) — *except* the DVR recorder's, which **fails closed**:
  `DvrRecorder` holds multi-step state (write offset, index, period records,
  byte totals) that a panic mid-append can leave disagreeing, so a poisoned
  recorder is abandoned (recording stops, `multimux_dvr_failed_total`
  increments) rather than recovered into writing an archive whose index does
  not match its data. Every other guard covers state mutated only by
  single, panic-free assignments, so recovery there is sound and a panic
  while one is held (a plugin factory, a segmenter, an allocation failure)
  no longer permanently disables a route, a router rebuild, or the admin
  API.
- **SCTE-35 → DASH `emsg` injection was dead in production** (#1083).
  `Trunk::events_in_segment` resolves against segment boundaries recorded by
  `SegmentWriter::note_segment_start`, which had no production caller — so
  every served segment saw an empty event list. `ProgramSegmenter` now
  reports each segment's start, derived from a real sample's PTS and the
  segment's exact internal-timeline offset (re-anchored on a PTS
  discontinuity), on the trunk's 90 kHz event clock. Id-less SCTE-35 events
  (a `time_signal` has no `splice_event_id`) get unique presentation-time
  ids instead of all colliding on `0` (which DASH clients de-duplicate by
  `(scheme, value, id)`, dropping every one after the first); a declared
  `splice_event_id` is kept verbatim.
- **RTSP's permanent-failure classification was dead, and there was no
  keepalive** (#1083). `run_rtsp` flattened every terminal session state into
  a `Connect` with a `{:?}`-formatted reason, discarding the typed
  `MultimuxError::Auth`/`Protocol` the session actually produced — so the
  supervisor's `is_auth_failure` checks (written for RTSP) never matched and
  a wrong password retried forever. The typed error is now returned as-is. A
  per-DESCRIBE-`404` classification that declared a route permanently failed
  on a single attempt is now bounded like the auth path (5 attempts), since
  relays commonly answer `404` until their publisher is up. A periodic
  `OPTIONS` keepalive keeps a session alive against servers that enforce
  `Session;timeout=N` (Live555-based cameras tore it down every 60 s); its
  interval is clamped to `[KEEPALIVE_MIN, KEEPALIVE_MAX]` so a hostile
  `timeout` value cannot overflow the clock or spam sub-second `OPTIONS`, and
  a non-success reply to the keepalive (many servers answer `405`) no longer
  tears the live session down.
- **DVR durability** (#1083). A partial write (ENOSPC mid-`write_all`) left
  the append-mode period file grown while the cached offset stayed put, making
  every later `IndexEntry.byte_offset` short by that amount and serving
  shifted, corrupt bytes for the rest of the period — the offset is now
  derived from the file's real length, and a failed append truncates any
  partial tail (`set_len`) so the next segment does not sit behind junk. The
  period data is `sync_data`d before its index entry is written, and the
  index write (`File::flush` was a no-op) `sync_all`s the sidecar and its
  directory, so the "flushed synchronously" claim is backed by real
  durability. Byte-based retention now counts the still-open period, and a
  byte-capped route rolls its period once it passes a per-period budget
  (`retention_bytes / retention_periods`) so one file can no longer grow past
  the whole cap; `period_duration_secs: 0` with only count-based retention is
  rejected as unbounded. `rebuild_index` reports a period shorter than the
  init prelude instead of panicking on the slice.
- **W1 (`StallIngest` deadlock) needed no new fix** (#1083): the ingest
  segmenter already publishes non-blockingly
  (`SegmentWriter::try_publish_segment` with a bounded retry queue,
  `ProgramSegmenter::drain_pending`), so the "DVR pin blocks the ingest task,
  which is the only DVR drainer" deadlock is not reachable; and the drain it
  does perform now runs off the runtime worker (the W3 change above). The
  covering test is the pre-existing
  `source::segment::tests::publish_without_blocking_never_blocks_the_calling_thread`
  — this release adds no new W1 test.
- **An RTSP session that completes its handshake and answers keepalives but
  sends no media is now detected as stalled** (#1083). The read timeout was
  measured per `timeout(..)` call, so a keepalive wakeup slightly under the
  read timeout reset it forever and the stall was never reported; the loop
  now tracks the instant of the last received byte and fails once
  `now - last_rx >= read_timeout`, independent of keepalive wakeups.
- **A mid-segment PTS discontinuity no longer retro-stamps the segment being
  filled** (#1083). The new source-clock anchor is recorded as *pending* and
  applied at the next segment boundary, so the in-progress segment keeps the
  anchor its start was actually on.
- **DVR byte-rolling and eviction hardened** (#1083). The roll guard checks
  `!index.is_empty()` (not `write_offset > 0`, which equals the init length
  right after `start_period`, so a single oversized segment rolled an empty
  period). The per-period budget is `retention_bytes / (retention_periods +
  1)`, reserving room for the open period, so a roll no longer immediately
  evicts the whole closed history. Eviction removes the period's index before
  its data file, so no racing catch-up request can see a listed period whose
  file is gone.
- **All three catch-up scan paths share the blocking-pool bound** (#1083):
  `vod_playlist` and `catchup_resource` now take the same scan semaphore
  `catchup_playlist` does, and a cached index hit in
  `read_period_segments` is an `Arc` bump rather than a `Vec` clone.
- **The `emsg` oracle test fails rather than silently skips** (#1083) when
  `mp4dump` is absent, unless `MULTIMUX_ALLOW_ORACLE_SKIP=1` is set.
- **Serving a catch-up playlist no longer blocks a runtime worker** (#1083).
  `scan_archive` read and JSON-parsed every `pN.idx` on each
  `GET catchup.m3u8`, synchronously, on a tokio worker; a handful of
  concurrent unauthenticated requests pinned every worker and stalled ingest
  for every route. The archive scan, the per-period index read, the archived
  byte read, the DVR persist, and the file-input probe/demux all run on the
  blocking pool now, and a parsed period index is cached keyed by the
  sidecar's `(mtime_ns, len, inode)` — a bounded LRU — so a finished period
  is not re-parsed per request. Concurrent archive scans are capped so a
  catch-up flood cannot occupy the blocking pool the DVR persist shares. The
  DVR persist itself is skipped entirely when the trunk has closed no new
  segment since the last poll (the UDP ingest path calls `advance_route` once
  per datagram).

- **A source reconnect no longer orphans push and WHEP egress** (audit
  r07-C4, #1083). Push (SRT/RTMP/RTSP) and WHEP outputs subscribed to the
  route's first `Trunk` once and kept draining it after a redial replaced it
  with a fresh one, so they went silent with no error. Each is now bound by
  `follow_trunk`: cancelled when the route's first program is bound to a
  different `Trunk`, given 5 s to wind down, and started again on the new one.
  The wait for a `Trunk` (`RouteHandle::await_first_trunk`) no longer loses a
  publish that lands between its check and its wait, and "the first program"
  is the lowest-numbered one rather than whichever the map yields (#1083).
- **The DVR archive survives a reconnect or restart** (audit r07-C5, #1083).
  A new recorder started at period 0, appended to the old `p0.<ext>` and
  rewrote `p0.idx` from its own entries alone — orphaning every earlier
  segment, skipping every segment of an fMP4 route (the file was non-empty so
  no init was written), and leaking the old periods past retention. It now
  adopts the periods already on disk (retention covers them), always opens a
  new one, and indexes its first segment `discontinuous`. Segments are indexed
  under the number the route's playlist shows (Trunk number + the origin's
  media-sequence offset), so archive, live tail and by-number fetch agree and a
  reconnect's restarted `Trunk` numbers no longer collide; the live tail the
  catch-up playlist merges is mapped to the same numbers. A process restart
  starts numbering above the highest archived number
  (`RouteHandle::with_archive_floor`) (#1083).
- **RTMP and WHIP ingest accept a publisher that connects after the route has
  been up for the connect timeout** (audit r07-C6, #1083). Every session was
  fed a clock measured from the route's start, so the handshake deadline
  (`now + connect timeout` from that origin) had passed for any later session,
  which was reaped on its first feed. Each session's clock now starts at its
  own admission (#1083).
- **A Data/Subtitle track appearing mid-stream no longer breaks an fMP4
  route** (audit r07-C7, #1083). The track-change rebuild installed a TS
  segmenter as a placeholder (its `expect` panicked the ingest task for an
  empty track set) and left it installed when the fMP4 rebuild failed — the
  route then published MPEG-TS bytes as `seg-*.m4s`. The rebuild now filters
  the fMP4-muxable tracks exactly as the initial build does, builds the
  replacement before touching anything, and on any failure keeps the working
  segmenter (#1083).
- **The metrics middleware no longer buffers response bodies** (audit
  r07-C10, #1083), so an in-progress LL-DASH segment streams while it is
  produced (#721). The byte count is taken from the frames as they pass and
  recorded when the body ends or is dropped; a sized body keeps its
  `Content-Length`. The global concurrency permit is carried into the body, so
  it still bounds the whole response (a stalled streaming response keeps its
  permit) (#1083).
- **`Cache-Control: immutable` only where the name carries the origin's
  instance token** (audit r09-C2, #1030/#1083). `hls-runtime`'s origin puts a
  per-instance token (distinct for every origin built, across reconnects *and*
  process restarts, with or without DVR) in every resource name it serves
  immutably, so a name never maps to other bytes. What multimux sends:

  | resource | success | otherwise |
  |---|---|---|
  | playlists / manifests | `no-cache` | `no-cache` |
  | `init-{t}-{instance}-{gen}.mp4`, `seg-{t}-{instance}-{msn}.*`, `part-{t}-{instance}-{msn}.{i}.*` | `max-age=31536000, immutable` | `no-cache` |
  | token-less `seg-{t}-{msn}.*` / `part-…` (DASH/Smooth templates), `catchup/seg-*` | `max-age=10` | `no-cache` |
  | bare `init-{t}.mp4` (the current init), anything else | `no-cache` | `no-cache` |

  A reconnect to a source with a different SPS/ASC, a restart of a route with
  no DVR (numbering starts again at 1), and the old origin's open segment's
  number no longer put new bytes under a cached `immutable` name; every
  non-success response was `immutable` before and is now `no-cache` (#1083).
- **A mid-stream codec-config change reaches the init segment** (deferred
  item, #1083). The FLV and TS demuxers report a changed `avcC`/ASC/SPS as
  `TrackUpdated`; the TS `ProgramTracker` and the RTMP session ignored it, so
  the init segment kept describing a stream that was no longer being sent. Both
  now announce the updated track set (`TracksChanged`) (#1083).
- **`multimux_route_up` was never `1`, and a deleted route stayed up**
  (audit r07-O5, #1083). `Live` is reached from the ingest facade, which has
  no route name, so only the supervisor wrote the gauge and never wrote `1`;
  `RouteHandle::set_health` now mirrors every state into it. A route removed
  through the admin API (delete, or dropped by a reload) is reported `0`
  after its supervisor has stopped — the metrics facade and exporter have no
  per-series removal, so the series stays at `0` rather than at its last value (#1083).
- **Push destinations never reach a log line, error text or `Debug` output**
  (audit T14, #1142/#1083): a push URL's userinfo and stream key (path/query)
  are replaced by `scheme://host[:port]/<redacted>` in every log line;
  transport errors are scrubbed of every secret derived from the URL before
  logging; `validate_srt_url`'s config error no longer echoes the URL
  (`streamid`/`passphrase`); `RtspTransport`'s and `OutputKind`'s `Debug` print
  the destination only (and `<redacted>` for a `Custom` output's params)
  (#1083).
- The DVR index sidecar is extended in place instead of rewritten and
  fsynced whole per segment (audit r07-O3, #1083): about 25 ms and 680 KB per
  segment, 3.6 GB per 3-hour period at 2 s segments, down to a few hundred
  bytes. A torn tail after a crash is recovered by the catch-up reader
  (every complete entry); the whole-file atomic rewrite remains for a period's
  first entry and after a failed append (#1083).
- A file route's sample ring no longer scales with the file (audit r07-O4,
  #1083): it is a fixed few drains deep, and the paced session hands out at
  most `MAX_SAMPLES_PER_DRAIN` samples per drain, so a stall that makes a long
  stretch due at once cannot overflow the ring (#1083).
- **`catchup.m3u8?window_secs=N` is correct across a reconnect** (audit
  r07-C5, #1083): the window is measured by walking back from the live edge
  summing segment durations, not by `start_pts_ns` arithmetic — that clock
  restarts near 0 with every new `Trunk`, so the old floor selected the whole
  previous run's archive.
- **The catch-up playlist carries a per-run `EXT-X-MAP`** (#1083): each archived
  period file starts with the init of the run that wrote it, served at
  `catchup/init-p{N}.mp4`; the playlist emits that run's map after the
  `EXT-X-DISCONTINUITY` that opens the run (a segment still in the live origin
  names the origin's versioned init). Previously only the bare, current
  `init-1.mp4` was named on the first segment, so older runs decoded with the
  wrong init. Validated with Apple's `mediastreamvalidator` over two real-CMAF
  runs with different inits.
- The DVR's EIT reader drops (and counts, `multimux_dvr_si_errors_total`) a
  section it cannot act on instead of keeping its packet in the carry buffer
  to fail again on every call (#1083).
- Hot-path copies (audit r07-O2, #1083): WHEP builds each RTP packet once
  (header + payload straight into the buffer it patches, not via a temporary
  contiguous `Bytes` and a second copy) and the DVR's EIT reader reuses its
  event and carry buffers across calls. The per-segment/once-at-start sites the
  audit also named (`dash_pull`, `smooth_pull`) copy a URL string or a first
  fragment and are not on a per-packet path; left as they are (#1083).
- The origin's ten near-identical `supervise_driver` wrappers are one
  `spawn_supervised`, and push/WHEP egress share `spawn_following`; the shift
  based `ReconnectPolicy::backoff_for` and the supervisor's `Backoff` are one
  capped-exponential implementation (`Backoff::delay_for_attempt`) (audit
  r07-O1, #1083).
- Catch-up's `EXT-X-MAP` is rendered by `broadcast-hls` instead of formatted
  by hand; a few `as` conversions on disk-derived lengths are checked; the
  catch-up index cache recovers from a poisoned lock; seven string literals
  that had lost their line-continuation (runs of spaces inside the message)
  are repaired (#1083).

### Changed (breaking)
- **The Smooth client Manifest changed shape** (#1083): HEVC tracks are no
  longer advertised (Smooth cannot serve them — they 404'd); each
  `StreamIndex` now carries its **own** `c` timeline in absolute 10 MHz ticks
  derived from that track's samples (so audio reflects AAC's real frame timing
  and a chunk's `t`/`d` is stable as the window slides), replacing the shared
  muxed segment duration; `QualityLevel@Bitrate` is now a real bitrate (was the
  quality ordinal), and a fragment URL's `QualityLevels(N)` carries that
  bitrate; `mfhd.sequence_number` is the segment sequence number (was the
  constant track ordinal). A client that keyed on the old values must be
  updated.
- **An SRT push URL is now validated at config time** (#1083): a missing host,
  an unsupported `mode` (only `caller`), a `passphrase` (SRT encryption is not
  implemented), an out-of-range `latency`, or an unbracketed IPv6 authority is
  a config error (surfaced on admin add/reload too).
- **A live DASH `$Number$` route now starts at the live edge** (#1083),
  derived from `availabilityStartTime`, `Period@start`, `presentationTimeOffset`
  and a documented 3-segment suggested-presentation-delay, computed in the
  template's tick domain — not at `@startNumber`. A dynamic `$Time$` MPD plans
  only its last few `SegmentTimeline` entries.
- **Config validation now rejects values it previously accepted** (#1083):
  a `ReconnectPolicy` with a zero backoff, an `initial_backoff_ms` greater
  than `max_backoff_ms`, or a backoff above 24 h; and an
  `ingest_connect_timeout_secs`/`ingest_read_timeout_secs` that is not a
  finite number in `(0, 86_400]` (so a non-finite or overflowing value such
  as `1e20` is now a config error, not a startup panic). A config that
  validated before this change still does unless it used one of those
  degenerate values.
- **WHIP/WHEP and the origin's CORS responses changed** (#1083): WHIP/WHEP
  now answer a preflight with `Access-Control-Allow-Methods: POST, PATCH,
  DELETE, OPTIONS` and `Access-Control-Allow-Headers: Authorization,
  Content-Type, If-Match`; the origin adds `Access-Control-Expose-Headers`
  (`Content-Length, Content-Range, Date, ETag`), `Vary: Origin` (appended),
  and `GET, HEAD, OPTIONS`.
- **Pull sources bound their responses** (#1083): a `$Number$` plan is capped
  at `dash_pull::MAX_PLAN_ENTRIES`, and every pull-source HTTP client uses
  `source::redirect_policy` (3 hops, no `https`→`http` downgrade). An origin
  that relied on following more redirects, or on a downgrade, is affected.
- `push::RtmpTransportConfig` gained a `connect_timeout: Option<Duration>`
  field (#1083) — `None` uses the new `RTMP_CONNECT_TIMEOUT` (15 s). A
  struct literal outside this crate must add it (`..Default::default()`).
- `RouteHandle::add_segment` returns `Result<(), AddSegmentError>` instead of
  logging and dropping a segment it could not publish (#1082).
- `source::advance_route` is now `async` (#1083): its DVR-drain step writes
  segment bytes to disk, so it is dispatched to the blocking pool rather than
  run inline on the calling runtime worker.
- `origin::admin::RouteRegistry::add_route` is now `async` (#1083): it
  builds the prospective router and drains a displaced route on overwrite.
- **Newly rejected configuration** (#1083): a route name that is not a single
  safe path segment (`[A-Za-z0-9._-]`, not `.`/`..`, at most
  `MAX_ROUTE_NAME_LEN` = 255 bytes), a duplicate name differing only in case
  (they share one `archive_root/<name>` directory on a case-insensitive
  filesystem), and two outputs mounting the same manifest path (which panics
  the axum router build). Multiple push/`custom`/`whep` outputs on one route
  — which mount no HTTP path — remain valid; the previous
  `mem::discriminant`-based check wrongly rejected them.
- `ProgramSegmenter` is now `pub` (#1083) so an embedder or test can drive the
  exact production segmenting path (`try_new` → push → `pump` → `flush`).
- **Two public structs gained fields** (#1083), so a struct literal or
  exhaustive pattern outside this crate must be updated:
  `origin::HttpLimits` gained `queue_timeout: Duration`, and
  `config::Config` gained `concurrency_queue_timeout_secs: f64` (its serde
  default is the 5 s `DEFAULT_QUEUE_TIMEOUT`). `HttpLimits::from(&Config)` now
  clamps an unvalidated (NaN/negative/overflowing) timeout to the default
  rather than panicking in `Duration::from_secs_f64`.
- `catchup::read_archived_bytes` now returns `Result<Bytes,
  ReadArchivedError>` instead of `Result<Bytes, String>` (crate-private), and
  `catchup::read_period_segments` returns `Arc<Vec<ArchivedSegment>>` — both
  to distinguish an evicted period (a `404` to a racing catch-up request)
  from a corrupt archive (a `500`).
- **`IndexEntry::seq` in the DVR archive, and the numbers `catchup/seg-{n}`
  uses, are now the route's playlist numbers** (Trunk number + media-sequence
  offset) instead of the bare `Trunk` number (#1083). They are equal until the
  route's source reconnects or the process restarts. A pre-existing archive
  written by an older version keeps its own numbers; a restarted process
  numbers above the highest it finds.
- `DvrRecorder` never appends into a period file an earlier recorder wrote: a
  recorder built over an existing archive starts a new period after the
  highest on disk (#1083).
- **Restarts rename.** The origin's playlist names every init, segment and
  part with the origin's instance token (`seg-{t}-{instance}-{msn}.*`, see the
  Cache-Control entry and `hls-runtime`); the bare `init-{t}.mp4` and the
  token-less `seg-{t}-{msn}`/`part-…` still resolve, but are no longer served
  `immutable`. A route restart renames everything and, without DVR, renumbers
  from 1 (media sequence continuity across a *process* restart needs the DVR
  archive's floor); within a process a reconnect continues the numbers and
  skips the old origin's open segment's (#1030/#1083).
- `OutputKind`'s `Debug` output redacts push URLs and `Custom` params (it was
  derived) (#1083).

### Added
- `RouteHandle::release_program` — gives a program's publish slot back once
  the driver-backed session that owned its `Trunk` is reaped, so a later
  publisher can bind (the counterpart to `publish_program`'s
  second-concurrent-publisher rejection) (#1083).
- `source::release_route` — the session-level wrapper a `Listener`-backed
  `run_*` (or a `Custom` factory driving its own sessions) calls right before
  reaping a driver, releasing every program that session held (#1083).
- `output::whep::run_whep` — binds a WHEP route's listen socket and drives up
  to `WhepRoute::with_max_sessions` viewers concurrently until cancelled;
  spawned once per `OutputKind::Whep` output (#1083).
- `DvrRecorder::needs_poll` — a cheap "could a poll make progress" check so
  the per-datagram `advance_route` path can skip the blocking-pool round trip
  (#1083).
- `source::rtp_udp::RtpUdpIngestSession::depay_dropped` — packets dropped so
  far for failing depayload (#1083).
- `source::rtsp::DEFAULT_KEEPALIVE_INTERVAL` (30 s) — the RTSP `OPTIONS`
  keepalive interval used when a server declares a session timeout and the
  config does not override it (#1083).
- `multimux_pull_stream_refresh_miss_total` Prometheus counter: manifest
  refreshes in which a live stream had no matching `StreamIndex` (#1083).
- `DashIngestSession::with_clock` — inject the wall clock used to place a live
  `$Number$` plan at the live edge (#1083), for a deterministic test.
- `source::redirect_policy`, the SRT push URL's `streamid`/`latency` query
  parsing (internal to the push transport, see the entries above), and
  `MAX_TIMEOUT_SECS` — see the entries above.
- `ReconnectPolicy::validate` — rejects a zero initial/max backoff (#1083),
  called from `Route::validate_standalone`.
- `source::MAX_HTTP_BODY_BYTES` — the response-body cap every pull-source
  fetch now applies (#1083).
- `DvrConfig::retention_active` — whether configured retention can evict
  leading segments (`retention_periods` or `retention_bytes` set), used by
  the catch-up playlist to decide whether it may claim
  `EXT-X-PLAYLIST-TYPE:EVENT` (#1083).
- `multimux_dvr_pin_rearmed_total` Prometheus counter (labels: `route`) —
  see the `DvrRecorder` re-arm fix above.
- `multimux_http_shed_total` Prometheus counter (label: `kind` =
  `ordinary` | `blocking_reload`): requests the global concurrency bound
  answered `503` before they reached any route (#1083).
- `multimux_pull_fragment_abandoned_total` Prometheus counter (label:
  `source`): a live-edge segment/fragment a DASH/Smooth pull abandoned after
  exhausting its bounded tolerated-`404` retries (#1083) — see the pull-source
  fix above.
- `multimux_dvr_failed_total` Prometheus counter (label: `route`): a DVR
  recorder abandoned because its mutex was poisoned (#1083). The poisoned
  DVR recorder is abandoned (recording stops) at whichever entry observes the
  poison first — `poll_dvr`'s production `try_lock` pre-check, or
  `poll_dvr_blocking`.
- `origin::limit`: `GlobalLimit`, `GlobalLimitLayer`, `GlobalLimitService`,
  `DEFAULT_QUEUE_TIMEOUT`, `DEFAULT_BLOCKING_RELOAD_DIVISOR`,
  `RETRY_AFTER_SECS`, `HTTP_SHED_TOTAL` — the server-wide concurrency bound,
  applied inside `origin::router` (#1083).
- `origin::DEFAULT_QUEUE_TIMEOUT` and the `concurrency_queue_timeout_secs`
  config key: how long a request waits for a concurrency permit before it is
  shed with `503 Service Unavailable` + `Retry-After` (RFC 9110 §10.2.3).
  Default 5 s.
- `source::rtsp::{KEEPALIVE_MIN, KEEPALIVE_MAX}`: bounds on the RTSP
  keepalive interval derived from a server's `Session;timeout` (#1083).
- `config::MAX_ROUTE_NAME_LEN`.
- `dvr::validate_route_dir_name` — the defence-in-depth check
  `DvrRecorder::new` applies to a route name before joining it into a path.
- `scte35-splice` as a dev-dependency, for the independent `emsg`
  `message_data` oracle in `tests/emsg_mp4box.rs`.
- `RouteHandle::with_archive_floor`, `RouteHandle::await_trunk_change` and
  `DvrRecorder::with_seq_offset` (see the DVR / egress fixes above) (#1083).
- `multimux_dvr_si_errors_total` counter (label: `route`); `catchup/init-p{N}.mp4`
  route; `source::SessionClocks` (crate-private) (#1083).
- `Backoff::delay_for_attempt` — the stateless form of repeated `next()`
  (#1083).

### Changed

- HLS-pull resource-fetch retry delay now comes from `origin::supervisor::Backoff::delay_for_attempt` (the third, shift-based capped-exponential copy is gone; schedule unchanged and pinned by a test) (#1141).


## [0.11.0] - 2026-09-26

### Security
Fixes GHSA-6cpc-jqv3-qcj3, GHSA-c5v7-p4jv-2fhc, GHSA-jwfh-m4vx-fhwx and GHSA-2w4r-qf2x-pqm6, and
picks up GHSA-48qq-7p78-2jvj (webrtc-runtime 0.2.0) for WHIP/WHEP.

### Changed (breaking)
- Requires `hls-runtime` 0.7 (was 0.6), `broadcast-auth` 0.3.1 (was 0.3, needs
  `Verifier::challenge_for`), `rtsp-runtime` 0.7 (was 0.6) and, with the `whip`/`whep` features,
  `webrtc-runtime` 0.2 (was 0.1).

### Fixed
- WHIP input and WHEP output pass the offer's `a=fingerprint` to the media transport, so the
  DTLS peer is authenticated (webrtc-runtime 0.2.0); an offer without a fingerprint is rejected.
- Push outputs await the trunk listener instead of blocking a runtime worker, and back off when
  every listener slot is taken.
- An RTP/UDP route drops a malformed packet instead of failing, and a failed session now ends
  the route so the supervisor reconnects it.
- WHEP applies the configured output auth, ends sessions after 30 s without inbound traffic, and
  WHIP/WHEP cap HTTP header and body size and time out slow requests; WHIP checks capacity before
  allocating a session.
- The WHIP/WHEP session-capacity slot is released on every failure path after it is reserved;
  both accept loops cap concurrent connections; chunked transfer-encoding is refused (411) and an
  unparseable `Content-Length` is a 400; the WHEP silence timer resets only on RTP, RTCP or a
  completed DTLS handshake.
- A WHEP viewer's session no longer ends after 30 seconds. The silence timer counted only
  RTCP that `rtcp-packet` could decode, but a browser viewer sends RFC 4585 feedback (PLI,
  NACK, REMB), which it cannot. Any SRTCP packet that authenticates now counts (needs
  `webrtc-runtime`'s new `MediaEvent::RtcpUnsupported`). A datagram that fails authentication
  still does not.
- A route now rejects a second, concurrent RTMP/WHIP publisher instead of letting it silently
  take over (or freeze, once the first disconnects) the program the route is already receiving
  from. The rejected connection's own session is reaped normally by its listener's usual
  timeout/error handling; once the active publisher's session ends, the route accepts a new one
  for that program again (a legitimate reconnect, or a backup encoder taking over).
- Output-auth and admin-auth Digest challenges now use `broadcast_auth::Verifier::challenge_for`
  instead of `challenge`, so a request that correctly answers an expired nonce gets a fresh
  challenge carrying `stale=true` (RFC 7616 §3.3) rather than being silently re-prompted for
  credentials.
- Config docs for `InputSpec::Rtmp`/`Whip`/`Srt` now say plainly when an ingest listener runs
  with no authentication at all, and a startup log line warns for each such route.

### Added
- **DASH SCTE-35 inband event signalling** (issue #969). MPD now declares
  `<InbandEventStream schemeIdUri="urn:scte:scte35:2013:bin">`, and served
  fMP4 segments carry serialized `emsg` boxes (after `styp`, before `moof`)
  for segments with resolved SCTE-35 events in the trunk's event ring.
  Non-segment resources and eventless segments pass through unchanged.

## [0.10.0] - 2026-08-14

### Fixed

- **Crate description and README understated the output set.** Both advertised
  `LL-HLS/DASH/LL-DASH` while `OutputKind` has shipped `Smooth`, `TsHls`,
  `Catchup`, `SrtPush`, `RtmpPush`, `RtspPush` and `Custom` for several
  releases; the 0.10.0 release note already listed TS-HLS, so the crates.io
  description was the surface that disagreed. Corrected everywhere the set is
  named (crate description, workspace README, `multimux-cli`'s description and
  README).
### Added
- **`InputSpec::File { path, loop }` — stream a media file as a route input**
  (issue #748). Point a route at a file on disk and it is served through the
  same `supervise_driver` + `advance_route` path as every other source, so the
  existing outputs (LL-HLS, DASH, LL-DASH, TS-HLS) work unchanged:

  ```json
  { "name": "slate",
    "input": { "type": "file", "path": "/media/slate.ts", "loop": true },
    "outputs": ["llhls"] }
  ```

  The container is identified with the new `container-probe` crate over the
  whole file, and the verdict selects the demuxer — including the ISOBMFF
  fragmented/progressive split via `IsobmffLayout`, so no box-walking is
  duplicated here. A format `transmux` cannot demux, and every ambiguous or
  undetermined verdict, fails with its own structured error: **a file is never
  fed to a guessed demuxer.**

  `loop` defaults to `true`. Looping refills from the already-parsed content
  with a per-track PTS offset so the output timeline stays strictly monotonic
  across the loop point. That offset derives from the **presentation-max** PTS
  rather than the decode-order last sample — a track with B-frames reorders, so
  a decode-order basis would step backwards across the loop.

  Samples are paced to wall clock on both the standalone reader and the route
  path, in every `pace` x `loop` combination, including across the loop
  boundary. Unpaced, the route republished the whole file every 10 ms (~300x
  realtime), leaving the playlist's segment cadence meaningless to a player.

  **Known limit:** a long asset holds its parsed samples in memory — the file is
  demuxed in one pass and the sample ring is sized to it. Suited to slates and
  short assets; a long-file path would need incremental demux.

### Note on scope

Issue #748 originally designed a linear-playout **channel** — scheduled
switching between sources, a rebased shared timeline, and SCTE-35 ad-break
signalling. **That was cancelled during implementation** and its code reverted
rather than left half-finished; the complexity was not earning its keep. Only
the file-input capability above shipped. See
`docs/superpowers/specs/2026-08-11-linear-playout-design.md`, marked
`SUPERSEDED`, for what was dropped and why.

(An earlier draft of this entry claimed the cancelled controller's dependency on
`playout-runtime`/`ssai-runtime` would have blocked publishing `multimux`
because neither was on crates.io. That was wrong — both are live at 0.1.0. The
check had queried crates.io without a `User-Agent`, which the API answers with
HTTP 403, and the rejection was misread as absence. The cancellation stands on
its own merits; the claim does not.)

## [0.9.0] - 2026-08-11

### Added
- **Catch-up / time-shift / VOD-from-live serving over the DVR archive**
  (issue #900, follow-up to #746/#903): a new `"catchup"` output
  (`OutputKind::Catchup`) serves `GET /catchup.m3u8` (optionally
  `?window_secs=N` to bound the trailing catch-up window),
  `GET /vod/p{N}.m3u8` (one archived period rendered as a complete,
  `#EXT-X-ENDLIST`/`PLAYLIST-TYPE:VOD` asset once a later period exists on
  disk proving it finished), and `GET /catchup/seg-{seq}.{ext}` (the
  resource route both playlists reference). Requires
  `routes.dvr.enabled: true`; rejected otherwise by `Config::validate`.
  Shares the same output auth (Basic/Digest/Bearer/Forwarded) every other
  output gets — no separate auth path.
  - **The straddle boundary** (the reason this exists): the DVR archive and
    the live `Trunk` are different sources sharing one sequence-number
    space. `catchup.m3u8` merges the archive's segments with only the live
    `Trunk`'s still-unarchived tail (`crate::catchup::merge_segments`),
    producing one continuous, gap-free, duplicate-free playlist across the
    boundary — never two disjoint lists a client has to stitch together.
    A request for an archived segment reads its exact byte range off disk;
    a request for the unarchived tail resolves through the same
    `HlsOrigin` the live outputs already share. No second in-memory cache
    of live data is built — per #746's hard constraint, the archive is
    read fresh from disk per request, and the live tail is read through
    the existing `HlsOrigin` cursor (`HlsOrigin::closed_segments`, new)
    rather than a second one.
  - `crate::dvr::IndexEntry` gained `duration_ns`/`discontinuous` fields
    (`#[serde(default)]`, so an old `pN.idx` still parses) — needed to
    render an archived segment's `#EXTINF`/`#EXT-X-DISCONTINUITY` without
    decoding its media bytes.
  - `hls_runtime::server::HlsOrigin` gained `closed_segments()` (returning
    the new public `hls_runtime::server::ClosedSegment`), reusing its
    existing live-window cursor rather than requiring a caller to open a
    second one on the same `Trunk`.
- **Programme-aligned DVR rolling** (issue #903, follow-up to #746): a DVR
  route can now opt in to rolling its archive period on the DVB EIT
  present/following transition (ETSI EN 300 468 §5.2.4) instead of only on
  the clock, so one recording is one programme rather than an arbitrary
  time slice. `DvrConfig::dvb_service_id` names the service to track;
  `DvrRecorder::feed_si` (fed raw TS bytes by `source::ts_udp`/
  `source::ts_http`/`source::srt` via the new
  `RouteHandle::feed_si_ts`/`ProgramServing::feed_si`) reassembles the
  service's EIT p/f actual section and rolls the period the moment the
  present `event_id` changes. Each period gets a `pN.event.json` sidecar
  (`event_id`, `service_id`, title, announced start/duration) alongside its
  `pN.<ext>`/`pN.idx`, so an operator can find a programme rather than a
  timestamp. The existing `period_duration_secs` clock stays active
  unconditionally as both the fallback (routes/sources with no SI) and the
  hard cap (an EPG whose EIT carousel never signals a transition) — EIT
  alignment only ever shortens a period, never removes the cap.
- **WHEP egress output** (`OutputKind::Whep`, config token `"whep"`, issue
  #743): accepts a viewer's HTTP `POST`ed SDP offer, negotiates ICE +
  DTLS-SRTP, and pushes the route's `Trunk` samples out as SRTP RTP over
  `transmux::RtpPacketiser::packetise_video` (RFC 6184 single-NAL/STAP-A/
  FU-A). A sibling `whep` Cargo feature to `whip` (below) — both pull in
  `webrtc-runtime/media` and share its rustc >= 1.88 floor, kept optional so
  an HTTP-only consumer isn't forced to build the ICE/DTLS-SRTP tree. Scope:
  video (H.264) only, no trickle ICE, no `PATCH`. Verified against a real
  second `RTCPeerConnection` (not a mock peer): 119 decoded H.264 frames,
  `<video>` `currentTime` advancing to ~6.13s, reproduced across two
  independent runs.
- **WHIP ingest** (`InputSpec::Whip`, config token `"whip"`, issues #740/
  #743): an inbound WHIP publisher (RFC 9725) — HTTP `POST`/SDP-offer
  signalling plus `webrtc-runtime`'s `MediaTransport` for ICE + DTLS-SRTP —
  behind a new `whip` feature. Real `avcC` config is captured from the
  in-band STAP-A SPS/PPS (browsers omit it from SDP) rather than fabricated,
  using the same deferred-announce gate `source::rtmp` already uses for
  "wait for the first sample". Video-only (H.264); no RTP/Opus depayloader
  exists anywhere in this workspace, so audio is out of scope rather than
  half-built. Verified against a real browser (headless Chromium, fake video
  device, forced H.264) over a real UDP socket and multimux's own generated
  certificate.
  - **Push egress converged onto the same sans-IO `PushEgress` shape as
    WHIP/WHEP** (issue #942): `PushTransport` gained `encode_media`
    (sans-IO encode) and `write_message` (raw verbatim write, distinct from
    the framing `send`), so `PushTransportEgress<T>` can implement
    `PushEgress` — `send()` encodes and queues, `flush_transmit()` writes.
    RTMP's ad-hoc `is_flv_codec`/`warned_refused_tracks` refusal logic now
    lives in `supports_codec` + `negotiate`/`renegotiate` returning
    `NegotiationOutcome::Refused`; `drive_push` also detects a mid-stream
    track-set change via `Trunk::track_generation` and renegotiates, which
    it previously never handled.

### Changed (BREAKING — issue #903, EIT-boundary DVR rolling)
- `InputSpec::Custom` and `OutputKind::Catchup` are **additive**. Both enums
  were already `#[non_exhaustive]` at the `multimux-v0.8.0` tag
  (`git show multimux-v0.8.0:multimux/src/config.rs`, line 75 —
  `#[non_exhaustive]` immediately above `pub enum InputSpec`; and
  `.../src/output/mod.rs` line 50 for `OutputKind`), so no external
  exhaustive `match` over either compiled at 0.8.0, and none breaks now. Two
  successive pre-release audits recorded one of these as a breaking
  `#[non_exhaustive]` *addition* — both were wrong, in opposite directions.
  This entry replaces them, and would have sent every consumer on a migration
  for a break that never happened.
- **`source::ts_udp::recv_and_feed` now returns `Result<usize>`** (was
  `Result<()>`): the number of bytes read, so a caller can also feed the
  exact same slice to the new `RouteHandle::feed_si_ts` EIT tracker without
  a second read. Source-breaking for any direct caller of this `pub` async
  function.
- **`source::srt::StreamStatus::Fed` and `source::ts_http::StreamStatus::Fed`
  now carry the payload (`Fed(Vec<u8>)`)** (were the unit variant `Fed`),
  for the same reason — feeding `RouteHandle::feed_si_ts` from the drive loop
  without a second read. Both `StreamStatus` enums are `pub`; a direct match
  on either no longer compiles without a binding on `Fed`.

### Changed
- **`broadcast-common` floor raised `9` -> `9.3`.** `output::smooth` calls
  `broadcast_common::hex::hex_encode`, absent at the 9.0.0/9.1.0 tags
  (verified against all three: 9.0.0 -> 0 hits, 9.1.0 -> 0, 9.2.0 -> 1).
  multimux 0.8.0 shipped this understated, so a lock resolving
  broadcast-common 9.0/9.1 fails to build published 0.8.0 today. The API
  reason bounds it at 9.2; the declared floor is 9.3 because the MSRV-1.95
  wave (#949) moved broadcast-common there in the same release.
- **`transmux` floor raised `0.23` -> `0.24`.** `push::rtmp` calls
  `transmux::flv_sequence_header_payloads`/`flv_frame_payloads`, absent at
  transmux-v0.23.0 and present at v0.23.1. Same understated-floor class as
  the broadcast-common entry above, found by the same audit. The API reason
  bounds it at 0.23.1; the declared floor is 0.24, the epoch this release
  builds against (a caret bucket spanning two epochs breaks consumers of
  both lines — #858).
- **`hls-runtime` floor raised `0.5` -> `0.6`.** `src/catchup.rs` uses
  `hls_runtime::server::ClosedSegment`/`HlsOrigin::closed_segments()`, which
  do not exist in published hls-runtime 0.5.0 (verified: 0 hits at the
  hls-runtime-v0.5.0 tag, 7 at HEAD). multimux declared `"0.5"`, so a
  consumer resolving 0.5.0 would fail to compile multimux; hls-runtime's own
  0.5.0 -> 0.6.0 minor (new public API) ships alongside this fix.
- **`broadcast-auth` floor raised `0.2` -> `0.3`.** `config.rs` uses
  `Verifier::signed_url` / `SignedUrlKeySet`, which were added in 0.2.1
  (#747). The `"0.2"` bound admitted 0.2.0, which does not have them, so
  multimux 0.8.0 shipped declaring a requirement weaker than its real one and
  fails to build against any lock that resolves 0.2.0. Hit for real while
  bumping `acap-multimux`, where the lock picked the floor. The API reason
  bounds it at 0.2.1; the declared floor is 0.3, the epoch this release
  builds against.
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Fixed

- **One authenticated `POST /admin/routes` could permanently disable the admin
  API** (requires the `whep` feature). `spawn_route` filtered a route's outputs
  by `!k.is_push()` alone, where the startup path filters
  `!k.is_push() && !k.is_whep()`. A `whep` output therefore reached
  `build_output`, which is `unreachable!()` for it — WHEP egress is driven by a
  raw listen socket, not an `Arc<dyn Output>` — and `validate_standalone` does
  not reject such a route, since it only checks the listen address.

  The panic is not the damage; where it panics is. `add_route` holds
  `self.inner.write()` across `spawn_route`, so the unwind **poisoned the
  registry lock**, and every later admin operation and router rebuild calls
  `.expect("RouteRegistry::inner lock poisoned")`. One request disabled the
  admin API for the remaining lifetime of the process. The `reload` path calls
  `spawn_route` without the lock and so lost only that one request.

  A one-word divergence between two filters that had to agree. The regression
  test asserts the registry is still usable afterwards — poisoning is what it
  destroyed, so a test that only asserted "no panic" would not have caught it.

- **A wrong RTSP password retried forever instead of failing the route**
  (issue #957, found end-to-end against a real Axis camera). `origin::
  supervisor::supervise_driver` treated every ingest failure as transient,
  including `MultimuxError::Auth` (a `401`/`403` that persisted after
  credentials were supplied) — a wrong password is never going to start
  working, so it just produced an endless warn-level reconnect loop. Auth
  failures (and a `404 Not Found` specifically on RTSP DESCRIBE, a wrong
  URL path — also never self-healing) now stop the supervisor outright and
  mark the route `HealthState::Failed`, surfaced by the admin API's route
  status without reading logs. A camera still finishing its boot can
  transiently answer `401` before its auth subsystem is ready, so auth
  failures are tolerated for a bounded number of consecutive attempts
  (`MAX_AUTH_ATTEMPTS_BEFORE_PERMANENT`, 5, chosen against the default
  backoff schedule's ~15.5s cumulative delay by attempt 5) before being
  declared permanent — every other failure kind keeps the existing
  supervised reconnect, unchanged.
- **`catchup::read_archived_bytes` allocated whatever a corrupt `pN.idx`
  sidecar claimed.** `byte_len` comes from a DVR-archive `IndexEntry` in a
  JSON sidecar on disk; `read_exact` caught a truncated container, but
  nothing bounded `byte_len` against the period file's real size *before*
  `vec![0u8; byte_len as usize]` allocated for it — a corrupt-but-JSON-valid
  sidecar (the same power-loss/full-disk class the DVR index-rebuild fix
  above addresses) meant an unbounded allocation. Now bound against
  `File::metadata().len()` (with a `checked_add` guard on `offset + len`
  too) before allocating, returning an error instead.
- **One unauthenticated datagram could tear down a live WHIP ingest.**
  `handle_datagram` returns `Err` on any SRTP authentication failure, and the
  WHIP read loop mapped that to a fatal transport error, reaping the session
  — but the post-handshake media socket accepts datagrams from anyone (no
  source-address/ICE-pair check), so a single garbage datagram in the RFC
  5764 §5.1.2 SRTP band ended an established ingest. Now logged at debug and
  skipped, matching how `output::whep` already handled the identical error;
  a genuine socket-level read error stays fatal.
- **Off-by-four panic in DVR index rebuild on a truncated period file.**
  `rebuild_index` guarded with `mdat_offset + 4 <= data.len()` but then read
  `&data[mdat_offset + 4..mdat_offset + 8]` — eight bytes behind a four-byte
  bound. A period file truncated between a `moof` and the `mdat` four-CC (a
  routine shape after a power loss or full disk mid-write — exactly when
  `rebuild_index` exists to recover) panicked instead of recovering.
- `routes.dvr` was validated (`Route::validate_dvr`) but never actually
  wired into the `RouteHandle` built for a config-driven or admin-API-added
  route — `RouteHandle::with_dvr` was only ever called from this crate's
  own unit tests, so DVR recording configured via `Config`/JSON silently
  never ran. Both `origin::serve_with_registry_impl` and
  `origin::admin::RouteRegistry::spawn_route` now call `.with_dvr(route.dvr.clone())`;
  found while implementing catch-up serving (issue #900), which depends on
  DVR actually recording.
- RTMP push (issue #934) shipped raw MPEG-2 TS as an RTMP `send_video`
  message payload — no RTMP server can decode that; RTMP Audio/Video
  messages carry FLV `AudioTagHeader`+`AACAUDIODATA` /
  `VideoTagHeader`+`AVCVIDEOPACKET` bodies. `PushTransport` gained
  `send_media` (default: mux with `TsMux` and send the blob, what SRT/RTSP
  push both want); `RtmpTransport` overrides it to split each batch into
  FLV-framed payloads (`transmux::flv_frame_payloads`) dispatched through
  `send_video`/`send_audio`, and its `setup` now sends `onMetaData` plus the
  AVC/AAC sequence headers once, before any frame data. Verified against a
  real, independent RTMP server implementation over a real TCP loopback
  connection (`tests/push_rtmp.rs`), not just an inspection of the client's
  own byte construction.
- RTMP push's `app`/`stream_key` were derived wrong from the push URL: `app`
  took the *whole* path and `stream_key` was always empty. An RTMP URL is
  `rtmp://host/app/streamkey` (or `.../app/instance/streamkey`) — the last
  path segment is now the stream key, everything before it the app.

## [0.8.0] - 2026-08-07

### Added
- **Push re-egress outputs** (issue #744): `OutputKind::SrtPush`,
  `OutputKind::RtmpPush`, `OutputKind::RtspPush` for relaying ingested media
  to downstream servers — turns multimux from an HTTP-only origin into a
  relay/gateway.
- `PushFormat` config enum (`Ts`, `Mp4`, `Mkv`) for per-output container
  format selection.
- `ReconnectPolicy` config for exponential-backoff reconnect on push outputs.
- SRT push transport (SRT Caller mode to a remote SRT Listener).
- RTSP push transport (client ANNOUNCE/RECORD to a remote RTSP server).
- RTMP push transport (client connect/createStream/publish to a remote RTMP
  server).
- `RouteHandle::await_first_trunk()` for push tasks to discover program
  availability.
- Supervisor lifecycle: push tasks spawn at route creation, cancel+join on
  route removal.

### Changed
- Requires `rtmp-runtime` 0.5 and `rtsp-runtime` 0.5 (new client-side
  publish APIs used by the push transports).

## [0.7.0] - 2026-08-05

### Added
- **MPTS (multi-programme transport stream) ingest** (issue #906):
  `ProgramTracker` now groups tracks by `TrackSpec::program_number`, producing
  one `ProgramId` per distinct TS programme. Every real DVB-T/S/C multiplex is
  MPTS — this was the sole gap preventing real DVB ingest. Non-TS sources
  (`program_number: None`) continue to collapse into one `ProgramId`.
- Mid-stream track additions (PMT version changes adding an elementary stream)
  now reach the running segmenter (issue #781). Previously, `track_specs` was a
  one-shot snapshot consumed at segmenter construction; a broadcaster adding an
  audio language or subtitle track mid-programme was logged and its samples
  silently dropped. Now `ProgramTracker` maps mid-stream additions to the same
  program, emits `SessionEvent::TracksChanged` with the complete track set, and
  `drive_program_segmenters` detects the `Trunk`'s `track_generation` change to
  admit the new track into the segmenter (or rebuild it, for fMP4) at the next
  segment boundary — with no media-sequence reset and no interruption to
  existing tracks. DASH output explicitly logs and continues serving existing
  tracks (adding a representation mid-stream needs a new `Period`, which is
  tracked for a follow-up).
- **Smooth Streaming output** (`OutputKind::Smooth`, config token `"smooth"`,
  issue #742): a route configured with it serves an MS-SSTR client Manifest
  (`/Manifest`) and fragment responses at the Smooth URL shape
  (`QualityLevels({bitrate})/Fragments({type}={start time})`), sharing the
  same fMP4 segment bytes every other output reads from the `Trunk`.
- **DVR durable segment archive** (issue #746): a per-route `dvr` config block
  with `enabled`, `archive_root`, `retention_periods`/`retention_bytes`,
  `period_duration_secs` (default 3 hours), and `overrun`
  (`"gap"`/`"stall"`/`"terminate"`). Finished segments are appended to a
  **period container file** (`<archive_root>/<route>/pN.<ext>`) — one file
  per period epoch, not one per segment. For fMP4, the init segment is
  written at the head of the file and media fragments follow; the resulting
  file is a valid CMAF track (init + concatenated fragments). For MPEG‑TS,
  segments are natively concatenable 188‑byte packets. A byte-range index
  sidecar (`pN.idx`) maps `(seq, pts, offset, len)` for O(1) lookup (issue
  #900) and is rebuildable by rescanning the period file. A new period is
  rolled on duration expiry or on fMP4 init change (mid-stream track
  addition — issue #781). Retention operates on whole periods, quantised
  to the configured duration. Recording is a `SegmentEgress` implementation
  draining its own pinning `SegmentCursor` — it never holds a lock the
  live-serving path needs and never perturbs live output.

## [0.6.0] - 2026-08-02

### Added
- **Classic MPEG-TS HLS output** (`OutputKind::TsHls`, config token
  `"ts_hls"`, issue #887): a route configured with it is served with whole
  `.ts` media segments (RFC 8216 §3/RFC 8216bis §3.1.1) instead of fMP4,
  self-initialising (no `#EXT-X-MAP`, no init segment) and classic (no
  low-latency parts — `transmux::ts_hls::StreamingTsHlsSegmenter` has no
  partial-segment concept). Container is a **per-route**, not per-output,
  property (`route::RouteHandle::with_container`): `"ts_hls"` is mutually
  exclusive with `"llhls"`/`"dash"`/`"ll_dash"` on the same route, rejected by
  `Config::validate()` at load time (a `media_plane::Trunk` has one segment
  ring per program — a program's samples are segmented into fMP4 *or* TS,
  never both, without a second ring; run two routes against the same source
  if both containers are needed today). See `output::ts_hls` and the
  README's new "Classic MPEG-TS HLS output" section.
- `OutputAuthSpec::SignedUrl` (issue #747): configures
  `broadcast_auth::Verifier::signed_url` as a route's output-auth scheme —
  `{ "scheme": "signed_url", "keys": [{ "kid": "...", "secret": "..." }, ...] }`.
  Multiple `keys` entries let secrets rotate without invalidating URLs signed
  under an older, still-listed key. `validate()` rejects an empty `keys`
  list, an empty `kid`, or a `secret` shorter than
  `broadcast_auth::SignedUrlKeySet::MIN_SECRET_LEN` (32 bytes) at config-load
  time, not per-request.
- **Runtime admin API** (issue #749): add/remove/list routes and reload the
  config file without restarting the origin — restarting previously dropped
  every live viewer on every route, not just the one being changed. Opt-in
  via the new `Config::admin` field (`AdminSpec { bind, auth }`), which binds
  a **separate** listener from the media port and requires auth (`auth` is
  mandatory, not `Option`) — starting the admin API unauthenticated is
  impossible by construction. Endpoints: `GET /admin/routes`,
  `GET /admin/routes/{name}`, `POST /admin/routes` (body: the same `Route`
  config shape, `409` on a duplicate name), `DELETE /admin/routes/{name}`
  (`404` if unknown; drains the removed route's supervisor without
  disturbing any other route), `POST /admin/reload` (re-reads the config
  file and converges added/removed/changed routes — an unchanged route is
  never restarted). See `crate::origin::admin` and the README's new
  "Runtime admin API" section.
- New `origin::serve_config_file`/`serve_config_file_with_registry` entry
  points: load a JSON config from a path and remember it, so
  `POST /admin/reload` has a file to re-read. `multimux-cli --config <FILE>`
  now goes through this path.
- `config::Route`/`InputSpec`/`AuthSpec`/`output::OutputKind` now derive
  `PartialEq` (used by the admin reload diff to detect an unchanged route).
- `error::MultimuxError` gained `RouteExists`/`RouteNotFound` variants (the
  admin API's `409`/`404` mappings).

### Changed
- `source::hls_pull` builds/parses HLS playlists via `broadcast-hls` directly
  instead of reaching through `transmux::hls` for it (issue #878). No public
  API or behaviour change.

## [0.5.2] - 2026-07-30

### Fixed
- Floor `media-plane` to `0.1.1`. The `^0.1` bucket also contains 0.1.0,
  which is built against `transmux` 0.20, so a consumer could resolve two
  `transmux` minors into one graph and hit trait-resolution errors pointing
  at this crate's internals (#858).

## [0.5.1] - 2026-07-30

### Fixed
- Smooth-pull ingest (`source::smooth_pull`) now skips any `StreamType`
  other than `Video`/`Audio` at manifest-parse time (previously only `Text`
  was filtered, so a future `StreamType` variant would have reached an
  exhaustive match and panicked). Surfaced by `transmux`'s `StreamType`
  gaining `#[non_exhaustive]` (issue #806).
- `output::llhls`/`origin::resource` now handle a future `LlHlsBody` variant
  defensively (same status as the existing `Resource`/`Playlist` fallback),
  consistent with `ll-hls-runtime`'s `LlHlsBody` gaining `#[non_exhaustive]`.
- **DASH and LL-DASH manifests returned 503 forever on every driver-backed
  route** (shipped in v0.5.0). `RouteHandle::set_track_specs` had no
  production call site — `report_driver_progress` now syncs track specs
  from each published program's `Trunk` into the route on every poll, using
  `track_generation()` to avoid redundant syncs (issue #831).

### Added
- `tests/label_coverage.rs` drift guard (issue #806).

## [0.5.0] - 2026-07-28

### Changed (BREAKING — pre-publish hardening)
- **Five publicly reachable enums in `crate::source` are now
  `#[non_exhaustive]`**: `DashResourceId`, `DashAction` (`source::dash_pull`),
  `HlsFetchId` (`source::hls_pull`), `SmoothResourceId`, `SmoothAction`
  (`source::smooth_pull`). Each answers "which resource am I fetching" or
  "which action next" for a pull protocol, so each is exactly the kind of enum
  that gains a variant as a protocol's surface is covered more fully — and
  adding a variant to a published exhaustive enum is a breaking change.
  Bundled into 0.5.0, which is already breaking, so it costs downstream
  matchers nothing beyond the wildcard arm they need for this release anyway;
  deferring it would have required a further breaking bump later purely to
  attach an attribute.
  - `media-plane` was audited the same way and needed nothing: every public
    enum there already carries it.

### Added (issue #805 task 6: per-program serving state, MPTS-ready)
- **`RouteHandle` gains per-program serving state.** A new crate-private
  `ProgramServing` bundle groups one program's `Trunk`, its
  `LlHlsOrigin`, and its `DashState` together, keyed by `ProgramId` in
  `RouteHandle`'s registry — replacing the single owned `Trunk`/`ll_hls`/
  `dash` triple `RouteHandle::new` used to build eagerly. A `ProgramServing`
  bundle is created the instant (and only the instant)
  `RouteHandle::publish_program` is called for that program, mirroring
  `media_plane::IngestDriver` minting a `Trunk` the instant it observes
  `SessionEvent::NewProgram`. Proven end to end by a new test
  (`route::program_registry_tests::two_programs_serve_distinct_media`): two
  programs on one route now serve genuinely distinct init bytes and segments.
- **`RouteHandle::publish_new_program(program) -> Arc<Trunk>`** (new, `pub`):
  mints a `Trunk` sized like the route's own configured ring capacities and
  publishes it under `program` in one step — the test/plugin-facing
  replacement for the deleted `publish_owned_trunk`, and the only way to get
  a `Trunk` handle back from a `RouteHandle` (it is always already
  registered).
- **`RouteHandle::name()`/`with_name()`** (new, `pub`): a route's own name,
  defaulting to `"unknown"`. `crate::origin::serve_with_registry`'s one
  production route-construction call site now chains `.with_name(route.name)`
  so `crate::source::segment::drive_program_segmenters` (issue #809, below)
  can label its metrics without threading a name parameter through every
  `run_*` call site.
- **`crate::source::{DriverProgress, advance_route}`** (new, `pub`): the one
  facade a driver-backed drive loop (in-tree `run_*`, or an external
  `SchemeRegistry` `Custom` factory) now calls once per iteration, replacing
  the caller-assembled pair `report_driver_progress` +
  `segment::drive_program_segmenters` those two functions used to require
  (both narrowed back to `pub(crate)` — see Changed, below). `DriverProgress`
  is the one opaque per-attempt state value a caller declares and threads
  through; `advance_route` performs both steps, in order, every time — a
  wrong order or a skipped step (the exact footgun the old two-call API
  invited) is no longer possible. `examples/custom_scheme.rs` and
  `tests/dispatch_ingest.rs`'s `Custom`-dispatch coverage are rewritten onto
  this facade.

### Fixed (issue #804)
- **`rtsps://` (RTSP over TLS) ingest works again.** The step-5a port left
  `source::rtsp::run_rtsp` wiring only a plain `TcpStream`, and refusing an
  `rtsps://` URL outright with an error quoting an internal task number — a
  user-visible capability regression against 0.4, on a scheme that is common
  on IP cameras. `run_rtsp` now branches on the URL scheme
  (`RtspDialer::is_tls`, already present and tested) and, for `rtsps://`,
  TCP-connects then performs a real `tokio_rustls` handshake before any RTSP
  is exchanged — trusting the public-CA `webpki-roots` bundle via
  `rtsp_runtime::io::default_tls_client_config`, the same config
  `rtsp_runtime::io::AsyncRtspClient::connect_tls_with` uses. Both transports
  are erased to a boxed `AsyncRead`/`AsyncWrite` pair, so the sans-IO
  `poll_transmit`/`feed` drive loop is written once and never has to know
  which one it got. Gated behind this crate's existing default-on `tls`
  feature; with `--no-default-features`, an `rtsps://` route fails fast with
  an error naming the missing feature.
  - The interim refusal did at least **fail safe** — it never opened a
    plaintext socket to a TLS port — and that property is preserved and now
    *asserted* rather than assumed: a new test points a real `rtsps://` route
    at a deliberately-plain loopback listener and checks the bytes the
    listener actually received begin with a TLS handshake record (`0x16`,
    major version `0x03`), never the ASCII first byte of an RTSP request
    line.
  - **SNI derivation restored.** `sni_server_name` (dropped in the 5a port,
    and *not* relocated — `AsyncRtspClient::connect_tls_with` takes a
    caller-supplied `server_name` and does no bracket-stripping of its own)
    strips brackets from IPv6 literals, since rustls'
    `ServerName::try_from` rejects them: `[2001:db8::1]` -> `2001:db8::1`,
    with hostnames and IPv4 literals passed through unchanged. Without it,
    an `rtsps://` camera addressed by IPv6 literal would fail — and *only*
    that case, which ordinary testing misses. Also exposed as
    `RtspDialer::sni_server_name`.
  - A real TLS handshake is exercised in-tree: a new loopback test runs a
    genuine `tokio_rustls` server (self-signed `CN=localhost` fixture cert,
    shared byte-for-byte with `rtsp-runtime`'s own TLS loopback test) and
    drives a full DESCRIBE -> SETUP -> PLAY -> interleaved-RTP-depayload
    exchange over the encrypted socket, asserting a real depayloaded sample
    lands in the `Trunk`.

### Fixed (issue #809)
- **`multimux_parts_produced_total`/`multimux_segments_produced_total` have
  an emitter again.** These two counters had no emitter at all since the
  media-plane port — they read zero while media flowed perfectly (worse than
  absent) before being deleted outright pending this fix (see the entry
  below). `crate::source::segment::drive_program_segmenters` — the one place
  in the driver-backed architecture that actually turns samples into
  parts/segments — now bumps both, labelled by the new `RouteHandle::name()`
  (see Added, above) rather than threading a route-name parameter through
  nine `run_*` call sites. A new test
  (`source::segment::tests::drive_program_segmenters_bumps_parts_and_segments_produced_counters`)
  asserts both counters actually increment for a driver-backed route.
  Swept the rest of `crate::prometheus` for the same failure mode: `ROUTE_UP`/
  `SOURCE_RECONNECTS_TOTAL` (`origin::supervisor`), `ACTIVE_BLOCKING_REQUESTS`
  (`origin::resource`/`output::llhls`), and `HTTP_REQUESTS_TOTAL`/
  `HTTP_REQUEST_DURATION_SECONDS`/`BYTES_SERVED_TOTAL` (`origin`'s HTTP
  middleware) all still have live emitters — no other casualty found.

### Removed (BREAKING — issue #805 task 6: the placeholder `Trunk` is gone)
- **`RouteHandle`'s owned placeholder `Trunk` field and `publish_owned_trunk`
  are deleted.** `RouteHandle::new` no longer builds any `Trunk`/`LlHlsOrigin`/
  `DashState` at all; every program's serving state lives only in the new
  per-program registry (see Added, above), created only by
  `publish_program`/`publish_new_program`. This makes the **publish-or-hang
  footgun structurally impossible** rather than merely documented against: a
  producer can no longer write into a `Trunk` that egress cannot resolve,
  because there is no `Trunk` to write into until it is published. Callers
  that used to build a route, write via `set_init`/`add_part`/`add_segment`,
  then call `publish_owned_trunk()` now call `publish_new_program(program)`
  **first** (it both mints and publishes), then write.
- **`RouteHandle::set_init`/`init_bytes`/`set_track_specs`/`track_specs`/
  `add_part`/`add_segment`/`window_segments` all take a new leading
  `ProgramId` parameter.** These used to operate on the single owned `Trunk`;
  they now resolve (or, for the writers, silently no-op with a logged
  warning if unpublished) the named program's `ProgramServing` bundle. Every
  in-tree egress call site resolves `crate::route::SPTS_PROGRAM_ID` (the
  SPTS default, unchanged behaviour for every existing route) via the new
  `crate::http::resolve_route_program` (renamed from `resolve_route_trunk`,
  which returned just the `Trunk`; the renamed function returns the whole
  `ProgramServing` bundle so a caller never risks pairing one program's
  `Trunk` with a different program's `LlHlsOrigin`).
- **MPTS addressing is documented, not implemented** (as scoped): with
  per-program serving state in place, a route can genuinely serve several
  programs, but there is still no way for an HTTP request to *select* a
  non-default one. `RouteHandle`'s own module doc records three options (URL
  path segment, query parameter, config-declared per-program route) and
  recommends the query parameter as the additive "MVP" choice, plus a known
  gap: `ProgramResolution::NotYetAnnounced` vs. `NotFound` is derived from
  "the registry is empty", which cannot yet distinguish "this MPTS program
  hasn't been minted yet" from "this program will never exist" for a
  route where at least one *other* program has already landed.

### Fixed (issue #808)
- **Samples published in the SAME `feed` call as `NewProgram` are no longer
  silently dropped.** `ProgramSegmenter::try_new` now subscribes with
  `media_plane::trunk::Trunk::subscribe_from_backlog` instead of `subscribe`:
  the driver's own feed batch that announces a program routinely carries its
  first samples too (a single MPEG-TS feed of 64 packets commonly carries
  the PMT and the first PES packets together), and those samples were
  already sitting in the ring by the time `drive_program_segmenters` built
  the segmenter — a live-tail `subscribe()` cursor never observed them. If
  the dropped batch held the opening IDR, the first segment either started
  on a non-keyframe or was delayed; this was silent (no error, no log).
  `examples/custom_scheme.rs` and `tests/dispatch_ingest.rs` no longer split
  their announce/sample script across two `feed` calls to work around this —
  both now announce and publish in one call, the ordinary shape.

### Removed (BREAKING — issue #805 task 5/6: convergence)
- **`SourceConnector`, `supervise`, and the whole `pipeline` module are
  deleted.** Every input kind now dials/listens over
  `media_plane::ingress`'s `Dialer`/`Listener` + `IngestSession` traits, driven
  by `supervisor::supervise_driver` — RTMP (task 4) was the last holdout, and
  once it moved, `SourceConnector`/`supervise` (and the `pipeline::SampleSource`/
  `run_pipeline`/`MockSource` trio it drove) had no remaining caller.
  - `pub use origin::supervisor::{Backoff, SourceConnector, supervise};` is now
    `pub use origin::supervisor::{Backoff, supervise_driver};` — a downstream
    crate implementing `SourceConnector` or calling `supervise`/
    `multimux::pipeline::*` directly no longer compiles against this crate;
    port onto `supervise_driver` over your own `Dialer`/`IngestSession` (see
    `examples/custom_scheme.rs`, rewritten to demonstrate exactly this).
  - The `testsupport` feature and the `serve_mock` example are removed with
    it: both existed solely to gate `pipeline::MockSource`.
- **`RouteHandle`'s owned `Trunk` field now has no production writer.** It
  stays (removing it forces `ll_hls`/`dash` to be built per-program instead of
  once in `RouteHandle::new`) but is now a pre-first-program **placeholder**,
  driven only by this crate's own `tests/*.rs` (via `set_init`/`add_part`/
  `add_segment` + `publish_owned_trunk`, which stays `pub` for exactly that
  reason). See `RouteHandle`'s own doc. **Superseded by issue #805 task 6,
  below: the placeholder and `publish_owned_trunk` are deleted outright.**
- **`crate::prometheus::{SEGMENTS_PRODUCED_TOTAL, PARTS_PRODUCED_TOTAL}`
  removed** (the two counters became dead code — the deleted `run_pipeline`
  was their only caller; no driver-backed `run_*` path ever bumped them, since
  `crate::source::segment::ProgramSegmenter` has no route name to label them
  with). Restoring `multimux_segments_produced_total`/
  `multimux_parts_produced_total` for the driver-backed architecture is a
  separate, unscoped follow-up (threading a route name through
  `drive_program_segmenters`/`ProgramSegmenter`, touching every `run_*` call
  site) — flagged here rather than silently dropped or hastily wired up
  underneath this task.

### Changed (issue #805 task 5/6: the plugin extension point)
- **`crate::source::report_driver_progress` and
  `crate::source::segment::{ProgramSegmenter, drive_program_segmenters}` are
  now `pub`** (were `pub(crate)`). These are the two per-iteration calls every
  in-tree driver-backed `run_*` makes; with `SourceConnector`/`supervise`
  gone, they are also the *only* way an external `SchemeRegistry`-registered
  `Custom` input factory driving its own `Dialer`/`IngestSession` can publish
  its ingest into `RouteHandle`'s (crate-private) program registry and turn
  its samples into LL-HLS-servable segments/parts — without this, the
  extension point documented in `crate::registry`/`examples/custom_scheme.rs`
  would be unusable for anything beyond a trivial connect-only stub.
  **Superseded by issue #805 task 6, above: narrowed back to `pub(crate)`
  behind the single `crate::source::advance_route` facade.**
- **`examples/custom_scheme.rs` rewritten** to demonstrate the supported
  plugin shape: a small `Dialer`/`IngestSession` pair (`DemoDialer`/
  `DemoSession`, synthetic single-track AVC media) driven by
  `supervise_driver`, publishing through `report_driver_progress` +
  `drive_program_segmenters` exactly like a built-in source. The example now
  actually invokes the registered factory and waits for real init bytes to
  land, rather than only checking registry lookup + config parsing. The
  `"silence"`-tagged scheme is renamed `"demo"` (`examples/custom-scheme.json`
  updated to match) since it now carries real synthetic media, not silence.
- **`examples/serve_mock.rs` deleted** rather than rewritten: its
  demonstration value (drive a synthetic ingest end to end, serve it over a
  real HTTP origin, no camera/ffmpeg needed) is now covered by
  `examples/custom_scheme.rs` (same Dialer/IngestSession/supervise_driver/
  segmenting mechanics) together with `tests/dispatch_ingest.rs`'s real
  end-to-end HTTP tests (`ts_udp`/`ts_http`/`rtmp` dispatch tests already
  serve real — not synthetic — fixture-derived media over real HTTP).
- **`tests/dispatch_ingest.rs`'s `InputSpec::Custom` coverage retargeted onto
  the new plugin shape**: `custom_dispatch_drives_a_driver_backed_source_and_serves_real_media`
  replaces `custom_dispatch_drives_run_pipeline_and_serves_real_media`,
  registering a `Custom` factory that spawns `supervise_driver` over a small
  `IngestSession` fed the real (demuxed, not synthetic) `h264_aac.ts` fixture
  — the dispatch path (`InputSpec::Custom` -> `SchemeRegistry` -> `InputCtx`
  -> factory -> real HTTP `#EXTINF:`) is still fully covered, just through
  the surviving architecture.
- **`tests/origin_llhls.rs`'s first test retargeted** onto a real
  `LlHlsSegmenter` fed directly (mirroring `tests/lldash_dashjs.rs`'s own
  `run_live_producer`) in place of the deleted `run_pipeline`/`MockSource`;
  the file's `#![cfg(feature = "testsupport")]` gate is removed (nothing in
  it needs `MockSource` any more).
- **`multimux::origin::supervisor::supervise_driver` gains direct unit
  test coverage** (`origin::supervisor::tests`): a fake `attempt` closure
  (replacing the deleted `SourceConnector`-based `FlakyConnector`/
  `PacedFlakyConnector` mocks) proves reconnect-after-failure,
  reconnect-after-live-attempt-ends, and shutdown-cancels-mid-backoff —
  properties `supervise`'s own tests used to prove for the now-deleted loop.

### Added (in progress — issue #805, task 1 of 6)
- **`RouteHandle` gained a `ProgramId -> Arc<Trunk>` registry**, the first step
  of converging multimux's two ingest architectures onto one. `publish_program`
  is the ingest-side write; `resolve_program` returns a typed three-case
  `ProgramResolution` — `Found(Arc<Trunk>)`, `NotYetAnnounced`, `NotFound`.
  Held in an `RwLock<HashMap<..>>` because resolution is the hottest read path
  once egress is wired to it (every served request, every viewer) while
  publication is rare and bounded by `IngestDriver`'s `max_programs`.
  - The three cases are deliberately **not** an `Option`: "this route is
    connected but no program has appeared yet" is a wait, whereas "no such
    program" is a 404, and collapsing them would make a still-connecting
    route indistinguishable from a typo in a request path.
  - The registry carries no single-program assumption, so MPTS support (one
    route, N programs, one handle) is an addition rather than a reshaping.
  - Additive only: `RouteHandle` still owns its legacy `trunk` field and
    egress still reads it, so nothing changes behaviourally yet.

### Fixed
- **`pipeline::run_pipeline` published nothing, so every consumer of it hung.**
  Once egress resolved exclusively through the route's program registry
  (task 2), a producer that writes `RouteHandle`'s own `Trunk` without
  indexing it there is served to nobody — and because a request then blocks on
  `ProgramResolution::NotYetAnnounced` waiting for a program that is already
  present, the symptom is an **infinite hang, not a 404**. `run_pipeline` is a
  public entry point and never published. It now calls
  `RouteHandle::publish_owned_trunk()`, matching `origin::supervisor::supervise`.
  - Caught by `ll-hls-runtime`, which dev-depends on this crate and drives
    `RouteHandle` + `LlHlsOutput` directly: its `glass_to_glass` test tripped a
    25 s hang guard and `golden_gate` a 20 s one, both reproducibly, both green
    before task 1/2 landed. An external consumer found this, not our own suite.
  - `RouteHandle::new`'s docs now state the contract and that the failure mode
    is a hang. `new()` deliberately does **not** auto-publish: a bare,
    unpublished route is the genuine driver-route connecting window that the
    four `NotYetAnnounced` → 503 tests assert, and the owned field carrying
    this hazard goes away once every route is driver-backed.
- **The workspace doc gate is green again (26 `error:` lines → 0).** The 5a/5b
  port renamed and deleted types without updating the prose that referenced
  them, leaving dead intra-doc links across twelve files: `RtspSource` (split
  into `RtspDialer` + `RtspIngestSession`), `RtpUdpSource`/`TsUdpSource`/
  `TsHttpSource`/`SrtSource` (renamed to their `*Route` config types), and
  `crate::store::MediaStore`/`HealthState` (that module was deleted outright;
  `HealthState` now lives at `crate::route::HealthState`). Public docs also
  linked to private items (`crate::http::resolve_blocking`,
  `into_response`, `select_representable_track`), which are now plain
  backticked code rather than links — no API was widened to satisfy rustdoc.
  - One correction went beyond relinking: `serve_with_registry`'s docs claimed
    every `InputSpec` variant dispatches through `supervisor::supervise`. Only
    `Rtmp` and `Custom` do. The prose now says so (see issue #805).
### Changed (issue #805 task 2/6 — wire the eight `media_plane`-ported inputs)
- **All nine ingest input kinds now genuinely ingest and serve, via
  `RouteHandle`'s program registry** (task 1 added the registry additively;
  this wires both sides of it). `rtsp`/`rtp`/`ts_udp`/`ts_http`/`srt`/
  `hls_pull`/`dash_pull`/`smooth_pull` were ported onto
  `media_plane::ingress::{Dialer, IngestSession, IngestDriver}` at plan step
  5a but left unreachable from `origin::serve_with_registry` — a combined
  match arm logged an error and spawned a no-op. That stub is deleted.
  - New `origin::supervisor::supervise_driver`: the driver-backed sibling of
    `supervise`, reusing its exact operational shape (`Backoff` between
    attempts, `RouteHandle::health` transitions, `record_route_up`/
    `record_reconnect`, a cancellable shutdown `watch::Receiver<bool>`) for
    the eight input kinds whose `run_*` entry point fuses dial+drive into one
    call rather than exposing a separate `SourceConnector::connect()` step.
  - New `crate::source::report_driver_progress`, called by every
    `run_rtsp`/`run_rtp_udp`/`run_ts_udp`/`run_ts_http`/`srt::drive_socket`/
    `run_hls_pull`/`run_dash_pull`/`run_smooth_pull` from inside its own drive
    loop: flips the route to `Live` the moment its `IngestDriver` establishes,
    and publishes each newly-announced program's driver-minted `Trunk` into
    the route's registry (`RouteHandle::publish_program`). All eight `run_*`/
    `drive_socket` entry points gained a `route_handle: &Arc<RouteHandle>`
    parameter for this (source-breaking for direct callers of those `pub`
    functions).
  - `origin::serve_with_registry`'s per-route ingest wiring moved into a new
    `spawn_ingest` helper, one arm per `InputSpec` variant, so it is
    individually testable without a real HTTP server.
- **Egress now resolves every route through the registry, uniformly** —
  migrated the five `RouteHandle::trunk()` call sites
  (`output::llhls::media_playlist`, `output::dash::manifest`,
  `output::ll_dash::manifest`, `origin::resource::dynamic_file`/`fetch_part`)
  onto a new shared `crate::http::resolve_route_trunk`, which resolves
  `RouteHandle::SPTS_PROGRAM_ID` (the single-program-route default; MPTS
  resolution is task 6) and maps the three-way
  `RouteHandle::ProgramResolution`: `Found` resolves and serves;
  `NotYetAnnounced` (connected, but no program has appeared yet) is a `503`
  "not ready", **not** a `404`; `NotFound` is a genuine `404`. The now-unused
  `RouteHandle::trunk()` accessor was deleted (the owned `Trunk` **field**
  stays — task 5 removes it).
  - So RTMP/`Custom` (the old `SourceConnector`-fed path) keep serving through
    this same migration with no fallback branch in egress:
    `origin::supervisor::supervise` now calls a new, `pub`
    `RouteHandle::publish_owned_trunk()` right after reaching `Live`,
    publishing its own owned `Trunk` into the registry under
    `SPTS_PROGRAM_ID`. `pub` (not `pub(crate)`) because this crate's own
    `tests/*.rs`/examples that drive `crate::pipeline::run_pipeline` directly
    (bypassing `supervise`) call it explicitly to stay resolvable.

### Added (issue #805 task 2b/6 — close the segmenter gap)
- **The eight driver-backed inputs now actually produce segments/parts, not
  just samples.** Task 2 wired ingest (each publishes its driver-minted
  `Trunk` into `RouteHandle`'s registry) and made egress resolve through that
  registry, but nothing turned the raw samples the driver publishes into
  segments/parts — a driver-backed route's LL-HLS/DASH playlists came back
  empty. New `crate::source::segment::ProgramSegmenter` +
  `drive_program_segmenters` close it: one `ProgramSegmenter` per announced
  `ProgramId`, subscribing a `SampleCursor` to that program's driver-minted
  `Trunk`, feeding a `transmux::ll_hls::LlHlsSegmenter`, and publishing the
  resulting parts/segments back into **the same `Trunk`** via its own
  `Trunk::segment_writer()` — never a second `Trunk`, never copying samples
  between trunks (the decision recorded in
  `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §8).
  Segmenting is per-program, so an MPTS route (several programs on one
  `IngestDriver`) segments each independently; only `SPTS_PROGRAM_ID`'s init
  bytes are wired to `RouteHandle`'s single-slot LL-HLS/DASH accessors today
  (MPTS egress resolution is task 6). All eight `run_*`/`drive_socket` entry
  points call `drive_program_segmenters` right alongside
  `report_driver_progress`, and again after `driver.finish()` where the loop
  shape made that easy (`ts_http`/`srt`/`hls_pull`/`dash_pull`/`smooth_pull`/
  `rtsp`), so a cleanly-ending session flushes its trailing partial segment
  instead of dropping it.
  - Killed the two remaining `MOVIE_TIMESCALE = 90_000` magic-number
    hardcodes (`source::ts_program`'s and `source::hls_pull`'s test fixtures)
    and `pipeline::run_pipeline`'s own copy, in favour of
    `transmux::VIDEO_CLOCK_RATE` — matching `source::smooth_pull`'s existing
    convention.
- **`RouteHandle` rebinds its `ll_hls`/`dash` to a driver-minted `Trunk`.**
  `LlHlsOrigin`/`DashState` are bound to one `Trunk` at construction, and
  `RouteHandle::new` builds them from its own placeholder `Trunk` before any
  program is known. `ll_hls`/`dash` are now `RwLock<Arc<..>>`, and
  `publish_program` — for `SPTS_PROGRAM_ID` only — rebuilds them over
  whichever `Trunk` was just published, but **only** if it is a genuinely
  different `Arc` from the one they are currently bound to (`Arc::ptr_eq`
  guard): a same-`Arc` republish (the legacy RTMP/`Custom` path always
  publishes this handle's own `trunk`, already what `ll_hls`/`dash` were
  built from) is a strict no-op, so it never discards init bytes/segments/
  parts a caller already wrote before publishing. New `active_trunk` field
  tracks which `Trunk` is authoritative for `latest_progress()`'s abuse-bound
  check. Without this, `ProgramSegmenter` publishing into the driver's own
  `Trunk` (as decided above) would never be visible to `route.ll_hls()`,
  which stayed bound to the route's own never-written placeholder forever.

### Added (issue #805 task 3/6 — the dispatch-path regression net)
- **Closed the hole that let eight of nine `InputSpec` variants dispatch to a
  no-op stub for a long time while build/clippy/doc and thousands of tests
  all stayed green** (see `docs/superpowers/specs/2026-07-26-media-plane-architecture.md`
  §8's own account): no test in the suite entered through
  `serve_with_registry`/`spawn_ingest` itself — every source was well-tested
  in isolation (`RtspDialer`/`RtspIngestSession` directly, a hand-built
  `RouteHandle`), but the dispatch that routes to them was tested by nothing.
  - **Layer 1 (exhaustive, cheap):** new
    `origin::tests::every_input_spec_variant_dispatches_to_real_ingest_not_a_stub`
    points every non-`Custom` `InputSpec` variant at a deliberately dead/
    unreachable endpoint (a refused TCP connect, a quiet UDP port, an SRT
    caller dialing nobody, or — for `Rtmp` — a listen port the test has
    already stolen) and asserts `RouteHandle::health` transitions
    `Connecting` -> `Reconnecting`; a stubbed arm would never touch
    `route_handle` at all, so it would still read `Connecting` at the hang
    guard. Exhaustive over the enum via a **compile-time** net: a second,
    independent exhaustive `match` over `InputSpec` (no `_ =>` arm) fails to
    compile the moment a new variant is added without also updating this
    test — stronger than any runtime assertion, and only possible because
    the test lives in-crate (`InputSpec` is `#[non_exhaustive]`, so an
    external `multimux/tests/*.rs` match would be forced to carry a
    wildcard arm that silently swallows a new variant).
  - **Layer 2 (deep, representative):** new `multimux/tests/dispatch_ingest.rs`
    drives real fixture bytes (`fixtures/ts/h264_aac.ts`, a real ffmpeg
    capture — never synthesised bytes) through `serve_with_registry` for
    `InputSpec::TsUdp` (a real UDP socket) and `InputSpec::TsHttp` (a small
    loopback HTTP server), and asserts a real HTTP `GET` of the resulting
    LL-HLS media playlist carries an actual `#EXTINF:` line (not merely the
    `#EXT-X-PART-INF`/`#EXT-X-MAP` headers a zero-segment route still emits)
    plus non-empty init/segment bytes.
  - **`pipeline::run_pipeline` coverage:** the same file's
    `custom_dispatch_drives_run_pipeline_and_serves_real_media` drives
    `run_pipeline` through the exact `InputSpec::Custom` ->
    `SchemeRegistry` -> `InputCtx` -> factory path a real embedding
    application uses, catching a regression of `run_pipeline`'s
    `publish_owned_trunk()` call (see the "Fixed" entry above) the same way
    a real deployment would notice it — every request hanging, not erroring.
  - Every new test is mutation-verified: the exact production stub/removed
    call was restored, the specific assertion/panic confirmed, then
    reverted — see each test's own `MUTATION VERIFIED` doc comment.

### Added (issue #805 task 4/6 — RTMP onto `media_plane::ingress::Listener`)
- **RTMP is off the old `SourceConnector`/`supervise` path** — the last of
  the nine input kinds to move onto `media_plane::ingress`, closing the
  "eight ported, one held back" gap tasks 2/2b/3 deliberately left open (see
  `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §8's
  sequencing note: RTMP was kept on the working old path until every other
  kind, and the registry reconciliation it depends on, was proven).
  - New `media_plane::ingress::ListenDriver::driver`/`driver_mut`/
    `reap_if_terminal` (in `media-plane`, additive): RTMP's `Listener::Session`
    is fed already-parsed-and-replied-to `rtmp_runtime::server::ServerEvent`s
    (`Stage::In<'a> = &'a [ServerEvent]`, not `&'a [u8]` — see
    `source::rtmp`'s module doc for why `AsyncRtmpServer`/`RtmpConnection`
    make that the honest shape), so `ListenDriver::feed`'s `&[u8]`-pinned
    convenience wrapper doesn't fit. These three accessors let a driving loop
    reassemble the same feed -> observe -> reap sequence for any `Stage::In`
    shape.
  - `source::rtmp::RtmpRoute` (replaces `RtmpSource`) implements
    `media_plane::ingress::Listener`: `AsyncRtmpServer::accept()` (blocking-
    async, no `try_accept`) is bridged into `Listener::poll_accept`'s
    non-blocking contract by an accept-pump task, spawned once alongside the
    bind-once listen socket, that loops `accept().await` and sends each
    `RtmpConnection` into a bounded `mpsc` channel; `poll_accept` drains it
    with `try_recv()`.
  - New `source::rtmp::run_rtmp`: a `ListenDriver`-backed loop that admits and
    drives up to `max_sessions` publishers **concurrently** (`FuturesUnordered`
    over one read task per admitted session), fixing a real, previously-only-
    mitigated defect — `supervise` awaited `RtmpSource::connect()`
    *sequentially* against the bind-once listener, so a publisher that
    completed the handshake and then went idle wedged the entire route, never
    accepting another publisher (#738 T11b review, Critical). One stalled
    publisher no longer blocks another (see `second_publisher_is_served_while_first_is_stalled_at_handshake`).
  - Preserved exactly: bind-once/reuse-forever (via `tokio::sync::OnceCell`,
    shared across every `run_rtmp` attempt), `Established` gating on the
    first `DemuxEvent::Sample` rather than the first `TrackAdded` (FLV has no
    `TracksResolved`; leans on `transmux::flv_stream`'s Annex E ordering
    assumption, exactly as `SessionEvent::Established`'s own doc prescribes
    for RTMP), the first sample never being dropped (buffered, then
    re-emitted immediately after the `NewProgram`/`Established` pair), and
    `IngestTimeouts` still bounding every read.
  - RTMP publishes under `SPTS_PROGRAM_ID` (`ProgramId(0)`) like every other
    single-program driver-backed source; MPTS is task 6, out of scope here.
  - Why RTMP *can* implement `Listener` where SRT's listener mode explicitly
    cannot (see `source::srt`'s "Why listener mode is not a `Listener` yet"):
    `AsyncRtmpServer::accept` takes `&self` (shareable via `Arc`, no `Mutex`
    needed) and each accepted connection owns its own `TcpStream` — SRT's
    blockers (a `&mut self` accept, one shared `UdpSocket` across the
    listener and every connection) are specific to `srt-runtime`'s current
    API, not a general objection to a push source using `Listener`.
  - `crate::pipeline`'s `SampleSource for RtmpSession` impl and
    `crate::origin::supervisor`'s `SourceConnector for RtmpSource` impl are
    deleted (RTMP no longer implements either). `SourceConnector`/`supervise`/
    `run_pipeline` themselves are unchanged — `InputSpec::Custom` still uses
    them (task 5 removes them, once `Custom` is the only remaining user).

### Changed (BREAKING, in progress — plan step 5b)
- **Ported the three outputs (LL-HLS, DASH, LL-DASH) and the shared
  init/segment/part resource route onto `media_plane::egress::ServedEgress`
  behind one axum adapter**, and **deleted `crate::store` (`MediaStore`,
  `HealthState`)** — the module was a re-export of `ll_hls_runtime::server`
  types Step 4 replaced with the sans-IO `LlHlsOrigin`/`Trunk` design, and
  multimux has not compiled since. This unblocks the whole workspace: it has
  not built since Step 4 deleted `MediaStore`.
  - New `crate::route::RouteHandle` replaces `MediaStore` as the shared
    per-stream state: a `media_plane::Trunk`, the `LlHlsOrigin` over it
    (serves every output's init/segment/part bytes and LL-HLS's own
    playlist), a small `Trunk`-drained `DashState` (track specs + created-at
    + closed-segment window — the only things no `Trunk` ring holds), and a
    new `crate::route::HealthState` (route up/down; distinct from
    `media_plane::ingress::HealthState`, which is generic over one ingest
    session's own connector error type and cannot give one route's
    homogeneous live/down status). Same public method surface as the deleted
    `MediaStore` (`set_init`/`init_bytes`/`set_track_specs`/`track_specs`/
    `add_part`/`add_segment`/`window_segments`/`health`/`set_health`) so
    `crate::pipeline::run_pipeline`/`crate::origin::supervisor::supervise`
    needed only a type-rename, not a rewrite.
  - New `crate::http` module: the **one** axum adapter every route goes
    through — `resolve_blocking` (the caller-driven blocking-reload wait
    loop `ll_hls_runtime::server`'s own module doc sketches, generalised to
    any `ServedEgress`) and `into_response` (`EgressResponse` -> HTTP
    response, `Await`/`NotFound`/`BadRequest` mapped once, not per output).
    `crate::output::dash`/`crate::output::ll_dash` now implement
    `ServedEgress` too (`Request = ()`, `Body = String`) purely so their
    manifest routes go through the same adapter as LL-HLS, even though
    neither ever answers `Await`.
  - `crate::origin::resource`'s shared `/:file` route now resolves through
    `LlHlsOrigin::resolve` (`LlHlsRequest::Resource`) instead of the deleted
    `MediaStore::resolve_resource`; the issue #721 chunked-transfer
    in-progress-segment path is unchanged in shape, re-fetching parts via
    the same adapter.
  - **Fix #776**: `render_mpd`/`render_ll_dash_mpd` took `specs.remove(0)`
    unconditionally, so a track set whose first elementary stream is an
    opaque `CodecConfig::Data`/`Subtitle` track (teletext, DSM-CC, SCTE-35 —
    routine in a real DVB multiplex) made `DashPackager`/`LlDashPackager`
    reject the `Media` and the whole DASH/LL-DASH route return a
    **permanent 503**. New `crate::output::dash::select_representable_track`
    selects the first track (preferring a video-shaped codec, then any
    other) that actually trial-packages successfully through the real
    `DashPackager` — reusing its codec-support decision rather than
    re-deriving it — so a representable track behind an opaque one is no
    longer starved. Only a track set with genuinely no representable track
    is still a 503.
  - The two RFC 8216bis behaviours that shipped as multimux 0.2.1 (a
    preload-hinted part blocks until produced rather than 404ing) and 0.2.2
    (a just-closed segment's final part still serves) now fall out of the
    `Trunk`'s live-part log for free (segment close deliberately never
    evicts parts — see `media-plane`'s own module doc) — both are asserted
    directly in `crate::origin::resource`'s own tests, not merely assumed to
    still hold.
  - Fixed two pre-existing, unrelated defects the first successful
    `cargo test --workspace`/`cargo clippy --workspace --all-targets` since
    Step 4 surfaced (not caused by this port): `ll-hls-runtime`'s own
    `LlHlsOrigin` test helpers and two of its examples
    (`client_stepping`/`origin_playlist`) called `Trunk::writer()` (the
    samples+events writer) instead of `Trunk::segment_writer()` for
    segment/part publishing — a stale call from before Step 4's
    `SegmentWriter` split that never compiled since; and multimux's
    `AuthSpec::to_credentials` had gone dead (its only call site was the
    per-route ingest wiring Step 5a rounds 2/3 replaced with a
    `tracing::error!` stub for every non-`rtmp` input), now `#[allow(dead_code)]`
    with its reason recorded rather than deleted, since the (tested) logic
    is still exactly what that wiring will need once it lands.
  - **Not fixed, reported**: `multimux/tests/rtsp_ingest.rs` has not
    compiled since Step 5a round 2 ported `crate::source::rtsp` off
    `RtspSource`/`SourceConnector` onto `run_rtsp`/`IngestDriver` — the test
    file was never updated and still imports the deleted `RtspSource` (and
    references a `Sample::flags` field the media-plane step 2c PTS/DTS
    refactor also removed independently). Out of this step's scope (RTSP
    ingest wiring is a later step's job, not the output port); every other
    target in the workspace builds and passes.

### Changed (BREAKING, in progress — plan step 5a, round 3)
- **Ported `hls_pull`, `dash_pull` and `smooth_pull` onto the plane's ingress
  traits**, on the back of `media-plane`'s round-3 `IngestSession` change
  (relaxed `Stage::In` + associated `Request` — see that crate's CHANGELOG
  for the trait diff and the reasoning). Round 2 deliberately did **not**
  port these three because contorting them into the then-current trait would
  have produced a session whose `Stage::feed` is never called; the trait now
  expresses what they actually do, so the port is honest rather than a
  workaround.
  - `HlsPullSource`/`HlsPullSession` → `HlsPullRoute`/`HlsPullDialer`/
    `HlsIngestSession` + `run_hls_pull`. **Now wraps the sans-IO
    `ll_hls_runtime::client::LlHlsClient` directly, not `TokioClient`.**
    `TokioClient` owns its own `reqwest` fetch loop internally, so there was
    nothing for `feed`/`poll_transmit` to drive; `LlHlsClient` is the sans-IO
    core `TokioClient` itself wraps. This module is now the *other* adapter
    over the same engine — all LL-HLS logic (reload scheduling, part/segment
    dedup, fMP4 and issue-#760 classic-TS demux) still lives entirely in
    `ll-hls-runtime`. `Request = ll_hls_runtime::client::Action`;
    `In<'a> = (HlsFetchId, &'a [u8])`, where `HlsFetchId::{Playlist,
    Resource(ResourceId)}` is this module's own correlation identity.
  - `DashPullSource`/`DashPullSession` → `DashPullRoute`/`DashPullDialer`/
    `DashIngestSession` + `run_dash_pull`. `Request = DashAction`;
    `In<'a> = (DashResourceId, &'a [u8])`.
  - `SmoothPullSource`/`SmoothPullSession` → `SmoothPullRoute`/
    `SmoothPullDialer`/`SmoothIngestSession` + `run_smooth_pull`.
    `Request = SmoothAction`; `In<'a> = (SmoothResourceId, &'a [u8])`.
  - **`dash_pull`'s in-read-path sleep is gone.** `maybe_refresh_mpd` buried
    a wall-clock `Instant::elapsed` + `tokio::time::sleep` inside what was
    nominally a "compute the next samples" step — a sans-IO session that
    sleeps internally is not sans-IO. The live-MPD refresh now goes through
    `Stage::next_deadline`/`on_deadline` (an absolute `Timestamp` on the
    driver's clock, exactly like `HandshakePolicy::establish_by`), and
    `run_dash_pull`'s own loop is the only place a clock is read or a sleep
    awaited. `smooth_pull`'s identical `maybe_refresh_manifest` sleep got the
    same treatment.
  - **`dash_pull`'s three pieces of per-Representation state, judged
    individually** (round-2 flagged them as possible `Trunk` duplication;
    all three are kept, with reasons recorded in the module doc):
    - `RepState::init_bytes` — **kept.** Genuinely per-source parsing state:
      the `Trunk` stores decoded `Sample`s, never raw container bytes, so
      there is nothing to duplicate. Re-concatenating the cached init onto
      every `moof`+`mdat` media segment is a structural requirement of DASH's
      own wire format (`Fmp4Demux::unpackage` needs the `moov` in the same
      buffer), not a cache of plane state.
    - `RepState::plan` — **kept.** Fetch *scheduling* state (what to request
      next), a concept the `Trunk` does not have at all — it only ever sees
      samples after they arrive.
    - `RepState::last_number` — **kept.** The high-water mark that stops a
      live-MPD refresh re-enqueueing an already-planned segment number;
      again a fact about *pending fetches*, recorded nowhere else in the
      pipeline.
  - **In-flight fetches are bounded.** New `source::MAX_INFLIGHT_FETCHES`
    (8): a pull session can hand back many `poll_transmit` requests in one
    drain (an LL-HLS playlist reload revealing a dozen available parts at
    once; a manifest refresh extending several Representations' plans
    simultaneously) with nothing in the session capping how many the driver
    launches concurrently. Each `run_*_pull` loop gates every `JoinSet::spawn`
    on that cap and queues the rest — this project has already shipped five
    unbounded-allocation vectors in remote-input-driven code, and an uncapped
    fan-out of open sockets per route is the same class of bug.
    `dash_pull`/`smooth_pull` additionally keep at most one fetch per
    Representation/StreamIndex outstanding (`in_flight`), preserving the
    pre-port per-round pacing.
  - `origin::serve_with_registry`'s `HlsPull`/`DashPull`/`SmoothPull` arms
    join the existing step-5a stub arm (an explicit `tracing::error!`)
    alongside `Rtsp`/`Rtp`/`TsUdp`/`TsHttp`/`Srt`, and their
    `SourceConnector`/`SampleSource` impls are removed. `rtmp` is now the
    only source still on the pre-5a `run_pipeline` path, which is therefore
    still load-bearing for exactly one input kind.
- New `MultimuxError::LlHls` variant wrapping `ll_hls_runtime::client::Error`
  (a malformed playlist, a demux failure, or a resource fed for an id the
  client never requested).

### Changed (BREAKING, in progress — plan step 5a, round 2)
- **Ported `ts_http` and `srt` onto the plane's ingress traits**, and
  **extracted the shared `source::ts_program::TsIngestSession`** — one copy
  of the `StreamingTsDemux` → `SessionEvent` translation (including the B5
  mid-stream `NewProgram` handling), now shared verbatim by `ts_udp`,
  `ts_http` and `srt`. Those three previously carried three near-identical
  ~60-line drain loops that had already drifted.
  - `TsHttpSource`/`TsHttpSession` → `TsHttpRoute`/`TsHttpDialer` +
    `open_stream`/`recv_and_feed`/`run_ts_http`.
  - `SrtSource`/`SrtSession` → `SrtRoute`/`SrtDialer` +
    `connect_caller`/`accept_listener`/`drive_socket`/`run_srt_caller`/
    `run_srt_listener_once`.
  - `ts_http` is the first ported source that produces **both**
    `HealthState::Ended` (a cleanly-finished HTTP body) and
    `HealthState::Failed` (a read stall) — the EOF≠error distinction step 3c
    made producible is now actually produced, and mutation-checked.
  - All 5 of `ts_http`'s Basic/Digest/Bearer/override/wrong-credential auth
    tests are preserved, retargeted at the new API.

### Findings from this round (design-level, not defects)
- **The segmenter gap is CLOSED** by `Trunk::segment_writer()`. Proven, not
  asserted: `source::ts_program`'s
  `a_segmenter_can_hold_a_segment_writer_while_ingest_holds_the_sample_writer`
  drives real muxed TS → `TsIngestSession` → `IngestDriver` (holding the
  `TrunkWriter`) → `SampleCursor` → a real `LlHlsSegmenter` → a
  `SegmentWriter` taken from the **same** `Trunk`, and asserts the segments
  land in `Trunk::last_closed_segment`/`segment_len`. It also asserts
  `trunk.writer().is_none()` so the test cannot silently degrade into "a
  `Trunk` nobody else was writing to". `MOVIE_TIMESCALE` is now a parameter
  of the segmenter component rather than a hardcoded `run_pipeline`
  constant, which is what makes it per-route-configurable at all; **which**
  component owns the segmenter is step 5b's call, since egress consumes
  segments.
- **The pull sources DO need a plane change** — this refutes round 1's
  answer ("a concrete per-source method suffices"). `ll-hls-runtime`
  *already* ships the request-addressing type 3c predicted: `LlHlsClient` is
  a sans-IO engine with `poll() -> Option<Action>`, `on_playlist(&[u8])`,
  `on_resource(ResourceId, &[u8])`, `on_error(Option<ResourceId>)`.
  The blocker is not the *request* side, which `poll_transmit` could be
  widened to carry — it is the **response** side: `Stage::feed(&[u8])` is a
  single, uncorrelated input, while a pull source has N in-flight requests
  whose responses must be routed back **by identity** (`ResourceId`) and
  arrive out of order. There is no honest way to express "these bytes are
  the response to `Part{msn:5,part:2}`" through `feed(&[u8], now)`.
  - **Minimum shape** (recorded, not implemented — it is a `media-plane`
    change with its own blast radius): relax `IngestSession`'s pin of
    `Stage::In` from `&'a [u8]` to the implementor's choice, and add an
    associated request type:
    ```rust
    pub trait IngestSession: for<'a> Stage<Out = SessionEvent> + Send {
        type Request: Send;
        fn poll_transmit(&mut self) -> Option<Self::Request>;
    }
    ```
    Stream sources keep `In<'a> = &'a [u8]`, `Request = Bytes` — no
    behaviour change. A pull source sets `In<'a> = (ResourceId, &'a [u8])`,
    `Request = Action`. `IngestDriver::feed` becomes
    `feed(&mut self, input: S::In<'_>, now)`, which is still fully generic.
    Associated types cannot have defaults on stable, so every implementor
    gains one line (`type Request = Bytes;`).
  - `hls_pull`/`dash_pull`/`smooth_pull` are therefore **deliberately not
    ported**. Contorting them into the current trait would mean a session
    whose `Stage::feed` is never called (all real input arriving through
    out-of-band `on_resource` calls made by the driver), i.e. a type that
    lies about the contract it implements.
  - Separately, `dash_pull` holds three pieces of state a segment store
    would duplicate (`RepState::init_bytes`, re-concatenated onto every
    segment; `plan`; `last_number`) and buries a wall-clock
    `Instant::elapsed` + `tokio::time::sleep` inside its read path
    (`maybe_refresh_mpd`) — a second, independent obstacle to a sans-IO
    port, which wants `next_deadline`/`on_deadline` instead.
- **`rtmp` and `srt`-listener: scoped estimate, not a half-port.**
  - **RTMP is genuinely cheap and worth doing.** `rtmp_runtime::server::ServerSession`
    is *already* exactly the right sans-IO shape:
    `handle_data(&[u8]) -> Result<(Vec<u8>, Vec<ServerEvent>)>` — bytes in,
    (reply bytes, events) out, buffering partial handshakes/chunks
    internally. That maps onto `Stage::feed` + `poll` + `poll_transmit` with
    no new machinery, and `rtmp_runtime::io` is a ~246-line adapter (half
    tests) that would mostly disappear. `AsyncRtmpServer::accept()` is
    `&self` (no `Mutex` needed) and a *pure* TCP accept exchanging zero
    protocol bytes. **Estimate: the session half is ~1 day** (close to a
    transcription of `RtmpConnection::next_events`). **The `Listener` half
    needs one small upstream addition** — a non-blocking
    `AsyncRtmpServer::poll_accept()` (a `TcpListener::poll_accept` wrapper,
    a few lines in `rtmp-runtime`), after which `Listener{max_sessions}` +
    `ListenDriver` fit with no further redesign. Total ≈ 1.5–2 days.
  - **SRT-listener is substantially harder and should be sequenced after a
    `srt-runtime` change.** `SrtListener::accept` is a blocking `.await`
    only (no `poll_accept`/`try_accept`) and takes `&mut self`; more
    fundamentally the listener and *every* accepted connection share one
    `UdpSocket` (`drain_completed` hands each new connection an
    `Arc::clone`), so "accept another while N are live" is a demultiplexing
    responsibility **inside `srt-runtime`**, not something `multimux` can
    arrange. `srt-runtime` does ship a full sans-IO core
    (`CallerHandshake`/`ListenerHandshake`/`arq`/`tsbpd`/`livecc`) but hides
    it behind a private `Driver` task; `SrtSocket` is only a pair of `mpsc`
    channels, so there is **no public "feed a datagram in, get a payload
    out" connection type**. **Estimate: ~3–5 days in `srt-runtime`**
    (expose a sans-IO connection + a pollable listener owning the shared
    socket demux), then ~1 day in `multimux`. Until then `srt`-listener
    keeps today's accept-one-serially semantics via
    `run_srt_listener_once`, which is documented rather than disguised.

### Changed (BREAKING, in progress — plan step 5a)
- **Ported `rtsp`, `rtp_udp`, and `ts_udp` onto `media_plane::ingress`'s
  `Dialer`/`IngestSession`/`IngestDriver`** (plan step 5, "port the 9 ingest
  sources" — `docs/superpowers/plans/2026-07-26-media-plane-implementation.md`):
  - `RtspSource`/`RtspSession` → `RtspRoute`/`RtspDialer`/`RtspIngestSession`
    + `run_rtsp`. `RtspDialer::dial` performs **no I/O** — it wraps
    `rtsp_runtime::client::ClientSession` (already sans-IO) directly, so the
    whole DESCRIBE→SETUP(×N)→PLAY handshake completes through the ordinary
    `poll_transmit`/`feed` pump, confirming `media_plane::ingress`'s central
    design bet for a protocol with a pre-existing sans-IO engine. `rtsps://`
    (TLS) is not yet wired into `run_rtsp`'s driver loop (scope cut, not a
    design gap — see the module doc).
  - `TsUdpSource`/`TsUdpSession` → `TsUdpRoute`/`TsUdpDialer`/`TsUdpIngestSession`
    + `run_ts_udp`. Fixes issue #774's silent-drop of a `DemuxEvent::TrackAdded`
    declared after the initial PMT resolution: it now mints a **new**
    `ProgramId` (media-plane finding B5) instead of being logged and
    dropped.
  - `RtpUdpSource`/`RtpUdpSession` → `RtpUdpRoute`/`RtpUdpDialer`/`RtpUdpIngestSession`
    + `run_rtp_udp`.
  - All three publish raw samples straight into a `media_plane::Trunk` via
    `IngestDriver`, replacing their own hand-rolled demux-drain loops.
  - **Not yet wired into `origin::serve_with_registry`/`origin::supervisor::supervise`**:
    that loop is built on `MediaStore`/`HealthState`, which plan step 4
    deleted from `ll-hls-runtime` (the crate does not currently build — see
    the 4 `ll_hls_runtime::server::{HealthState,MediaStore,PlaylistOutcome,
    ResourceOutcome,media_playlist_m3u8}` import errors, all pre-existing
    and unrelated to this change) — rewiring these three routes needs step
    5b's `Trunk`-backed replacement, not just a source rename; the affected
    `InputSpec` match arms are stubbed with a clear "not yet wired" log line
    rather than left silently referencing removed types.
  - **Not ported** (superseded by round 2 above, which added `ts_http` and
    `srt`): `rtmp`, `hls_pull`, `dash_pull`, `smooth_pull` remain on the
    pre-5a `SampleSource`/`run_pipeline`/`SourceConnector` path, which stays
    in place (`crate::pipeline`/`crate::origin::supervisor`) for exactly
    those four. See "Findings from this round" for why each is deferred.
  - `crate::pipeline::SampleSource`/`run_pipeline` and
    `crate::origin::supervisor::SourceConnector` are **not** deleted this
    pass (still load-bearing for the six not-yet-ported sources); only the
    three ported sources' `impl`s were removed from them.

### Fixed
- **Cleared this crate's share of the latest-stable clippy canary** (issue
  #770 — the non-blocking `clippy (latest stable)` CI job, which had been
  failing on `main` unnoticed across many merges). Both changes are
  behaviour-preserving; no `#[allow]` was added.
  - `source::rtsp::RtspClient` (a private enum) now boxes **both** its
    `Plain` and `Tls` variants (`clippy::large_enum_variant`). The unboxed
    TLS variant carried rustls connection state at ~1472 bytes against the
    plain client's ~408, so every `RtspClient` — and the `RtspSession`
    embedding it — was sized for TLS even on a plain `rtsp://` connect.
    Boxing only the larger variant merely flips the lint onto the other one,
    so both are boxed and the enum is now pointer-sized. The single
    allocation happens once per connect, never on the per-packet
    `recv_interleaved` path.
  - `source::sdp::parse_sdp_tracks` derives its 1-based `track_id` from the
    media-order iterator instead of a hand-rolled counter
    (`clippy::explicit_counter_loop`). The interleaved `channel` keeps its
    own explicit `saturating_add(CHANNEL_STEP)` stepping — it advances by 2,
    not 1, and the saturating behaviour at the `u8` ceiling is deliberate.

## [0.4.0] - 2026-07-26

### Added
- **RTMP push ingest input `InputSpec::Rtmp`** (issue #738, via
  `rtmp-runtime`'s sans-IO ingest server + `transmux::StreamingFlvDemux`):
  a route can now accept an inbound RTMP publisher (`ffmpeg`/OBS-class
  encoders) instead of only pulling from a source — `RtmpSource` binds its
  `listen` address once and reuses it across reconnects, accepts a
  publisher, and drives its FLV byte stream through the same
  segmenter/`MediaStore` pipeline every other input uses. Optional `app`/
  `stream_key` gate which publishers are accepted.
- **`HlsPullSource` now ingests classic MPEG-TS-segment HLS** (issue #760,
  via `ll-hls-runtime` 0.1.x's new `LlHlsClient` TS routing): pulling a
  legacy/IPTV origin whose Media Playlist carries no `EXT-X-MAP` (whole
  `.ts` segments, no init resource) now works end to end — `connect()`
  recovers real `TrackSpec`s from the client's synthesized `Output::Init`
  and `next_samples()` yields every real access unit, with no TS-specific
  code needed in this crate at all.
- **DASH-pull ingest input `InputSpec::DashPull`** (issue #758, via
  `transmux`'s hand-rolled MPD parser, `transmux::dash_parse`): a route can
  now pull a remote MPEG-DASH presentation — fetch + parse the MPD, resolve
  each selected Representation's `SegmentTemplate`/`SegmentTimeline`
  init+media segment URLs, and demux the fetched fMP4/CMAF bytes via
  `transmux::Fmp4Demux` (each media segment concatenated onto its
  Representation's cached init bytes, matching `ll-hls-runtime`'s own
  CMAF-part demux pattern), remapping each Representation's local track_id
  to a session-wide unique id. Supports `$Number$`/`$Time$` addressing, fMP4,
  and both static and dynamic (live, MPD-refresh) presentations;
  `SegmentList`/`SegmentBase` addressing is deferred. Every network step is
  bounded by `IngestTimeouts`, so a stalled/unreachable DASH origin cannot
  wedge the route.
- **Smooth-pull ingest input `InputSpec::SmoothPull`** (issue #759, via
  `transmux`'s hand-rolled MS-SSTR client-Manifest parser,
  `transmux::smooth_parse`): a route can now pull a remote Microsoft Smooth
  Streaming presentation — fetch + parse the client Manifest, resolve each
  `StreamIndex`'s fragment-URL template, and demux the fetched fragments via
  `transmux::Fmp4Demux`. Smooth carries no bootstrapping init segment, so
  `SmoothPullSource` synthesizes one per stream from
  `QualityLevel@CodecPrivateData` (`track_spec_from_quality_level` +
  `build_init_segment`), discovering and matching each stream's wire
  `track_id` from its first fragment's `tfhd` (no `moov` to read one from,
  unlike DASH) and remapping it to a session-wide unique id. Supports
  static and dynamic (live, manifest-refresh) presentations. Every network
  step is bounded by `IngestTimeouts`, every expected track is resolved
  before the route starts, and a PlayReady/PIFF sample-encrypted source
  (`<Protection>` manifest element, or CENC/PIFF sample-encryption boxes in
  a fragment) fails with a clear typed `MultimuxError::Encrypted` instead of
  silently demuxing garbage — decrypting Smooth-protected content is out of
  scope.
- **SRT ingest input `InputSpec::Srt`** (issue #739, via `srt-runtime`'s
  real-socket `SrtListener`/`SrtSocket` adapter): a route can now ingest an
  SRT-carried MPEG-2 Transport Stream in either listener mode (binds once
  and accepts inbound Callers, reused across reconnects — like
  `InputSpec::Rtmp`'s push pattern) or caller mode (dials out fresh on every
  reconnect). The track set comes from the stream's own in-band PMT via
  `transmux::StreamingTsDemux`, exactly like `InputSpec::TsUdp`. Encrypted
  SRT is out of scope (no passphrase field).

## [0.3.1] - 2026-07-21

### Changed
- **True chunked-transfer LL-DASH** (issue #721): `LlDashOutput`'s
  `manifest-ll.mpd` now renders a real chunked-CMAF MPD via
  `transmux::LlDashPackager` — a whole-segment `SegmentTemplate` (the same
  `seg-{track}-{seq}.m4s` addressing `manifest.mpd` uses) with a genuinely
  non-zero `availabilityTimeOffset`/`availabilityTimeComplete="false"` —
  replacing the previous discrete-parts-signalling fallback (which
  re-addressed `part-*.m4s` files directly and could only claim an honest
  `availabilityTimeOffset="0"`). The shared origin resource route now serves
  a not-yet-closed segment over **HTTP chunked transfer-encoding**,
  streaming the store's live parts as they land and completing once the
  segment closes; a closed segment is served exactly as before (whole
  bytes, `Content-Length`). Because whole closed segments stay in the
  store's window, `manifest-ll.mpd` now also advertises a real
  `timeShiftBufferDepth`. Validated against a real headless dash.js
  low-latency player (`multimux/tests/lldash_dashjs.rs`, vendored
  `dash.all.min.js` 5.2.0 under `multimux/tests/assets/`): real playback
  advances past 1.8s with measured live latency below the 1s segment
  target, proving genuine chunked (not whole-segment) availability.

## [0.3.0] - 2026-07-21

### Added
- **External scheme plugin registry** (issue #663): a third-party crate can
  now add a new input, output, or output-auth scheme to the multimux origin
  **without editing multimux**, wired purely via config JSON. Built-ins
  (RTSP/RTP/TS-UDP/TS-HTTP/HLS-pull inputs; LL-HLS/DASH/LL-DASH outputs;
  Basic/Digest/Bearer/Forwarded output-auth) are unchanged — the typed,
  validated fast path. Extension is additive:
  - New `Custom { type_tag, params }` variants on `config::InputSpec` (JSON
    `{ "type": "custom", "type_tag": "...", "params": { ... } }`),
    `output::OutputKind` (JSON `{ "custom": { "type_tag": "...", "params":
    { ... } } }` — not internally tagged like the other two, since the other
    `OutputKind` variants are plain strings), and `config::OutputAuthSpec`
    (JSON `{ "scheme": "custom", "type_tag": "...", "params": { ... } }`).
    `params` is an opaque `serde_json::Value`, always structurally valid at
    `Config::validate` time — the registered factory validates it, at
    route-build time. Every `Custom` variant's hand-written `Debug` shows
    `type_tag` but always redacts `params` as `"<params>"` (it may carry an
    external scheme's credentials).
  - A new `registry::SchemeRegistry` — built by the embedding application,
    never by multimux itself — mapping each `type_tag` to a factory closure
    that builds the real thing from the opaque `params`: `register_input`/
    `register_output`/`register_auth` (and their `input`/`output`/`auth`
    lookups). `InputFactory` closures construct their own concrete
    `SourceConnector` and spawn `supervise` themselves (returning its
    `JoinHandle`) rather than returning a connector — `SourceConnector` is
    not object-safe, so this is how a factory erases the connector type;
    `OutputFactory`/`AuthFactory` return `Arc<dyn output::Output>`/
    `broadcast_auth::Verifier` directly (both already concrete/object-safe).
  - `origin::serve_with_registry(config, registry)` — `origin::serve(config)`
    is now `serve_with_registry(config, SchemeRegistry::new())`. An
    unregistered `Custom` `type_tag` fails route setup with the new
    `MultimuxError::UnknownScheme { kind, tag }` (`kind` is `"input"`,
    `"output"`, or `"auth"`) rather than panicking or silently no-opping.
  - New re-exports at the crate root for external factory authors:
    `SchemeRegistry`, `InputCtx`/`OutputCtx`/`AuthCtx`,
    `InputFactory`/`OutputFactory`/`AuthFactory`, `serve`/
    `serve_with_registry`, `supervise`/`SourceConnector`/`Backoff`,
    `Source`, `MediaStore`, `Output`, and the `broadcast_auth` crate itself
    (so a registered `AuthFactory` can build a `Verifier` without an
    external crate needing its own direct dependency on `broadcast-auth`).
  - New example `examples/custom_scheme.rs`: registers a custom input
    scheme with zero multimux edits.

### Changed (breaking)
- **`output::OutputKind` no longer derives `Copy`/`PartialEq`/`Eq`/`Hash`**
  (only `Debug`/`Clone`/`Deserialize`/`Serialize` remain): its new `Custom`
  variant carries a `serde_json::Value`, which is `Clone` but not `Copy`.
  Compare `OutputKind` values via `.name()` or `matches!` instead of `==`.
  `OutputKind::name()`'s return type changed from `&'static str` to `&str`
  (`Custom` labels itself by its own `type_tag`, borrowed from `self`).

### Added
- **`OutputAuthSpec::Forwarded` — reverse-proxy forwarded-auth output-auth
  scheme** (issue #663 extensibility wave part 1, built on
  `broadcast_auth::Verifier::forwarded`): configures the shared output-auth
  gate to trust a fronting reverse proxy that has already authenticated the
  caller, rather than checking a Basic/Digest/Bearer credential itself.
  JSON: `{ "scheme": "forwarded", "user_header": "X-Forwarded-User",
  "forwarded_for_header": "X-Forwarded-For" }` — both fields optional,
  defaulting to `X-Forwarded-User`/`Some("X-Forwarded-For")`; set
  `forwarded_for_header: null` to disable reading it. A request is allowed
  iff `user_header` is present and non-empty; `forwarded_for_header`, if
  set, is read for tracing/observability only — no trust decision is made
  from it. **Safe ONLY behind a trusted reverse proxy that strips any
  client-supplied copies of both headers before forwarding** — see
  `OutputAuthSpec::Forwarded`'s doc comment. `output_auth_gate` now builds a
  `broadcast_auth::RequestContext` carrying every request header (not just
  `Authorization`) plus the transport peer address (via
  `into_make_service_with_connect_info`, wired in `serve`), so any
  `Verifier` scheme — this one included — can see beyond `Authorization`.
- **`InputSpec`/`AuthSpec`/`OutputAuthSpec`/`output::OutputKind` are now
  `#[non_exhaustive]`** (issue #663 extensibility wave part 1): a future
  ingest transport, client-auth scheme, output-auth scheme, or delivery
  protocol can be added later without it being a breaking change for
  external matches on these types.

### Changed (breaking)
- **`OutputAuthSpec::to_credentials` replaced by
  `OutputAuthSpec::build_verifier(realm)`** (`pub(crate)`, so only affects
  this crate's own `serve`): returns the configured
  `broadcast_auth::Verifier` directly rather than a `Credentials` value —
  needed because `Forwarded` has no `Credentials` mapping at all (no
  username/password/token, no challenge/response round-trip).

### Added
- **Shared output auth** (issue #663 "shared output auth",
  `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`): one
  Basic/Digest/Bearer credential can now gate **every** media output route
  (`/{stream}/…` — manifests and init/segment/part bytes alike, across every
  configured route, e.g. 40 cameras under `/camN/index.m3u8`) via a new
  `Config::output_auth` (`config::OutputAuthSpec`, JSON tagged on `scheme`:
  `{ "scheme": "basic"|"digest"|"bearer", ... }`). Independent of, and
  unrelated to, each route's own ingest auth (`config::AuthSpec`/URL
  userinfo) — one output credential guards the whole origin regardless of how
  differently each camera authenticates upstream. Built on the new
  `broadcast_auth::Verifier` (the server-side challenge+verify half,
  promoted out of `testutil`'s test-only mock — see that crate's changelog).
  Missing/wrong credentials get `401` + `WWW-Authenticate` (Basic/Digest
  challenge, or the bare `Bearer` token for Bearer); `output_auth: None` (the
  default) leaves every route open, unchanged from pre-#663 behaviour.
  **Ops endpoints (`/healthz`/`/readyz`/`/metrics`) are never gated** — load
  balancer probes and metrics scraping stay open regardless of
  `output_auth`. CORS/`Cache-Control` headers still apply to a `401`
  response from this gate (needed for a cross-origin browser client to see
  the status/challenge at all, not just a successful response).
- **Configurable `playlist_name`** (issue #663 "configurable
  `playlist_name`"): a new `Config::playlist_name` (default `"media.m3u8"`)
  names the LL-HLS media playlist filename served at
  `/{stream}/{playlist_name}`; `master.m3u8`'s `#EXT-X-STREAM-INF` reference
  follows suit (`output::llhls::LlHlsOutput::new`). Validated non-empty,
  `.m3u8`-suffixed, slash-free, and not `"master.m3u8"` (which would collide
  with the fixed master-playlist route). `master.m3u8`'s own name is not
  configurable; DASH's `manifest.mpd` is unaffected. Breaking (internal):
  `LlHlsOutput` is no longer a unit struct — use `LlHlsOutput::default()` (or
  `OutputKind::build()`) for the pre-existing `/media.m3u8` behaviour, or
  `LlHlsOutput::new(name)`/`OutputKind::build_with_playlist_name(name)` for a
  configured name; `output::llhls::{master_playlist, media_playlist}` are
  narrowed from `pub` to `pub(crate)` (their `State` type changed shape, and
  nothing outside this crate called them directly). Depends on
  `ll_hls_runtime::server::master_playlist_m3u8` now taking the media
  playlist's filename as an argument (see that crate's changelog).
- **RTSP config-auth (`with_auth`) Digest coverage against a real server**
  (the gap flagged in the client-auth story): a new loopback test drives
  `source::rtsp::RtspSource` with config-supplied (not URL-userinfo) Digest
  credentials against a mock server verified by the real
  `broadcast_auth::Verifier`, proving the `with_auth` -> `ClientSession`
  wiring end-to-end (success and wrong-password cases), mirroring
  `rtsp-runtime/tests/io_loopback.rs::digest_auth_over_loopback`.
- **Config-supplied + Bearer credentials, finishing client-side
  multi-scheme auth** (issue #663 — completes the P3c "Shared auth layer"
  story): `InputSpec::Rtsp`/`TsHttp`/`HlsPull` each gained an optional
  `auth` field (`config::AuthSpec` — either `{ username, password }` or
  `{ bearer_token }`), config-parseable via `--config <FILE>`. A Bearer
  token has no URL-userinfo form, so this is the *only* way to supply one;
  when both a config `auth` and URL userinfo are present, config wins
  (`source::http_auth::resolve_credentials`, now used by `RtspSource`,
  `TsHttpSource`, and `HlsPullSource` alike, each via a new
  `with_auth(Option<Credentials>)` builder mirroring `with_timeouts`).
  `AuthSpec`'s `Debug` redacts both `password` and `bearer_token`;
  `Config::validate` rejects an empty `username`/`bearer_token` (an empty
  `password` is accepted — some devices genuinely use one). Every
  pre-existing config still parses unchanged (`#[serde(default)]`).
  - **Digest/Basic/Bearer now proven end-to-end**, not just unit-tested in
    isolation: a new test-only mock auth server (`testutil`, gated
    `#[cfg(test)]`) gates a real axum router behind any of the three
    schemes — Digest verification is a real, independent RFC 7616 §3.4.1
    computation (not a literal-string match), so a client with the wrong
    password genuinely gets rejected. `source::ts_http` and
    `source::hls_pull` each gained Basic/Digest/Bearer/wrong-credentials
    tests driving the real `TsHttpSource`/`HlsPullSource` against it, plus a
    `config_auth_overrides_wrong_url_userinfo` precedence test.
  - No change needed to answer Digest's re-challenge-on-every-request
    concern: `ll_hls_runtime::client::tokio_client::TokioClient` already
    caches its Digest `Authenticator` across requests (from P3c), and
    `TsHttpSource`'s streaming GET only ever makes one request per
    `connect()`, so there was nothing further to cache there.
- **DASH output alongside LL-HLS, from the same shared CMAF segments**
  (issue #663 P4 — `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`,
  "DASH output"): one ingested stream can now serve LL-HLS *and* MPEG-DASH
  simultaneously, both reading the exact same `MediaStore`-produced
  init/segment bytes (ingest-once, many-outputs — no per-output re-mux).
  - **The multi-output nest collision fix (the load-bearing refactor)**: two
    `Output`s each mounting their own `/:file` catch-all under the same
    `/{stream}` nest previously panicked axum. Fixed by splitting
    responsibilities: the `Output` trait's `router` method is now
    `manifest_routes` — each output contributes *only* its manifest
    route(s) (`master.m3u8`+`media.m3u8` for LL-HLS, `manifest.mpd` for
    DASH) — while the init/segment/part byte serving (`init-*.mp4`/
    `seg-*.m4s`/`part-*.m4s`, protocol-neutral since both outputs are
    fMP4/CMAF) moves to a new shared `origin::resource` route, mounted
    **once per stream** by `origin::router` (merging every output's
    manifest routes with the one shared resource route, then `nest`ing the
    merged router once — instead of nesting per-output). LL-HLS's URLs and
    behaviour (routes, blocking reload, `Cache-Control`/CORS policy) are
    unchanged; the shared `Cache-Control`/CORS middleware (generalized to
    treat `.mpd` the same as `.m3u8`) now lives at the origin level so it
    covers the shared resource route too.
  - `output::dash::DashOutput`: renders a live (`type="dynamic"`) MPD via
    `transmux::dash::DashPackager`, `$Number$`-addressed `SegmentTemplate`
    (not `$Time$`/`SegmentTimeline` — the store's `seg-{track}-{seq}.m4s`
    filenames are sequence-numbered, not time-addressed, so `$Number$` is
    the only mode whose URIs the shared resource route actually resolves),
    with `minimumUpdatePeriod`/`timeShiftBufferDepth`/
    `availabilityStartTime` derived from the store's target duration/window/
    construction time. Single-rendition model matching LL-HLS's own
    `DEFAULT_TRACK_ID` convention: the `Representation`'s `@id` is forced to
    `DEFAULT_TRACK_ID` regardless of the source's own track numbering, so
    `$RepresentationID$` substitution produces the same `init-1.mp4`/
    `seg-1-<N>.m4s` filenames LL-HLS already references. **True chunked-CMAF
    LL-DASH (`transmux::LlDashPackager`/`LlSegmenter`) is not implemented**
    — the store's `part-*.m4s` files are LL-HLS-shaped, not CMAF byte-range
    chunks; P4.2 below ships a signalled-MPD LL-DASH output addressing those
    existing parts instead, with true chunked transfer tracked as P4.3.
  - `ll_hls_runtime::server::MediaStore` gained the accessors a DASH
    renderer needs beyond LL-HLS's own bytes+timing: `set_track_specs`/
    `track_specs` (recorded once by `pipeline::run_pipeline` so DASH can
    build a real RFC 6381 `codecs` string), `window_segments` (a
    protocol-neutral snapshot of the closed-segment window), `created_at`
    (the live presentation's `availabilityStartTime` anchor); the previously
    crate-private `target_duration_secs`/`part_target_ms` accessors are now
    `pub` for the same cross-`Output` reason.
  - Config: `config::Route::outputs: Vec<output::OutputKind>` selects which
    protocol(s) to serve a route as (`"llhls"`/`"dash"`, per-route rather
    than a single global default — different routes may reasonably want
    different output sets), defaulting to LL-HLS only so every existing
    config is unaffected. `Config::validate` rejects an empty `outputs`
    list. `multimux-cli` gained `--outputs llhls,dash` (and the `--dash`
    shorthand for "both") on the single-route quick start.
- **LL-DASH output (low-latency DASH signalling)** (issue #663 P4.2 —
  `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`, "LL-DASH"): a
  new `output::ll_dash::LlDashOutput`/`OutputKind::LlDash` (`"ll_dash"`)
  renders `manifest-ll.mpd`, an LL-DASH-**signalled** MPD carrying
  `availabilityTimeOffset`, `<ServiceDescription><Latency target="…"/></ServiceDescription>`
  (ISO/IEC 23009-1 §5.13.2), and a `minimumUpdatePeriod` tuned to the part
  target — served at its own path (not a mode flag on `manifest.mpd`) so a
  route can enable `dash` (DVR) and `ll_dash` (live edge) together.
  - **Scope decision: discrete-parts signalling, not true chunked-transfer
    LL-DASH.** As flagged by P4's own follow-up note, the store's
    `part-*.m4s` files are LL-HLS-shaped (a whole extra fMP4 `moof`+`mdat`
    per part), not CMAF byte-range chunks within one in-progress segment —
    wiring `transmux::LlDashPackager`/`LlSegmenter` for *true* chunked
    delivery needs a second, chunk-shaped segmenter output, a larger lift
    than this story's scope. Instead, `LlDashOutput` re-addresses the
    **existing** parts: its `SegmentTemplate` uses `$Number$` addressing
    with `@duration` = the real part target (not the whole-segment target),
    `startNumber="0"`, and a media template that bakes the in-progress
    segment's sequence number in as literal text (refreshed on every
    fetch — the MPD is always `type="dynamic"`, never cached) around the
    real `$RepresentationID$`/`$Number$` tokens, so a real client's
    substitution produces exactly the `part-{track}-{seq}.{idx}.m4s`
    filenames the shared resource route already serves for `ll_hls`. This
    covers **only the live edge** (no `timeShiftBufferDepth` — an absent
    value is spec-honest "unknown", not a fabricated DVR window this
    origin cannot serve); pair with `dash`'s `manifest.mpd` for seek-back.
    `availabilityTimeOffset` is honestly `"0"`: a part is produced
    atomically (never partially available), so the low-latency win here
    comes from the small nominal segment(=part) duration, not partial
    delivery — reusing `transmux::LlDashPackager`'s `segment − chunk`
    formula would misrepresent that, so this module hand-rolls its own
    small `<ServiceDescription>`/`availabilityTimeOffset` XML injection
    instead. True chunked-transfer CMAF remains tracked as **P4.3**.
  - `ll_hls_runtime::server::MediaStore::latest_progress` (the in-progress
    segment's sequence number + live part count) is now `pub` (was
    `pub(crate)`) for the same cross-`Output` reason as `window_segments`/
    `track_specs` before it.
  - Config: `outputs: ["llhls", "dash", "ll_dash"]` is now accepted;
    `Config::validate`/serde behave the same as any other `OutputKind`
    (unknown tokens rejected, empty `outputs` rejected).
- **Generalized input model + UDP-family ingest** (issue #663 P3a —
  `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`, "Input
  adapters"): a route's ingest transport is now a tagged `config::InputSpec`
  (`Rtsp { url }` / `Rtp { addr, sdp, multicast_group }` / `TsUdp { addr,
  multicast_group }`, `#[serde(tag = "type", rename_all = "snake_case")]`),
  replacing the RTSP-only `Route::rtsp_url` field — a **breaking config
  change**: JSON routes now nest under `"input": { "type": "rtsp", "url":
  ... }` instead of a bare `"rtsp_url"` key. `origin::serve` dispatches each
  route to the matching `SourceConnector` with one `match` arm per
  `InputSpec` variant (kept monomorphized, not boxed, since each connector's
  `Source` associated type differs) — reconnect/backoff/health via
  `origin::supervisor::supervise` applies identically to every input kind.
  - `source::rtp_udp::RtpUdpSource` — raw RTP over UDP (uni/multicast, no
    RTSP control plane): binds a `tokio::net::UdpSocket` (+ optional
    multicast join via the new `source::udp::bind_udp` helper shared with
    `TsUdpSource`), parses the configured out-of-band SDP with the *same*
    `source::sdp::parse_sdp_tracks` RTSP already uses (no parallel SDP
    implementation), and depayloads with `transmux::RtpStreamDepacketiser`
    exactly as `source::rtsp::RtspSession` does. Since raw RTP/UDP has no
    RTSP interleaved-channel framing, incoming packets are routed to their
    track by RTP payload type (RFC 3550 §5.1) matched against each SDP
    media's declared payload type — `source::TrackInit` gained a
    `payload_type` field (populated identically for both the RTSP and raw-RTP
    ingest paths, since both share `parse_sdp_tracks`) and
    `source::sdp::load_sdp` loads an SDP body from either inline text or an
    `@path` file reference (re-read on every reconnect).
  - `source::ts_udp::TsUdpSource` — MPEG-2 Transport Stream over UDP
    (uni/multicast): binds the same shared UDP transport, then feeds
    datagrams to `transmux::StreamingTsDemux` (the same streaming demux core
    every other TS consumer in this workspace drives) until the in-band PMT
    resolves every declared track (bounded by a 10 s connect timeout) — the
    TS-over-UDP analogue of RTSP's DESCRIBE step — before the pipeline builds
    its segmenter.
  - No new codec/container parsing in multimux: both sources are transport
    (socket bind + multicast join) plus wiring over transmux's existing
    `RtpStreamDepacketiser`/`StreamingTsDemux`.
  - `Config::validate` now validates every route's `InputSpec` fields (RTSP
    scheme, UDP address parseability, multicast-group IP validity, RTP SDP
    non-empty/parseable) in addition to the existing duplicate-name/timing
    checks.
- **HTTP-based ingest: TS-over-HTTP + HLS-pull** (issue #663 P3c / #717 —
  `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`, "Input
  adapters" / "Shared auth layer"): two new `InputSpec` variants,
  `TsHttp { url }` and `HlsPull { url }`, both `http(s)://` URLs that may
  carry `user:pass@` userinfo (redacted in `Debug`, validated for scheme by
  `Config::validate`).
  - `source::ts_http::TsHttpSource` — MPEG-2 Transport Stream over a
    streaming HTTP GET (chunked/progressive `reqwest`, `stream` feature):
    reads response chunks into `transmux::StreamingTsDemux` until the
    in-band PMT resolves every declared track (mirrors `TsUdpSource`'s own
    connect-time PMT wait, bounded the same 10 s). Unlike UDP, the HTTP body
    stream *does* end — `next_samples` returns `Ok(None)` on end-of-stream,
    so `origin::supervisor::supervise` reconnects exactly as for any other
    source's EOF.
  - `source::hls_pull::HlsPullSource` — wraps
    `ll_hls_runtime::client::tokio_client::TokioClient` (the sans-IO LL-HLS
    playback client engine, driven over real HTTP) as a `SourceConnector`/
    `SampleSource`: `connect()` drives the client until its first
    `Output::Init`, recovering the pulled stream's `TrackSpec`s by feeding
    those init bytes through `transmux::Fmp4Demux` once (the *same* demuxer
    the client itself already uses internally — no hand-rolled `moov`
    parse); `next_samples()` relays `Output::Samples` straight through. No
    re-demuxing: the client's own `Fmp4Demux`-based decode is reused
    verbatim.
  - `source::http_auth` — shared auth glue for both HTTP sources: reqwest
    answers Basic/Bearer natively, but not Digest (RFC 7616), so
    `authenticated_get` sends once and, on a `401`, answers the
    `WWW-Authenticate` challenge via the new `broadcast-auth` crate (issue
    #663 P3b) before resending — the same shared `Credentials`/
    `Authenticator` model `rtsp-runtime` already uses. Credentials come from
    the ingest URL's userinfo (mirrors `source::rtsp`'s own handling,
    generalized to any URL).
  - `ll-hls-runtime`'s `client::tokio_client::TokioClient` was itself
    upgraded in lockstep to authenticate via `broadcast-auth` (Basic/Digest/
    Bearer, replacing its previous ad hoc Basic/Bearer-only `Auth` enum) —
    see `ll-hls-runtime`'s own changelog — so `HlsPullSource` gets Digest
    support for free rather than multimux re-implementing the challenge/
    response for the pull path.
  - No new codec/container parsing in multimux: `ts_http` is transport
    (streaming GET) plus `StreamingTsDemux`; `hls_pull` is a thin wrapper
    over `ll-hls-runtime`'s existing client engine + `Fmp4Demux`.

### Security
- **HTTP resource limits on the origin listener** (issue #663 P5, audit-
  concurrency #3): a new `origin::HttpLimits` (`request_timeout`,
  `max_concurrent_requests`, `max_request_body_bytes`) is applied to every
  route via `tower_http::timeout::TimeoutLayer` (default 10 s — comfortably
  above the 5 s LL-HLS blocking-reload cap, so an ordinary long-poll request
  is unaffected), `tower::limit::ConcurrencyLimitLayer` (default 4096), and
  `tower_http::limit::RequestBodyLimitLayer` (default 16 KiB — no legitimate
  request here carries a body). Configurable via the new
  `Config::request_timeout_secs`/`max_concurrent_requests`/
  `max_request_body_bytes`; `Config::validate` rejects a
  `request_timeout_secs` at or below the 5 s blocking-reload cap (it would
  cut off a legitimate blocking reload before the LL-HLS engine's own
  timeout ever gets to resolve or fall back). Defaults preserve every
  existing config/deployment's behaviour.
- **Configurable ingest connect/read timeouts** (issue #663 P5, audit-ingest
  #3): a new `source::IngestTimeouts { connect, read }` (default 10 s
  connect / 30 s read) is now threaded through every ingest source
  (`RtspSource`/`TsUdpSource`/`RtpUdpSource`/`TsHttpSource`/`HlsPullSource`,
  each via a `with_timeouts` builder), bounding both the initial
  connect/handshake step and every subsequent read so a source that never
  responds — or stops responding — surfaces a recoverable error for
  `origin::supervisor::supervise` to reconnect on, rather than hanging a
  route's ingest task forever. Configurable via the new
  `Config::ingest_connect_timeout_secs`/`ingest_read_timeout_secs` (applied
  uniformly to every route); `Config::validate` rejects a non-positive
  value. Defaults preserve every existing config's behaviour.
- **UDP ingest read-timeout** (issue #663 P5.2, audit-ingest #3):
  `source::rtp_udp::RtpUdpSource`/`source::ts_udp::TsUdpSource`'s
  `next_samples()` previously called `UdpSocket::recv` with no timeout — a
  source that stopped sending (dropped multicast feed, wedged encoder) left
  the read pending forever, so `origin::supervisor::supervise` never saw an
  error to reconnect on. Both sessions' per-datagram `recv` is now wrapped in
  `tokio::time::timeout(self.read_timeout, …)`; on expiry `next_samples()`
  returns the same recoverable `MultimuxError::Connect` the supervisor
  already reconnects on for every other read error. `RtpUdpSource` gained
  the `timeouts: IngestTimeouts` field + `with_timeouts` builder it was
  previously missing (mirroring `TsUdpSource`/`RtspSource`); no config or
  behaviour change for a healthy source (default read timeout unchanged at
  30 s).
  - **Deferred (documented, not implemented this pass)**: RTCP Sender
    Report -> wall-clock A/V sync (issue #663 P5.2, audit-ingest #9/#10) —
    `source::rtsp::route_channel` (the interleaved RTCP channel) and
    `source::rtp_udp::RtpUdpSource::connect` (the RTCP companion UDP port)
    each still discard/never bind RTCP; both carry a
    `// TODO(P5.3): RTCP SR wallclock A/V sync` at the exact drop point.
    Judged too large a lift to land safely alongside the bounded-buffer and
    read-timeout hardening above (it would mean redesigning
    `transmux::rtp_stream`'s per-track timing/rebase model, not just this
    crate) — raw per-track RTP-timestamp rebasing is unchanged.

### Changed
- **LL-HLS origin engine moved to `ll-hls-runtime::server`** (issue
  #663/#717 Stage 2 —
  `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`, "ll-hls-runtime
  — client + server in one crate"): `multimux` is now a thin tokio+axum
  adapter over the sans-IO engine, mirroring how it already wraps
  `rtsp-runtime` on the input side. Behaviour-preserving — every existing
  test still passes, same served bytes/URLs/timing:
  - `store::MediaStore`/`store::HealthState` are now re-exports of
    `ll_hls_runtime::server::{MediaStore, HealthState}` (the rolling window,
    the part-404-boundary fix, and health tracking moved there verbatim);
    `crate::store::...` call sites are unaffected.
  - `output::llhls` no longer renders playlists or decides blocking-reload/
    part-availability outcomes itself — it calls
    `MediaStore::resolve_playlist`/`resolve_resource` and drives the actual
    bounded `.await` (the one thing the sans-IO engine can't do): on
    `WouldBlock`, it registers `MediaStore::listen()` before re-resolving (no
    missed-wakeup race), then awaits the listener under its own
    `tokio::time::timeout` (still the 5 s `BLOCKING_RELOAD_TIMEOUT`). The
    reentrant-lock deadlock fix in playlist rendering is preserved (now in
    `ll_hls_runtime::server::media_playlist_m3u8`).
  - New dependency: `ll-hls-runtime` (path + version, `std` feature).

### Added
- **Supervised route lifecycle** (issue #663, P0.2+P0.3+P0.4): each route's
  ingest task is now driven by `origin::supervisor::supervise`, which
  reconnects with capped exponential backoff (`origin::supervisor::Backoff`,
  default 500ms min / 30s max / factor 2.0) on connect failure, pipeline
  error, *or* source end-of-stream, instead of the old one-shot task that
  died for good on the first failure (leaving the HTTP origin serving a
  frozen last playlist as `200 OK` forever). The connect step is abstracted
  behind `origin::supervisor::SourceConnector` (implemented for
  `source::rtsp::RtspSource`) so reconnect is testable without a real RTSP
  server. A route never gives up permanently by default — sources like
  cameras come back.
- **Store health** (`store::MediaStore::{health, set_health}` /
  `store::HealthState`): each route's `MediaStore` now tracks
  `Connecting`/`Live`/`Reconnecting`/`Failed`, set by the supervisor at each
  connect/pipeline transition; a state change bumps the store's existing
  progress watch so a blocked reader (e.g. an LL-HLS long-poll reload) wakes
  on a health transition too, not just new media.
- **Graceful shutdown**: `origin::serve` now installs a shutdown signal
  (Ctrl-C, plus `SIGTERM` on unix) that both drains in-flight HTTP requests
  via `axum::serve(..).with_graceful_shutdown(..)` and breaks every route's
  supervise loop; `serve` joins each supervisor task (aborting one that
  doesn't return within a short grace period) before returning `Ok(())`,
  rather than leaving ingest tasks running detached past shutdown.
- **Structured errors + secret redaction + tracing** (issue #663, P1a):
  - `error::MultimuxError` replaces the stringly `Config(String)`/
    `Source(String)` variants with field-carrying `thiserror` variants
    (`ConfigRead`/`ConfigParse`/`ConfigInvalid`/`Connect`/`Protocol`/`Sdp`/
    `Auth`/`Depay`, plus the existing `Transmux`/`Io`), so callers can match
    on failure *kind* instead of parsing a string, following the
    `rtsp-runtime` convention.
  - **Secret redaction**: an RTSP source URL's `user:pass@` userinfo can no
    longer leak into an error message, `Debug` output, or a log line.
    `config::Route` and `source::rtsp::RtspSource` now have manual `Debug`
    impls that redact `rtsp_url`/`url` to `***@host`; every connect-time
    error path (bad URL parse, connect/TLS/SNI failure, userinfo-stripping
    failure) redacts or uses the already userinfo-stripped URL rather than
    the raw credentialed one.
  - `tracing` throughout: `origin::supervisor::supervise` is
    `#[tracing::instrument]`ed per route (connect/live `info!`, disconnect/
    reconnect `warn!` with backoff delay + attempt count, health
    transitions logged) and `origin::serve` logs startup/shutdown/aborted
    supervisor tasks, replacing the ingest supervisor's `eprintln!`s. The
    library only emits events — `multimux-cli` owns subscriber init
    (`tracing-subscriber` `fmt` + `EnvFilter`, `RUST_LOG`-overridable,
    default `info`); the CLI's own top-level fatal-error report stays a
    plain `eprintln!` so it's never swallowed by a log filter.
- **Prometheus metrics + health/readiness endpoints** (issue #663, P1c):
  - New `prometheus` module: installs a single process-wide
    `metrics-exporter-prometheus` recorder (idempotent — safe to call from
    every `AppState::new`, including many tests sharing one process) and
    renders its snapshot for `GET /metrics`.
  - Metrics recorded throughout the crate via the `metrics` facade:
    `multimux_route_up` (gauge, `route`; mirrors `HealthState` — 1.0 while
    `Live`), `multimux_source_reconnects_total` (counter, `route`; bumped by
    `origin::supervisor::supervise` on each `Reconnecting` transition),
    `multimux_segments_produced_total`/`multimux_parts_produced_total`
    (counters, `route`; bumped in `pipeline::run_pipeline`, which now takes a
    `route: &str` parameter for this label),
    `multimux_active_blocking_requests` (gauge; inc/dec around
    `output::llhls`'s blocking `wait_for_progress`/`wait_for_part` waits via
    an RAII guard), and `multimux_http_requests_total`/
    `multimux_http_request_duration_seconds`/`multimux_bytes_served_total`
    (labels `route`, `path`, and `status` for the requests counter; recorded
    by a new `origin::router` global middleware layer for every request,
    root endpoints included). Cardinality is bounded on purpose: `route` is a
    configured stream name or `"unknown"`, `path` is one of a small fixed set
    of kinds (`playlist`/`segment`/`part`/`init`/`metrics`/`health`/`other`).
  - `GET /healthz` (liveness, always 200) and `GET /readyz` (readiness: 200
    once at least one configured route is `Live`, 503 otherwise) mounted at
    the origin root alongside `/metrics`, above the per-stream `/{stream}/`
    nests.
  - `origin::AppState` gained a `metrics_handle` field and an `AppState::new`
    constructor (replacing the old bare struct literal at every call site).

### Fixed
- **LL-HLS spec-conformance** (issue #663, P2 — RFC 8216bis):
  - `#EXT-X-TARGETDURATION` is now `round(max(configured target, max actual
    segment duration ever seen))`, not `ceil(configured target)`. The
    segmenter cuts on the next keyframe *after* the configured target, so a
    real segment routinely runs longer than the configured value — advertising
    the configured target alone under-declared TARGETDURATION and violated RFC
    8216bis §4.4.3.1 (a MUST: every Media Segment's rounded EXTINF ≤
    TARGETDURATION). `store::MediaStore` now tracks a lifetime
    `max_segment_duration` (never reset on window eviction) that the LL-HLS
    renderer folds into the tag.
  - Blocking-reload `_HLS_msn` semantics (§6.2.5.2): a bare `_HLS_msn` (no
    `_HLS_part`) now waits until segment `msn` is a fully-present **closed**
    Media Segment, rather than resolving as soon as the segment merely *opens*
    with one live part (the old `unwrap_or(0)` conflated it with
    `_HLS_part=0`). `_HLS_msn`+`_HLS_part` keeps the existing part-count
    semantics.
  - `_HLS_msn`/`_HLS_part` abuse bounds (§6.2.5.2): `_HLS_part` without
    `_HLS_msn`, or an `_HLS_msn` more than a small bound beyond the current
    live edge, is now rejected promptly with `400 Bad Request` instead of
    always blocking to the 5 s timeout and returning `200`.
  - `Cache-Control` + permissive CORS on every origin response: immutable
    `max-age=31536000, immutable` on init/segment/part byte ranges, `no-cache`
    on playlists, and `Access-Control-Allow-Origin: *` (+ methods/headers, with
    an `OPTIONS` preflight handler) on everything — browser LL-HLS players
    (hls.js) are commonly on a different origin than the API.
  - **`GET /media.m3u8` deadlocked on every request** (found by
    `ll-hls-client`'s issue #717 slice 5 acceptance test, the first thing in
    the workspace to ever drive this endpoint over a real HTTP round trip
    rather than calling `output::llhls::media_playlist_m3u8` directly):
    `media_playlist_m3u8` called `store::MediaStore::with_segments_and_parts`
    (which locks `MediaStore`'s internal `std::sync::Mutex`) and, from
    *inside* that closure, called `store.max_segment_duration()` — which
    locks the same, non-reentrant mutex again. Every request to `/media.m3u8`
    (blocking or not, empty store or not) self-deadlocked the handling task
    forever. `target_duration_secs()`/`max_segment_duration()` are now read
    *before* taking the `with_segments_and_parts` lock.

## [0.2.2] - 2026-07-18

### Fixed
- **LL-HLS preload-hint parts no longer 404 at every segment boundary.** The
  segmenter emits a segment's *final* part and closes the segment in the same
  step; `add_segment` then evicted that segment's parts from `live_parts`
  immediately — so the final part (exactly the one the `#EXT-X-PRELOAD-HINT`
  points at) existed for only microseconds, and the in-flight blocking part
  request that raced the close still 404'd. 0.2.1 made not-yet-produced parts
  *block*; this makes the just-produced final part *survive*: `add_segment` now
  moves a closed segment's parts into a bounded `recent_parts` buffer that
  `part_bytes` also checks, so the hinted final part is served (HTTP 200) after
  its segment closes instead of 404ing. Eliminates the per-segment 404 spam +
  the boundary latency bump. Bounded oldest-first like `live_parts`; closed
  parts are still not rendered in the playlist (the whole segment is).

### Fixed
- **LL-HLS preload-hint parts no longer 404.** A request for a Partial Segment
  the media playlist promised via `#EXT-X-PRELOAD-HINT` but that the origin had
  not produced yet returned `404` immediately, instead of holding the request
  open until the part became available (RFC 8216bis §6.2.2 / §6.3.1 blocking
  Partial-Segment delivery). Every low-latency client (hls.js, Safari native)
  therefore hammered the hinted part with 404s until it happened to exist,
  spamming errors and forcing a fall back to full-segment loads — defeating the
  low-latency path. The part byte handler now blocks (reusing the same progress
  watch as the blocking playlist reload) until the part is produced, or returns
  `404` *promptly* once its segment closes without it (a real segment boundary)
  or the blocking timeout elapses. Observed against a live on-camera stream.

### Breaking
- The bundled `multimux` **binary** (the RTSP→LL-HLS CLI) moved to a new
  dedicated crate, **`multimux-cli`**. `multimux` is now a **library only**
  (its `serve`/`config`/`origin`/`pipeline`/`source`/`store` API is unchanged).
  `cargo install multimux-cli` provides the `multimux` binary as before. The
  `cli` cargo feature (and the `clap` dependency) were removed from `multimux`.

## [0.1.0] - 2026-07-15
### Added
First release (issue #663): a live RTSP → LL-HLS just-in-time repackaging HTTP
origin — a thin client + server wrap around `rtsp-runtime` (RTSP pull) and
`transmux` (RTP depayload + LL-HLS CMAF segmentation).

- **Config** (`config::Config`/`Route`): CLI-first, with an optional JSON
  config file for multiple routes; `bind`, `target_duration_secs`,
  `part_target_ms`, `window_segments`, and `routes: [{ name, rtsp_url }]`;
  `Config::validate()` rejects empty route sets, duplicate stream names, and
  nonsensical timing/window values.
- **RTSP ingest** (`source::rtsp::RtspSource`/`RtspSession`): DESCRIBE → SETUP
  (interleaved TCP, one media per SETUP) → PLAY over
  `rtsp_runtime::io::AsyncRtspClient`; SDP → per-track `CodecConfig` via
  `transmux`'s SDP-fmtp helpers; interleaved RTP routed per channel into
  `transmux::RtpStreamDepacketizer`, yielding timed `Sample`s.
- **Per-route pipeline** (`pipeline::run_pipeline`): drives a `SampleSource`
  (real `RtspSession` or, for tests/examples, `MockSource`) through a
  `transmux::ll_hls::LlHlsSegmenter`, publishing every init segment, ready
  part, and ready segment into a `StreamStore`; flushes the buffered tail at
  end-of-stream.
- **`StreamStore`** (`store::StreamStore`): per-stream in-RAM rolling window
  (init segment + a bounded `VecDeque` of closed segments + the in-progress
  segment's live parts), oldest segment evicted on roll; a
  `tokio::sync::watch` bumped on every new part/segment drives blocking
  playlist reload; renders the LL-HLS media playlist per RFC 8216bis
  (`#EXT-X-PART-INF`/`#EXT-X-SERVER-CONTROL`/`#EXT-X-PART`/
  `#EXT-X-PRELOAD-HINT`), never advertising an `#EXTINF`/URI for an
  unclosed segment (§4.4.4.9).
- **HTTP origin** (`origin::{router, serve}`, axum): `master.m3u8`,
  `media.m3u8` (blocking reload on `_HLS_msn`/`_HLS_part`, RFC 8216bis
  §6.2.5.2, bounded so a stalled source can't hang a request forever), and a
  catch-all serving the dynamic `init-*.mp4`/`seg-*.m4s`/`part-*.m4s`
  filenames the playlist emits. `origin::serve(config)` wires one
  `StreamStore` + one spawned RTSP pipeline task per configured route, then
  binds and serves — a single bad/unreachable source logs to stderr and ends
  only that route's task, never the server.
- **CLI binary** (`multimux`, `cli` feature, on by default): `--config <FILE>`
  or the single-route quick start `--rtsp <URL> --name <NAME>`, plus
  `--bind`/`--target-duration`/`--part-ms`/`--window`, per
  `docs/CLI-STANDARD.md`.
- **Examples**: `serve_mock` (synthetic stream, no RTSP/network needed) and
  `serve_rtsp` (serves one real RTSP URL given on the command line).

### v1 scope
LL-HLS only (DASH/LL-DASH is v1.1); RTSP pull only (no SRT/TS/file ingest); no
per-viewer sessions/SSAI/manifest rewrites; no DVR/VOD/disk spill (RAM-only
rolling window); no TLS/auth (front it with a reverse proxy); no trick-play.
