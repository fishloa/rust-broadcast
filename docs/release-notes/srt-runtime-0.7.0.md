# srt-runtime 0.7.0

_Released 2026-11-02._

### Added
- Tests: RFC 3394 AES key wrap (all six section 4 vectors, wrap + unwrap) and RFC 6070 PBKDF2-HMAC-SHA1 vectors, plus SRT KEK derivation checks (low 64 bits of salt, 2048 iterations) against the private transcription paths (#1653).
- Encryption in the tokio adapter data path: AES-CTR encrypt/decrypt with KM refresh (#1653).
- Encryption on the tokio rendezvous path: `rendezvous`/`rendezvous_with` now negotiate Key Material (the cookie-contest winner sends it) and use the same encrypt/decrypt data path and KM refresh as Caller/Listener (#1658).

### Changed
- Removed `Error::EncryptionUnsupported` (no longer used) (#1658).

### Added
- `arq::Receiver::skip_range` and `tsbpd::TsbpdScheduler::skip_range` are public (and no longer gated on the `tokio` feature), so a sans-IO driver can apply a DROPREQ and the Too-Late Packet Drop "fake ACK" without the tokio adapter (used by `broadcast-client`'s SRT caller).

---

Published from tag `srt-runtime-v0.7.0`.
