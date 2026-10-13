# multimux 0.12.0

_Released 2026-10-13._

Epoch bump forced by `transmux` 0.26. Fixes in this release: SRT/RTSP push now uses a persistent per-connection TS muxer (PAT/PMT per frame, restarting continuity counters and repeated PTS are gone), `PushTransportEgress` passes the full track set (it used to map every track onto PID 0x100), and `SrtTransport` splits frames into 1316-byte messages (whole frames over the receiver MTU were silently dropped, losing every keyframe). DASH/LL-DASH `startNumber` is now fixed at the first segment (a sliding window made players request segments that 404).


### Fixed
- SRT and RTSP push now deliver one continuous MPEG-2 TS (#1024, audit r07-C1). The default `PushTransport::encode_media` ran a fresh one-shot `TsMux::package` per sample, so the receiver saw PAT/PMT before every frame, continuity counters restarting at 0 and the same PTS/PCR on every frame. `SrtTransport`/`RtspTransport` now own a per-connection `transmux::StreamingTsMux` (new hook `PushTransport::ts_mux`, default `None` = the old stateless path, so out-of-tree transports are unaffected): continuity counters, PCR and one shared DTS/PTS origin continue across samples, and PAT/PMT repeat on a media-time interval. `PushTransportEgress` also hands the muxer the full carried track set with every sample (a one-track `Media` per sample mapped every track onto the first ES PID and flipped the PMT).
- `SrtTransport::send` splits a muxed frame into 1316-byte (7 x 188) SRT messages. SRT delivers one message per UDP datagram and the receiver drops and counts anything over its MTU-sized cap, so every frame over ~1.3 kB (all keyframes) was silently lost.
- **Behaviour change:** the DASH and LL-DASH MPDs now keep `SegmentTemplate@startNumber` fixed at the first segment number (`1`, the number a `Trunk` gives its first segment) (and `@duration` at the nominal target) instead of sliding it to the window's oldest segment (#1058, audit r07-C9). With `availabilityStartTime` fixed, a sliding `startNumber` made players derive a live edge of segments that were never published (404, stalled playback) once the window slid (ISO/IEC 23009-1 section 5.3.9.5.3). Segment URLs are unchanged (`seg-1-<N>.m4s`); the DVR depth is still bounded by `timeShiftBufferDepth`. `availabilityStartTime` remains the route's creation time, so number-derived availability is exact only to the extent segments close at their nominal duration. Smooth already emitted absolute `c@t`.

### Added
- `dash_sliding_window` and `push_ts_stream` integration tests: the sliding window is run past several slides with every derivable segment URL requested and the MPD validated with `media-doctor`; the SRT push is loopback-captured and checked for continuity, PSI cadence, source-matching DTS and TR 101 290 (`dvb-conformance`).

---

Published from tag `multimux-v0.12.0`.
