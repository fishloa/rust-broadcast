## Table 84 — DVB-J application descriptor syntax
_§10.9.1, PDF p.171. AIT application (inner) descriptor loop._

| Syntax | No. of bits | Identifier | Value |
|---|---|---|---|
| dvb_j_application_descriptor() { |  |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf | 0x03 |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |  |
| &nbsp;&nbsp;for( i=0; i<N; i++ ) { |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;parameter_length | 8 | uimsbf |  |
| &nbsp;&nbsp;&nbsp;&nbsp;for( j=0; j<parameter_length; j++ ) { |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;parameter_byte | 8 | uimsbf |  |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |  |
| &nbsp;&nbsp;} |  |  |  |
| } |  |  |  |

- **descriptor_tag**: `0x03`.
- **parameter_length**: number of bytes in the `parameter_byte` string.
- **parameter_byte**: a string passed to the application as a parameter. The
  descriptor body is a run of such length-prefixed parameter strings until
  `descriptor_length` is consumed.

---

