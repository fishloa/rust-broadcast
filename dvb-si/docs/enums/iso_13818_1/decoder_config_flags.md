## Table 2-90 — decoder_config_flags
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.61, Table 2-90; PDF p.104._

| Value | Description |
|---|---|
| 0b000 | No decoder configuration is needed |
| 0b001 | Decoder configuration carried in this descriptor (decoder_config_byte) |
| 0b010 | Decoder configuration carried in the same metadata service |
| 0b011 | Decoder configuration carried in a DSM-CC carousel |
| 0b100 | Decoder configuration carried in another metadata service in the same program |
| 0b101..0b110 | Reserved |
| 0b111 | Privately defined |
