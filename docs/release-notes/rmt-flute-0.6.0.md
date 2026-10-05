# rmt-flute 0.6.0

_Released 2026-10-05._

Breaking (0.x minor) release, driven by the run-09 audit and issues #1070 and #1122. Three groups of source-level breaks: two public structs gained fields (`ExtCenc`, `ExtTime`, so struct literals stop compiling), `ExtTime` is no longer `Copy`, and `NormCmd` was reshaped (new `parse` parameter, typed `body` in place of `head`). Two behaviour changes follow: `ExtTime::parse` now accepts input it used to reject, and the NORM message parsers now reject input they used to accept. Who must act: anyone constructing `ExtCenc` or `ExtTime` by literal, anyone calling `NormCmd::parse` or reading `NormCmd.head`/`content`, and anyone feeding NORM parsers a mix of message types. The previous published version is 0.5.0. The only dependency change is `broadcast-common` 9.3 to 9.4.

## Breaking changes

### `ExtCenc` and `ExtTime` gained fields (RMT-W1)

These exist so that input carrying non-zero reserved bits, or extra trailing bytes, now round-trips byte-exactly instead of being zeroed or truncated.

- `ExtCenc` (EXT_CENC, RFC 6726) gained `reserved: u16`: the 16 reserved bits, preserved verbatim. `to_content` used to always write them as 0.
- `ExtTime` (EXT_TIME, RFC 5651) gained `use_reserved: u16` (the reserved-by-LCT Use bits `Use & 0x0F00`, kept in their original bit position) and `trailing: Vec<u8>` (any content bytes after the last time value the Use field selects). `serialized_len`, `use_field` and `to_content` account for both.
- Because `trailing` is a `Vec`, `ExtTime` no longer derives `Copy`. It still derives `Clone`, `Debug`, `PartialEq`, `Eq` and `Default`.
- With the `serde` feature, the derived `Serialize` output of both structs has the new fields.

```rust
// 0.5.0
let c = ExtCenc { algorithm: CencAlgorithm::Gzip };
let t = ExtTime { ert: Some(42), pi_specific: 0, sct_high: None, sct_low: None, slc: None }; // Copy let you reuse `t`

// 0.6.0
let c = ExtCenc { algorithm: CencAlgorithm::Gzip, reserved: 0 };
let t = ExtTime { ert: Some(42), ..Default::default() };    // set use_reserved/trailing only if you need them
let t2 = t.clone();                                         // no longer Copy
```

### `NormCmd` reshaped (#1070)

- `NormCmd::parse` now takes `fec_payload_id_len: usize`, the FEC-scheme-defined size of `fec_payload_id`. It is used only for FLUSH and SQUELCH and ignored for the other sub-types, matching `NormData::parse`.
- The untyped `head: [u8; 3]` field is replaced by `body: NormCmdBody<'a>`. `NormCmdBody` (new, `#[non_exhaustive]`, re-exported at the crate root) has one variant per sub-type: `Flush { fec_id, object_transport_id, fec_payload_id }`, `Eot`, `Squelch { fec_id, object_transport_id, fec_payload_id }`, `Cc { cc_sequence, send_time_sec, send_time_usec }`, `RepairAdv { flags }`, `AckReq { ack_type, ack_id }`, `Application`, and `Other([u8; 3])` for an unrecognised sub-type.
- `NormCmd.sub_type` is kept alongside `body`, so an unrecognised wire value still round-trips.
- `NormCmd.content` now holds only the genuinely optional trailing content (node lists, application content). FLUSH/SQUELCH `fec_payload_id` and CC's `send_time_sec`/`send_time_usec` moved out of `content` and into `body`.

```rust
// 0.5.0
let cmd = NormCmd::parse(bytes)?;
let first_three = cmd.head;          // untyped
// 0.6.0
let cmd = NormCmd::parse(bytes, fec_payload_id_len)?;
match &cmd.body {
    NormCmdBody::Cc { cc_sequence, send_time_sec, send_time_usec } => { /* ... */ }
    NormCmdBody::Flush { fec_payload_id, .. } => { /* ... */ }
    _ => {}
}
```

When building a `NormCmd` by hand, put FLUSH/SQUELCH `fec_payload_id` and CC send time into `body`, not `content`.

## Behaviour changes

- **`ExtTime::parse` accepts non-zero reserved-by-LCT Use bits.** RFC 5651 makes them sender-MUST-zero and receiver-MUST-ignore, not receiver-MUST-reject, so rejecting them failed on input RFC 5651 promises stays parseable. They are preserved in `use_reserved`. It also keeps trailing content bytes in `trailing` instead of dropping them.
- **NORM parsers check the message type.** `NormInfo::parse`, `NormData::parse`, `NormCmd::parse` and `NormFeedback::parse` now return `Error::InvalidField` if `common.message_type` is not the type each parses (`NormFeedback` accepts only the feedback types), where they previously produced a plausible-looking struct with garbage fields from the wrong wire layout (#1122). If you dispatch on `NormCommonHeader::parse` first and call the matching parser, nothing changes.
- **`NormFeedback::serialize_into` rejects a non-word-aligned header** (`hdr_len` not a whole number of 32-bit words) instead of silently truncating it, as `NormInfo`, `NormData` and `NormCmd` already did. All four share one `hdr_len_words` helper (#1122).

## Fixes

- **NORM_CMD FLUSH, SQUELCH and CC parsed their fixed body as header extensions (#1070).** RFC 5740 §4.2.3 (Figures 10, 12, 13) puts `fec_payload_id` (FLUSH/SQUELCH) and `send_time_sec`/`send_time_usec` (CC) ahead of the optional extension chain, and a conformant sender's `hdr_len` includes them. Real probes in that spec-conformant shape were rejected outright because the extension parser rejects a header-extension length of 0; and a value built with those bytes in `content` wrote a short `hdr_len` on serialize. Both are fixed by the typed `body`.
- The reserved-bit and trailing-byte round-trip losses described above (RMT-W1).

## Dependencies

Verified from the `Cargo.toml` diff against `rmt-flute-v0.5.0`: only `broadcast-common = { version = "9.3" -> "9.4", default-features = false }`. No feature changes.

## Read together with

[atsc3-route-0.2.0.md](atsc3-route-0.2.0.md), which is built on this crate's LCT/ALC/FLUTE types.

---

Published from tag `rmt-flute-v0.6.0`.
