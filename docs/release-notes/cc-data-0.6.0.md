# cc-data 0.6.0

_Released 2026-10-05._

Breaking release (0.5 -> 0.6, so a new caret bucket) that fixes several CEA-608 and CEA-708 decoding defects and makes `CcData::parse` strict about the fixed bits of Table B.9. Two public structs gained a field, so anything that builds `CcData` or `Window` with a struct literal stops compiling, and input that `CcData::parse` used to accept with non-conforming reserved bits is now an error. If you only parse and decode, the practical effect is better caption output and a possible new `Err` on malformed streams.

## Breaking changes

### 1. New public fields

- `CcData` gained `reserved_byte1: u8`, the byte after `cc_count` (Table B.9 `reserved`, `em_data` in the ATSC A/53 lineage). It was previously discarded on parse and always re-serialized as `0xFF`, so a real stream with another value did not round-trip byte-exactly (#1107). It is now preserved verbatim.
- `Window` (the CEA-708 decode type) gained `edge_color: Color` (#1107). See the `SetPenColor` fix below.

Neither struct is `#[non_exhaustive]`, so struct-literal construction breaks.

```rust
// before (0.5)
let cc = CcData { process_cc_data_flag: true, triplets };
// after (0.6): 0xFF matches Table B.9's own default
let cc = CcData { process_cc_data_flag: true, reserved_byte1: 0xFF, triplets };
```

Code that only reads these structs, or that builds a `CcData` from `parse`, is unaffected. For `Window`, add `edge_color` to any literal (the decoder itself defaults it to `Color::BLACK`).

### 2. `CcData::parse` rejects reserved-bit deviations

Header byte 0's `reserved`/`zero_bit`, each triplet's `one_bit`/`reserved`, and the trailing marker byte are now validated. A deviation returns the new `Error::InvalidFixedBits { what, got, expected, mask }` (`Error` is `#[non_exhaustive]`, so a wildcard arm already covers the new variant). Input that was previously accepted silently now fails. If you must tolerate sloppy muxers, catch this error at your boundary; the crate offers no lenient mode.

## Fixes

CEA-708:
- **Captions spanning more than one `cc_data()` were destroyed (#1042).** `Cea708Decoder::push_triplets` decoded and cleared the partial Caption Channel Packet buffer at the end of every call. A CCP is typically 32-128 bytes while one frame carries far fewer DTVCC bytes, so most real captions were lost. The buffer is now kept across calls and decoded only when the packet is complete or a new one starts.
- **`Window::text()` split WebVTT cues (#1107).** Interior blank rows were kept as `"\n\n"`, which ends a WebVTT cue at the blank line. Blank rows, interior or trailing, are now dropped.
- **`SetPenColor` overwrote the window border (#1107).** Its edge colour (section 8.10.5.10 parm3) was written to `Window::border_color` on every call. It now goes to the new `Window::edge_color`.

CEA-608 (`Cea608Decoder`):
- **Field-2 miscellaneous control codes were never recognised (#1043).** Only field 1's first bytes `0x14`/`0x1C` matched; field 2's `0x15` (CC3) and `0x1D` (CC4) (CTA-608-E section 8.4 a/b) did not, so RCL/EOC/EDM/RU2-4/CR and the rest of the set were ignored there. They are now recognised.
- **Text went to the wrong channel (#1107).** Standard (untagged) characters were always routed to a hardcoded CC1/CC3, so a CC2 caption's text landed in CC1's memory and interleaved with it. The channel is now tracked per field from the last control code.
- **A caption control code interrupted by XDS was swallowed (#1107).** A control code on field 2 while an XDS sub-packet was open was eaten by the XDS-active check, which ran first; CTA-608 lets a caption code interrupt XDS. The control-code check now runs first.

## Dependencies

`broadcast-common` `9.3` -> `9.4` (see `broadcast-common-9.4.0.md`). No other `Cargo.toml` change. `caption-convert-0.2.0.md` builds on this release.

---

Published from tag `cc-data-v0.6.0`.
