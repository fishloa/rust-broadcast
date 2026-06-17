## Table 87 — Scrambling mode coding
_§6.2.33, PDF pp. 99-99_

<!-- Page-break split scattered Table 87 into Table 88; both reconstructed verbatim from the PDF (2026-06-12). -->

| scrambling_mode | Description |
|---|---|
| 0x00 | reserved for future use |
| 0x01 | this value indicates use of DVB-Common Scrambling Algorithm Version 1 (CSA1). It is the default mode and shall be used when the scrambling_descriptor is not present in the program map section |
| 0x02 | this value indicates use of DVB-Common Scrambling Algorithm Version 2 (CSA2) |
| 0x03 | this value indicates use of DVB-Common Scrambling Algorithm Version 3 (CSA3) |
| 0x04 to 0x0F | reserved for future use |
| 0x10 | this value indicates use of DVB-Common IPTV Software-oriented Scrambling Algorithm (CISSA) version 1 |
| 0x11 to 0x1F | reserved for future use for DVB-CISSA versions |
| 0x20 to 0x6F | reserved for future use |
| 0x70 to 0x7F | Alliance for Telecommunications Industry Solutions (ATIS) defined (see annex J of ATIS 0800006 [i.6]) |
| 0x80 to 0xFE | user defined |
| 0xFF | reserved for future use |

