# srt-runtime 0.6.0

_Released 2026-10-17._

**Epoch-forced major-class release with one additive feature.** `srt-runtime` follows the `broadcast-common` 9 -> 10 caret epoch. New: `io::SrtSocket::rendezvous` / `rendezvous_with`, so the tokio adapter now drives the sans-IO Rendezvous handshake (section 4.3.2) over a bound UDP socket; before, only the sans-IO engine existed. It is verified against a real peer: `tests/libsrt_rendezvous.rs` runs `srt-live-transmit` in `mode=rendezvous` over loopback in both data directions, repeated with fresh random cookies so both Initiator and Responder outcomes occur (it skips loudly when libsrt is absent; `SRT_REQUIRE_LIBSRT=1` turns the skip into a failure), and `tests/io_rendezvous.rs` is the self-vs-self counterpart that runs everywhere.

## Why this version

- **Proposed version:** 0.5.0 -> **0.6.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** minor (additive `SrtSocket::rendezvous`/`rendezvous_with`; the Caller path's post-handshake wiring is now the shared `SrtSocket::established`, an internal refactor).
- **cargo-semver-checks vs live 0.5.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 0.5 -> 0.6; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

`srt-runtime` has no source-breaking change of its own; the major is the `broadcast-common` epoch cascade.

### 1. `broadcast-common` 10

`broadcast-common` 10 removed `Serialize::to_bytes()` (the default method that panicked when `serialize_into` failed). Code that serialises any `srt-runtime` type through it must switch to the fallible form:

```rust
// before
let bytes = value.to_bytes();
// after
let bytes = value.try_to_bytes()?;   // Result<Vec<u8>, _>
```

Your dependency graph must resolve a single `broadcast-common` 10 (a second copy on 9.x is a different `Serialize` trait and the types will not satisfy it).

### 2. Rendezvous mode (new)

Both peers bind a known local port and send to each other's address at the same time; the cookie contest picks Initiator or Responder. Enable the `tokio` feature and call:

```rust
let sock = SrtSocket::rendezvous(local_addr, remote_addr, handshake_config).await?;
// or SrtSocket::rendezvous_with(local_addr, remote_addr, handshake_config, io_config).await?
```

`local_addr`'s port must be the one the peer targets. Like `connect`, it refuses a `crypto`-enabled `HandshakeConfig` with `Error::EncryptionUnsupported` before any I/O (the Rendezvous engine never negotiates keys). `Error::Rejected` carries the peer's rejection reason and `Error::HandshakeTimedOut` means the peer never completed the exchange within the handshake budget.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Added
- `io::SrtSocket::rendezvous` / `rendezvous_with`: the tokio adapter now drives the sans-IO `RendezvousHandshake` (§4.3.2) over a bound UDP socket, so two peers can connect in Rendezvous mode (previously only the sans-IO engine existed). Verified against a REAL peer: `tests/libsrt_rendezvous.rs` runs `srt-live-transmit` in `mode=rendezvous` over loopback in both data directions, repeated with fresh random cookies so both Initiator/Responder outcomes occur (skips loudly when libsrt is absent, like the other `libsrt_*` tests; `SRT_REQUIRE_LIBSRT=1` makes the skip a failure). `tests/io_rendezvous.rs` is the self-vs-self counterpart that runs everywhere. The Caller path's post-handshake wiring is now the shared `SrtSocket::established`.

## Merged changes since `srt-runtime-v0.5.0`

- docs: clear per-crate documentation for cluster B (16 crates) (#1291)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- chore: pre-release hygiene (README coverage, snippet tool, docsrs, examples, CLAUDE.md) (#1278)
- srt-runtime: tokio Rendezvous mode + real libsrt rendezvous interop (#1200) (#1201)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
