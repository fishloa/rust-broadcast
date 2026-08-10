# ts-fix 0.5.0

**Release date:** 2026-08-10

A new `cli` feature separates the `ts-fix` binary from the library, making
the crate's long-standing `no_std` claim genuinely true for the first time,
plus an internal consolidation of duplicate detection logic and the
workspace-wide MSRV bump. `cargo install ts-fix` is unaffected.

## What changed

- **New feature `cli`, on by default.** It gates the `ts-fix` binary and its
  `clap` dependency. The crate root has carried
  `#![cfg_attr(not(feature = "std"), no_std)]` since 0.1.0, but no build
  could actually satisfy it: `clap` was an unconditional dependency, so
  `--no-default-features` still dragged in `anstyle` and the std runtime
  through it. The library itself needed no source change to go `no_std` —
  it used no `std` path — the only thing blocking the claim was the
  binary's dependency leaking into the library build. `ts-fix` is now built
  for `thumbv7em-none-eabi` by CI's `no_std` job, so the claim is now
  CI-verified rather than aspirational. `--no-default-features` now yields
  a library with no `clap` in its tree; `cargo install ts-fix` still builds
  the binary because `cli` is on by default.
- Internal consolidation: `ops::continuity::ContinuityOp`'s legal-duplicate
  detection (ITU-T H.222.0 §2.4.3.3) now delegates to the new shared
  `broadcast_common::ts_dup::is_legal_duplicate_pair`, replacing a
  hand-rolled hash-based check that independently implemented the same
  byte-identity-except-PCR rule already duplicated in `dvb-conformance` and
  `media-doctor`. `PidState` now stores the previous packet's raw bytes
  instead of a hash of them. No behaviour change — confirmed by the
  crate's full test suite, including the exact 5-legal-duplicate /
  0-remaining-error assertions on the `m6-duplicate.ts`/`m6-single.ts`
  fixtures, plus a new regression test pinning the non-PCR byte-identity
  requirement this refactor preserves.
- MSRV raised to **1.95.0** (issue #949), the workspace-wide MSRV
  unification.

## Migration

No API changes; no action required for existing consumers. A consumer
building with `--no-default-features` now gets a library build with no
`clap`/`anstyle` in its dependency tree — previously that flag combination
still pulled them in despite documenting a `no_std`-capable library.
Consumers who want the `ts-fix` binary need the `cli` feature, which is on
by default (so `cargo install ts-fix` needs no flag change). MSRV requires
rustc 1.95.0 or newer.
