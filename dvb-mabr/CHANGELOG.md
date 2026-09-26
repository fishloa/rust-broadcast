# Changelog

All notable changes to dvb-mabr will be documented in this file.

## [Unreleased]

### Fixed
- `child`/`children` (used by every element parser) now match `(namespace,
  local name)` instead of local name alone, so a private/implementation
  extension element (Annex A.1) in a foreign namespace that reuses a
  baseline local name is correctly skipped, matching this crate's own
  documented behaviour (#1121). The document root's namespace is now checked
  too, not just its local name.
- `to_xml` now emits the namespace matching `schema_version` (`1` -> the
  2019 namespace, everything else -> 2024) instead of always emitting 2024,
  which previously produced a contradictory `xmlns="...:2024"` with
  `schemaVersion="1"` when re-serializing a parsed v1 document (#1121).
- `ReportingLocator::proportion` now rejects a non-finite value (`NaN`/
  `inf`/`infinity` all parse as `f64` but are not valid `xs:decimal`/
  `xs:double` lexical forms, and `NaN != NaN` broke the documented
  parse -> to_xml -> parse round trip) and a value outside its documented
  `(0.0, 1.0]` range (#1121).
- Element text (`own_text`, used for every URI/string leaf element) is no
  longer truncated at the first comment or CDATA boundary — it now
  concatenates every text-node child instead of using `roxmltree`'s
  `Node::text()`, which returns only the first one (#1121).

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
