# dvb-cc

[![crates.io](https://img.shields.io/crates/v/dvb-cc.svg)](https://crates.io/crates/dvb-cc)
[![docs.rs](https://img.shields.io/docsrs/dvb-cc)](https://docs.rs/dvb-cc)

DVB closed-caption **carriage** — `cc_data()` per **ETSI TS 101 154 §B.5, Table B.9**
(the DVB-native, normative form of the ATSC/CEA caption-carriage structure carried in
MPEG-2 / AVC / HEVC picture `user_data`).

Parses `cc_data()` into typed caption triplets (`cc_valid`, `cc_type`, `cc_data_1/2`)
and splits **CEA-608** (line-21, `cc_type` 0/1) from **CEA-708** (DTVCC, `cc_type` 2/3).
Symmetric `Parse`/`Serialize` with byte-exact round-trip. `no_std` + `alloc`, depends
only on `dvb-common`.

## Scope

In: the `cc_data()` carriage structure (Table B.9) — extract + demux the caption
triplets. Out: locating `cc_data()` within the picture user_data / SEI (codec-level,
the caller's job) and the **meaning** of the `cc_data_1/2` byte pair (the CEA-708-E
character/control decode — a layer above carriage).

## Examples

Run with `cargo run -p dvb-cc --example <name>`:

- **`parse_cc_data`** — parse a `cc_data()` byte sequence; list triplets + 608/708 split.
- **`build_cc_data`** — build a `cc_data()` from typed triplets, serialize, round-trip.

## License

MIT OR Apache-2.0.
