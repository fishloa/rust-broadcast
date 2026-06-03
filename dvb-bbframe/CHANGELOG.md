# Changelog

All notable changes to the `dvb_bbframe` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0]

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
