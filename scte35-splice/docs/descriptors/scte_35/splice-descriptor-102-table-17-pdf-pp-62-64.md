## splice_descriptor() — §10.2, Table 17, PDF pp. 62-64

The prototype/template for all splice descriptors; all descriptors use the
same syntax for the first six bytes.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `splice_descriptor() {` |  |  |
| &nbsp;&nbsp;splice_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;identifier | 32 | uimsbf |
| &nbsp;&nbsp;`for(i=0; i<N; i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;private_byte | 8 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

Field semantics (§10.2.1):

- **splice_descriptor_tag** — 8 bits; defines the syntax for the private
  bytes that make up the body of the descriptor. Tags are defined by the
  owner of the descriptor, as registered using the identifier.
- **descriptor_length** — 8 bits; the length, in bytes, of the descriptor
  following this field. Descriptors are limited to 256 bytes, so this value
  is limited to **254**. Since some descriptors have optional elements, a
  decoder should never attempt to decode beyond the descriptor length.
- **identifier** — 32-bit field per ISO/IEC 13818-1 §2.6.8/2.6.9
  (registration_descriptor() format_identifier), registered with the SMPTE
  Registration Authority, LLC. The code **0x43554549 (ASCII "CUEI")** is
  registered for descriptors defined in this specification. Private Splice
  Descriptors (§10.2.2) shall conform to this same format and shall **not**
  use 0x43554549 as their identifier.
- **private_byte** — the remainder of the descriptor, data fields as
  required by the descriptor being defined.

