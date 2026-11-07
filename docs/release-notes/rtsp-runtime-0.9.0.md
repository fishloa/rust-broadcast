---
status: DRAFT
---

# rtsp-runtime 0.9.0

_Released 2026-11-07._

Breaking-class release (0.8 -> 0.9): `Error::Auth` wraps `broadcast_auth::Error` instead of a flattened `String`, a `401` without a challenge is the new `Error::MissingAuthChallenge`, three never-constructed error variants are removed, and the `log` dependency is dropped.

Version: 0.8.1 -> 0.9.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- `Error::Auth` wraps `broadcast_auth::Error` (it was a flattened `String`); a `401` without a `WWW-Authenticate` header is the new `Error::MissingAuthChallenge`.
- `Error::UnknownCSeq`, `Error::MissingSession` and `Error::MissingRequestUri`, never constructed: RFC 2326 puts the Session id (§10.4) and the CSeq echo on the server, and the client deliberately skips an uncorrelatable response.
- Unused `AsyncRtspServer::accept_with_timeouts`; `ClientSession::session_warning_count` is now test-only.

### Renamed and removed items

```text
Error::Auth(String)
    -> Error::Auth(broadcast_auth::Error)
Error::UnknownCSeq / Error::MissingSession / Error::MissingRequestUri
    -> (removed; never constructed)
AsyncRtspServer::accept_with_timeouts
    -> (removed; unused)
```

### Behaviour change: logging

The two `log::warn!` calls on a malformed `Session` header are gone with the `log` dependency; the same warnings stay available through `ClientSession::session_warnings()`.

## Other changes

- Session-id comparison uses `broadcast_auth::constant_time_eq`; the direct `subtle` dependency is gone.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

No workspace sibling requirement changes epoch.

---

Published from tag `rtsp-runtime-v0.9.0`.
