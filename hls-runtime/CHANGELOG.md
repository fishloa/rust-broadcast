# Changelog

All notable changes to `hls-runtime` (formerly `ll-hls-runtime`) are
documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]


## [0.7.0] - 2026-10-05

### Security
Fixes GHSA-grg8-55qr-gxgf. Upgrade if you use `client::tokio_client::TokioClient` with
`TokioClientConfig::auth` set.

### Fixed
- `TokioClient` sends `TokioClientConfig::auth` credentials (Basic, Bearer,
  and Digest challenge answers) only to the playlist URL's origin (scheme +
  host + port); requests a playlist directs to any other host go without an
  `Authorization` header.
- `TokioClient` no longer retries a resource fetch answered with a `4xx`
  other than `408`/`429` (e.g. a `404` for a stale preload hint); it goes to
  `HlsClient::on_error` at once instead of after the full backoff.

- Defect 7: relative playlist references containing `..` were joined textually (`http://h/a/b/../x.m4s`); see the URL resolution entry above.
- `HlsOrigin`'s locks are poison-tolerant (a panic in one request no longer
  panics every later one), `render_playlist` copies the window and releases
  its lock before querying the `Trunk`, and a request with no segment
  published since the last drain skips the cursor and window locks (audit
  r09-O4, #1089, #1134).
- Client state no longer grows for the life of a pull: every record keyed by
  a Media Sequence Number below the playlist's first segment, and the
  byte-range cursor of every URL a full playlist no longer references, is
  dropped after each playlist; a fetch still in flight keeps being accepted
  (#1089).
- `HlsClient`'s Media Sequence arithmetic, `EXT-X-SKIP` merge index and part
  indexes use checked/`try_from` conversions instead of `as` casts and
  unchecked additions (#1089).
- `server::engine`'s DATERANGE-per-window render now follows
  `timed-metadata::daterange::DateRange::to_tag_line`'s new fallible
  signature (issue #1140): an event whose DATERANGE can't be rendered as a
  valid attribute list (e.g. a `"`/CR/LF from an upstream
  `segmentation_upid`, or a non-finite duration) is skipped for that
  window, the same treatment already given to a `to_daterange` failure,
  rather than the crate failing to build.

### Added
- `server::HlsOrigin` now renders SCTE-35 cues published to the trunk's
  event ring as `#EXT-X-DATERANGE` tag lines in Media Playlists (issue
  #965). Events are per-segment via `Trunk::events_in_segment`, and carry a
  wall-clock `START-DATE` (via `timed_metadata::Timeline::with_anchor`) only
  once the trunk has been given a `time_anchor`; events skipped otherwise.
  Rendered after the unconditional `#EXT-X-MAP` line under `Container::Fmp4`.
  Purely internal to `render_playlist` — no new public API on `HlsOrigin`.

- `HlsClient::next_wait()` (the queued `WaitMs` hint as a `Duration`, `no_std`) and `HlsClient::poll_timeout(&mut self, now)` (`std`): the wait's ABSOLUTE deadline, anchored at the first query and identical on every re-query until the wait is drained with `poll()`, so unrelated wake-ups cannot re-arm it. A wait is queued behind the fetches `on_playlist` queued with it, so it is reported once those are drained.
- `server::HlsOrigin::master_playlist(name)` — a master playlist whose
  `BANDWIDTH` is the measured peak segment bitrate (segment bytes over
  duration, rounded up, never lowered when the peak segment leaves the
  window) and whose `CODECS` is derived from the init segment; before the
  first closed segment it falls back to the 5 Mb/s estimate. New
  `HlsOrigin::set_track_specs` supplies the tracks for a TS origin, which has
  no init segment to read (#1089). Its error is the typed, `#[non_exhaustive]`
  `server::HlsMasterError` (`BandwidthOverflow`, `Render`).
- `server::HlsOriginBuilder::media_sequence_offset(u64)`,
  `server::HlsOrigin::media_sequence_offset()` and
  `server::HlsOrigin::next_media_sequence()` — a fresh `Trunk` restarts its
  segment numbers at 1, so an origin replacing another (a reconnect) passes
  `previous.next_media_sequence()` as the offset: the Media Sequence Number
  in the playlist and `_HLS_msn` then keeps increasing (RFC 8216bis §6.2.2),
  skipping the number the previous origin's *open* segment was shown with
  (its parts were already served) (#1089).
- `server::HlsOriginBuilder::instance(u64)`, `server::HlsOrigin::instance()`
  and `server::HlsOrigin::init_name(track, generation)` — the per-origin
  instance token (see the naming entry under "Changed (breaking)");
  `server::ClosedSegment::init_generation` (and `with_init_generation`) names
  the init generation a segment was cut against (#1030).
- `client::Error::DuplicateResource` and `client::Error::MediaSequenceOverflow`
  (`Error` is `#[non_exhaustive]`); `server::HlsOriginBuildError::ZeroPartTarget`,
  `MediaSequenceOffsetTooLarge` and `InvalidTargetDuration` (a NaN, infinite,
  zero or negative `target_duration_secs` is a build error instead of reaching
  `EXT-X-TARGETDURATION`) (#1089).

### Changed (breaking)
- **`reqwest` 0.12 → 0.13.** `tokio::TokioError::Http`'s `source` is still
  `reqwest::Error`, but the `reqwest` type's epoch changes. The `tokio`
  feature's `rustls-tls` feature slot is renamed to `rustls` (reqwest 0.13's
  name).

- `client::TokioClientConfig` gained `connect_timeout` (10 s), `jitter` (`true`) and `cancel` (a `tokio_util::sync::CancellationToken`) with `with_auth`/`with_cancel`/`with_connect_timeout`/`with_jitter`: a struct literal without `..Default::default()` no longer compiles. Once `cancel` fires, `TokioClient::next_output` abandons its in-flight request or backoff sleep and returns `Ok(None)`. Retries use `backon`'s exponential schedule with jitter (each delay `d` becomes a random value in `[d, 2d)`, clamped to `max_retry_backoff`, which stays a hard maximum); `TokioError::Stalled` replaces the 10 ms defensive sleep when the core queued no action and the stream had not ended (never observed); byte ranges are sent as a typed `headers::Range`. The `tokio` feature now needs `backon`, `headers` and `tokio-util`.
- Playlist URI resolution is RFC 3986 (`url::Url::join`): `..` segments are resolved (`../x.m4s` against `http://h/a/b/p.m3u8` is `http://h/a/x.m4s`, not `http://h/a/b/../x.m4s`) and a `://` inside a relative reference's query no longer makes it look absolute. Results are normalised (`HTTP://H.Example/X` becomes `http://h.example/X`, spaces are percent-encoded, a default port is dropped). A relative playlist URL still gives a relative result.
- `Action::playlist_request_url` builds the query with `query_pairs_mut`: the `_HLS_msn`/`_HLS_part`/`_HLS_skip` pairs now precede a fragment (`...?a=1#f` gives `...?a=1&_HLS_msn=5#f`; the old string builder produced `...?a=1#f&_HLS_msn=5`, where the pair was part of the fragment and never sent).
- `EXT-X-PROGRAM-DATE-TIME` is parsed with `jiff::Timestamp`: a leap second (`23:59:60`) is clamped to `:59`, where the hand-written parser added 60 s to the minute.
- **Every resource name served `immutable` carries a per-origin instance
  token** (audit r09-C2, #1030). `HlsOrigin::instance()` is a number that
  differs for every origin built — in this process (strictly increasing) and
  across process restarts (seeded from the wall clock in milliseconds, no
  persistence needed) — and is part of every cached-`immutable` name:
  `init-{track}-{instance}-{generation}.mp4`,
  `seg-{track}-{instance}-{msn}.{ext}`,
  `part-{track}-{instance}-{msn}.{idx}.{ext}`. A name therefore maps to one
  origin's bytes for ever: a reconnect (a new `Trunk`, numbers restarting,
  possibly a different SPS/ASC), a restart with no DVR, a replacement origin
  reusing the open segment's number — none can put new bytes under a name a
  cache holds. Playlists name these URIs, so players are unaffected; a
  request carrying another instance's token is `NotFound`. The token-less
  `seg-{track}-{msn}`/`part-{track}-{msn}.{idx}` (DASH/Smooth templates cannot
  carry the token) still resolve but are `CachePolicy::NoCache`, as is the
  bare `init-{track}.mp4`, which now means "the current init". The pre-release
  two-field `init-{track}-{generation}.mp4` form is gone.
  A changed init (`set_init` with different bytes) is a new generation; the
  last 8 stay resolvable. The playlist's `EXT-X-MAP` now sits on the segments
  (each names the init it was cut against) instead of one unconditional line,
  with `EXT-X-DISCONTINUITY` before the first segment using a new generation —
  and, as rendered by `broadcast-hls`, **before** that segment's
  `EXT-X-MAP` (previously the one `EXT-X-MAP` line came first, before every
  segment-level tag) (#1030).
- The offset passed to `HlsOriginBuilder::media_sequence_offset` for a
  replacement origin is now `previous.next_media_sequence()` (was
  `.saturating_sub(1)`): the previous origin's open segment number is skipped
  (#1030).
- **`HlsClient` treats a Media Sequence Number below the previous playlist's
  as an origin restart** (audit r09-C3, #1031) unless it is an older copy of the
  same stream — the numbers a window shares with the previous one name the same
  segments (URI, duration, byte range, `PROGRAM-DATE-TIME`), or, for a wholly
  older window, `PROGRAM-DATE-TIME` shows it ends before the previous one began
  (its segments are then skipped, never delivered behind what was delivered).
  A restart to an equal or higher number is detected the same way (a shared
  number naming a different segment or time, or a decreasing
  `EXT-X-DISCONTINUITY-SEQUENCE`), as is a name reused under a lower number.
  On a restart: all
  sequence-keyed state, queued-but-unpolled fetches and the init are dropped,
  the live-edge join is redone, and `Output::Discontinuity` is emitted, so
  new segments reusing delivered numbers are no longer skipped as "already
  delivered". A response still in flight for the old numbering is rejected as
  `Error::UnrequestedResource`. Undetectable from the playlist alone (documented
  on `HlsClient`): an edge lagging by a whole window behind an origin with no
  `PROGRAM-DATE-TIME` and all-new names reads as a restart (one spurious
  `Discontinuity`), and a restart to a higher, non-overlapping number with a
  consistent discontinuity sequence and no `PROGRAM-DATE-TIME` reads as the
  client having fallen behind (#1031).
- **`TokioClient` retries a blocking or delta playlist reload that the origin
  rejects with a non-transient `4xx` once, at once, as a plain GET** of the
  playlist URL, instead of repeating the rejected URL forever (an origin
  restarted below the requested `_HLS_msn` answers `400`) (#1031).
- **`HlsClient` joins at the server's hold-back, not at the start.** On the
  first playlist it no longer fetches the whole window. A live playlist (no
  `EXT-X-ENDLIST`) is joined no closer to its end than `PART-HOLD-BACK` when
  the client is in Low-Latency Mode (`CAN-BLOCK-RELOAD=YES`; the open
  segment's parts count towards the distance), else the larger of `HOLD-BACK`
  and three Target Durations (RFC 8216 §6.3.3, RFC 8216bis §4.4.3.8/§6.3.3).
  `EXT-X-START` is honoured at segment granularity (a live start is pulled
  back to the hold-back); a VOD playlist without it is played from its start.
  The byte-range cursor advances over skipped segments, so omitted-offset
  `EXT-X-BYTERANGE` continuations stay correct (#1089).
- **`HlsClient::on_resource` rejects a second delivery of an id it already
  accepted** with `Error::DuplicateResource`, instead of emitting the samples
  twice and decrementing the outstanding-fetch count again (#1089).
- **`HlsClient` rejects a playlist whose `EXT-X-MEDIA-SEQUENCE` plus segment
  count overflows `u64`** with `Error::MediaSequenceOverflow` (it used to
  overflow-panic in debug and wrap in release) (#1089).
- **Classic MPEG-TS-segment HLS keeps one demuxer across segments**, so the
  33-bit PTS/DTS wrap is unrolled across a segment boundary and each
  segment's last access unit gets its real duration. As a consequence the last
  access unit per stream of a segment is emitted when the next segment
  arrives (at `EXT-X-ENDLIST` or `EXT-X-DISCONTINUITY` it is flushed). A new
  `Output::Init` follows when the track set changes, and the demuxer restarts
  at `EXT-X-DISCONTINUITY` (#1089).
- **`HlsOrigin` holds a part request open only for the hinted part.** A part
  request is `NotFound` at once unless it is the one after the last part the
  ring holds, in the open segment or the one after it (an evicted or
  never-hinted index, and anything more than two segments past the last
  closed one, no longer parks for the blocking timeout); a classic (non-LL)
  origin answers a request for a part the `Trunk` does not hold with
  `NotFound` (#1089).
- **`HlsOrigin` answers a playlist `_HLS_part` beyond the Advance Part Limit
  with `BadRequest`** (RFC 8216bis §6.2.5.2: three divided by the part target
  when that is under a second, else three, past the last part of the live-edge
  segment) instead of parking it (#1089).
- **`HlsOrigin` playlists keep `EXT-X-PART` tags on a closed segment** while it
  is within three Target Durations of the end (RFC 8216bis §6.2.2), and only
  when every part of it is still in the `Trunk`'s part ring (#1089).
- **`HlsOrigin` restarts its window when sequence numbers are not
  consecutive**, or after the cursor reports lost segments: the old entries
  roll off, the next segment is marked `EXT-X-DISCONTINUITY`, and
  `EXT-X-MEDIA-SEQUENCE` can no longer disagree with a segment's real number.
  `bytes_of` addresses exactly one entry (#1089).
- **`HlsOriginBuilder::window_segments` below 3 is raised to 3** (RFC 8216bis
  §6.2.2: a live playlist is never shortened below three Target Durations)
  (#1089).
- `HlsOriginBuilder::low_latency(0)` and an offset that cannot hold the `u32`
  sequence range are now build errors (#1089).
- `HlsOrigin::set_init` with an init that cannot be read now drops the
  `CODECS` derived from the previous init instead of keeping them (#1089).

## [0.6.0] - 2026-08-11

### Added
- `server::HlsOrigin::closed_segments()` — a snapshot of the origin's
  currently-advertised closed segments (sequence number, absolute
  `start_ns`, duration, discontinuity bit) as the new public
  `server::ClosedSegment` (with a `ClosedSegment::new` constructor, since
  the type is `#[non_exhaustive]`). Reuses the origin's existing live-
  window cursor rather than requiring a caller to open a second one on the
  same `Trunk` just to learn the same window `render_playlist` itself
  already renders. Added for multimux's DVR catch-up serving (issue #900),
  which needs to merge this origin's live window with a different segment
  source (an on-disk archive) over the same sequence-number space.

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Fixed
- Doc accuracy (#941 rows 7-8): README install snippet corrected from
  `"0.1"` to `"0.5"` (the crate is 0.5.0). README now states explicitly
  that `server::HlsOrigin` does not emit `EXT-X-SKIP`/`CAN-SKIP-UNTIL`/
  `EXT-X-RENDITION-REPORT`, so the bundled client's Playlist Delta Update
  support (`ClientSession::merge_delta`) cannot be exercised against the
  bundled server. No behaviour change — the bundled server has never
  implemented server-side delta updates.
- `client::HlsClient` no longer panics (debug) or silently computes a wrong
  byte range (release) from an untrusted remote playlist's
  `EXT-X-BYTERANGE`/preload-hint length (RFC 8216bis §4.4.4.9/§4.4.5.3):
  `offset + length` — and the per-URL omitted-offset running cursor it
  accumulates into — is now checked (`checked_add`), returning the new
  `client::Error::ByteRangeOverflow` instead of wrapping. `merge_delta`'s
  `EXT-X-SKIP` `SKIPPED-SEGMENTS` handling (§4.4.5.2) got the same
  overflow guard, falling back to its existing "return the delta unmerged"
  path rather than panicking. `client::tokio_client::TokioClient`'s
  `Range:`-header builder got the matching `checked_add` guard
  (`TokioError::ByteRangeOverflow`) as defense-in-depth, though in normal
  operation the sans-IO core already rejects an overflowing range before
  it reaches the HTTP adapter. `broadcast_hls::MediaPlaylist::parse` places
  no upper bound on any of these values (confirmed while fixing this) —
  tracked separately as a `broadcast-hls` follow-up, not fixed here.

## [0.5.0] - 2026-08-05

### Changed
- Requires `transmux` 0.23 (epoch-pure caret bump from ^0.22).
- Requires `media-plane` 0.3 (epoch-pure caret bump from ^0.2).

## [0.4.0] - 2026-08-03

### Changed (Breaking)
- **Crate renamed** `ll-hls-runtime` -> `hls-runtime` (issue #868): the
  crate is gaining a non-low-latency mode, so the `ll-` name became wrong.
  A clean break — no deprecated alias, no re-export shim. `ll-hls-runtime`'s
  already-published versions stay live on crates.io, untouched.
- Public types dropped their `Ll` prefix: `LlHlsOrigin` -> `HlsOrigin`,
  `LlHlsRequest` -> `HlsRequest`, `LlHlsBody` -> `HlsBody`, `LlHlsClient` ->
  `HlsClient`. No other behaviour change.
- **`HlsOrigin::new` deleted**, replaced by `HlsOrigin::builder(trunk)`
  (issue #873): the old four-positional constructor made `part_target_ms`
  mandatory, so classic (non-low-latency) HLS was inexpressible. No
  compatibility shim — this crate is at 0.4.0, unpublished at the time of
  this change. `multimux`'s `HlsOrigin::new` call site is updated to the
  builder in the same release.

### Added
- `server::Container` (`Fmp4` / `MpegTs`, `#[non_exhaustive]`) and
  `HlsOrigin::builder(trunk)` -> `HlsOriginBuilder` (issue #873, closes #865):
  `HlsOrigin`'s origin can now serve classic HLS-of-TS (`.ts` segments, no
  `#EXT-X-MAP` by default) as well as fMP4, and container is orthogonal to
  low latency — `.container(Container::MpegTs)` and `.low_latency(part_target_ms)`
  are independent builder calls, so classic HLS (no low-latency tags at all)
  is finally expressible. `HlsOriginBuildError` (`#[non_exhaustive]`,
  `thiserror`) surfaces a missing `target_duration_secs`/`window_segments` as
  an error rather than a silent default.
  - RFC 8216bis §3.1.1's `EXT-X-MAP` rule for MPEG-2 TS is a disjunction
    (PAT+PMT in-band, *or* `EXT-X-MAP`), not a container restriction —
    `Container::MpegTs` omits the tag by default (matching `transmux`'s
    self-initialising TS segments) without forbidding it structurally.
  - `HlsOrigin::set_init` stays callable under `MpegTs` — a documented no-op
    (bytes stored, never advertised or served), so a caller sharing one code
    path across both containers need not branch.

### Changed
- `HlsOrigin`'s `_HLS_msn` abuse bound tightened from +4 to the spec's +2
  (RFC 8216bis §6.2.5.2 SHOULD: "greater than the Media Sequence Number of
  the last Media Segment in the current Playlist plus two").
- The client/origin engines parse and render HLS playlists via `broadcast-hls`
  directly instead of reaching through `transmux::hls` for it (issue #878).
  `Error::PlaylistParse` now wraps `broadcast_hls::Error` instead of
  `transmux::Error` — a source-type change to a `#[from]`-wrapped variant,
  not a shape change to `Error` itself. Playlist docs moved from this
  crate's `docs/` to `broadcast-hls/docs/`, since they document the tags
  that crate now implements.

### Fixed
- The LL-HLS origin (`server::engine`) no longer supplies a hardcoded
  `EXT-X-VERSION:9` (`LL_HLS_VERSION`, deleted). `broadcast_hls::MediaPlaylist::to_m3u8`
  now computes the version from the content actually emitted (RFC 8216bis
  §8) — the true minimum for this origin's fMP4 playlist is 6, since none
  of the LL-HLS directives it renders (`EXT-X-PART`/`EXT-X-PART-INF`/
  `EXT-X-PRELOAD-HINT`/`EXT-X-SERVER-CONTROL`) carry any version requirement
  at all. The old hardcoded 9 over-declared and would have caused every
  client on protocol version 6, 7, or 8 to refuse a stream it could
  otherwise have played (issue #871).

> **Versions 0.1.0 through 0.3.1 below shipped under this crate's previous
> name, `ll-hls-runtime`** (tags `ll-hls-runtime-v0.1.0` .. `-v0.3.1`), and
> are still live on crates.io under that name — they are not versions of
> `hls-runtime`, whose own published history starts at 0.4.0. The rename came
> with the client+server split that made "ll-hls-client" wrong. The entries
> are kept because they are this code's real history.

## [0.3.1] - 2026-07-30

### Fixed
- Floor `media-plane` to `0.1.1`. The `^0.1` bucket also contains 0.1.0,
  which is built against `transmux` 0.20, so a consumer could resolve two
  `transmux` minors into one graph and hit trait-resolution errors pointing
  at this crate's internals (#858).

## [0.3.0] - 2026-07-30

### Changed (Breaking)
- `LlHlsRequest`, `LlHlsBody` (`server::engine`) now carry
  `#[non_exhaustive]` (issue #806's non_exhaustive drift-guard audit). A
  downstream `match` on either of these now needs a wildcard arm.
- The client's part-prefetch now ignores (rather than exhaustively assumes
  only `PART`/`MAP`) a future `PreloadHintType` variant from a newer
  `transmux`, consistent with that enum's own new `#[non_exhaustive]`.

### Added
- `tests/non_exhaustive_coverage.rs` drift guard (issue #806).

## [0.2.0] - 2026-07-28

### Fixed
- `server::engine`'s own test helpers and the `client_stepping`/
  `origin_playlist` examples called `Trunk::writer()` (the samples+events
  writer) instead of `Trunk::segment_writer()` to publish segments/parts — a
  stale call from before the `SegmentWriter` split that never actually
  compiled since (found via multimux plan step 5b's first successful
  `cargo test --workspace`/`--all-targets` across the whole workspace since
  then). No behaviour change to any shipped API — test/example-only.


- **Cleared this crate's share of the latest-stable clippy canary** (issue
  #770 — the non-blocking `clippy (latest stable)` CI job, which had been
  failing on `main` unnoticed across many merges): the `golden_gate`
  integration test builds its single-track init segment with
  `std::slice::from_ref(&spec)` instead of `&[spec.clone()]`
  (`clippy::cloned_ref_to_slice_refs`). Test-only, behaviour-preserving.

### Changed
- **BREAKING: `server::MediaStore`/`HealthState`/`SegmentWindowEntry`/
  `PlaylistOutcome`/`ResourceOutcome` are gone.** Media-plane implementation
  plan step 4: the LL-HLS origin is now [`server::LlHlsOrigin`], a
  `media_plane::egress::ServedEgress` that renders playlists and resolves
  blocking-reload/part-availability requests directly from a shared
  `media_plane::Trunk`, instead of a second, push-fed rolling-window store
  that duplicated exactly what the `Trunk` now holds. `server/store.rs`
  (732 lines of `MediaStore`) is deleted outright — live parts, whether a
  segment has closed, and the just-closed-segment-final-part-still-serves
  guarantee (multimux 0.2.1/0.2.2's hard-won bug fixes) all now fall out of
  the `Trunk`'s own live-part log with no cache of any kind on this crate's
  side. The one thing that genuinely cannot come from a `&Trunk` call alone —
  the rolling window of currently-advertised *closed* segments (bytes,
  duration, discontinuity bit), the lifetime-max segment duration, and the
  cumulative `#EXT-X-DISCONTINUITY-SEQUENCE` count — is assembled by
  `LlHlsOrigin` draining exactly **one** `Trunk` segment cursor, per this
  crate's own `media_plane::egress` module doc ("a `ServedEgress`
  implementation... keeps its own resolvable window in sync by draining
  [cursors]"), not a second `MediaStore`. See `docs/superpowers/plans/
  2026-07-26-media-plane-implementation.md` Step 4.
- The engine-level `BlockingQuery`/`DEFAULT_TRACK_ID`/`master_playlist_m3u8`
  are unchanged; the local `CachePolicy` duplicate is gone in favour of
  `media_plane::egress::CachePolicy`, re-exported via `LlHlsOrigin`'s
  `EgressResponse`.
- Dropped the `event-listener` direct dependency (now reached transitively
  through `media-plane`, whose `Trunk::listen` supersedes `MediaStore::listen`)
  and added `media-plane`/`bytes` (both gated by this crate's `std` feature,
  same as `event-listener` was).

### Removed
- **`multimux` no longer builds against this crate.** `multimux::store`
  re-exported `server::MediaStore`/`HealthState` directly, and
  `multimux::output::{llhls,dash,ll_dash}`/`origin::resource` called the
  deleted `resolve_playlist`/`resolve_resource`/`media_playlist_m3u8(&MediaStore,
  _)` shapes — none of that is a cheap adapter-level fix (it is `multimux`'s
  full `ServedEgress` port, scoped to plan Step 5, which this step
  deliberately does not half-port). `ll-hls-runtime`'s own `tests/
  golden_gate.rs`/`tests/glass_to_glass.rs` (both `tokio`-feature-gated,
  both depending on `multimux` as a dev-dependency) do not build until
  Step 5 lands.

## [0.1.1] - 2026-07-26

### Added
- **`client::LlHlsClient` now ingests classic MPEG-TS-segment HLS** (issue
  #760): a Media Playlist that never advertises an `EXT-X-MAP` (HLS v3, RFC
  8216 — the dominant legacy/IPTV form: self-contained `.ts` segments
  carrying their own PAT/PMT/PES, no separate init resource) routes each
  fetched Part/Segment through `transmux::TsDemux` instead of the fMP4/CMAF
  `transmux::Fmp4Demux` path, content-sniffed by the MPEG-TS sync byte
  (`0x47`) once the playlist itself is known to carry no map. The first
  successfully demuxed segment's recovered `TrackSpec`s synthesize the one
  `Output::Init` the crate's output contract requires (via
  `transmux::build_init_segment`), so downstream callers built against the
  fMP4 path (e.g. `multimux`'s `HlsPull`) need no TS-specific handling of
  their own. The fMP4/CMAF + LL (parts/preload-hint) path is entirely
  unchanged; the two never overlap for a single playlist.

## [0.1.0] - 2026-07-21

### Fixed
- **`MediaStore::window_segments`/`last_closed_segment_seq` no longer use a
  bare `.lock().unwrap()`.** Pre-release audit finding: these two lock sites
  (the bare-`_HLS_msn` reload path and the DASH `SegmentTemplate` window
  accessor) panicked on a poisoned `Mutex`, unlike the store's other 11 lock
  sites, which already tolerate poisoning via
  `.unwrap_or_else(std::sync::PoisonError::into_inner)`. Both now follow the
  same poison-tolerant pattern, so one panicking holder of the lock can no
  longer cascade into panics on ordinary requests.
- **`client::LlHlsClient::on_resource` now actually enforces
  `Error::UnrequestedResource`.** Pre-release audit finding: the variant's
  docs already claimed a `ResourceId` the client never requested was
  rejected, but `on_resource` never checked — any bytes for any id (a
  caller/driver bug, or a stale/duplicate delivery) were silently accepted.
  Now checked against the client's internal `requested` bookkeeping (`Init`
  against whether an init fetch is outstanding/cached), returning
  `Error::UnrequestedResource` instead.

### Changed
- **`server::master_playlist_m3u8` now takes a `media_playlist_name: &str`
  argument** (issue #663 "shared output auth + configurable playlist_name"):
  the master playlist's `#EXT-X-STREAM-INF` reference is the caller's
  configured media-playlist filename instead of the hardcoded `"media.m3u8"`
  literal, so a server (e.g. multimux's `Config::playlist_name`) can serve
  its media playlist under any `*.m3u8` name. Breaking: pass the intended
  filename explicitly (`master_playlist_m3u8("media.m3u8")` reproduces the
  old behaviour).
- **`client::tokio_client::TokioClient` now authenticates via `broadcast-auth`**
  (issue #663 P3c): `TokioClientConfig::auth` takes a
  `broadcast_auth::Credentials` (Basic/Digest/Bearer) instead of the ad hoc
  `Auth` enum (Basic/Bearer only) — fulfilling the TODO the field's doc
  comment carried since P3a. Basic/Bearer are still pre-applied on every
  request via reqwest's own helpers; Digest now works end-to-end: on a `401`,
  `TokioClient` reads `WWW-Authenticate`, computes the response via
  `broadcast_auth::Authenticator`, resends once, and caches the resulting
  authenticator (applied preemptively, advancing `nc`, on later requests).
  New `TokioError::Auth` variant for a challenge/response failure. Breaking:
  `tokio_client::Auth` is removed; construct `broadcast_auth::Credentials`
  instead (added as a `tokio`-feature-gated optional dependency).

### Added

#### Server (issue #663/#717 Stage 2)

- **`server` — the sans-IO LL-HLS origin engine** (Stage 2 of the
  ll-hls-runtime unification, issue #663/#717 —
  `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`, "ll-hls-runtime
  — client + server in one crate"), moved out of `multimux` behind the new
  `std` feature (needs `std::sync::Mutex`, unlike the no_std-capable
  `client`):
  - **`server::MediaStore`** — the protocol-neutral rolling in-RAM window
    (init/segments/live parts/`recent_parts`/health/max-segment-duration),
    moved verbatim from `multimux::store::MediaStore` **including the
    part-404-boundary fix** (`recent_parts`, so an in-flight preload-hint
    request for a segment's final part still resolves after the segment
    closes). The `tokio::sync::watch<u64>` progress signal is replaced with a
    runtime-agnostic wakeup: `MediaStore::progress_version()` (a monotonic
    counter) and `MediaStore::listen()` (an `event_listener::EventListener` —
    a plain `Future<Output = ()>` any executor, or none via its blocking
    `.wait()`, can drive), via the new `event-listener` dependency.
  - **`server::MediaStore::resolve_playlist`/`resolve_resource`** — the
    blocking-reload (RFC 8216bis §6.2.5.2) and part-availability decision
    logic as synchronous poll methods returning `PlaylistOutcome`
    (`Ready`/`WouldBlock`/`BadRequest`) / `ResourceOutcome`
    (`Ready`/`WouldBlock`/`NotFound`) — never blocking, never touching a
    clock. An async adapter (e.g. `multimux`) turns `WouldBlock` into an
    actual bounded wait via `MediaStore::listen()` + its own
    `tokio::time::timeout`; see the `server` module docs for the exact
    caller-driven wait-loop shape.
  - **`server::media_playlist_m3u8`/`master_playlist_m3u8`** — the LL-HLS
    playlist renderers, moved verbatim from
    `multimux::output::llhls::media_playlist_m3u8` **including the
    reentrant-lock deadlock fix** (`max_segment_duration()`/
    `target_duration_secs()` read *before* `MediaStore::with_segments_and_parts`'s
    lock).
  - **`server::CachePolicy`** (`Immutable`/`NoCache`) — the cache-control
    policy a resolved `ResourceOutcome::Ready` carries, for an adapter to
    apply as HTTP `Cache-Control`.

### Changed

- **Renamed `ll-hls-client` → `ll-hls-runtime`** (Stage 1 of the ll-hls-runtime
  unification; never published, so a free rename — no `0.1.0` behaviour
  change). The client engine moved under a `client` module
  (`ll_hls_runtime::client::LlHlsClient` etc., mirroring `rtsp-runtime`'s
  client+server split); an empty `server` module is reserved for the LL-HLS
  origin engine currently in `multimux`, to be folded in as Stage 2.

#### Client (issue #717)

- **`LlHlsClient` — sans-IO Low-Latency HLS playback client engine** (issue
  #717, slices 2-4). A caller-driven state machine in the same sans-IO shape
  as `srt-runtime` (#565): `poll()`/`next_output()` drain queued `Action`s /
  `Output`s; `on_playlist()`/`on_resource()`/`on_error()` feed responses back
  in. No socket, no clock, no `tokio`/`reqwest` dependency in the core.
  - **Reload scheduler** (slice 2): Blocking Playlist Reload
    (`_HLS_msn`/`_HLS_part`, RFC 8216bis §6.2.5.2) once a playlist advertises
    Low-Latency support, correctly distinguishing a bare `_HLS_msn` (waits for
    a closed segment) from `_HLS_part=0`; non-blocking-reload backoff derived
    from `#EXT-X-TARGETDURATION` for non-LL origins; best-effort `EXT-X-SKIP`/
    `CAN-SKIP-UNTIL` Playlist Delta Update merge.
  - **Fetch pipeline** (slice 3): `EXT-X-PRELOAD-HINT` part prefetch ahead of
    its own numbered appearance; `BYTERANGE` part/segment/map support
    (including the "omitted offset continues the previous sub-range" rule);
    the init segment (`EXT-X-MAP`) fetched once.
  - **Output adapter** (slice 4): ordered `Output::Init` then `Output::Samples`
    (real access units via `transmux::Fmp4Demux`, not opaque container bytes);
    `EXT-X-DISCONTINUITY` forwarded as `Output::Discontinuity`; parts already
    individually fetched are never double-counted when their parent segment
    later closes (dedup/coalescing); a non-LL playlist (no parts at all) plays
    via the full-segment fallback path; resources arriving before the init
    segment are buffered and replayed once it arrives.
  - Reuses `transmux::hls::MediaPlaylist::parse` (issue #717 slice 1) for the
    playlist model — this crate defines no playlist types of its own.
  - `tests/origin_loop.rs`: an in-process origin↔client loop against a real
    `transmux::ll_hls::LlHlsSegmenter`, asserting the exact blocking-reload
    `_HLS_msn`/`_HLS_part` requested, the preload-hint prefetch actually
    issued, ordered/deduped/byte-identical sample reconstruction, and the
    non-LL full-segment fallback path.
  - `CAN-BLOCK-RELOAD` (issue #717 slice 1 follow-up, fixed alongside slice
    5): reload scheduling now honours `transmux::hls::LowLatencyConfig::can_block_reload`
    rather than inferring blocking-reload support from `low_latency` being
    `Some` — an origin advertising `CAN-BLOCK-RELOAD=NO` (while still
    carrying `PART-INF`/`PART` tags) now correctly gets a plain, non-blocking
    reload paced by `Action::WaitMs`, never a blocking `_HLS_msn`/`_HLS_part`
    request. Covered by `tests/origin_loop.rs`'s
    `can_block_reload_no_yields_non_blocking_reload_with_backoff`.
- **`TokioClient` — tokio + reqwest (rustls) IO adapter** (issue #717 slice
  5), behind a new, non-default `tokio` cargo feature. Drives `LlHlsClient`
  over real HTTP: performs the blocking `_HLS_msn`/`_HLS_part` reload and
  plain playlist GETs, resource fetches (including `Range` byte-ranges),
  retries resource fetches with capped backoff before falling back to
  `on_error` (letting the next reload naturally re-request them), and
  retries a failing playlist reload indefinitely with capped backoff (the
  sans-IO core has no other recovery path for a playlist fetch failure).
  Optional HTTP Basic/Bearer auth via `TokioClientConfig::auth`, with a
  documented TODO to swap in the workspace's planned shared multi-scheme
  auth crate once it exists. Exposes `TokioClientStats` (playlist fetches,
  blocking reloads, resource fetches, preload-hint-triggered resource
  fetches) so blocking-reload/prefetch behaviour is externally observable,
  not just internally exercised. The sans-IO core (`client.rs`) gained no
  new dependency from this — `tokio`/`reqwest` are entirely behind the new
  feature.
  - `tests/glass_to_glass.rs` (gated on the `tokio` feature; epic #717's
    done-bar): drives `TokioClient` against a **real** `multimux`-served
    LL-HLS origin over real loopback HTTP, fed by a real-time-paced
    `transmux::ll_hls::LlHlsSegmenter` producer (live-shaped, ~30fps/120ms
    parts) — measures glass-to-glass latency (wall-clock push-to-emit,
    embedded per-sample) and asserts it is **sub-second**, asserts at least
    one Blocking Playlist Reload and one preload-hint-triggered resource
    fetch actually occurred (`TokioClientStats`), and asserts a genuinely
    non-LL playlist (no `PART` tags, served from a minimal hand-built axum
    origin) still plays via the full-segment fallback with zero blocking
    reloads.
  - `tests/golden_gate.rs` (gated on the `tokio` feature, `ffprobe`-gated,
    non-blocking CI lane — closes issue #717's last acceptance box,
    "Integrated into #569's golden-gate harness as the reference client"):
    `TokioClient` is now the **reference client** in the #569 player-validated
    golden gate. `transmux/tests/golden_gate.rs` (#569) validates only the
    origin half — transmux's own muxer output handed to an independent
    decoder (`ffprobe`). This closes the other half: demuxes the workspace's
    real `fixtures/ts/h264_aac.ts` capture (Main profile, 320x240, 25fps, 75
    real video frames) via `TsDemux`, live-paces those real samples through
    the same `LlHlsSegmenter`/`MediaStore`/`LlHlsOutput` origin stack
    `glass_to_glass.rs` uses, drives a real `TokioClient` against it over
    loopback HTTP, then muxes the **client's own** reconstructed init +
    samples (not the origin's) into a real fMP4 and hands that to `ffprobe`:
    asserts it decodes as H.264 at the source's resolution, and that
    `ffprobe -count_frames`'s own decoded frame count exactly matches the
    frames fed in — catching a drop/duplicate/reorder that corrupts the
    bitstream even when the container alone still looks well-formed. Also
    covers the non-LL/full-segment fallback path decoding correctly, and a
    self-test (`dropped_sample_changes_the_decoded_frame_count`) proving the
    frame-count oracle isn't vacuous. `.github/workflows/ci.yml`'s existing
    non-blocking `golden-gate` job now also runs this suite alongside
    transmux's.
