## splice_info_section() — §9.6, Table 5, PDF pp. 38-43

Carriage (§9.6, §9.9.1): the splice_info_section shall be carried in
transport packets whereby only one section or partial section may be in any
transport packet. Sections shall always start at the beginning of a transport
packet payload; in the first packet of a section the `pointer_field` shall be
present and equal to 0x00 and `payload_unit_start_indicator` shall be 1. The
splice_info_section PID(s) shall be identified in the PMT by **stream_type
0x86** and shall contain only information about splice events in that
program.

An `E` in the Encrypted column marks the portion of the section that is
encrypted when `encrypted_packet` == 1 (from `splice_command_type` up to and
including `E_CRC_32`).

| Syntax | Bits | Mnemonic | Encrypted |
|---|---|---|---|
| `splice_info_section() {` |  |  |  |
| &nbsp;&nbsp;table_id | 8 | uimsbf |  |
| &nbsp;&nbsp;section_syntax_indicator | 1 | bslbf |  |
| &nbsp;&nbsp;private_indicator | 1 | bslbf |  |
| &nbsp;&nbsp;sap_type | 2 | bslbf |  |
| &nbsp;&nbsp;section_length | 12 | uimsbf |  |
| &nbsp;&nbsp;protocol_version | 8 | uimsbf |  |
| &nbsp;&nbsp;encrypted_packet | 1 | bslbf |  |
| &nbsp;&nbsp;encryption_algorithm | 6 | uimsbf |  |
| &nbsp;&nbsp;pts_adjustment | 33 | uimsbf |  |
| &nbsp;&nbsp;cw_index | 8 | uimsbf |  |
| &nbsp;&nbsp;tier | 12 | bslbf |  |
| &nbsp;&nbsp;splice_command_length | 12 | uimsbf |  |
| &nbsp;&nbsp;splice_command_type | 8 | uimsbf | E |
| &nbsp;&nbsp;`if(splice_command_type == 0x00)` |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;splice_null() |  |  | E |
| &nbsp;&nbsp;`if(splice_command_type == 0x04)` |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;splice_schedule() |  |  | E |
| &nbsp;&nbsp;`if(splice_command_type == 0x05)` |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;splice_insert() |  |  | E |
| &nbsp;&nbsp;`if(splice_command_type == 0x06)` |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;time_signal() |  |  | E |
| &nbsp;&nbsp;`if(splice_command_type == 0x07)` |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;bandwidth_reservation() |  |  | E |
| &nbsp;&nbsp;`if(splice_command_type == 0xff)` |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;private_command() |  |  | E |
| &nbsp;&nbsp;descriptor_loop_length | 16 | uimsbf | E |
| &nbsp;&nbsp;`for(i=0; i<N1; i++)` |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;splice_descriptor() |  |  | E |
| &nbsp;&nbsp;`for(i=0; i<N2; i++)` |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;alignment_stuffing | 8 | bslbf | E |
| &nbsp;&nbsp;`if(encrypted_packet)` |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;E_CRC_32 | 32 | rpchof | E |
| &nbsp;&nbsp;CRC_32 | 32 | rpchof |  |
| `}` |  |  |  |

Field semantics (§9.6.1):

- **table_id** — 8 bits; its value shall be **0xFC**.
- **section_syntax_indicator** — 1 bit; should always be set to '0',
  indicating that MPEG short sections are to be used.
- **private_indicator** — 1-bit flag that shall be set to 0.
- **sap_type** — 2 bits; may be populated if the content preparation system
  has created a Stream Access Point (SAP, ISO 14496-12 Annex I) at the
  signaled point; values per Table 6 below. Note: prior versions of this
  specification defined these two bits as "reserved" bits, always set to
  '1' (i.e. the field read 0x3, "SAP type not specified").
- **section_length** — 12 bits; number of remaining bytes in the section
  immediately following this field up to the end of the section. Shall not
  exceed **4093**.
- **protocol_version** — 8 bits; at present the only valid value is **0**.
  Non-zero values may be used by a future version to indicate structurally
  different tables.
- **encrypted_packet** — 1 bit; when '1', the portion of the section from
  `splice_command_type` through and including `E_CRC_32` (the `E` column) is
  encrypted; when '0', no part of the message is encrypted.
- **encryption_algorithm** — 6 bits; which algorithm encrypted the message
  (Table 29 below). When `encrypted_packet` is 0 this field is present but
  undefined.
- **pts_adjustment** — 33 bits, always in the clear; an offset to be added
  (carry ignored on wrap/overflow) to every (possibly encrypted) `pts_time`
  field throughout the message to obtain the intended splice time(s) in the
  current time base. Zero means use `pts_time` unmodified; the message
  creator normally writes zero, and each device that restamps PCR/PTS/DTS
  adds its input→output delta to the field (and shall recalculate CRC_32).
- **cw_index** — 8 bits; which control word (key) of up to 256 previously
  provided keys decrypts the message. Present but undefined when
  `encrypted_packet` is 0.
- **tier** — 12 bits; assigns messages to authorization tiers, any value
  0x000–0xFFF. **0xFFF** provides backwards compatibility and shall be
  ignored by downstream equipment.
- **splice_command_length** — 12 bits; the number of bytes following
  `splice_command_type` up to but not including `descriptor_loop_length`.
  Compliant devices shall populate the actual length; the value **0xFFF**
  provides backwards compatibility and shall be ignored downstream.
- **splice_command_type** — 8 bits; one of the values in Table 7 below.
- **descriptor_loop_length** — 16 bits; the number of bytes used in the
  splice descriptor loop immediately following.
- **alignment_stuffing** — when encryption is used, stuffing bytes required
  by the chosen algorithm (e.g. DES needs a multiple of 8 bytes from
  `splice_command_type` through `E_CRC_32`). When encryption is not used this
  field shall not carry valid data but may be present.
- **E_CRC_32** — 32 bits, present only when `encrypted_packet` == 1; CRC
  (MPEG Systems decoder) giving a zero output after processing the entire
  **decrypted** portion, i.e. the fields `splice_command_type` through
  `E_CRC_32`. Indicates the decryption was performed successfully.
- **CRC_32** — 32 bits; CRC giving a zero output after processing the entire
  splice_info_section from `table_id` through `CRC_32`. CRC_32 processing
  shall occur **prior to decryption**, over the encrypted fields in their
  encrypted state.

### sap_type values — §9.6.1, Table 6, PDF p. 40

| sap_type value | ISOBMFF SAP type | Usage Notes |
|---|---|---|
| 0x0 | Type 1 | Closed GOP with no leading pictures |
| 0x1 | Type 2 | Closed GOP with leading pictures |
| 0x2 | Type 3 | Open GOP |
| 0x3 | SAP type not specified | The type of SAP, if any, is not signaled |

### splice_command_type values — §9.6.1, Table 7, PDF p. 42

| Command | splice_command_type value | XML Element |
|---|---|---|
| splice_null | 0x00 | SpliceNull |
| Reserved | 0x01 |  |
| Reserved | 0x02 |  |
| Reserved | 0x03 |  |
| splice_schedule | 0x04 | SpliceSchedule |
| splice_insert | 0x05 | SpliceInsert |
| time_signal | 0x06 | TimeSignal |
| bandwidth_reservation | 0x07 | BandwidthReservation |
| Reserved | 0x08 – 0xfe |  |
| private_command | 0xff | PrivateCommand |

