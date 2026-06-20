# Application Protocol Data Unit (APDU) coding + length field

_Source: EN 50221 §7 (Table 1, PDF p. 11) + §8.3 (Table 16 + Figure 12-13, PDF pp. 24-25), render-verified_

All protocols in the Application Layer use a common APDU structure to send application
data between module and host (or between modules). The communication of data across
the command interface is a general Tag-Length-Value (TLV) coding derived from ASN.1.

## Table 1 — Length field (used by all PDUs at Transport, Session & Application Layers, p. 11)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `length_field() {` | | |
| &nbsp;&nbsp;size_indicator | 1 | bslbf |
| &nbsp;&nbsp;`if (size_indicator == 0)` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;length_value | 7 | uimsbf |
| &nbsp;&nbsp;`else if (size_indicator == 1) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;length_field_size | 7 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<length_field_size; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;length_value_byte | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Semantics:
- `size_indicator` is the first (MSB) bit of the length field.
- `size_indicator == 0`: the data field length is coded in the succeeding 7 bits.
  Any length 0..127 is encoded in one byte.
- `size_indicator == 1`: the succeeding 7 bits (`length_field_size`) code the number
  of subsequent bytes in the length field. Those bytes are concatenated, first byte
  at the most significant end, to encode an integer value. Any length up to 65535 can
  be encoded by three bytes.
- The indefinite length format of ASN.1 basic encoding rules is NOT used.

## Table 16 — APDU coding (p. 25)

Figure 12: APDU structure = Header (apdu_tag, length_field) + Body ([data field]).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `APDU() {` | | |
| &nbsp;&nbsp;apdu_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;`for (i=0; i<length_value; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;data_byte | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

The APDU has two parts: a mandatory header (a 3-byte `apdu_tag` indicating which
parameter is sent, and a `length_field` coding the length of the following data
field; both coded per ASN.1 BER), and a conditional body of variable length equal to
the length coded by `length_field`.

## Chaining of APDU data fields (§8.3.2, p. 25)

The data field of an APDU may be split into smaller blocks if required by the
transmission/reception buffer sizes. Chaining uses two different `apdu_tag` values:
- `M_apdu_tag` — all blocks except the last (more data will follow);
- `L_apdu_tag` — the last block.

When the last block is received, the receiving entity concatenates all received data
fields. This mechanism is valid for all APDUs with two defined tag values
(`M_apdu_tag` / `L_apdu_tag`) — i.e. the `..._more` / `..._last` and
`..._send_more` / `..._send_last` tag pairs in `apdu-tag-values.md`.

An integer number of APDUs belonging to the same session can be transported in the
body of one SPDU (§8.3.3).
