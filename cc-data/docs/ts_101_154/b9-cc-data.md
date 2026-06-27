# Table B.9 — cc_data syntax

_Source: ETSI TS 101 154 V2.6.1 (2019-09) §B.5 / Table B.9 (PDF p. 198). Verified
against the PDF render. The DVB-native, normative definition of the closed-caption
carriage structure (cc_data() carried in MPEG/AVC/HEVC picture user_data). The
meaning of the `cc_data_1`/`cc_data_2` byte pair is defined in CEA-708-E [26] — a
layer above this carriage structure._

| Syntax | No. of Bits | Identifier |
|---|---|---|
| cc_data() { |  |  |
| reserved (set to "1") | 1 | bslbf |
| process_cc_data_flag | 1 | bslbf |
| zero_bit (set to "0") | 1 | bslbf |
| cc_count | 5 | uimsbf |
| reserved (set to "1111 1111") | 8 | bslbf |
| for (i=0; i<cc_count; i++) { |  |  |
| one_bit (set to "1") | 1 |  |
| reserved (set to "1111") | 4 |  |
| cc_valid | 1 | bslbf |
| cc_type | 2 | bslbf |
| cc_data_1 | 8 | bslbf |
| cc_data_2 | 8 | bslbf |
| } |  |  |
| marker_bits = "11111111" | 8 | bslbf |
| } |  |  |

Semantics:

- **process_cc_data_flag**: when `1`, the cc_data shall be parsed + processed; when
  `0`, discarded.
- **zero_bit**: `0` (CEA-708-E backwards compatibility).
- **cc_count**: 5-bit count of closed-caption constructs (0–31). Set per frame
  rate / picture structure to maintain a fixed 9 600 bit/s caption payload (16 bits
  per cc_data_1/cc_data_2 pair).
- **one_bit**: `1` (CEA-708-E backwards compatibility).
- **cc_valid**: `1` ⇒ the following two caption data bytes are valid.
- **cc_type**: type of the two caption bytes (CEA-708-E). 0/1 = CEA-608 NTSC line-21
  field 1/2; 2 = DTVCC channel packet data; 3 = DTVCC channel packet start.
- **cc_data_1 / cc_data_2**: the caption data byte pair (contents per CEA-708-E).
- **marker_bits**: `0xFF`.
