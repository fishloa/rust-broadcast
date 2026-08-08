# Changelog

## [Unreleased]

## [0.1.0] — 2026-08-08

Initial release.

- `LlsEnvelope` parse/serialize for the binary LLS envelope (A/331 §6.2).
- `LlsTableId` discriminant enum with `name()` + Display.
- `Slt` / `SltService` XML parser for the Service List Table (A/331 §6.3).
- `ServiceCategory`, `SlsProtocol`, `BroadcastSvcSignaling` typed field enums.
- Gzip decompression of LLS payloads (behind `std` feature).
- `#[non_exhaustive]` + `name()` + `impl_spec_display!` on all spec enums.
- `no_std` + `alloc`, optional `serde`.
