# rmt-flute 0.5.0

**Release date:** 2026-08-10

First feature release since the `dvb-flute` -> `rmt-flute` rename (0.4.0). Fixes
a byte-exact round-trip defect in the LCT `O` (TOI word-count) bound, adds the
RFC 5052 Block Partitioning Algorithm as a new scheme-agnostic building block,
and raises the MSRV. Nothing here changes the rename's compatibility story:
`dvb-flute` remains yanked and unshimmed.

> **Renamed from `dvb-flute` at 0.4.0.** The old name implied a DVB standard
> this crate does not implement, and made it read oddly as a dependency of
> non-DVB consumers such as ATSC 3.0 ROUTE. All `dvb-flute` versions are
> yanked; there is no shim. Same code, accurate name. A consumer still
> depending on `dvb-flute` will fail to resolve new builds and needs to
> switch to `rmt-flute` (see the 0.4.0 release note for the exact diff).

## What changed

- **Fixed:** the LCT `O` (TOI word-count) bound is corrected from `0..=7` to
  `0..=3`. `O` is a two-bit field (RFC 5651 §5.1) masked `& 0x03` on
  serialize, but the validator previously allowed up to 7 — a TOI of 16/20/24/
  28/30 bytes passed validation, then serialized with a truncated `O`, so the
  wire declared a shorter TOI than the bytes actually written and a reparse
  read the surplus as a header-extension chain. This is a byte-exact
  round-trip failure reachable by a well-behaved sender (wide TOIs are
  legitimate in FLUTE). Such headers are now rejected with
  `Error::InvalidField { what: "O" }`. **Behaviour change:** input that
  previously serialized (incorrectly) now errors.
- **Added:** `SourceBlockPartition` — the RFC 5052 §9.1 Block Partitioning
  Algorithm. Given a transport object's Transfer-Length, Encoding-Symbol-Length
  and Maximum-Source-Block-Length, `SourceBlockPartition::new` derives the
  number of source blocks and each block's length in symbols
  (`block_len`, `last_symbol_len`). This is the scheme-agnostic substrate both
  `dvb-mabr` and a future `atsc3-route` need for FEC transport-object/
  super-object construction (issue #944); it operates purely on symbol counts
  and deliberately does not define any FEC Payload ID or Scheme-specific FEC
  OTI byte layout, matching the crate's existing stance on `FecPayloadId128`.
- MSRV raised to **1.95.0** (issue #949), part of a workspace-wide bump that
  removes the MSRV split `webrtc-runtime`'s optional `media` feature used to
  require. No functional or API change on its own.

## Migration

No breaking API change. If your code constructs an `LctHeader`/TOI with a
word-count outside `0..=3`, it will now be rejected at construction/serialize
time with `Error::InvalidField { what: "O" }` instead of silently
mis-encoding — this is a bug fix, not a new restriction on valid FLUTE
headers. Rebuild with rustc >= 1.95.0.

If you still depend on `dvb-flute`, switch to `rmt-flute`; the old crate name
has no live, unyanked versions.
