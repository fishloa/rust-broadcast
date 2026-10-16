# Changelog

## [Unreleased]

### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed
- **BREAKING:** `PayloadHeader::extension` is now `Option<HeaderExtension<'a>>`
  (typed TLV view: `HeaderExtension::new`, `items()`, `ExtensionItem::{Pad, Tlv}`,
  `PayloadHeader::extension_items()`), not a raw `&[u8]`; a malformed TLV chain
  still round-trips byte-identically and is reported by the item walk (r13-S22-W1, #1115).
### Fixed
- `PayloadHeader::validate` rejects a `Reserved(code)` variant that holds a
  defined code (it serialized as the defined code and parsed back as the named
  variant, breaking the value round-trip) (r13-S22-W2, #1115).

## [0.1.0] - 2026-10-05
### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Fixed

- Doc accuracy (#940): `Cargo.toml` `description`, the crate-root doc
  comment, and the README no longer claim ST 2022-7 seamless protection
  switching / hitless failover support — this crate implements only the
  ST 2022-6 HBRMT payload header. Aspirational scope moved to a README
  "Planned" section.
- Removed two dead validation branches in `PayloadHeader::validate` (#941
  row 9): the `FrameStructure::Reserved`/`FrameRate::Reserved` range checks
  compared a `u8` against an 8-bit mask (`0xFF`), which can never be true.
- The 0.1.0 entry below claimed a golden-bytes test "with real HBRMT payload".
  It is synthetic, hand-derived from the spec's field tables. Corrected in
  place rather than silently (#926). A real BSD-3-Clause capture has since
  been committed and is tested; see "Publish status" below.

### Publish status — block lifted

The owner lifted the publish block on 2026-10-03, and this is the crate's
first release. The original blocker — no ST 2022-6/HBRMT capture under a
licence compatible with this workspace's MIT OR Apache-2.0 — no longer holds: a
genuine HBRMT/RTP/UDP capture from `cisco/herisson` (`ip2vf`), licensed
**BSD-3-Clause** (licence text verified; see `fixtures/st2022/PROVENANCE.md`),
is committed at
`fixtures/st2022/st2022-6-hbrmt-1080i5994-single-frame-loopback.pcap` (one full
1080i59.94 frame, 4,497 RTP packets) and exercised by
`tests/hbrmt_fixture_pcap.rs`, which parses and byte-exact round-trips every
payload header in it and checks the `RESERVE` field width against real data.
The `release-st2022.yml` block step is removed.

### Added

First release (see "Publish status" above).

- `PayloadHeader` parse/serialize for the 4/8/12+-byte HBRMT payload header
  (SMPTE ST 2022-6:2012 §6.4).
- `VideoSourceFormat` with MAP/FRAME/FRATE/SAMPLE field accessors.
- Typed field enums: `ClockFrequency`, `FecUsage`, `FrameRate`,
  `FrameStructure`, `MapStructure`, `SampleStructure`, `Scrambling`,
  `TimestampRef`, `VideoSourceId`.
- Golden-bytes round-trip test with a **synthetic** HBRMT payload, hand-derived
  from the §6.4 field tables (marker `0xDEADBEEF`). This line previously said
  "real HBRMT payload", which was untrue — see the `[Unreleased]` correction.
  A real-capture test (`tests/hbrmt_fixture_pcap.rs`) has since been added.
- `#[non_exhaustive]` + `name()` + `impl_spec_display!` on all spec enums.
- `no_std` + `alloc`, optional `serde`.
