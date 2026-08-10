# multimux-cli 0.7.0

**Release date:** 2026-08-10

MSRV-only release for this crate itself; it now depends on `multimux` 0.9,
which is the substantial release in this wave (DVR catch-up serving, WHIP
ingest, WHEP egress, and two source-breaking signature changes — see
`multimux-0.9.0.md`). No functional or API change in `multimux-cli` itself.

## What changed

- MSRV raised to **1.95.0** (issue #949) — the workspace-wide move that
  removes the separate MSRV lane `webrtc-runtime`'s optional `media` feature
  previously required.
- Depends on `multimux` 0.9 (path dependency bump alongside the rest of the
  wave).

## Migration

No API changes in this crate; no action required beyond building with rustc
>= 1.95.0. If you consume `multimux` directly rather than only through this
CLI binary, see `multimux-0.9.0.md`'s Breaking changes section.
