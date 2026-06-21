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

## Caption decode (`decode` feature, default-on)

The `decode` feature adds the layer **above** carriage — interpreting the demuxed
caption byte pairs into displayed text:

- **`Cea608Decoder`** — the line-21 control-code state machine (ANSI/CTA-608-E):
  pop-on (RCL/EOC), roll-up (RU2/RU3/RU4 + CR), paint-on (RDC); Preamble Address
  Codes (row + indent + colour/italics/underline); mid-row codes; tab offsets;
  the standard / special / extended Western-European character sets (with the
  automatic backspace on extended chars); the four data channels **CC1–CC4**;
  control-code doubling; and field-2 **XDS** detect-and-skip. Exposes a caption
  screen model (rows × styled cells) and the displayed text per channel.
- **`Cea708Decoder`** — the DTVCC pipeline (ANSI/CTA-708-E + 47 CFR §79.102):
  Caption Channel Packet reassembly → Service Block parsing (incl. the extended-
  service escape) → the C0/C1/G0/G1/G2/G3 command interpreter — the window model
  (DefineWindow DF0–7, SetWindowAttributes, SetCurrentWindow, Clear/Display/Hide/
  Toggle/Delete) and pen model (SetPenAttributes/Color/Location), DLY/DLC/RST.
  Exposes the **six** services' decoded window text.

Both decoders are one-way (bytes → caption state) and panic-free on arbitrary /
truncated / malformed input. Decode is grounded in the spec transcriptions under
`dvb-cc/docs/decode/`.

## Scope

In: the `cc_data()` carriage structure (Table B.9) — extract + demux the caption
triplets — and (with `decode`) the CEA-608/708 character/control **decode** into
caption text. Out: locating `cc_data()` within the picture user_data / SEI
(codec-level, the caller's job); pixel rendering / rasterisation; and the full XDS
metadata catalogue (608 XDS is detected and skipped, not decoded).

## Examples

Run with `cargo run -p dvb-cc --example <name>`:

- **`parse_cc_data`** — parse a `cc_data()` byte sequence; list triplets + 608/708 split.
- **`build_cc_data`** — build a `cc_data()` from typed triplets, serialize, round-trip.
- **`decode_cea608`** — decode a pop-on + roll-up line-21 caption to on-screen text.
- **`decode_cea708`** — decode a DTVCC caption packet (DefineWindow + text) to window text.

## License

MIT OR Apache-2.0.
