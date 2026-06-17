## PMT descriptors — §8, Tables 1-4, PDF pp. 32-35

These are ordinary MPEG-2 PSI descriptors (carried in the PMT, not in the
splice_info_section) that label SCTE 35 carriage.

### registration_descriptor() — §8.1, Table 1, PDF pp. 32-33

Shall be carried in the program_info loop of the PMT for each program that
complies with this standard (in all PMTs of all complying programs within a
multiplex); its presence indicates that splice_info_sections are carried in
one or more PID stream(s) within this program.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `registration_descriptor() {` |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;SCTE_splice_format_identifier | 32 | uimsbf |
| `}` |  |  |

- **descriptor_tag** — shall be **0x05**; **descriptor_length** — shall be
  **0x04**.
- **SCTE_splice_format_identifier** — SCTE has assigned the value
  **0x43554549 (ASCII "CUEI")** to this 4-byte field to identify the program
  (within a multiplex) as complying with this standard.

### cue_identifier_descriptor() — §8.2, Tables 2-3, PDF pp. 33-34

May be used in the PMT elementary descriptor loop to label PIDs that carry
splice commands by the type or level of commands they carry. If absent, the
stream may carry any valid command in this specification.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `cue_identifier_descriptor() {` |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;cue_stream_type | 8 | uimsbf |
| `}` |  |  |

- **descriptor_tag** — shall be **0x8A**; **descriptor_length** — shall be
  **0x01**.

cue_stream_type values (Table 3):

| cue_stream_type | PID usage |
|---|---|
| 0x00 | splice_insert, splice_null, splice_schedule — only these cue messages are allowed in this PID stream; at most one such PID, which (if it exists) shall be the first complying stream in the PMT elementary stream loop |
| 0x01 | All Commands (default if this descriptor is not present) |
| 0x02 | Segmentation — carries time_signal + segmentation descriptor; may also carry all other commands |
| 0x03 | Tiered Splicing |
| 0x04 | Tiered Segmentation |
| 0x05–0x7f | Reserved |
| 0x80–0xff | User Defined |

### stream_identifier_descriptor() — §8.3, Table 4, PDF pp. 34-35

May be used in the PMT elementary descriptor loop (following the relevant
ES_info_length field) to label component streams of a service. Shall be used
if either program_splice_flag or program_segmentation_flag is zero (the
deprecated Component modes); when used, one shall be present in each
occurrence of the elementary stream loop, with a unique component_tag within
the program.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `stream_identifier_descriptor() {` |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;component_tag | 8 | uimsbf |
| `}` |  |  |

- **descriptor_tag** — shall be **0x52**; **descriptor_length** — shall be
  **0x01**.
- **component_tag** — identifies the component stream for associating it
  with a description given in a component descriptor; within a program map
  section each stream_identifier_descriptor shall have a different value for
  this field.

