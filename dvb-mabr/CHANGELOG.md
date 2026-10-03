# Changelog

All notable changes to dvb-mabr will be documented in this file.

## [Unreleased]

### Changed (breaking)
- **BREAKING: XML support now requires the `std` feature; hand-rolled XML
  replaced by `quick-xml`; `roxmltree` dropped.** The whole XML API
  (`MulticastServerConfiguration`/`MulticastGatewayConfiguration` and every
  element type) is gated behind `std`; a `--no-default-features` build exposes
  only `error`. Parsing is a single pass over `quick_xml::NsReader` events straight
  into the typed structs (no document tree, so hostile nesting cannot overflow
  the stack); serialization writes `BytesStart`/`BytesText` events through
  `quick_xml::Writer`, so every attribute value and text node is escaped by
  quick-xml (an
  attribute value containing `\t`/`\n`/`\r` is now written as a character
  reference so it survives a re-parse). Parse results and serialized output are
  identical to the previous release for every committed fixture. Malformed
  input stays a structured `Error::XmlParse` (a `DOCTYPE`, undefined entity,
  unbound namespace prefix, second root element, or content after the root is
  rejected).
- **Behaviour differences from the roxmltree-based parser (all follow XML 1.0):**
  - a numeric reference to a control character (`&#x1;`), or a literal one, in text or in an
    attribute is rejected with `Error::XmlParse` (it is outside the XML 1.0 `Char` production);
  - an entity reference or CDATA section before the root element is rejected
    (`&amp;<MulticastGatewayConfiguration ...>` is `Error::XmlParse`);
  - a document with more than 128 namespace declarations in scope at once is rejected with
    `Error::XmlParse` (quick-xml's namespace-binding limit);
  - an attribute value containing `\t`, `\n` or `\r` is written as `&#9;`, `&#10;`, `&#13;`
    (`serviceIdentifier="a\nb"` renders `serviceIdentifier="a&#10;b"`) so it survives a re-parse.
- **Behaviour change (#1121)**: documents that parsed before are now rejected.
  The document root's namespace is checked (not just its local name): a
  `MulticastServerConfiguration`/`MulticastGatewayConfiguration` in any
  namespace other than the 2019/2024 baseline now fails with
  `Error::UnexpectedRoot`.
- **Behaviour change (#1121)**: `ReportingLocator::proportion` now rejects a
  non-finite value (`NaN`/`inf`/`infinity` all parse as `f64` but are not valid
  `xs:decimal`/`xs:double` lexical forms, and `NaN != NaN` broke the documented
  parse -> to_xml -> parse round trip) and a value outside its documented
  `(0.0, 1.0]` range.
- `MulticastServerConfiguration` and `MulticastGatewayConfiguration` gain a
  `namespace: BaselineNamespace` field (both structs are `#[non_exhaustive]`).

### Added
- `BaselineNamespace` (`V2019`/`V2024`, with `uri()`/`name()`/`Display`): the
  baseline namespace a root declared. `to_xml` re-emits it as parsed, so a
  `schemaVersion="2"` document that declares the 2019 namespace is no longer
  rewritten to 2024 (the round trip is byte-stable).

### Fixed
- `child`/`children` (used by every element parser) now match `(namespace,
  local name)` instead of local name alone, so a private/implementation
  extension element (Annex A.1) in a foreign namespace that reuses a
  baseline local name is correctly skipped, matching this crate's own
  documented behaviour (#1121).
- `to_xml` now emits the namespace the document declared (2019 or 2024)
  instead of always emitting 2024, which previously produced a contradictory
  `xmlns="...:2024"` with `schemaVersion="1"` when re-serializing a parsed v1
  document (#1121).
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
