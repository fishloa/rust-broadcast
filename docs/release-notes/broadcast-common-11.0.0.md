
# broadcast-common 11.0.0

_Released 2026-11-07._

**Breaking release.** The deprecated `Timestamp::checked_add_nanos` is removed, and the crate gains the building blocks the 2026-11 wave moved out of some forty crates: the shared `error` kinds, `timescale`, `len` pre-checks, `xml_chars` plus the `xml` feature, and `declare_object_trait!`. Because almost every crate in the workspace depends on `broadcast-common`, its caret epoch moving 10 -> 11 is why the other crates in this wave release too; `broadcast-common-v11.0.0` must be live on crates.io before any other tag is pushed.

Version: 10.0.0 -> 11.0.0 (major).

## Breaking changes and migration

What the CHANGELOG records as breaking:

- `Timestamp::checked_add_nanos` (deprecated since 9.5.0): use `saturating_add_nanos`.

### Renamed and removed items

```text
ts.checked_add_nanos(n)
    -> ts.saturating_add_nanos(n)
```

### Why this version

Removing the deprecated `Timestamp::checked_add_nanos` (it always saturated and never checked) is a public API removal, so the epoch moves 10 -> 11. The additive modules would alone have been 10.1.0.

### New shared building blocks

- `error`: `BufferTooShort`, `OutputBufferTooSmall`, `FieldOverflow` (moved from `len`, which re-exports them, so existing paths still work), plus new `CrcMismatch`, `InvalidSync`, `ReservedBits`, `InvalidValue`, `InvalidField`, `UnsupportedVersion` and `LengthMismatch`. A crate's own error enum wraps one with `#[error(transparent)] Kind(#[from] broadcast_common::error::Kind)`.
- `len::{need_in, need_out}`: the shared parse-side and serialize-side length pre-checks.
- `timescale`: `rescale`, `rescale_i64`, `rescale_u64`, `to_micros`, `from_micros`, `cmp_ticks` with an explicit `Rounding` (`Floor`, `Nearest`, `TowardZero`, `Ceil`) and one zero-timescale rule (0 counts as 1).
- `xml` feature and `xml_chars`: XML 1.0 character-data validation, bounded-input limits (`XmlLimits`, `XmlBudget`) and the attribute and event helpers shared by the `quick-xml` parsers.
- `erased::declare_object_trait!` and `Demand::saturated_if`.

## Added

- `xml_chars::{XmlLimits, XmlLimit, LimitExceeded, XmlBudget}` and the `MAX_XML_*` defaults: the shared hostile-input bounds (size, depth, element count, attribute size and count) for the `quick-xml` pull parsers; `dash-runtime` and `transmux` use the same ceilings (#1708).
- `error` module: the common failure kinds defined once as `Copy` structs that a crate's own error enum wraps with `#[error(transparent)] Kind(#[from] broadcast_common::error::Kind)`: `BufferTooShort`, `OutputBufferTooSmall`, `FieldOverflow` (moved from `len`, which re-exports them, so no path changes), plus new `CrcMismatch { what, computed, expected }`, `InvalidSync { what, expected, found }`, `ReservedBits { field, expected, found }`, `InvalidValue { field, value, reason }`, `InvalidField { field, reason }`, `UnsupportedVersion { what, found }`, `LengthMismatch { what, declared, actual }` (shared-errors E0).
- `timescale::{MICROS_PER_SECOND, to_micros, from_micros}`: tick/microsecond conversion with an explicit `Rounding`, replacing the private `ticks_to_us` / `us_to_ticks_ceil` wrappers in `broadcast-client`, `multimux`, `dash-runtime`.
- `len::{BufferTooShort, OutputBufferTooSmall, need_in, need_out}`: the shared parse-side and serialize-side length pre-checks, returning structs identical to the `BufferTooShort { need, have, what }` / `OutputBufferTooSmall { need, have }` error variants so each crate maps them with one `From` impl.
- `timescale::Rounding::Ceil` and `timescale::cmp_ticks` (exact cross-timescale comparison, 0 counts as 1) for the call sites that still rescaled or compared ticks by hand.
- `xml_chars::{TagAttribute, read_attributes, read_attributes_checked, read_event, message_at}`: the start-tag attribute loop (xmlns skipping, normalization, `Char` validation, namespace resolution) and the event-read / error-position helpers that `dvb-mabr`, `ttml-subtitle` and `dash-runtime` each carried a copy of.
- `timescale` module: `rescale`, `rescale_i64`, `rescale_u64` with an explicit `Rounding` (`Floor`, `Nearest`, `TowardZero`), saturating arithmetic and one zero-timescale rule (0 counts as 1); replaces about fifteen divergent private copies in `transmux`, `multimux`, `dash-runtime` and `broadcast-client`.
- `erased::declare_object_trait!`: the type-erased registry object trait, blanket impl, `downcast_ref`/`is` helpers and `serialize_erased` shared by the `dvb-si` and `dvb-t2mi` registries; the `serde` feature now owns the `erased-serde` dependency.
- `xml` feature (implies `std`, pulls `quick-xml`) and `xml_chars` module: `is_xml_char`, `char_data`, `namespace_of`, `BAD_CHAR` - the XML 1.0 character-data core previously copied into four crates.
- `Demand::saturated_if(bool)`: `Demand::saturated()` when full, else the default.

## Other changes

- `Stage::next_deadline`, `Stage::on_deadline` and `Stage::demand` have default bodies (no deadline, nothing to do, `Demand::default()`), so a stage that only reacts to `feed`/`finish` omits them; the trivial overrides in `transmux` are gone.

## XML writer helpers (#1716)

- `xml_chars::{XmlWriter, new_writer, write_declaration, start_tag, open_element, empty_element, close_element, write_text, text_element, finish_document, NO_ATTRS}`: one two-space-indenting quick-xml writer shared by the transmux, multimux and dvb-mabr manifest writers. Every attribute value and text node is escaped by quick-xml.

## Dependencies

No workspace sibling requirement changes epoch.

---

Published from tag `broadcast-common-v11.0.0`.
