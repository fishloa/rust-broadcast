---
status: DRAFT
---
# srt-runtime 0.6.0

_Released 2026-10-16._

**Epoch-forced major-class release.** `SrtSocket::rendezvous` / `rendezvous_with`: the tokio adapter now drives the sans-IO Rendezvous handshake (section 4.3.2) over a bound UDP socket, verified against a real peer (`srt-live-transmit` in `mode=rendezvous`).

## Why this version

- **Proposed version:** 0.5.0 -> **0.6.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** minor (SrtSocket::rendezvous (additive)).
- **cargo-semver-checks vs live 0.5.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 0.5 -> 0.6; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

No API break of its own; the major is the `broadcast-common` epoch cascade.

## What changed (from CHANGELOG `[Unreleased]`)

#### Added
- `io::SrtSocket::rendezvous` / `rendezvous_with`: the tokio adapter now drives the sans-IO `RendezvousHandshake` (§4.3.2) over a bound UDP socket, so two peers can connect in Rendezvous mode (previously only the sans-IO engine existed). Verified against a REAL peer: `tests/libsrt_rendezvous.rs` runs `srt-live-transmit` in `mode=rendezvous` over loopback in both data directions, repeated with fresh random cookies so both Initiator/Responder outcomes occur (skips loudly when libsrt is absent, like the other `libsrt_*` tests; `SRT_REQUIRE_LIBSRT=1` makes the skip a failure). `tests/io_rendezvous.rs` is the self-vs-self counterpart that runs everywhere. The Caller path's post-handshake wiring is now the shared `SrtSocket::established`.

## Merged changes since `srt-runtime-v0.5.0`

- srt-runtime: tokio Rendezvous mode + real libsrt rendezvous interop (#1200) (#1201)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
