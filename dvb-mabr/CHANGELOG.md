# Changelog

All notable changes to dvb-mabr will be documented in this file.

## [Unreleased]

## [0.1.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.

### Fixed

- Doc accuracy (#940): removed the `serde` feature claim from this file —
  the crate has never had a `serde` feature or dependency, only `default`
  and `std`. Removed the `flute`/`dash` crates.io keywords (`Cargo.toml`),
  since both are explicitly out of scope per the README's "Scope" section.

DVB Multicast ABR (ETSI TS 103 769 V1.2.1) session
configuration XML parser/serializer.

- `MulticastServerConfiguration` and `MulticastGatewayConfiguration` —
  top-level document types (`parse_str` / `serialize`).
- Full structural model: `MulticastSession`, `MulticastTransportSession`,
  `PresentationManifestLocator`, transport parameters, FEC, repair,
  carousel, component, gateway, and reporting types.
- Round-trip test: parse → serialize → reparse.
- `no_std` + `alloc` support.
