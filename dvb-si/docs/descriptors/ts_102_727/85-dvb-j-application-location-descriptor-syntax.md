## Table 85 — DVB-J application location descriptor syntax
_§10.9.2, PDF p.171. Exactly one per DVB-J application in the AIT application (inner) loop._

| Syntax | No. of bits | Identifier | Value |
|---|---|---|---|
| dvb_j_application_location_descriptor { |  |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf | 0x04 |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |  |
| &nbsp;&nbsp;base_directory_length | 8 | uimsbf |  |
| &nbsp;&nbsp;for( i=0; i<N; i++ ) { base_directory_byte | 8 | uimsbf |  |
| &nbsp;&nbsp;} |  |  |  |
| &nbsp;&nbsp;classpath_extension_length | 8 | uimsbf |  |
| &nbsp;&nbsp;for( i=0; i<N; i++ ) { classpath_extension_byte | 8 | uimsbf |  |
| &nbsp;&nbsp;} |  |  |  |
| &nbsp;&nbsp;for( i=0; i<N; i++ ) { initial_class_byte | 8 | uimsbf |  |
| &nbsp;&nbsp;} |  |  |  |
| } |  |  |  |

- **descriptor_tag**: `0x04`.
- **base_directory_length** / **base_directory_byte**: a directory-name string
  (slash `/`-delimited; "/" if root); length shall be ≥ 1.
- **classpath_extension_length** / **classpath_extension_byte**: a class-path
  extension string (elements `;`-delimited, dirs `/`-delimited).
- **initial_class_byte**: the remaining bytes — a DVB-J class name (length ≥ 1).
  Consumes the rest of the descriptor after the two length-prefixed strings.

---

