# cc-data 0.6.0

_Released 2026-10-05._

### Fixed
- **#1042**: `Cea708Decoder::push_triplets` unconditionally decoded and
  cleared the partial Caption Channel Packet buffer at the end of every
  call, destroying any CCP spanning more than one `cc_data()` access unit
  (CEA-708 §4/§5) — the common case, since a CCP is typically 32-128 bytes
  and one frame carries far fewer DTVCC bytes. The buffer is now kept
  across calls and only decoded once it is complete or a new packet starts.
- **#1043**: CEA-608 field-2 Miscellaneous Control Code pairs (first byte
  `0x15`/CC3, `0x1D`/CC4 — CTA-608-E §8.4 a/b) were never recognised; the
  fold only matched field-1's `0x14`/`0x1C`. RCL/EOC/EDM/RU2-4/CR and the
  rest of the misc-control set are now recognised on field 2.
- **#1107**: CEA-608 standard (untagged) characters always routed to a
  hardcoded CC1/CC3, instead of the channel targeted by the field's last
  control code — a CC2 caption's text was written into CC1's memory and
  interleaved with it. Now tracked and routed per field.
- **#1107**: a caption control code sent on field 2 while an XDS sub-packet
  was open was silently swallowed by the XDS-active check, which ran
  before the control-code check; CTA-608 allows a caption code to interrupt
  XDS. The control-code check now runs first.
- **#1107**: `Cea708Decoder`'s `Window::text()` kept interior blank rows as
  `"\n\n"`, which truncated the downstream WebVTT cue at the blank line.
  Blank rows (interior or trailing) are now dropped instead of emitted.
- **#1107**: `SetPenColor`'s edge colour (§8.10.5.10 parm3) was written
  into `Window::border_color`, clobbering the window border on every
  `SetPenColor`. It now has its own `Window::edge_color` field.
- **#1107**: `CcData::parse` didn't validate Table B.9's fixed bits
  (`reserved`/`zero_bit`/`one_bit`/`marker_bits`), and the reserved byte
  after `cc_count` (`em_data` in the shared ATSC A/53 lineage) was
  discarded and always re-serialized as `0xFF`, so a real stream whose
  value there wasn't `0xFF` didn't round-trip byte-exact. The fixed bits
  are now validated on parse (`Error::InvalidFixedBits`), and the reserved
  byte is preserved verbatim as `CcData::reserved_byte1`.

### Changed (breaking)
- **#1107**: `CcData` gained a new field, `reserved_byte1: u8` (see above).
  `Window` (cc-data's CEA-708 decode types) gained a new field,
  `edge_color: Color` (see above). Both are additive struct-literal breaks.
- `CcData::parse` now hard-rejects reserved-bit deviations (header byte 0
  `reserved`/`zero_bit`, each triplet's `one_bit`/`reserved`, the trailing
  marker byte) with `Error::InvalidFixedBits`; input it previously accepted
  silently now fails — a behaviour tightening.

---

Published from tag `cc-data-v0.6.0`.
