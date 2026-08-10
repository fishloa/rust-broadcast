# mpeg-pes 0.4.0

**Release date:** 2026-08-10

A minor breaking change to `TrickMode`, plus the workspace-wide MSRV bump.
A consumer that matches on `TrickMode` exhaustively needs a wildcard arm;
everyone else is unaffected.

## What changed

- MSRV raised to **1.95.0** (issue #949) — the workspace-wide move that
  removes the separate MSRV lane `webrtc-runtime`'s optional `media` feature
  previously required. Only let-chain / `is_multiple_of` style adoption
  where the 1.95 lints require it.
- Added `tests/label_coverage.rs` and `tests/non_exhaustive_coverage.rs`
  drift guards (issue #806).

## Breaking changes

- `packet::TrickMode` is now `#[non_exhaustive]` (issue #806's
  non-exhaustive drift-guard audit — every other public enum in the
  workspace already carried it).

  ```rust
  // Before (0.3.x): exhaustive match compiled.
  match trick_mode {
      TrickMode::FastForward { .. } => ..,
      TrickMode::SlowMotion { .. } => ..,
      TrickMode::FreezeFrame { .. } => ..,
      TrickMode::FastReverse { .. } => ..,
      TrickMode::SlowReverse { .. } => ..,
      TrickMode::Reserved { .. } => ..,
  }

  // After (0.4.0): needs a wildcard arm.
  match trick_mode {
      TrickMode::FastForward { .. } => ..,
      TrickMode::SlowMotion { .. } => ..,
      TrickMode::FreezeFrame { .. } => ..,
      TrickMode::FastReverse { .. } => ..,
      TrickMode::SlowReverse { .. } => ..,
      TrickMode::Reserved { .. } => ..,
      _ => ..,
  }
  ```

## Migration

Add a wildcard (`_ => ..`) arm to any exhaustive `match` over
`packet::TrickMode`. No other API changes.
