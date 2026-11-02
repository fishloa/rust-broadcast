# hls-runtime 0.11.0

_Released 2026-11-02._

### Added
- `HlsOrigin::published_target_duration()`: the `EXT-X-TARGETDURATION` the origin's Media Playlists carry (frozen value once published, otherwise the largest of its own, its longest segment and its siblings'), so an adapter can bound a blocking reload by three Target Durations (RFC 8216bis section 6.2.5.2, issue #1651).
- `HlsOrigin::set_declared_closed_caption_languages` hands the BCP 47 language the stream declares for each caption stream to the master playlist; a `CLOSED-CAPTIONS` rendition takes it as `LANGUAGE` unless a `ClosedCaptionDecl` names one, and carries none when the stream declares none (issue #1428, Apple authoring spec 4.4).
- `HlsOrigin::set_detected_closed_captions(Vec::new())` now means the video was scanned and carries no captions: the master playlist states `CLOSED-CAPTIONS=NONE` on every `EXT-X-STREAM-INF` (RFC 8216bis section 4.4.6.2). Until the first call the master claims nothing, as before; a later non-empty call replaces `NONE` with the caption group (issue #1428).
- `client::choose_variant(&[VariantInfo], &VariantPolicy) -> usize`, the variant-selection rule the client applies to a master playlist, made public so another adaptive source (the Smooth Streaming source of `broadcast-client`) picks a quality level by the same policy (issue #1460).

---

Published from tag `hls-runtime-v0.11.0`.
