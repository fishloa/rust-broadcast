# ssai-runtime 0.6.0

_Released 2026-11-14._

Breaking-class release (0.5 -> 0.6): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

Version: 0.5.0 -> 0.6.0.

## Breaking changes and migration

- `InterstitialDateRange::parse_tag_line` keeps unknown `X-SNAP`/`X-RESTRICT` in `SnapMode::Other`/`RestrictMode::Other`; both enums lose `Copy`.
- Rejects quoted `DURATION`/`X-RESUME-OFFSET`/`X-PLAYOUT-LIMIT` or unquoted `ID`/`START-DATE`/`X-ASSET-URI`/`X-ASSET-LIST`/`X-SNAP`/`X-RESTRICT`.
- `X-RESUME-OFFSET` accepts signed values.

```rust
// SnapMode/RestrictMode now carry String; cannot derive Copy.
```

## Added

- `SessionStore` bounded: `DEFAULT_MAX_SESSIONS`, `with_max_sessions`, `Error::SessionStoreFull`.
- `advance(now_us)`, `expire_idle(ttl_us)`.
- `Error::BreakAlreadyResumed`.

## Fixed

- `SessionStore::resume` on already-resumed break returns `Error::BreakAlreadyResumed`.
- `X-RESUME-OFFSET` signed decimal; `DURATION`/`X-PLAYOUT-LIMIT` non-negative.
- `DURATION` of 2^63 no longer renders as `i64::MAX`.
- `X-SNAP`/`X-RESTRICT` tokens trimmed; empty dropped; unknown kept as `Other`.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `broadcast-hls` 0.6 -> 0.7.

---

Published from tag `ssai-runtime-v0.6.0`.
