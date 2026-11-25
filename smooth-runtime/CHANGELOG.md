# Changelog

All notable changes to `smooth-runtime` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Changed

- The client demuxes each fragment on the stream's own `transmux::mp4_demux::Mp4Demux` stage, fed the synthetic init segment once, instead of building `init ++ fragment` and running the whole-input `Fmp4Demux` over it for every fragment: the init is parsed once per stream (a new one only on a quality switch), and a damaged fragment costs only itself (`Dropped::Fragment`) because the stage is rebuilt from the same init and the next fragment demuxes normally. `tfxd` rebasing and the encrypted-fragment probe are unchanged, and so are the samples delivered (`Output::Sample`).
- The origin demuxes the window's segments on one stage per stream too, both when it renders the chunk timeline and when it packages a requested fragment.


## [0.4.0] - 2026-11-22
### Breaking

- Builds against new caret epochs of workspace siblings (`broadcast-common` 13 -> 14, `broadcast-smooth` 0.2 -> 0.3, `media-plane` 0.13 -> 0.14, `transmux` 0.32 -> 0.33). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever its own API diff, so this release is a major-class bump; consumers must resolve a single copy of each at the jiff 1.0 epoch, and the manifest/fragment types it hands to `media-plane` are those versions' types.

### Tests

- **`codec_family` is pinned by its FourCC groups.** Quality switching is only allowed within one codec family, so a constant answer would let an `avc1` ladder switch to `hvc1` (or `AACL` to `EC-3`) and break the decoder, and the alternate HEVC FourCC would be classified as unknown (audit 22 finding 43, #3011).
- **The 404 predicate, the `ManifestFault` labels and the selector's `Debug` are pinned.** Only 404/410 are "not (yet) there", every fault renders its own label, and a `QualitySelector` still names its type when formatted (audit 22 finding 46, #3014).
- **The origin's fragment lookup, init accessor, prewarm contract and advertised bitrate are pinned.** A fragment is served only at the bitrate the manifest advertises, packaged from its own segment, kept cached while its chunk is still planned, and reported warm only after `prepare`; `init` returns exactly the stored segment and the interleaved ordinal follows the track id (audit 22 finding 45, #3013).
- **The live refresh cadence, miss abandonment and drain/end lifecycle are pinned.** A drained live presentation refreshes after half its last chunk duration clamped to `[refresh_min, refresh_max]`, a stream missing from `max_refresh_misses` refreshes is abandoned while fewer are not, a live 404 schedules an immediate manifest fetch (a 500 or a static failure does not), a static presentation ends after draining and stays ended, a fatal session polls nothing and has no deadline, the held-bytes and fragment-byte caps are checked at their exact value, and the `is_*` accessors and `with_*` setters report what the session and the caller set (audit 22 finding 42, #3010).
- **A live refresh join reports a real gap and never a first-join one.** A refreshed timeline that continues at the known end, overlaps it, or starts a fresh stream is not a discontinuity; only a start past it is, and a zero `DVRWindowLength` keeps the whole timeline (audit 22 finding 44, #3012).

## [0.3.0] - 2026-11-17
### Added
- `SmoothClientConfig::max_held_bytes` (default `DEFAULT_MAX_HELD_BYTES`, 32 MiB, set with `with_max_held_bytes`): fragment bodies are held, across every stream, until all of them have announced their local track id, and an origin that serves one stream nothing usable could make the healthy streams' fragments accumulate without bound. Past the bound the streams that have not announced are abandoned (`Dropped::StreamLost`) so the rest are announced and their held fragments released, and `SmoothError::AnnounceStalled { max }` ends the session when nothing is left to abandon (#2816).
- `SmoothClientConfig::{max_manifest_bytes, max_fragment_bytes, max_plan_len}` (defaults `DEFAULT_MAX_MANIFEST_BYTES` 4 MiB, `DEFAULT_MAX_FRAGMENT_BYTES` 16 MiB, `DEFAULT_MAX_PLAN_LEN` 10 000, set with the matching `with_*` setters), so a remote origin cannot exhaust memory through the manifest, a fragment body, or a live refresh that appends chunks faster than they are fetched (#2823):
  - a manifest body past `max_manifest_bytes` is the new `ManifestFault::TooLarge`, retried and then fatal like any other manifest fault, and `max_manifest_bytes` is the document bound the manifest parser is given;
  - a fragment body past `max_fragment_bytes` ends the session with the new `SmoothError::FragmentTooLarge { stream, bytes, max }`;
  - a stream's plan past `max_plan_len` after a refresh drops its oldest chunks and reports `Output::Discontinuity`.
  `broadcast-client` reports `SmoothError::FragmentTooLarge` as `ErrorKind::SegmentFetchFailed { status: None }`.

### Fixed
- A fragment URL resolved from the manifest's `Url` template must now be an `http`/`https` URL reached without an `https`-to-`http` downgrade (`broadcast_common::url_ref::resolve_http`): a template naming `file:`, `ftp:` or downgrading the connection is `Dropped::FragmentUrl` and is never requested, instead of becoming a fetch the host performs (#2853).


## [0.2.0] - 2026-11-15
### Changed
- BREAKING: `client::FetchFailure::retry_after_us: u64` is `retry_after: core::time::Duration`, the server's `Retry-After` wait as a span rather than a raw microsecond count (RFC 9110 §10.2.3) (#2731).
- BREAKING: `error::SmoothError` no longer implements `Eq` (it keeps `PartialEq`), because its `InvalidBackoff` payload no longer does: `InvalidBackoff` carries a NaN growth factor, which is not equal to itself (broadcast-common #2725). Nothing compared `SmoothError` values for a total order.
- BREAKING: `client::FetchFailure::status`, `client::Dropped::Fragment::status` and `error::SmoothError::Fragment::status` are `Option<http::StatusCode>` (was `Option<u16>`), so a host reports a refused fetch with the status it already holds (#2692).
- Adopts `transmux::smooth_fragment::fragment_track_id`'s `TrackId` return (transmux #2647): a stream's discovered local track id is held as a `TrackId` end to end rather than a raw integer. No public API change.
- BREAKING: `SmoothError::InvalidBackoff(broadcast_common::backoff::InvalidBackoff)`: a config whose retry cap or steps a `RetrySchedule` refuses (a cap above `broadcast_common::backoff::MAX_FLOOR`) makes `SmoothClient::new` fail at construction, and its first output is that `Output::Fatal`, instead of the client panicking or repairing the schedule. Match the variant to surface the config error (#2701).
- BREAKING: `client::SYNTHETIC_MOVIE_TIMESCALE` is a `Timescale`; the client reads each stream's `Timescale` from the manifest (a stream or manifest with a zero `TimeScale` is refused by `broadcast-smooth` at parse, so the client no longer skips such streams) (#2657).
- BREAKING: every `now` the client engine takes is a `broadcast_common::stage::Timestamp` (`poll`, `on_manifest`, `on_fragment`, `on_failure`), the clock type the server engine already takes.
- BREAKING: `SmoothConfig`'s `backoff_ms: Vec<u64>` and `backoff_cap_ms: u64` are one `backoff: broadcast_common::backoff::BackoffSteps` (`with_backoff`), and `refresh_min_us`/`refresh_max_us` are `refresh_min`/`refresh_max: Duration` (`with_refresh_min`/`with_refresh_max`); `DEFAULT_BACKOFF_MS`/`DEFAULT_BACKOFF_CAP_MS` become `DEFAULT_BACKOFF_STEPS`/`DEFAULT_BACKOFF_CAP` and `DEFAULT_REFRESH_MIN_US`/`DEFAULT_REFRESH_MAX_US` become `DEFAULT_REFRESH_MIN`/`DEFAULT_REFRESH_MAX`.
- BREAKING: `SmoothConfig` is `SmoothClientConfig`, matching `RtspClientConfig`/`RtmpClientConfig`/`HlsClientConfig`. Migration: every `SmoothConfig` becomes `SmoothClientConfig` with the same fields and `with_*` setters.
- BREAKING: `SmoothClient::new(manifest_url, config)` is `SmoothClient::new(config)`: the manifest URL is `SmoothClientConfig::manifest_url`, set with the new `SmoothClientConfig::new(manifest_url: impl Into<String>)` (the same shape as `HlsClientConfig::new`/`DashClientConfig::new`) or `with_manifest_url(Url)`. Migration: `SmoothClient::new(url, SmoothClientConfig::default().with_x(..))` -> `SmoothClient::new(SmoothClientConfig::new(url.as_str()).with_x(..))`, or `.with_manifest_url(url)` for a `Url` already parsed.

## [0.1.0] - 2026-11-14
Initial release: sans-IO Smooth Streaming (MS-SSTR) client and origin engines over
`broadcast-smooth`, replacing the two diverged pull clients that lived in `broadcast-client` and
`multimux` (#2370) and adding the live origin (#2453).

### Added
- `SmoothClient`: manifest fetch and live refresh, retry paced by `broadcast_common::backoff::RetrySchedule` and never shorter than the server's `Retry-After` (`FetchFailure::retry_after_us`, RFC 9110 §10.2.3), protection refusal, stream and quality selection, chunk plans, fragment URLs, first-fragment track-id discovery, synthetic init segments, `tfxd` timestamp rebasing, the encrypted-fragment probe, live join look-ahead inside the DVR window, and discontinuities. `select_video_quality(index)` switches the first video stream's `QualityLevel` within one codec family (H264/AVC1, H265/HEVC) and re-announces the tracks once with `Output::TracksChanged`. A `TimeScale="0"` stream is skipped (`SmoothError::NoUsableStream` when none is left), an un-enumerable chunk timeline ends the session with `SmoothError::Timeline`, and a fragment whose `tfxd` time overflows is `Dropped::Fragment`. Relative URLs resolve through `broadcast_common::url_ref`.
- `SmoothConfig` (`#[non_exhaustive]`): start from `SmoothConfig::default()` and set what differs with `with_live_join_fragments`, `with_max_fragment_failures`, `with_max_manifest_failures`, `with_max_refresh_misses`, `with_backoff_ms`, `with_backoff_cap_ms`, `with_jitter_seed`, `with_refresh_min_us`, `with_refresh_max_us` and `with_video_quality`.
- `server::SmoothOrigin` (feature `std`): the live origin engine, built from `SmoothOriginConfig::new(trunk, segments, tracks)` (`#[non_exhaustive]`; all three parts are required). Over a `media_plane::Trunk` it renders the client Manifest through `broadcast_smooth::SmoothManifest::live(..)`/`to_xml` and resolves `SmoothRequest::Fragment` to re-muxed fragment bytes, answering `EgressResponse::Ready`/`Await`/`BadRequest`/`NotFound` without I/O or a clock read; a start time the window does not hold is `NotFound`. Codec parameters come from `transmux::resolve_codec`. Its plan (chunk timelines plus the rendered Manifest) is cached by init-segment generation, track set and window, built once for concurrent misses; `ServedEgress::is_warm`/`prepare` let a host run a miss's packaging on a thread of its choosing (#2522).
- `SmoothOrigin::stream_keys`/`fragments` and `SmoothRequest::parse`: the streams a manifest advertises and the `QualityLevels(BITRATE)/Fragments(KIND=START)` request grammar; a second stream of a kind is named `TYPE_N`, as the packager names it.
