# srt-runtime 0.7.0

_Released 2026-11-03._

Minor release: encryption on the tokio data and rendezvous paths, plus public `skip_range` helpers (breaking: `Error::EncryptionUnsupported` removed).

### Added
- Tests: RFC 3394 AES key wrap (all six section 4 vectors, wrap + unwrap) and RFC 6070 PBKDF2-HMAC-SHA1 vectors, plus SRT KEK derivation checks (low 64 bits of salt, 2048 iterations) against the private transcription paths (#1653).
- Encryption in the tokio adapter data path: AES-CTR encrypt/decrypt with KM refresh (#1653).
- Encryption on the tokio rendezvous path: `rendezvous`/`rendezvous_with` now negotiate Key Material (the cookie-contest winner sends it) and use the same encrypt/decrypt data path and KM refresh as Caller/Listener (#1658).
- `arq::Receiver::skip_range` and `tsbpd::TsbpdScheduler::skip_range` are public (and no longer gated on the `tokio` feature), so a sans-IO driver can apply a DROPREQ and the Too-Late Packet Drop "fake ACK" without the tokio adapter (used by `broadcast-client`'s SRT caller).

### Breaking
- Removed `Error::EncryptionUnsupported` (no longer used) (#1658).

## Upgrading

- `Error::EncryptionUnsupported` is gone. Drop any match arm or construction that named it; encryption is now supported on both the data path and the rendezvous path.

```rust
// before
Error::EncryptionUnsupported => { /* ... */ }
// after
// (arm removed)
```

---

Published from tag `srt-runtime-v0.7.0`.
