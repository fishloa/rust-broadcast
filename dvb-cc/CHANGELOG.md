# Changelog

All notable changes to `dvb-cc` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.1.0 — 2026-06-20

### Added
- Initial release. DVB closed-caption carriage `cc_data()` per ETSI TS 101 154
  §B.5, Table B.9:
  - `CcData` — `process_cc_data_flag` + the caption triplet loop, with byte-exact
    symmetric `Parse`/`Serialize` (computed `cc_count`, reserved/marker bits, 5-bit
    `cc_count` overflow guard).
  - `CcTriplet` / `CcType` — `cc_valid`, `cc_type` (CEA-608 field 1/2, CEA-708
    DTVCC data/start), `cc_data_1/2`.
  - `cea608()` / `cea708()` triplet split by `cc_type`.
- Two runnable examples (`parse_cc_data`, `build_cc_data`); `no_std` + `alloc`.
