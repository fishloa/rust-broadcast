---
status: DRAFT
---
# webrtc-runtime 0.7.0

_Released 2026-11-10._

Breaking-class release (0.6 -> 0.7): the test-hook feature is renamed `test-support` -> `test-seams`.

Version: 0.6.0 -> 0.7.0.

## Breaking changes and migration

- The non-default test-hook feature is renamed `test-support` -> `test-seams`. Only a test suite enabling it by name is affected.

```toml
# before
webrtc-runtime = { version = "0.6", features = ["test-support"] }
# after
webrtc-runtime = { version = "0.7", features = ["test-seams"] }
```

## Fixed

- Trickle PATCH `412` adopts response `ETag`.
- In-flight request tracking in send order.
- Non-2xx offer POST returns client to `Idle`.
- `Session::accept`, `ack_restart`, `ack_answer` no longer reopen closed session.
- `WhepSession::ack_answer` validates entity-tag and sends it as `ETag`.

---

Published from tag `webrtc-runtime-v0.7.0`.
