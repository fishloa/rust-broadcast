# smooth-runtime 0.1.0

_Released 2026-11-14._

Initial release.

Version: 0.1.0 (first publish).

Initial release: sans-IO Smooth Streaming (MS-SSTR) client and origin engines over
`broadcast-smooth`, replacing the two diverged pull clients that lived in `broadcast-client` and
`multimux` (#2370) and adding the live origin (#2453).

### Added
- `SmoothClient`: manifest fetch and live refresh, retry paced by `broadcast_common::backoff::RetrySchedule` and never shorter than the server's `Retry-After` (`FetchFailure::retry_after_us`, RFC 9110 §10.2.3), protection refusal, stream and quality selection, chunk plans, fragment URLs, first-fragment track-id discovery, synthetic init segments, `tfxd` timestamp rebasing, the encrypted-fragment probe, live join look-ahead inside the DVR window, and discontinuities. `select_video_quality(index)` switches the first video stream's `QualityLevel` within one codec family (H264/AVC1, H265/HEVC) and re-announces the tracks once with `Output::TracksChanged`. A `TimeScale="0"` stream is skipped (`SmoothError::NoUsableStream` when none is left), an un-enumerable chunk timeline ends the session with `SmoothError::Timeline`, and a fragment whose `tfxd` time overflows is `Dropped::Fragment`. Relative URLs resolve through `broadcast_common::url_ref`.
- `server::SmoothOrigin` (feature `std`): the live origin engine, built from a `SmoothOriginConfig { trunk, segments, tracks }`. Over a `media_plane::Trunk` it renders the client Manifest through `broadcast_smooth::SmoothManifest::live(..)`/`to_xml` and resolves `SmoothRequest::Fragment` to re-muxed fragment bytes, answering `EgressResponse::Ready`/`Await`/`BadRequest`/`NotFound` without I/O or a clock read; a start time the window does not hold is `NotFound`. Codec parameters come from `transmux::resolve_codec`. Its plan (chunk timelines plus the rendered Manifest) is cached by init-segment generation, track set and window, built once for concurrent misses; `ServedEgress::is_warm`/`prepare` let a host run a miss's packaging on a thread of its choosing (#2522).
- `SmoothOrigin::stream_keys`/`fragments` and `SmoothRequest::parse`: the streams a manifest advertises and the `QualityLevels(BITRATE)/Fragments(KIND=START)` request grammar; a second stream of a kind is named `TYPE_N`, as the packager names it.

---

Published from tag `smooth-runtime-v0.1.0`.
