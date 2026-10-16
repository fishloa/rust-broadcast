---
status: DRAFT
---
# webrtc-runtime 0.4.0

_Released 2026-10-16._

**Epoch-forced major-class release.** WHIP/WHEP bearer tokens are sent only to the configured endpoint's origin; `WhipSession::on_patch` requires `Content-Type: application/trickle-ice-sdpfrag` like WHEP; `MediaTransport::restart_ice` adds an in-place ICE restart (RFC 8445 section 9 / RFC 9725 section 4.3.2) with media still flowing.

## Why this version

- **Proposed version:** 0.3.0 -> **0.4.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** minor (restart_ice added; behaviour changes (token origin gating, WHIP PATCH Content-Type)).
- **cargo-semver-checks vs live 0.3.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** behaviour changes (token origin gating, WHIP PATCH Content-Type); epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common, rtcp-packet, rtp-packet.
- **Caret epoch:** 0.3 -> 0.4; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-rtcp-packet = "0.5"
+rtcp-packet = "0.6"
-rtp-packet = "0.4"
+rtp-packet = "0.5"
```

## Breaking changes and migration

- `WhipClient`/`WhepPlayer` send the bearer token only to the origin (scheme, host, port) of the configured endpoint: a session `Location` on another origin gets no `Authorization`. `url` is a non-optional dependency now.
- `WhipSession::on_patch` rejects a PATCH whose `Content-Type` is missing or not `application/trickle-ice-sdpfrag` (parameters such as `charset` ignored) with `Error::InvalidSdpFragment`. **Migrate: send the trickle-ICE media type on every WHIP PATCH.**
- `Debug` of `WhipClient`, `WhepPlayer`, `IceServer`, `MediaTransportConfig` no longer prints secrets.

## What changed (from CHANGELOG `[Unreleased]`)

#### Security (secrets and identity handling, #1142)

- **BREAKING (behaviour):** `WhipClient`/`WhepPlayer` send the bearer token only to the origin of the configured endpoint. A session URL (`Location`) on another scheme, host or port gets no `Authorization` header. `url` is now a non-optional dependency (it was `media`-only).
- `Debug` of `WhipClient`, `WhepPlayer`, `IceServer` (TURN credential) and `MediaTransportConfig` (ICE passwords) no longer prints the secret.
- The DTLS certificate-fingerprint comparison uses `subtle::ConstantTimeEq` (new optional `subtle` dependency under `media`) instead of a hand-written fold.
- New `tests/secret_hygiene.rs`, `tests/secret_debug_guard.rs`.
#### Changed
- Internal: adapted to `broadcast-common`'s removal of the panicking `Serialize::to_bytes` (tests, examples and docs now use `try_to_bytes`); requires the new `broadcast-common` major (umbrella #1143).
- **BREAKING (behaviour):** `WhipSession::on_patch` now rejects a PATCH whose `Content-Type` is missing or is not `application/trickle-ice-sdpfrag` (RFC 9725 section 4; parameters such as `charset` are ignored) with `Error::InvalidSdpFragment`, matching `WhepSession::on_patch` (audit r09-W21). Migrate: send the trickle-ICE media type on every WHIP PATCH.
#### Added
- `MediaTransport::restart_ice` (feature `media`): an in-place ICE restart (RFC 8445 section 9, as WHIP/WHEP need for RFC 9725 section 4.3.2). Installs new local and remote ICE credentials, drops the previous remote candidates and pairs, restarts connectivity checks and resets the remote-candidate budget; local candidates, DTLS and SRTP are untouched, so media keeps flowing and no new handshake runs. Covered by a two-transport loopback test (pair re-nominated, same SRTP session decrypts both ways).

## Merged changes since `webrtc-runtime-v0.3.0`

- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit systemic wave 2: T16 resync, largesize, ttml drop, WHIP/WHEP DELETE/PATCH/ICE restart, mpeg-ts/pes reserved+stuffing preservation, real-capture guards (NO release) (#1175)
