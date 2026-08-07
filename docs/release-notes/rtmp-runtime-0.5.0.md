# rtmp-runtime 0.5.0

Released 2026-08-07.

### Added

- `client` module — sans-IO RTMP 1.0 **client** publish session engine
  (`ClientSession`, `ClientHandshake`, `ClientConfig`, `ClientEvent`):
  `connect` → `createStream` → `publish` auto-advance,
  `send_audio()`/`send_video()`/`send_metadata()` for the Publishing state
  (#744).
