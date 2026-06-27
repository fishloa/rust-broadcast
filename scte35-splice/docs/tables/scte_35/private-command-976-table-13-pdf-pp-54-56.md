## private_command() — §9.7.6, Table 13, PDF pp. 54-56

Distributes user-defined commands using the SCTE 35 protocol. Receiving
equipment should skip any splice_info_section() messages containing
private_command() structures with unknown identifiers.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `private_command() {` |  |  |
| &nbsp;&nbsp;identifier | 32 | uimsbf |
| &nbsp;&nbsp;`for(i=0; i<N; i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;private_byte | 8 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

- **identifier** — 32-bit field as defined in ISO/IEC 13818-1 §2.6.8/2.6.9
  for the registration_descriptor() format_identifier; only values
  registered with the SMPTE Registration Authority, LLC should be used.
  Identifies the owner of the command and scopes the private information
  within it.
- **private_byte** — the remainder of the structure, data fields as required
  by the command being defined.

