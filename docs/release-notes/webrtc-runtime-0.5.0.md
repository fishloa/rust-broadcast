---
status: DRAFT
---

# webrtc-runtime 0.5.0

_Released 2026-11-07._

Breaking-class release (0.4 -> 0.5) driven by `rtp-packet` 0.6: the optional `media` feature builds on `rtp_packet::rtcp` instead of the absorbed `rtcp-packet` crate. The WHIP client and WHEP player now share one private session core; the public API is unchanged.

Version: 0.4.1 -> 0.5.0.

## Other changes

- The `media` feature builds on `rtp-packet`'s `rtcp` module instead of the absorbed `rtcp-packet` crate; the types are unchanged.
- The WHIP client and WHEP player share a `SessionCore` (endpoint, bearer token, pending request, request builders, redacting `Debug`).

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `rtp-packet` 0.5 -> 0.6

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `webrtc-runtime-v0.5.0`.
