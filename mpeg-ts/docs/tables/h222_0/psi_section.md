# PSI sections + reassembly — ITU-T H.222.0 (08/2023) §2.4.4 = ISO/IEC 13818-1

Transcribed from `specs/itu_t_h222_0_202308_mpeg2_systems.pdf` (pp.74–78), pdf2md
value-verified (one **false-positive** verifier warning on the Table 2-31 hex column
— eyeballed, the rendered values 0x00..0x03 are correct; see pdf2md issue #46), then
structure-cleaned. Scope = the **generic section framing** that `Section<'a>` /
`SectionReassembler` implement. PAT/CA sections below are shown as the canonical
*generic-section* template. Parsing the concrete PSI tables (PAT/PMT/CAT) is **not** in
`mpeg-ts`'s scope by design — that lives in `dvb-si`, which builds on this framing layer;
`mpeg-ts` stays the generic section/framing layer and does not duplicate it.

## §2.4.4.1 General — PSI structures (Table 2-28)

PSI is segmented into **sections** carried in TS packets. Six table structures:

| Structure | Stream type | Reserved PID | Description |
|---|---|---|---|
| Program association table (PAT) | H.222.0 \| 13818-1 | 0x0000 | program_number → program_map_PID |
| Program map table (PMT) | H.222.0 \| 13818-1 | assigned in PAT | PIDs of a program's components |
| Network information table (NIT) | Private | assigned in PAT | physical network params (FDM freqs, transponders…) |
| Conditional access table (CAT) | H.222.0 \| 13818-1 | 0x0001 | EMM streams, each a unique PID |
| Transport stream description table (TSDT) | H.222.0 \| 13818-1 | 0x0002 | descriptors (Table 2-45) for the whole TS |
| IPMP control information table | H.222.0 \| 13818-1 | 0x0003 | IPMP tool list/rights/tool container (ISO/IEC 13818-11) |

Private data may reuse the section structure: if carried on the same PID as a PMT,
the `private_section` syntax/semantics shall be used.

Section size limits: a PSI-table section ≤ **1024** bytes; a `private_section` ≤ **4096** bytes.

## §2.4.4.2 Pointer — Table 2-29 (pointer_field)

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| pointer_field | 8 | uimsbf |

**pointer_field** — number of bytes between this field and the first byte of the first
section in the packet payload (`0x00` ⇒ section starts immediately after the pointer).
When ≥1 section begins in a packet, `payload_unit_start_indicator` = `'1'` and the first
payload byte is the pointer. When none begins, `payload_unit_start_indicator` = `'0'` and
no pointer is sent.

## §2.4.4.1 Reassembly + stuffing rules (framing-relevant)

- A section is a variable-length structure; its start is marked by the `pointer_field`.
- Adaptation fields may occur in TS packets carrying sections.
- Packet **stuffing** bytes of value **0xFF** may appear in a section-carrying packet's
  payload **only after the last byte of a section**; once stuffing starts, all remaining
  bytes to the end of the 188-byte packet shall also be `0xFF` (a decoder may discard
  them). The next packet of that PID then begins with a `pointer_field` of `0x00`.
- There are no restrictions on start codes / sync bytes / bit patterns inside section data.
- A section becomes valid when its last byte (new `version_number`, `current_next_indicator`
  = `'1'`) has been fully received.

## §2.4.4.4 Program association section — Table 2-30 (the generic section template)

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| `program_association_section() {` | | |
| &nbsp;&nbsp;table_id | 8 | uimsbf |
| &nbsp;&nbsp;section_syntax_indicator | 1 | bslbf |
| &nbsp;&nbsp;'0' | 1 | bslbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;section_length | 12 | uimsbf |
| &nbsp;&nbsp;transport_stream_id | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;version_number | 5 | uimsbf |
| &nbsp;&nbsp;current_next_indicator | 1 | bslbf |
| &nbsp;&nbsp;section_number | 8 | uimsbf |
| &nbsp;&nbsp;last_section_number | 8 | uimsbf |
| &nbsp;&nbsp;`for (i = 0; i < N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;program_number | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (program_number == 0) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;network_PID | 13 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`} else {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;program_map_PID | 13 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;CRC_32 | 32 | rpchof |
| `}` | | |

## §2.4.4.7 Conditional access section — Table 2-32 (generic section + descriptor loop)

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| `CA_section() {` | | |
| &nbsp;&nbsp;table_id | 8 | uimsbf |
| &nbsp;&nbsp;section_syntax_indicator | 1 | bslbf |
| &nbsp;&nbsp;'0' | 1 | bslbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;section_length | 12 | uimsbf |
| &nbsp;&nbsp;reserved | 18 | bslbf |
| &nbsp;&nbsp;version_number | 5 | uimsbf |
| &nbsp;&nbsp;current_next_indicator | 1 | bslbf |
| &nbsp;&nbsp;section_number | 8 | uimsbf |
| &nbsp;&nbsp;last_section_number | 8 | uimsbf |
| &nbsp;&nbsp;`for (i = 0; i < N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;descriptor() | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;CRC_32 | 32 | rpchof |
| `}` | | |

## §2.4.4.5 table_id assignments — Table 2-31

| Value | Description |
|---|---|
| 0x00 | program_association_section |
| 0x01 | conditional_access_section (CA_section) |
| 0x02 | TS_program_map_section |
| 0x03 | TS_description_section |
| 0x04 | ISO_IEC_14496_scene_description_section |
| 0x05 | ISO_IEC_14496_object_descriptor_section |
| 0x06 | Metadata_section |
| 0x07 | IPMP Control Information Section (ISO/IEC 13818-11) |
| 0x08 | ISO_IEC_14496_section |
| 0x09 | ISO/IEC 23001-11 (Green access unit) section |
| 0x0A | ISO/IEC 23001-10 (Quality access unit) section |
| 0x0B | ISO/IEC 23001-13 (Media Orchestration access unit) section |
| 0x0C..0x37 | H.222.0 \| ISO/IEC 13818-1 reserved |
| 0x38..0x3F | Defined in ISO/IEC 13818-6 |
| 0x40..0xFE | User private |
| 0xFF | Forbidden |

## §2.4.4.6 Semantics — generic section header fields

- **table_id** — 8-bit; identifies the section contents per Table 2-31 (0x00 for PAT, 0x01 for CAT, …).
- **section_syntax_indicator** — 1-bit; shall be `'1'` for the long (CRC'd) section form.
- **section_length** — 12-bit; first two bits shall be `'00'`. The remaining 10 bits = number of bytes of the section *following* the section_length field, **including the CRC_32**. Shall not exceed **1021 (0x3FD)**.
- **transport_stream_id** (PAT) — 16-bit label distinguishing this TS from others in a network; user-defined.
- **version_number** — 5-bit; incremented (mod 32) whenever the table definition changes. With `current_next_indicator = '1'` it is the currently-applicable version; with `'0'`, the next.
- **current_next_indicator** — 1-bit; `'1'` = this table is currently applicable, `'0'` = not yet (the next to become valid).
- **section_number** — 8-bit; this section's number (first = 0x00, incremented per section).
- **last_section_number** — 8-bit; highest section_number of the complete table.
- **program_number** (PAT) — 16-bit; `0x0000` ⇒ the following PID is the network_PID; otherwise the program whose `program_map_PID` follows. Unique within one PAT version.
- **network_PID / program_map_PID** (PAT) — 13-bit PIDs (values per Table 2-3); network_PID only with program_number `0x0000`.
- **CRC_32** — 32-bit (`rpchof`); the CRC that yields a zero remainder in the Annex A decoder register after processing the entire section (CRC-32/MPEG-2).
