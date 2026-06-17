## Table 35 — PHY_stream_id encoding
_§8.4.5.15, PDF pp. 39-39_

| modulation_system_type | Modulation system | PHY_strem_id encoding |
|---|---|---|
| 0x00 | DVB-S2, see ETSI EN 302 307-1 [22] | The Input Stream Identifier (ISI) shall be encoded as a 16-bit uimsbf (see note). |
| 0x01 | DVB-T2, see ETSI EN 302 755 [23] | The Physical Layer Pipe (PLP) identifier shall be encoded as a 16-bit uimsbf. |
| 0x02 | DVB-C2, see ETSI EN 302 769 [24] | The data slice identifier shall be encoded in the bits b15 through b8 as a 8-bit uimsbf, and the Physical Layer Pipe (PLP) identifier shall be encoded in the bits b7 through b0 as an 8-bit uimsbf. |
| 0x03 | DVB-NGH, see DVB BlueBook A160 [25] | The Physical Layer Pipe (PLP) identifier shall be encoded as a 16-bit uimsbf. |
| 0x04 | DVB-S2X, see ETSI EN 302 307-2 [26] | The time slice number shall be encoded in the bits b15 through b8 as a 8-bit uimsbf, and the Input Stream Identifier (ISI) shall be encoded in the bits b7 through b0 as an 8-bit uimsbf (see note). |

> **NOTE:** The unconditional presence of this field implies that DVB-S2 or DVB-S2X is operated in multiple input stream mode. If only a single input stream is intended to be used, DVB-S2 or DVB-S2X is still operated in multiple input stream mode, but only a single input stream identifier value is used.

> **Spec note:** The column header in the PDF reads `PHY_strem_id encoding` — the word
> "stream" is missing the 'a' — this is a typo in the original PDF and is transcribed
> faithfully here.

