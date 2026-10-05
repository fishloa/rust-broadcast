# hls-runtime 0.7.0

_Released 2026-10-05._

Breaking (0.x minor) and security release of the sans-IO LL-HLS client and origin engine. It bundles two things that were never tagged separately: the GHSA-grg8-55qr-gxgf fix and SCTE-35 `EXT-X-DATERANGE` rendering (both prepared as 0.7.0 in August; the last published version is 0.6.0), and the de-hand-roll plus audit pass (reqwest 0.13, RFC 3986 URL resolution, `backon` retries, instance-named resources, client restart detection, many origin-conformance fixes). **Upgrade if you use `TokioClient` with `TokioClientConfig::auth` set** (credential leak, below). **Breaking for:** callers that construct `TokioClientConfig` as a struct literal, name `reqwest::Error`, cache resource URIs by name, or depend on the exact playlist text `HlsOrigin` renders.

Read together with: [multimux-0.11.0.md](multimux-0.11.0.md) (the consumer of the origin), [media-plane-0.5.0.md](media-plane-0.5.0.md), [broadcast-hls-0.3.0.md](broadcast-hls-0.3.0.md), [broadcast-auth-0.4.0.md](broadcast-auth-0.4.0.md), [timed-metadata-0.6.0.md](timed-metadata-0.6.0.md), [transmux-0.25.0.md](transmux-0.25.0.md).

## Security: GHSA-grg8-55qr-gxgf

`client::tokio_client::TokioClient` sent `TokioClientConfig::auth` credentials (Basic, Bearer and Digest challenge answers) to every host a playlist's URLs pointed at, not only the playlist's own origin. A malicious or compromised playlist could redirect a segment, part or rendition-report request to an attacker's host and receive the `Authorization` header. `TokioClient` also retried any `4xx` with the full backoff schedule, repeating credentialed requests against a host that had already rejected them.

Fixed: credentials go only to the playlist URL's origin (scheme, host and port); any other host is requested without `Authorization`. A resource fetch answered with a `4xx` other than `408`/`429` (for example a `404` for a stale preload hint) goes to `HlsClient::on_error` immediately instead of after the full backoff.

## Breaking changes

### `reqwest` 0.13 and the `tokio` feature

`TokioError::Http`'s source is still `reqwest::Error`, but the type is now reqwest 0.13's. reqwest 0.13 renames its `rustls-tls` feature to `rustls`, and hls-runtime's own dependency uses the new name; a crate that also depends on `reqwest` 0.12 will carry both versions. The `tokio` feature now also needs `backon`, `headers` and `tokio-util`.

### `TokioClientConfig` gained fields

New fields `connect_timeout` (default 10 s), `jitter` (default `true`) and `cancel` (`tokio_util::sync::CancellationToken`), with builders `with_auth`, `with_cancel`, `with_connect_timeout`, `with_jitter`. A struct literal without `..Default::default()` no longer compiles.

```rust
// 0.6
let cfg = TokioClientConfig { request_timeout, blocking_timeout, max_resource_retries,
                              retry_backoff, max_retry_backoff, auth: None };
// 0.7
let cancel = tokio_util::sync::CancellationToken::new();
let cfg = TokioClientConfig { request_timeout, ..Default::default() }
    .with_cancel(cancel.clone());
```

Once `cancel` fires, `TokioClient::next_output` abandons its in-flight request or backoff sleep and returns `Ok(None)`. Retries use `backon`'s exponential schedule with jitter: each delay `d` becomes a random value in `[d, 2d)`, clamped to `max_retry_backoff`, which remains a hard maximum. `TokioError::Stalled` replaces a former 10 ms defensive sleep for the case where the core queued no action and the stream had not ended (never observed in practice). Byte ranges are sent as a typed `headers::Range`.

### Resource names carry an instance token (#1030)

Every resource name served `immutable` now includes `HlsOrigin::instance()`, a number that differs for every origin built in this process (strictly increasing) and across process restarts (seeded from the wall clock in milliseconds):

```text
init-{track}-{instance}-{generation}.mp4
seg-{track}-{instance}-{msn}.{ext}
part-{track}-{instance}-{msn}.{idx}.{ext}
```

A name therefore maps to one origin's bytes for ever, so a reconnect (new `Trunk`, numbers restarting, possibly a new SPS/ASC), a restart without DVR, or a replacement origin reusing the open segment's number can no longer place new bytes under a name a cache holds. Players follow playlist URIs and are unaffected; a request carrying another instance's token is `NotFound`. The token-less `seg-{track}-{msn}` and `part-{track}-{msn}.{idx}` (DASH/Smooth templates cannot carry the token) still resolve but are `CachePolicy::NoCache`, as is the bare `init-{track}.mp4`, which now means "the current init". The pre-release two-field `init-{track}-{generation}.mp4` form is gone. A changed init (`set_init` with different bytes) is a new generation; the last 8 stay resolvable. New API: `HlsOriginBuilder::instance(u64)`, `HlsOrigin::instance()`, `HlsOrigin::init_name(track, generation)`, `server::ClosedSegment::init_generation` and `with_init_generation`.

The playlist's `EXT-X-MAP` now sits on the segments (each names the init it was cut against) instead of one unconditional line, with `EXT-X-DISCONTINUITY` before the first segment that uses a new generation; as rendered by `broadcast-hls` 0.3, the discontinuity comes before that segment's `EXT-X-MAP`.

For a replacement origin the offset passed to `HlsOriginBuilder::media_sequence_offset` is now `previous.next_media_sequence()` (it was `.saturating_sub(1)`): the previous origin's open segment number is skipped because its parts were already served.

```rust
// 0.6
let b = builder.media_sequence_offset(prev_next_msn.saturating_sub(1));
// 0.7
let b = builder.media_sequence_offset(previous.next_media_sequence());
```

### URL resolution and request URLs

- Relative playlist references resolve by RFC 3986 (`url::Url::join`): `..` segments are resolved (`../x.m4s` against `http://h/a/b/p.m3u8` is `http://h/a/x.m4s`; before it was the textual `http://h/a/b/../x.m4s`, defect 7), and a `://` inside a relative reference's query no longer makes it look absolute. Results are normalised (`HTTP://H.Example/X` becomes `http://h.example/X`, spaces percent-encoded, a default port dropped). A relative playlist URL still yields a relative result.
- `Action::playlist_request_url` builds the query with `query_pairs_mut`, so the `_HLS_msn`/`_HLS_part`/`_HLS_skip` pairs precede a fragment: `...?a=1#f` gives `...?a=1&_HLS_msn=5#f`. The old builder produced `...?a=1#f&_HLS_msn=5`, where the pair was part of the fragment and never sent.
- `EXT-X-PROGRAM-DATE-TIME` is parsed with `jiff::Timestamp`: a leap second (`23:59:60`) is clamped to `:59`, where the old parser added 60 s to the minute.

### Client behaviour

- **Origin restart detection (#1031).** A Media Sequence Number below the previous playlist's is treated as an origin restart unless it is an older copy of the same stream (shared numbers name the same segments, or, for a wholly older window, `PROGRAM-DATE-TIME` shows it ends before the previous one began, in which case its segments are skipped). A restart to an equal or higher number is detected from a shared number naming a different segment or time, a decreasing `EXT-X-DISCONTINUITY-SEQUENCE`, or a name reused under a lower number. On a restart, sequence-keyed state, queued fetches and the init are dropped, the live-edge join is redone and `Output::Discontinuity` is emitted, so new segments reusing delivered numbers are no longer skipped as already delivered. A response still in flight for the old numbering is rejected as `Error::UnrequestedResource`. Documented blind spots: a client lagging by a whole window behind an origin with no `PROGRAM-DATE-TIME` and all-new names reads as a restart (one spurious `Discontinuity`), and a restart to a higher, non-overlapping number with a consistent discontinuity sequence and no `PROGRAM-DATE-TIME` reads as the client having fallen behind.
- `TokioClient` retries a blocking or delta reload that the origin rejects with a non-transient `4xx` once, at once, as a plain GET of the playlist URL, instead of repeating the rejected URL forever (an origin restarted below the requested `_HLS_msn` answers `400`).
- **The client joins at the server's hold-back, not at the start.** A live playlist (no `EXT-X-ENDLIST`) is joined no closer to its end than `PART-HOLD-BACK` in Low-Latency Mode (`CAN-BLOCK-RELOAD=YES`; the open segment's parts count), else the larger of `HOLD-BACK` and three Target Durations (RFC 8216 §6.3.3, RFC 8216bis §4.4.3.8/§6.3.3). `EXT-X-START` is honoured at segment granularity; a VOD playlist without it plays from its start. The byte-range cursor advances over skipped segments. If you relied on the client fetching the whole first window, it no longer does.
- `HlsClient::on_resource` returns `Error::DuplicateResource` for a second delivery of an accepted id (it used to emit the samples twice). A playlist whose `EXT-X-MEDIA-SEQUENCE` plus segment count overflows `u64` returns `Error::MediaSequenceOverflow` (it overflow-panicked in debug and wrapped in release). Both are new variants of the `#[non_exhaustive]` `client::Error`.
- Classic MPEG-TS-segment HLS keeps one demuxer across segments, so the 33-bit PTS/DTS wrap is unrolled across segment boundaries and each segment's last access unit gets its real duration. Consequence: the last access unit per stream of a segment is emitted when the next segment arrives (flushed at `EXT-X-ENDLIST` or `EXT-X-DISCONTINUITY`). A new `Output::Init` follows when the track set changes, and the demuxer restarts at `EXT-X-DISCONTINUITY`.

### Origin behaviour

- A part request is held open only for the hinted part: it is `NotFound` at once unless it is the one after the last part the ring holds, in the open segment or the one after it. A classic (non-LL) origin answers a request for a part the `Trunk` does not hold with `NotFound`.
- A playlist request with `_HLS_part` beyond the Advance Part Limit (RFC 8216bis §6.2.5.2) is answered `BadRequest` instead of parked.
- `EXT-X-PART` tags stay on a closed segment while it is within three Target Durations of the end (§6.2.2), and only when every part of it is still in the `Trunk`'s part ring.
- The window restarts when sequence numbers are not consecutive or the cursor reports lost segments: old entries roll off, the next segment is marked `EXT-X-DISCONTINUITY`, and `EXT-X-MEDIA-SEQUENCE` can no longer disagree with a segment's real number.
- `HlsOriginBuilder::window_segments` below 3 is raised to 3. New build errors on `HlsOriginBuildError`: `InvalidTargetDuration` (NaN, infinite, zero or negative `target_duration_secs`), `ZeroPartTarget` (`low_latency(0)`), `MediaSequenceOffsetTooLarge`.
- `HlsOrigin::set_init` with an unreadable init drops the `CODECS` derived from the previous init instead of keeping them.

## Added

- **SCTE-35 to `EXT-X-DATERANGE` (#965).** `HlsOrigin::render_playlist` queries the trunk's event ring per segment (`Trunk::events_in_segment`) and emits one `#EXT-X-DATERANGE` per resolved SCTE-35 event, via `timed_metadata`. A wall-clock `START-DATE` requires the trunk to have a `time_anchor`; events are skipped until it has one, and non-SCTE-35 or unrenderable events are skipped silently. An event whose DATERANGE cannot be rendered as a valid attribute list (a `"`, CR or LF from an upstream `segmentation_upid`, or a non-finite duration) is skipped for that window. Clients that do not understand `EXT-X-DATERANGE` must ignore it (RFC 8216 §4.4.5.1).
- `HlsOrigin::master_playlist(name) -> Result<String, HlsMasterError>`: `BANDWIDTH` is the measured peak segment bitrate (segment bytes over duration, rounded up, never lowered when the peak segment leaves the window) and `CODECS` comes from the init segment; before the first closed segment it falls back to a 5 Mb/s estimate. `HlsOrigin::set_track_specs` supplies the tracks for a TS origin, which has no init segment (#1089). `HlsMasterError` is `#[non_exhaustive]` (`BandwidthOverflow`, `Render`).
- `HlsOriginBuilder::media_sequence_offset(u64)`, `HlsOrigin::media_sequence_offset()`, `HlsOrigin::next_media_sequence()`: a fresh `Trunk` restarts segment numbers at 1, so a replacement origin keeps the playlist's Media Sequence Number (and `_HLS_msn`) increasing (RFC 8216bis §6.2.2) (#1089).
- `HlsClient::next_wait()` (the queued `WaitMs` hint as a `Duration`, `no_std`) and `HlsClient::poll_timeout(&mut self, now)` (`std`): the wait's absolute deadline, anchored at the first query and identical on re-query until drained with `poll()`, so unrelated wake-ups cannot re-arm it.

## Dependency changes

Verified from `git diff hls-runtime-v0.6.0..HEAD -- hls-runtime/Cargo.toml`:

```toml
broadcast-common = "9.4"      # was 9.3
transmux         = "0.25"     # was 0.24
broadcast-hls    = "0.3"      # was 0.2
media-plane      = "0.5"      # was 0.4 (optional, std)
broadcast-auth   = "0.4"      # was 0.3 (tokio feature)
reqwest          = { version = "0.13", features = ["rustls"] }  # was 0.12, rustls-tls
timed-metadata   = "0.6"      # new, default-features = false (no_std + alloc)
url              = "2"        # new, default-features = false
jiff             = "0.2"      # new, alloc (std feature enables jiff/std, url/std)
backon  = "1"; headers = "0.4"; tokio-util = "0.7"   # new, tokio feature only
```

The `no_std` core still builds: `url` and `jiff` are `alloc`-only unless `std` is on.

## Fixes

- Relative references containing `..` are no longer joined textually (defect 7, above).
- `HlsOrigin`'s locks tolerate poisoning (a panic in one request no longer panics every later one), `render_playlist` copies the window and releases its lock before querying the `Trunk`, and a request with no new segment since the last drain skips the cursor and window locks (#1089, #1134).
- Client state no longer grows for the life of a pull: records keyed by a Media Sequence Number below the playlist's first segment, and byte-range cursors for URLs a full playlist no longer references, are dropped after each playlist (#1089).
- Media Sequence arithmetic, `EXT-X-SKIP` merge index and part indexes use checked conversions instead of `as` casts (#1089).

MSRV is 1.95.0 (raised in 0.6.0).

---

Published from tag `hls-runtime-v0.7.0`.
