---
status: DRAFT
---
# webrtc-runtime 0.4.0

_Released 2026-10-16._

**Breaking release (behavioural changes plus the `broadcast-common` 10 epoch).** No type signature of the WHIP/WHEP engine changes, but three behaviours do: `WhipClient`/`WhepPlayer` send the bearer token only to the origin of the configured endpoint; `WhipSession::on_patch` now rejects a PATCH with no `If-Match` (new `Error::PreconditionRequired`, HTTP 428) and one whose `Content-Type` is not `application/trickle-ice-sdpfrag` (`Error::InvalidSdpFragment`), matching WHEP. It also adds `MediaTransport::restart_ice` (feature `media`), an in-place ICE restart (RFC 8445 section 9, RFC 9725 section 4.3.2) with media still flowing, and stops printing secrets in `Debug`.

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

`rtp-packet` and `rtcp-packet` are optional (enabled by the `media` feature). Two other manifest changes are not version bumps: `url = "2"` is now an unconditional dependency (it was `media`-only), and `subtle = "2"` (no default features) is a new optional dependency pulled in by `media` for the constant-time DTLS fingerprint comparison.

## Breaking changes and migration

- Move every `broadcast-common` requirement in your tree to `10` (and `rtp-packet`/`rtcp-packet` to `0.5`/`0.6` if you use `media`); `Serialize::to_bytes` was removed there, use `try_to_bytes()?`.
- `WhipClient`/`WhepPlayer` send the bearer token only to the origin (scheme, host, port) of the configured endpoint: a session `Location` on another origin gets no `Authorization` header. If your server hands back a session URL on a different host (a CDN, a sharded origin), the follow-up PATCH/DELETE will now be unauthenticated; configure the endpoint on the origin that issues the session, or authenticate that other origin yourself. The `url` crate is now an unconditional dependency.
- `WhipSession::on_patch` returns the new `Error::PreconditionRequired` for a PATCH with no `If-Match` (RFC 9725 sections 4.3.1/4.3.2: the session issues an `ETag`, so a missing entity-tag is `428 Precondition Required`); it used to surface as a `TrickleIce` event with `if_match: None`. `Event::TrickleIce::if_match` is therefore always `Some` from `WhipSession`. `Error` is `#[non_exhaustive]`, so the variant is additive; map it to a 428 response in your HTTP layer. `WhepSession` is unchanged.
- `WhipSession::on_patch` rejects a PATCH whose `Content-Type` is missing or not `application/trickle-ice-sdpfrag` (parameters such as `charset` are ignored) with `Error::InvalidSdpFragment`. **Migrate: send the trickle-ICE media type on every WHIP PATCH** (WHEP already required it).
- `Debug` of `WhipClient`, `WhepPlayer`, `IceServer` (TURN credential) and `MediaTransportConfig` (ICE passwords) no longer prints the secret; log output that relied on seeing them will be redacted.

New, additive: `MediaTransport::restart_ice` (feature `media`) installs new local and remote ICE credentials, drops the previous remote candidates and pairs, restarts connectivity checks and resets the remote-candidate budget; local candidates, DTLS and SRTP are untouched, so no new handshake runs.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `rtcp-packet` 0.5 -> 0.6, `rtp-packet` 0.4 -> 0.5). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Changed
- **BREAKING (behaviour):** `WhipSession::on_patch` returns the new `Error::PreconditionRequired` for a PATCH with no `If-Match` (RFC 9725 sections 4.3.1/4.3.2: the session issues an `ETag`, so a missing entity-tag is `428 Precondition Required`); it used to be a `TrickleIce` event with `if_match: None`. `Event::TrickleIce::if_match` is therefore always `Some` from `WhipSession`. `Error` is `#[non_exhaustive]`, so the new variant is additive. (`WhepSession` is unchanged.)

#### Security (secrets and identity handling, #1142)

- **BREAKING (behaviour):** `WhipClient`/`WhepPlayer` send the bearer token only to the origin of the configured endpoint. A session URL (`Location`) on another scheme, host or port gets no `Authorization` header. `url` is now a non-optional dependency (it was `media`-only).
- `Debug` of `WhipClient`, `WhepPlayer`, `IceServer` (TURN credential) and `MediaTransportConfig` (ICE passwords) no longer prints the secret.
- The DTLS certificate-fingerprint comparison uses `subtle::ConstantTimeEq` (new optional `subtle` dependency under `media`) instead of a hand-written fold.
- New `tests/secret_hygiene.rs`, `tests/secret_debug_guard.rs`.

#### Changed
- **BREAKING (behaviour):** `WhipSession::on_patch` now rejects a PATCH whose `Content-Type` is missing or is not `application/trickle-ice-sdpfrag` (RFC 9725 section 4; parameters such as `charset` are ignored) with `Error::InvalidSdpFragment`, matching `WhepSession::on_patch` (audit r09-W21). Migrate: send the trickle-ICE media type on every WHIP PATCH.

#### Added
- `MediaTransport::restart_ice` (feature `media`): an in-place ICE restart (RFC 8445 section 9, as WHIP/WHEP need for RFC 9725 section 4.3.2). Installs new local and remote ICE credentials, drops the previous remote candidates and pairs, restarts connectivity checks and resets the remote-candidate budget; local candidates, DTLS and SRTP are untouched, so media keeps flowing and no new handshake runs. Covered by a two-transport loopback test (pair re-nominated, same SRTP session decrypts both ways).

## Merged changes since `webrtc-runtime-v0.3.0`

- docs: clear per-crate documentation for cluster B (16 crates) (#1291)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- Fix the confirmed WHIP/WHEP/DASH-pull review findings (supersedes #1267) (#1271)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit systemic wave 2: T16 resync, largesize, ttml drop, WHIP/WHEP DELETE/PATCH/ICE restart, mpeg-ts/pes reserved+stuffing preservation, real-capture guards (NO release) (#1175)
