## CDR / alignment — the one bounded caveat
_§4.7.3.1, PDF pp. 30–31_

BIOP uses CDR-Lite encoding (ISO/IEC 13818-6 §11, citing OMG CORBA CDR). The only
alignment rule that surfaces in these tables is the `alignment_gap` in
Table 4.3, taken `if (type_id_length % 4 ≠ 0)`. TR 101 202's DVB guideline
mandates **alias type_ids only** — always 3 chars + NUL = 4 bytes — so
`N1 % 4 == 0` always and the gap is **always zero bytes** in a conformant DVB
stream. The implementation therefore parses the IOR with no alignment gap and
**rejects** a non-alias `type_id_length` (`N1 % 4 ≠ 0`) as unsupported rather
than guessing the ISO alignment rule. The `*_byte_order` fields are the CDR
byte-order flag and are fixed at `0x00` (big-endian) by DVB guideline.

