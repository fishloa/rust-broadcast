# multimux 0.15.0

_Released 2026-11-02._

Minor release: caption language discovery from the ATSC descriptor and caption detection (issue #1428), plus a blocking-reload fix (issue #1651).

### Fixed
- A Playlist blocking reload now waits three published Target Durations (never less than 5 s) before answering `503`, instead of a fixed 5 s, so a request for the next segment of a route with 5 to 6 s segments is served rather than shed just before the media arrives (RFC 8216bis section 6.2.5.2, issue #1651). The `503` carries `Retry-After: 1`. Applies to the media, I-frame, audio and subtitle playlists and the TS-HLS playlist.

### Added
- `CLOSED-CAPTIONS` renditions take `LANGUAGE` from the ATSC `caption_service_descriptor` in the video's PMT loop (issue #1428): a 708 service by `caption_service_number`, 608 channels when all declared 608 entries agree on the language. `closed_caption_overrides` still win, and a stream without the descriptor gets no `LANGUAGE`. `RouteHandle::note_closed_caption_languages` is the new hand-off.
- Caption detection reads the real channel and service (issue #1428): CEA-608 text is attributed to `CC1` to `CC4` by field and control-code channel bit (a stream carrying text on both fields declares `CC1` and `CC3`), XDS, text mode, null padding and bad-parity pairs declare nothing, and every CEA-708 service present is declared as `SERVICEn`. After 10 seconds of video with no caption found a route declares `CLOSED-CAPTIONS=NONE` on every variant; a caption found later replaces it with the caption group.

---

Published from tag `multimux-v0.15.0`.
