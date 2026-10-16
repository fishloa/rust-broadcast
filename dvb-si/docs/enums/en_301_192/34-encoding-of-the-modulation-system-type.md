## Table 34 — Encoding of the modulation_system_type
_§8.4.5.15, PDF pp. 38-38 (ETSI EN 301 192 V1.7.1; verified against the rendered page)_

| modulation_system_type | Description |
|---|---|
| 0x00 | DVB-S2, see ETSI EN 302 307-1 [22] |
| 0x01 | DVB-T2, see ETSI EN 302 755 [23] |
| 0x02 | DVB-C2, see ETSI EN 302 769 [24] |
| 0x03 | DVB-NGH, see DVB BlueBook A160 [25] |
| 0x04 | DVB-S2X, see ETSI EN 302 307-2 [26] |
| 0x05 to 0xFF | reserved for future use |

## Table 35 — PHY_stream_id encoding
_§8.4.5.15, PDF pp. 39-39_

| modulation_system_type | PHY_stream_id encoding |
|---|---|
| 0x00 DVB-S2 | The Input Stream Identifier (ISI) shall be encoded as a 16-bit uimsbf (see note). |
| 0x01 DVB-T2 | The Physical Layer Pipe (PLP) identifier shall be encoded as a 16-bit uimsbf. |
| 0x02 DVB-C2 | The data slice identifier shall be encoded in the bits b15 through b8 as an 8-bit uimsbf, and the Physical Layer Pipe (PLP) identifier shall be encoded in the bits b7 through b0 as an 8-bit uimsbf. |
| 0x03 DVB-NGH | The Physical Layer Pipe (PLP) identifier shall be encoded as a 16-bit uimsbf. |
| 0x04 DVB-S2X | The time slice number shall be encoded in the bits b15 through b8 as an 8-bit uimsbf, and the Input Stream Identifier (ISI) shall be encoded in the bits b7 through b0 as an 8-bit uimsbf (see note). |

NOTE: The unconditional presence of this field implies that DVB-S2 or DVB-S2X is operated in multiple input stream mode. If only a single input stream is intended to be used, DVB-S2 or DVB-S2X is still operated in multiple input stream mode, but only a single input stream identifier value is used.

`selector_length_flag`, `selector_flags`, `selector_length`, `selector_byte` "shall be encoded according to ETSI TS 102 606-2 [27], clause 5.2.2.8" (ETSI TS 102 606-2 is not vendored, so there is no spec text here to transcribe; the selector content is opaque to this crate).
