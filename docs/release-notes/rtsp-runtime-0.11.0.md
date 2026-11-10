---
status: DRAFT
---
# rtsp-runtime 0.11.0

_Released 2026-11-10._

Breaking-class release (0.10 -> 0.11): `Error::MissingCSeq` is removed; sibling epochs move.

Version: 0.10.0 -> 0.11.0.

## Breaking changes and migration

- `Error::MissingCSeq` is removed. `ServerSession::handle_request` answers a request with no parseable `CSeq` with `400 Bad Request` and carries on, instead of returning an error that ended the connection.

```rust
// before: match err { Error::MissingCSeq => ... }
// after: handle the 400 response path; no variant to match
```

## Added

- `ServerSession::with_describe_sdp` and `with_server_ports`.
- `OPTIONS` reply carries `Public` listing implemented methods.

## Fixed

- Reversed `port`/`client_port`/`server_port`/`interleaved` range is a `Transport` parse error.
- Server picks first servable `Transport` spec; unmodelled spec no longer fails whole header.
- Unimplemented method answered `501` instead of `455`.
- Client forgets session state on `3xx`/`454`; non-2xx `SETUP` no longer records session; `401` stale replay capped at 3.
- `handle_data` applies 1 MiB inbound cap after decoding.
- `Session` `timeout=` parsed as `u64` delta-seconds.
- `AsyncRtspClient` send-side drain keeps decode error.

## Other changes

- A `3xx` or `454 Session Not Found` response resets the session.

## Dependencies

- `broadcast-auth` 0.5 -> 0.6

---

Published from tag `rtsp-runtime-v0.11.0`.
