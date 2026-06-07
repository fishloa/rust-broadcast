# Changelog

## [Unreleased]

## [3.1.1] — 2026-06-07

Version-lockstep release with the workspace (dvb-si `SectionReassembler`
concatenated-section fix); no changes to this crate.

All notable changes to the `dvb_bbframe` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [3.1.0] — 2026-06-05

Lockstep release with the `dvb-si` 3.x line.

### Breaking
- **serde is Serialize-only.** Every `Deserialize` derive is removed from
  `Bbheader`, `Issy`, and the matype/mode types. JSON is a display/export
  format; re-`parse` from wire bytes to reconstruct. `Serialize` output is
  unchanged.

### Added
- **`yoke` feature (off by default, additive).** `yoke::Yokeable` is derived on
  the public borrowing user-packet iterator view types `NmTsIter<'a>` and
  `HemTsIter<'a>` (both also gain `Clone`/`Copy`), so a parsed view can be
  retained past the input buffer's borrow without a re-parse. (`Bbheader`, the
  crate's actual parse output, is a fully owned `Copy` struct with no lifetime
  and needs no yoking.) Optional; adds no dependencies to default builds.

## [2.1.0] — 2026-06-05

Version-lockstep release with the workspace (Any* `name()` additions in
dvb-si / dvb-t2mi); no functional changes.

## [2.0.0] — 2026-06-05

Version-lockstep release with the workspace (dvb-si 2.0 typed client API);
no functional changes.

## [1.1.0] — 2026-06-04

Version-lockstep release with the workspace (dvb-si: complete EN 300 468
Table 12 descriptor coverage); doc-comment formatting/link cleanup only,
no functional changes.

## [1.0.1] — 2026-06-04

Version-lockstep release with the workspace (dvb-si README overhaul); no code changes.

## [1.0.0] — 2026-06-04

### Fixed

- Removed the fictitious "HEM CRC-8 init = 0xB5" model (`CRC8_INIT_DVB_T2`,
  `crc8_with_init`). EN 302 755 §5.1.7 defines the wire byte as
  `crc8(header) XOR MODE`; 0xB5 was init-0 propagated through exactly nine
  zero bytes and only coincided for 9-byte inputs. Parse now uses the spec
  formula directly; the `Crc8Mismatch` error variant (unreachable by
  construction — the XOR already constrains the byte) was removed. The only
  integrity signal the spec's scheme supports is `InvalidMode`.

## [0.1.0] — internal

Initial release. DVB-S2 / S2X / T2 Base-Band Frame (BBFRAME) header parser and
builder, Normal Mode (NM) and High Efficiency Mode (HEM).

### Added

- **`Bbheader`** — the 10-byte BBHEADER (MATYPE-1/2, UPL, DFL, SYNC, SYNCD,
  CRC-8) with `Parse` + `Serialize` round-trip (CRC-8 recomputed on serialize).
- **`Mode`** (NM/HEM) recovered from `computed_crc8 ^ stored_byte`
  (EN 302 307-1 §5.1.4 / EN 302 755 Annex F).
- **`Matype`** / **`TsGs`** decoding — SIS/MIS, CCM/ACM, ISSYI, NPD, ext, ISI.
- **`packet::up_iter`** — user-packet extraction from the data field honouring
  SYNCD and the UPL stride.
- **`issy`** — ISSY (Input Stream Synchronizer) field parser, EN 302 755 Annex C.
- **`serde` feature** — optional `Serialize`/`Deserialize` derives on the public
  data types (off by default).
- **Error handling** — `Error` enum with `thiserror`. The `DflOutOfRange`
  ceiling (`DFL_MAX_BITS` = 64800) is the DVB-S2 normal-FECFRAME bound
  (EN 302 307-1 §5.1.4); DVB-T2 is tighter (EN 302 755 Table 2).

### Known Limitations

- **Physical layer out of scope** — LDPC/BCH coding and modulation are not
  implemented; this crate handles the BBHEADER and data-field framing only.
- **No `no_std` support** — `thiserror` requires `std`.
