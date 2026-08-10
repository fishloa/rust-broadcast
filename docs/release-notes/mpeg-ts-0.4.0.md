# mpeg-ts 0.4.0

**Release date:** 2026-08-10

No functional or API change inside this crate — but the version step itself
matters. Raising a `0.x` crate's minor version is a caret-epoch break under
Cargo's SemVer rules (`^0.3` does not resolve `0.4.0`), and the project's own
MSRV policy (`rust-toolchain.toml`) requires every published crate to take a
minor bump when the MSRV moves. That makes this an epoch-crossing release for
every workspace-sibling that names `mpeg-ts` as a dependency: `dvb-si`,
`dvb-conformance`, and `dvb-tools` (three of the five lockstep crates) all
depend on it directly and have their own `Cargo.toml` requirements moved from
`"0.3"` to `"0.4"` in this same release wave, to stay epoch-pure (issue #858)
rather than leave a published bucket spanning two `mpeg-ts` lines.

## What changed

- MSRV raised to **1.95.0** (issue #949) — the workspace-wide move that
  removes the separate MSRV lane `webrtc-runtime`'s optional `media` feature
  previously required. Only let-chain / `is_multiple_of` style adoption
  where the 1.95 lints require it.
- Added `tests/non_exhaustive_coverage.rs` drift guard (issue #806).
  Test-only; no public API or behaviour change.

## Migration

No code changes are required — `mpeg-ts`'s own public API is unchanged. A
consumer pinned to `mpeg-ts = "0.3"` will not resolve this release and must
bump its requirement to `"0.4"`; `dvb-si`, `dvb-conformance`, and `dvb-tools`
already do this in their own 0.4.0-line releases in this wave.
