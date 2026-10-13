# rtsp-runtime 0.7.1

_Released 2026-10-13._

### Fixed
- `AsyncRtspClient::send_interleaved` no longer parks ~1.5 ms per frame. Its
  per-frame inbound probe was `timeout(Duration::ZERO, ..)`, which tokio
  resolves only on its next 1 ms timer tick, capping an RTSP pusher at roughly
  650 frames/s (~0.85 MB/s of TS, ~7 Mbit/s) regardless of the link. The probe
  is now a single `now_or_never()` poll (#1165).

---

Published from tag `rtsp-runtime-v0.7.1`.
