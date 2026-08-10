# srt-runtime 0.4.0

**Release date:** 2026-08-10

Fixes a silent peer-ISN mis-seeding defect in the tokio Caller adapter, adds
`#[non_exhaustive]` to two enums, raises the MSRV, and corrects a stale README
install snippet.

## What changed

- **Fixed:** `io::SrtSocket::connect_from` (the tokio Caller adapter) no
  longer defaults the peer's `initial_seq_number` to `0` when it can't be
  extracted from the handshake bytes that just drove the connection to
  `Connected`. That fallback made a genuine peer ISN of 0 indistinguishable
  from an extraction failure, silently mis-seeding ARQ/TSBPD sequence
  tracking for the whole connection. The extraction is now
  `require_peer_isn`, which returns `Error::InvalidField` instead of `0` on
  any parse failure; `SrtSocket::connect`/`connect_from` propagate that error
  instead of silently continuing. No public API previously guaranteed the
  old default, so this is not expected to be observable as a behaviour
  change in practice.
- `KeyParity` (`km_refresh`) and `LossListEntry` (`packet::nak`) now carry
  `#[non_exhaustive]` (issue #806's non_exhaustive drift-guard audit).
- Added `tests/non_exhaustive_coverage.rs`, a drift guard for
  `#[non_exhaustive]` coverage (issue #806).
- MSRV raised to **1.95.0** (issue #949), part of a workspace-wide bump that
  removes the MSRV split `webrtc-runtime`'s optional `media` feature used to
  require. No functional or API change on its own.
- Doc accuracy (#941 row 6): README install snippet corrected from `"0.2"` to
  `"0.3"` (the crate was 0.3.0 at the time of the fix).

## Breaking changes

`KeyParity` and `LossListEntry` are now `#[non_exhaustive]`. Any exhaustive
downstream `match` on either type needs a wildcard arm added:

```rust
match parity {
    KeyParity::Even => { /* ... */ }
    KeyParity::Odd => { /* ... */ }
    _ => { /* ... */ }
}
```

## Migration

Add a wildcard arm to any exhaustive `match` on `KeyParity` or
`LossListEntry`. Rebuild with rustc >= 1.95.0. If you relied on
`connect`/`connect_from` succeeding with a peer ISN silently defaulted to 0,
that path now returns `Error::InvalidField` instead.
