## Table 2-2 — Transport Stream packet
_§2.4.3.2–2.4.3.3, PDF pp. 31-33_

Transport Stream packets shall be 188 bytes long (§2.4.3).

| Syntax | Bits | Mnemonic |
|---|---|---|
| transport_packet() { |  |  |
| sync_byte | 8 | bslbf |
| transport_error_indicator | 1 | bslbf |
| payload_unit_start_indicator | 1 | bslbf |
| transport_priority | 1 | bslbf |
| PID | 13 | uimsbf |
| transport_scrambling_control | 2 | bslbf |
| adaptation_field_control | 2 | bslbf |
| continuity_counter | 4 | uimsbf |
| if(adaptation_field_control=='10' \|\| adaptation_field_control=='11'){ |  |  |
| adaptation_field() |  |  |
| } |  |  |
| if(adaptation_field_control=='01' \|\| adaptation_field_control=='11') { |  |  |
| for (i = 0; i < N; i++){ |  |  |
| data_byte | 8 | bslbf |
| } |  |  |
| } |  |  |
| } |  |  |

Cross-check notes (§2.4.3.3):

- `sync_byte` is fixed `'0100 0111'` (0x47).
- `payload_unit_start_indicator` for PSI: `'1'` iff the packet carries the
  first byte of a PSI section, in which case the first payload byte is the
  pointer_field; `'0'` means no pointer_field is present. For null packets it
  shall be `'0'`. (Full PSI rules under
  [pointer_field](#psi-section-carriage-and-pointer_field) below.)
- `adaptation_field_control` (Table 2-5): `'00'` reserved (decoders shall
  discard such packets), `'01'` payload only, `'10'` adaptation_field only,
  `'11'` adaptation_field followed by payload. Null packets shall use `'01'`.
- `continuity_counter` increments per packet of the same PID, wraps to 0; it
  shall **not** be incremented when adaptation_field_control is `'00'` or
  `'10'`. Duplicate packets: at most two consecutive packets of the same PID
  with the same continuity_counter, adaptation_field_control `'01'` or `'11'`,
  byte-identical except that a PCR, if present, shall carry a valid value.
- `data_byte` count N = 184 minus the bytes of the adaptation_field().
- PID 0x0000 = PAT, 0x0001 = CAT, 0x0002 = TSDT, 0x0003 = IPMP control
  information, 0x0004–0x000F reserved, 0x1FFF = null packets (Table 2-3).
  Packets with PID 0x0000, 0x0001 and 0x0010–0x1FFE are allowed to carry a
  PCR (Table 2-3 NOTE).

