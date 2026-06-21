# CI Plus Sample-Mode descriptors (IV / key-identifier)

_Source: ETSI TS 103 205 v1.4.1 §7.5.5.4, Tables 45-47 (PDF pp. 79-80), render-verified.
(Table 44 is the §7.5.5.3.3 SEP message frame, not a descriptor — not transcribed here.)_

These are the descriptors that may be carried in CI Plus Sample-Mode messages
(Sample Start TS Packet, Sample End TS Packet, Flush Table) and in the
USB/second-generation media-interface **fragment-header descriptor loop** (see
`../ci_plus/fragment-header.md`). They close the deferral noted in
`fragment-header.md`, which pointed at "unvendored TS 103 205 §7.5.5.4.2 /
§7.5.5.4.3, Table 46 / Table 47" — those layouts are transcribed here.

All descriptors follow the standard `descriptor_tag` (8) + `descriptor_length` (8)
+ body format (§7.5.5.4.1). The `descriptor_tag` uniquely identifies the
descriptor; `descriptor_length` is the number of body bytes following it.

## §7.5.5.4.1 — Table 45: Message descriptors (tag allocation) (PDF p. 79)

A "*" marks the message types in which the descriptor may be included. Columns:
SSP = Sample Start TS Packet, SEP = Sample End TS Packet, FLT = Flush Table.

| Descriptor | Tag Value | SSP | SEP | FLT |
|------------|-----------|:---:|:---:|:---:|
| Forbidden | `0x00` | | | |
| ciplus_initialization_vector_descriptor | `0xD0` | * | | |
| ciplus_key_identifier_descriptor | `0xD1` | * | | |
| Reserved | `0xD2`–`0xEF` | | | |
| Host defined | `0xF0`–`0xFE` | * | * | * |
| Forbidden | `0xFF` | | | |

(The IV and key-identifier descriptors are allowed only in the Sample Start TS
Packet; host-defined descriptors `0xF0`–`0xFE` may appear in all three message
types.)

## §7.5.5.4.2 — CI Plus initialization vector descriptor — Table 46 (PDF p. 80)

Used by the Host to provide the initialization vector associated with the
following Sample.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ciplus_initialization_vector_descriptor() {` | | |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;IV_data_byte | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field semantics (§7.5.5.4.2, p. 80):
- **descriptor_tag** (8) — `0xD0`.
- **descriptor_length** (8) — the number of `IV_data_byte` bytes following.
- **IV_data_byte** (8) — the sequence of `IV_data_byte` fields contains the IV. The IV bytes are an **opaque** crypto blob; only the variable length (`descriptor_length`) and the byte sequence are structural. The loop runs `descriptor_length` bytes (`N` = `descriptor_length`).

## §7.5.5.4.3 — CI Plus key identifier descriptor — Table 47 (PDF p. 80)

Used by the Host to provide the content key identifier associated with the
following Sample.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ciplus_key_identifier_descriptor() {` | | |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;key_id_data_byte | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field semantics (§7.5.5.4.3, p. 80):
- **descriptor_tag** (8) — `0xD1`.
- **descriptor_length** (8) — the number of `key_id_data_byte` bytes following.
- **key_id_data_byte** (8) — the sequence of `key_id_data_byte` fields contains the Key Identifier. **Opaque** key-identifier blob; only the variable length and byte sequence are structural. The loop runs `descriptor_length` bytes.

---

**Deferral closure note (for `../ci_plus/fragment-header.md`):** both descriptors
are **variable-length** (TLV: 8-bit tag, 8-bit length, then `descriptor_length`
opaque body bytes) — NOT fixed-width. The earlier `fragment-header.md` ⚠ flags
("Width not in TS 103 605 — see TS 103 205 Table 46 / Table 47") are now resolved:
the width is whatever `descriptor_length` specifies at runtime, with the body
being the raw IV / key-identifier octets respectively.
