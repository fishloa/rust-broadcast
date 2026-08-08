# Changelog

All notable changes to dvb-mabr will be documented in this file.

## [Unreleased]

### Fixed

- Doc accuracy (#940): removed the `serde` feature claim from this file —
  the crate has never had a `serde` feature or dependency, only `default`
  and `std`. Removed the `flute`/`dash` crates.io keywords (`Cargo.toml`),
  since both are explicitly out of scope per the README's "Scope" section.

## [0.1.0]

Initial release — DVB Multicast ABR (ETSI TS 103 769 V1.2.1) session
configuration XML parser/serializer.

- `MulticastServerConfiguration` and `MulticastGatewayConfiguration` —
  top-level document types (`parse_str` / `serialize`).
- Full structural model: `MulticastSession`, `MulticastTransportSession`,
  `PresentationManifestLocator`, transport parameters, FEC, repair,
  carousel, component, gateway, and reporting types.
- Round-trip test: parse → serialize → reparse.
- `no_std` + `alloc` support.
