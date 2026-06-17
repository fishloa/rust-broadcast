## Table 60 — Linkage type coding
_§6.2.19.1, PDF pp. 84-84_

| linkage_type | Description |
|---|---|
| 0x00 | reserved for future use |
| 0x01 | information service |
| 0x02 | EPG service |
| 0x03 | CA replacement service |
| 0x04 | TS containing complete network/bouquet SI |
| 0x05 | service replacement service |
| 0x06 | data broadcast service |
| 0x07 | Return Channel Satellite (RCS) map |
| 0x08 | mobile hand-over |
| 0x09 | System Software Update (SSU) service (ETSI TS 102 006 [20]) |
| 0x0A | TS containing SSU BAT or NIT (ETSI TS 102 006 [20]) |
| 0x0B | Internet Protocol/Medium Access Control (IP/MAC) notification service |
|  | (ETSI EN 301 192 [3]) |
| 0x0C | TS containing INT BAT or NIT (ETSI EN 301 192 [3]) |
| 0x0D | event linkage (see note) |
| 0x0E to 0x1F | extended event linkage (see note) |
| 0x20 | downloadable font info linkage (ETSI EN 303 560 [12]) |
| 0x21 | Native IP bootstrap MPE stream (DVB BlueBook A180) [57] |
| 0x22 to 0x7F | reserved for future use |
| 0x80 to 0xFE | user defined |
| 0xFF | reserved for future use |
| NOTE: A linkage_type | with a value in the range 0x0D to 0x1F is only valid when the |
| descriptor | is carried in the EIT. |

