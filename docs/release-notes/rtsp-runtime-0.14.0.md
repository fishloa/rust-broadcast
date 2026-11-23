# rtsp-runtime 0.14.0

_Released 2026-11-22._

Minor for a 0.x crate. One behaviour change: `rtsps://` now trusts the OS trust store through `rustls-platform-verifier` instead of the bundled Mozilla root list, so a private CA installed in the OS works and `webpki-roots` is gone. Additive: `client::MAX_INBOUND_BYTES` is public. The breaking part is the epoch move (`broadcast-auth` 0.9, `broadcast-common` 14).

## Breaking changes

- Builds against new caret epochs of workspace siblings (`broadcast-auth` 0.8 -> 0.9, `broadcast-common` 13 -> 14). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch, and `broadcast-auth`'s `Verifier`/`Credentials` it takes are 0.9 ones.

## Added

- `client::MAX_INBOUND_BYTES` (1 MiB) is public: the bound on the client's inbound buffer.

## Changed

- Behaviour change: `rtsps://` (`default_tls_client_config`) trusts the OS trust store through `rustls-platform-verifier` (ring provider); a private CA installed in the OS now works and the bundled Mozilla root list (`webpki-roots`) is gone (#3960). A host that relied on the bundled roots without the CA being in the OS store must install it there or pass a custom `rustls::ClientConfig`.
- Internal: error types derive `thiserror::Error` (messages unchanged) (#3974, #4020).
- README's `rtsps://` row names the OS trust store (#4012).

## Tests

The whole RFC 2326 Appendix A state table is exercised transition by transition for both roles; session-id allocation, the SDP builder and per-scheme default port, the 64 KiB header and 2 MiB message caps with both edges, interleaved framing, `CSeq` on the wire and `PlayPlan::channels` are pinned, and the async client is tested to refuse a `PLAY` answered 454/455, drop a stale response and bound its pending queue (audit 21/22 findings 27-33, #2944/#2945, #2995-#3001).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4012 (README trust-store row), #4020 (`thiserror`), #4021 (test bullets under `### Tests`).

---

Published from tag `rtsp-runtime-v0.14.0`.
