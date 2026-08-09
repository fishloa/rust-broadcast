# Changelog

## [Unreleased]

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Added

- Real ATSC 3.0 LLS/SLT fixture (#926/#943):
  `fixtures/atsc3/slt-lls-2019-01-07.bin`, a genuine `LLS_table()` envelope
  (real gzip payload decompressing to a real SLT XML document) extracted
  from `junhuac/libatsc3`'s test suite — see `fixtures/atsc3/PROVENANCE.md`
  for the full verification. Added `tests/fixture_slt.rs` exercising the
  full envelope-parse -> gunzip -> SLT-parse pipeline against it, and
  replaced the pre-existing `lls.rs` inline unit tests' `b"payload-bytes"`
  (neither valid gzip nor XML) with a real gzip-compressed spec-valid SLT
  XML payload, so the crate's decompress path is now exercised by every
  test that touches the payload field, not skipped.

### Fixed

- Doc accuracy (#940): `Cargo.toml` `description`/`keywords`, the crate-root
  doc comment, and the README no longer claim A/321 bootstrap, A/331
  ROUTE/DASH, or MMT support — only the A/331 §6.2 LLS envelope and §6.3 SLT
  are implemented. Aspirational scope moved to a README "Planned" section.

## [0.1.0] — 2026-08-08

Initial release.

- `LlsEnvelope` parse/serialize for the binary LLS envelope (A/331 §6.2).
- `LlsTableId` discriminant enum with `name()` + Display.
- `Slt` / `SltService` XML parser for the Service List Table (A/331 §6.3).
- `ServiceCategory`, `SlsProtocol`, `BroadcastSvcSignaling` typed field enums.
- Gzip decompression of LLS payloads (behind `std` feature).
- `#[non_exhaustive]` + `name()` + `impl_spec_display!` on all spec enums.
- `no_std` + `alloc`, optional `serde`.
