# cc-data

[![crates.io](https://img.shields.io/crates/v/cc-data.svg)](https://crates.io/crates/cc-data)
[![docs.rs](https://img.shields.io/docsrs/cc-data)](https://docs.rs/cc-data)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

Closed-caption **`cc_data()` carriage** (CEA-608 / CEA-708) parser and builder, plus
an optional CEA-608 / CEA-708 **caption decoder**. `no_std` + `alloc`, depends only
on `broadcast-common`.

> Formerly `dvb-cc` (through 0.2.0); a `dvb-cc` 0.2.1 shim re-exports this crate.

```toml
[dependencies]
cc-data = "0.7"
```

## What it does / what it does not

`cc_data()` is the structure that carries closed captions inside MPEG-2 / AVC / HEVC
picture `user_data` (ATSC A/53 style, and the DVB-native form in ETSI TS 101 154).
This crate parses it into typed caption triplets (`cc_valid`, `cc_type`,
`cc_data_1`, `cc_data_2`), splits CEA-608 (line 21, `cc_type` 0/1) from CEA-708
(DTVCC, `cc_type` 2/3), and serializes it back byte for byte. With the `decode`
feature it interprets those byte pairs into displayed text.

It does **not** locate `cc_data()` inside a picture's `user_data` / SEI (the caller,
or `transmux`, does that), render pixels, or decode the XDS metadata catalogue
(field-2 XDS is detected and skipped). Format conversion to WebVTT / SRT lives in
`timed-metadata` and `caption-convert`.

## Standards supported

| Standard | Edition | Scope / clauses | Status | Notes |
|---|---|---|---|---|
| ETSI TS 101 154 DVB audio/video coding | V2.6.1 (2019-09), as transcribed in `docs/ts_101_154/b9-cc-data.md`; also present in `private/specs-md` as V2.6.1, V2.7.1 draft and V2.10.1 | §B.5 / Table B.9 `cc_data()` | Full | `process_em_data_flag` / `additional_data_flag` preserved verbatim (real A/53 captures carry `process_em_data_flag = 0`). |
| ANSI/CTA-608-E (line-21 captions) | -E; **not vendored** (copyright, cart-gated; no copy in `private/specs` or `specs/`). `docs/decode/cea608-decode.md` was transcribed in June 2026 from a locally held PDF copy that is no longer retrievable, so its tables cannot be re-verified | pop-on / roll-up / paint-on, PACs, mid-row codes, tab offsets, Western-European character sets, CC1-CC4, doubling, XDS detect-and-skip | Partial (decode) | feature `decode`; XDS content is not decoded |
| ANSI/CTA-708-E (DTVCC) | -E; **not vendored**, same caveat for `docs/decode/cea708-decode.md`. Vendored cross-source: GY/T 270-2013 (`private/specs/nrta_gyt_270_2013_dtv_closed_caption_system.pdf`, a Chinese adaptation: check differences from CTA-708) | CCP reassembly, Service Blocks (extended service), C0/C1/G0/G1/G2/G3, windows (DF0-7, attributes, show/hide/toggle/delete), pens, DLY/DLC/RST; services 1-6 | Partial (decode) | feature `decode` |
| 47 CFR §79.102 (US decoder requirements) | public domain; transcribed in `docs/decode/cea708-conformance.md` | 708 semantic model, window/pen enumerations | Full as a grounding source | |

## Feature flags

| Feature | Default | Description |
|---|---|---|
| `std` | on | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `decode` | on | `Cea608Decoder` / `Cea708Decoder` and the caption screen / window model (additive, `no_std` + `alloc`). |
| `serde` | off | `serde::Serialize` on public types. |

## Quick start

```rust
use broadcast_common::{Parse, Serialize};
use cc_data::{CcData, CcTriplet, CcType};

// Build a cc_data() with one 608 field-1 pair and one 708 start triplet.
let cc = CcData {
    process_em_data_flag: true,
    process_cc_data_flag: true,
    additional_data_flag: false,
    reserved_byte1: 0xFF,
    triplets: vec![
        CcTriplet {
            cc_valid: true,
            cc_type: CcType::Dtvcc708Start,
            cc_data_1: 0xC1,
            cc_data_2: 0x02,
        },
        CcTriplet {
            cc_valid: true,
            cc_type: CcType::Ntsc608Field1,
            cc_data_1: 0x94,
            cc_data_2: 0x2C,
        },
    ],
};

// Serialize, parse back byte-exactly, and split 608 from 708.
let bytes = cc.try_to_bytes().unwrap();
let back = CcData::parse(&bytes).unwrap();
assert_eq!(back, cc);
assert_eq!(back.cea608().count(), 1);
assert_eq!(back.cea708().count(), 1);
```

Decoding (feature `decode`):

```rust
use cc_data::decode::{Cea608Channel, Cea608Decoder};

let mut dec = Cea608Decoder::new();
// RCL, PAC row 15, "HI", EOC: a pop-on caption on CC1 (field 1).
dec.push_pair(false, 0x14, 0x20); // RCL (field 1 -> CC1)
dec.push_pair(false, 0x14, 0x20); // doubled control, ignored
dec.push_pair(false, 0x14, 0x70); // PAC row 15 indent 0
dec.push_pair(false, b'H', b'I');
dec.push_pair(false, 0x14, 0x2F); // EOC -> flip
assert_eq!(dec.channel_text(Cea608Channel::Cc1), "HI");
```

Examples: `parse_cc_data`, `build_cc_data`, `decode_cea608`, `decode_cea708`
(`cargo run -p cc-data --example <name>`).

## What is implemented

- **Carriage**: `CcData`, `CcTriplet`, `CcType` (`Ntsc608Field1`, `Ntsc608Field2`,
  `Dtvcc708Data`, `Dtvcc708Start`), `cea608()` / `cea708()` iterators.
- **`Cea608Decoder`** (`decode`): the line-21 state machine with per-channel
  screen model (rows x styled cells) and displayed text per channel. RCL only
  selects pop-on style (ENM erases), so real SCC files that re-issue RCL before
  EDM + EOC decode; field-2 control codes are recognised; characters route to the
  channel of the last control code on their field.
- **`Cea708Decoder`** (`decode`): a Caption Channel Packet that spans several
  `cc_data()` access units is kept across calls and decoded when complete; exposes
  the six services' window text.
- Both decoders are one-way (bytes to caption state) and panic-free on arbitrary,
  truncated or malformed input (`tests/it/hostility.rs`).

## Design notes

- Zero-copy-style `Parse` / `Serialize` pair; the carriage round trip is
  byte-identical, including the preserved flag bits and `reserved_byte1`.
- `Error` is `#[non_exhaustive]`; odd-parity checking is `broadcast_common::parity`.
- Decode is grounded in the transcriptions under `docs/decode/`; CTA-608-E/708-E
  are not vendored (see the standards table); GY/T 270-2013 is the vendored cross-source for the DTVCC layer.

## Related crates

Depends only on `broadcast-common`. Used by `timed-metadata` (cue extraction),
and `caption-convert`. `dvb-vbi` carries VBI data units (Teletext, Closed Captioning)
separately.

## MSRV and licence

Rust **1.95.0**. MIT OR Apache-2.0.
