# multimux 0.8.0

Released 2026-08-07.

### Added

- **Push re-egress outputs** (#744): `OutputKind::SrtPush`, `RtmpPush`,
  `RtspPush` for relaying ingested media to downstream servers — turns multimux
  from an HTTP-only origin into a relay/gateway.
- `PushFormat` config enum (`Ts`, `Mp4`, `Mkv`) for per-output container
  format selection.
- `ReconnectPolicy` for exponential-backoff reconnect on push outputs.
- SRT push transport (SRT Caller mode to a remote SRT Listener).
- RTSP push transport (client ANNOUNCE/RECORD to a remote RTSP server).
- RTMP push transport (client connect/createStream/publish to a remote RTMP
  server).
- `RouteHandle::await_first_trunk()` for push tasks to discover program
  availability.
- Supervisor lifecycle: push tasks spawn at route creation, cancel+join on
  route removal.

### Changed

- Requires `rtmp-runtime` 0.5 and `rtsp-runtime` 0.5 (new client-side
  publish APIs used by the push transports).
