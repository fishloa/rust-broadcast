---
status: DRAFT
---
# rtsp-runtime 0.8.0

_Released 2026-10-16._

**Breaking release.** `ClientSession::handle_data` returns `HandleOutcome { events, error }` so a terminal protocol error no longer discards the media frames and responses decoded in the same call (audit T16, #1144); session-id comparison is constant-time.

## Why this version

- **Proposed version:** 0.7.1 -> **0.8.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (ClientSession::handle_data returns HandleOutcome).
- **cargo-semver-checks vs live 0.7.1 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-auth.
- **Caret epoch:** 0.7 -> 0.8; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-auth = "0.4"
+broadcast-auth = "0.5"
```

## Breaking changes and migration

```rust
// before
let events = session.handle_data(bytes)?;
// after: identical all-or-nothing behaviour
let events = session.handle_data(bytes).into_result()?;     // or handle_data_strict(bytes)?
// or: keep what was decoded before the damage
let out = session.handle_data(bytes);
for ev in out.events { handle(ev); }
if let Some(err) = out.error { return Err(err.into()); }
```

The tokio `ClientCodec` delivers the events decoded before the error and raises the error on the next `decode`. `multimux`'s caret epoch on this crate moves 0.7 -> 0.8.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

#### Changed (BREAKING: epoch — requires broadcast-auth 0.5)
- Builds against new caret epochs of workspace siblings (`broadcast-auth` 0.4 -> 0.5). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff.

#### Security (secrets and identity handling, #1142)

- The `Session` id comparison uses `subtle::ConstantTimeEq` (new `subtle` dependency) instead of a hand-written fold; new `tests/secret_debug_guard.rs`.

#### Changed
- `ClientSession::handle_data` decodes with a cursor and drains the consumed prefix once per call instead of memmoving the remainder per frame/message (audit r09-O3): 600 interleaved frames in one read decode ~17x faster (4.2 ms -> 0.24 ms, debug build).
- **BREAKING:** `ClientSession::handle_data` now returns `HandleOutcome { events, error }` instead of `Result<Vec<ClientEvent>>` (audit T16, #1144). A terminal protocol error (an unparseable message head, an over-long header, an unfulfillable `Content-Length`: TCP framing is lost) used to discard every event already decoded in the same call, including media frames and responses that preceded the damage; they are now returned alongside the error. Migrate: `session.handle_data(b)?` becomes `session.handle_data(b).into_result()?` (identical all-or-nothing behaviour; also available as the new `handle_data_strict`), or read `outcome.events` first and then act on `outcome.error`. The tokio `ClientCodec` delivers the events decoded before the error and raises the error on the next `decode`.

#### Added
- `HandleOutcome` (exported at the crate root), `HandleOutcome::into_result` and `ClientSession::handle_data_strict`.

## Merged changes since `rtsp-runtime-v0.7.1`

- docs: clear per-crate documentation for cluster B (16 crates) (#1291)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- chore: pre-release hygiene (README coverage, snippet tool, docsrs, examples, CLAUDE.md) (#1278)
- tests: one fixture resolver (public -> private/fixtures -> .test-streams); move local-only captures to the private repo (#1259) (#1260)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit systemic wave 2: T16 resync, largesize, ttml drop, WHIP/WHEP DELETE/PATCH/ICE restart, mpeg-ts/pes reserved+stuffing preservation, real-capture guards (NO release) (#1175)
