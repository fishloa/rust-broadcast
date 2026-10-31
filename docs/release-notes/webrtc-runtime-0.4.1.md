# webrtc-runtime 0.4.1

_Released 2026-10-31._

Patch release, but it raises a dependency floor: with the `media` feature, `webrtc-runtime` now requires `rtc-dtls` >= 0.21.1 (the release that adds `EndpointEvent::PeerClosed`). It fixes a wrong-fingerprint DTLS rejection that hung under `rtc-dtls` 0.21.1, and a peer's graceful DTLS close is now reported as `MediaEvent::DtlsClosed` instead of surfacing as an error; `MediaTransport::close` sends one. No change without the `media` feature.

### Added
- `MediaEvent::DtlsClosed`, reported when the peer ends the DTLS association with `close_notify`, and `MediaTransport::close` to send one. Requires `rtc-dtls` >= 0.21.1 (the release that adds `EndpointEvent::PeerClosed`); a peer's graceful close used to surface as `Error::Media("unrecognised rtc-dtls endpoint event")`.

### Fixed
- `MediaTransport::handle_datagram` tears down the DTLS association when a DTLS read fails. `rtc-dtls` 0.21.1 now sends the fatal alert it queues when a read errors, and a received fatal alert is echoed back as a fatal alert, so two transports whose handshake failed (for example a `remote_fingerprint` mismatch) answered each other's alerts endlessly. A wrong fingerprint is now rejected promptly with `Error::Media("dtls read: ...")` under both 0.21.0 and 0.21.1 (#1628).

### Changed
- Dependency `sdp-types` 0.2 -> 0.3 (optional, `media` feature; used internally; no public API exposure).

---

Published from tag `webrtc-runtime-v0.4.1`.
