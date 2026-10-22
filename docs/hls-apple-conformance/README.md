# Apple HLS Authoring Specification — origin conformance audit (2026-10-22)

Code-reading audit of the `hls-runtime` + `multimux` origin against every numbered requirement of the Apple HLS Authoring Specification for Apple Devices (`private/specs-md/apple_hls_authoring_specification_for_apple_devices.md`). Classes: MET-VERIFIED (a test asserts it), MET-UNVERIFIED, PARTIAL, GAP, GAP-OPTIONAL (encoder/content rule the origin could check but does not), NOT-APPLICABLE.

| Group | Sections | File |
|---|---|---|
| A | 1 video, 2 audio, 3 ads, 4 accessibility, 5 subtitles, 7 segmentation | [audit-A](audit-A-video-audio-ads-captions-subtitles-segmentation.md) |
| B | 6 trick play, 8 media playlists, 9 multivariant, 10 delivery | [audit-B](audit-B-trickplay-media-master-delivery.md) |
| C | 11-16, appendix (privacy, TLS, live/LL-HLS, ...) | [audit-C](audit-C-live-ll-hls-and-remaining.md) |

Real gaps are tracked as issues #1422-#1429 (plus #1413). The reports are point-in-time (HEAD `90a30b58e`); re-audit after the fixes land.
