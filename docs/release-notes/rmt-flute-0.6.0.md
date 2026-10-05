# rmt-flute 0.6.0

_Released 2026-10-05._

### Changed (breaking)
- `ExtCenc` gained a `reserved: u16` field (the 16 reserved bits, preserved
  verbatim instead of always re-encoded as 0). `ExtTime` gained
  `use_reserved: u16` (the reserved-by-LCT Use bits, preserved instead of
  rejected) and `trailing: Vec<u8>` (any content bytes past the last
  present time value, preserved instead of dropped); it also no longer
  derives `Copy` (`trailing` is a `Vec`). All are needed for a byte-exact
  round-trip on input carrying either (audit run-09 RMT-W1).
- `ExtTime::parse` no longer rejects a non-zero reserved-by-LCT Use bit as
  an error: RFC 5651 makes it sender-MUST-zero / receiver-MUST-ignore, not
  receiver-MUST-reject (audit run-09 RMT-W1).
- `NormCmd::parse` now takes a `fec_payload_id_len: usize` parameter (the
  FEC-scheme-defined size of `fec_payload_id`, unused outside FLUSH/SQUELCH),
  matching `NormData::parse`'s existing convention. `NormCmd`'s `head`/
  `content`-only representation is replaced by a typed `body: NormCmdBody<'a>`
  field, one variant per sub-type (#1070).

### Fixed
- `NormCmd` (NORM_CMD, RFC 5740 §4.2.3) parsed FLUSH/SQUELCH's `fec_payload_id`
  and CC's fixed `send_time_sec`/`send_time_usec` as if they were
  header-extension bytes, instead of the sub-type's own fixed body ahead of
  the (genuinely optional) extension chain. A conformant sender's `hdr_len`
  includes that fixed body ("hdr_len (no ext) = 4 + size of fec_payload_id"
  for FLUSH/SQUELCH, "= 6" for CC — Figures 10/12/13), so real probes in that
  exact, spec-conformant shape were rejected outright (`HeaderExtension`
  parsing rejects `HEL=0`), and a value constructed with those bytes in
  `content` instead of the fixed body wrote a short `hdr_len` on serialize
  (#1070).
- `NormInfo`/`NormData`/`NormCmd`/`NormFeedback::parse` now check
  `common.message_type` against the type each parses, instead of accepting
  any message type and producing a "valid"-looking struct with garbage
  fields from the wrong wire layout (#1122).
- `NormFeedback::serialize_into` now rejects a non-word-aligned header
  (`hdr_len` not a whole number of 32-bit words) instead of silently
  truncating it, matching the check `NormInfo`/`NormData`/`NormCmd` already
  had; all four now share one `hdr_len_words` helper (#1122).

---

Published from tag `rmt-flute-v0.6.0`.
