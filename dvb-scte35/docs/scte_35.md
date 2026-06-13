# ANSI/SCTE 35 2023r1 — Digital Program Insertion Cueing Message (syntax reference)

> **✓ Accuracy-verified against the PDF — 2026-06-13.** The entire transcription
> was audited against the vendored `specs/ansi_scte_35_2023r1_dpi_cueing.pdf`
> (final arbiter), cross-checked against an independent Mistral-OCR rendering of
> the table pages (via BlazeDocs), with the PDF page opened directly wherever the
> two diverged. Every enum value↔name mapping, bit-field width/mnemonic, fixed
> length, and reserved range across ~30 tables (§8–§11) was confirmed; the §14
> sample messages were validated deterministically (each base64 decodes to its
> stated length and matches its hex + CRC-32). **Result: 0 discrepancies** — the
> md is faithful to the PDF. (OCR artifacts such as "Program Runner" for
> "Runover" were identified as OCR errors and the md's reading confirmed correct.)

**Provenance.** The canonical PDF is vendored at
`specs/ansi_scte_35_2023r1_dpi_cueing.pdf` and is the authoritative source.
The tables below were **hand-transcribed from that PDF on 2026-06-09**: the
SCTE page layout (ruled multi-column tables with an extra "Encrypted" column,
wrapped enumeration rows) is outside the table model of the ETSI-specific
geometry extractor in `tools/dvb-si-audit/`, which produced zero tables for
this document. Every syntax row (field name, bit width, mnemonic) and every
enumeration value was copied verbatim from the PDF page cited in each section
header. SCTE 35 is published by SCTE at no cost. Section numbers (§) are the
document's own; "PDF pp." are the printed page numbers, which coincide with
the PDF page numbers in this file.

Mnemonics are the MPEG conventions: `uimsbf` = unsigned integer, most
significant bit first; `bslbf` = bit string, left bit first; `rpchof` =
remainder polynomial coefficients, highest order first. Indentation inside
the Syntax column (rendered with `&nbsp;`) reproduces the nesting of the
spec's tables; the `if(...)`/`for(...)` lines are part of the normative
syntax and define the parse logic.

## Contents

- [splice_info_section()](#splice_info_section--96-table-5-pdf-pp-38-43)
  - [sap_type values](#sap_type-values--961-table-6-pdf-p-40)
  - [splice_command_type values](#splice_command_type-values--961-table-7-pdf-p-42)
- [splice_null()](#splice_null--971-table-8-pdf-p-43)
- [splice_schedule()](#splice_schedule--972-table-9-pdf-pp-44-48)
- [splice_insert()](#splice_insert--973-table-10-pdf-pp-49-52)
- [time_signal()](#time_signal--974-table-11-pdf-pp-52-53)
- [bandwidth_reservation()](#bandwidth_reservation--975-table-12-pdf-p-54)
- [private_command()](#private_command--976-table-13-pdf-pp-54-56)
- [splice_time()](#splice_time--981-table-14-pdf-pp-56-57)
- [break_duration()](#break_duration--982-table-15-pdf-pp-57-58)
- [Event id partitioning](#event-id-partitioning--993-pdf-pp-60-61)
- [Splice Descriptor Tags](#splice-descriptor-tags--101-table-16-pdf-p-62)
- [splice_descriptor()](#splice_descriptor--102-table-17-pdf-pp-62-64)
- [avail_descriptor()](#avail_descriptor--1031-table-18-pdf-pp-65-66)
- [DTMF_descriptor()](#dtmf_descriptor--1032-table-19-pdf-pp-66-68)
- [segmentation_descriptor()](#segmentation_descriptor--1033-table-20-pdf-pp-68-79)
  - [device_restrictions](#device_restrictions--10331-table-21-pdf-p-73)
  - [segmentation_upid_type](#segmentation_upid_type--10331-table-22-pdf-pp-74-75)
  - [segmentation_type_id](#segmentation_type_id--10331-table-23-pdf-pp-76-79)
  - [MPU()](#mpu--10333-table-24-pdf-p-80)
  - [MID()](#mid--10334-table-25-pdf-pp-80-81)
- [time_descriptor()](#time_descriptor--1034-table-27-pdf-pp-97-100)
- [audio_descriptor()](#audio_descriptor--1035-table-28-pdf-pp-102-104)
- [Encryption algorithm](#encryption-algorithm--113-table-29-pdf-pp-104-106)
- [PMT descriptors](#pmt-descriptors--8-tables-1-4-pdf-pp-32-35)

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

## splice_null() — §9.7.1, Table 8, PDF p. 43

Provided for extensibility: lets a splice_info_table carry descriptors
without sending one of the other defined commands; may also be used as a
"heartbeat message" for monitoring cue injection equipment integrity and
link integrity.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `splice_null() {` |  |  |
| `}` |  |  |

## splice_schedule() — §9.7.2, Table 9, PDF pp. 44-48

Allows a schedule of splice events to be conveyed in advance. Component
Splice Mode (`program_splice_flag == '0'`) is **deprecated**.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `splice_schedule() {` |  |  |
| &nbsp;&nbsp;splice_count | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<splice_count; i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;splice_event_id | 32 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;splice_event_cancel_indicator | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;event_id_compliance_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (splice_event_cancel_indicator == '0') {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;out_of_network_indicator | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;program_splice_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;duration_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if (program_splice_flag == '1')` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;utc_splice_time | 32 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if (program_splice_flag == '0') {` (deprecated) |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;component_count | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for(j=0;j<component_count;j++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;component_tag | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;utc_splice_time | 32 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if (duration_flag)` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;break_duration() |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;unique_program_id | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;avail_num | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;avails_expected | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

Field semantics (§9.7.2.1):

- **splice_count** — 8-bit unsigned integer; the number of splice events
  specified in the loop that follows.
- **splice_event_id** — a 32-bit unique splice event identifier (see Event
  id partitioning, §9.9.3, below).
- **splice_event_cancel_indicator** — 1-bit flag; when '1', a previously
  sent splice event identified by `splice_event_id` has been cancelled.
- **event_id_compliance_flag** — when '0', the `splice_event_id` value for
  each event in this command complies with §9.9.3; when '1', compliance is
  not specified.
- **out_of_network_indicator** — 1-bit flag; '1' = the splice event is an
  opportunity to exit from the network feed and `utc_splice_time` refers to
  an intended Out Point / Program Out Point; '0' = an opportunity to return
  to the network feed and `utc_splice_time` refers to an intended In Point /
  Program In Point.
- **program_splice_flag** — should be '1': Program Splice Mode, all
  PIDs/components of the program are spliced. '0' = Component Splice Mode
  (**deprecated**), each component to be spliced is listed separately.
- **duration_flag** — 1-bit flag indicating the presence of the
  `break_duration()` field.
- **utc_splice_time** — 32-bit unsigned integer: the time of the signaled
  splice event as the number of seconds since 00 hours UTC, January 6th,
  1980, **with** the count of intervening leap seconds included (i.e.
  GPS-epoch seconds; convertible to UTC without the System Time table's
  GPS_UTC_offset). Used only in the splice_schedule() command.
- **component_count** (deprecated) — 8-bit count of elementary PID stream
  entries in the loop; shall be ≥ 1 when `program_splice_flag` == '0'.
- **component_tag** (deprecated) — 8-bit value identifying the elementary
  PID stream containing the Splice Point; the same value as in the PMT
  stream_identifier_descriptor() for that stream.
- **unique_program_id** — should provide a unique identification for a
  viewing event within the service (see SCTE 118-2 / SCTE 224 for guidance).
- **avail_num** — (previously 'avail') identifies a specific avail within
  one `unique_program_id`; expected to reset to one for the first avail in a
  new viewing event and to increment for each new avail; may carry a zero
  value to indicate non-usage.
- **avails_expected** — (previously 'avail_count') the expected number of
  individual avails within the current viewing event; zero means `avail_num`
  has no meaning.

## splice_insert() — §9.7.3, Table 10, PDF pp. 49-52

Signals an upcoming splice event. Shall be sent at least once before each
legacy Splice Point; packets containing the entirety of the table shall
always precede the packet containing the related Splice Point. Component
Splice Mode is **deprecated**.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `splice_insert() {` |  |  |
| &nbsp;&nbsp;splice_event_id | 32 | uimsbf |
| &nbsp;&nbsp;splice_event_cancel_indicator | 1 | bslbf |
| &nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;`if(splice_event_cancel_indicator == '0') {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;out_of_network_indicator | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;program_splice_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;duration_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;splice_immediate_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;event_id_compliance_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if((program_splice_flag == '1') && (splice_immediate_flag == '0'))` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;splice_time() |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`if(program_splice_flag == '0') {` (deprecated) |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;component_count | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for(i=0;i<component_count;i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;component_tag | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if(splice_immediate_flag == '0')` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;splice_time() |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`if(duration_flag == '1')` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;break_duration() |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;unique_program_id | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;avail_num | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;avails_expected | 8 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

Field semantics (§9.7.3.1) — `splice_event_id`,
`splice_event_cancel_indicator`, `event_id_compliance_flag`,
`program_splice_flag`, `duration_flag`, `component_count`, `component_tag`,
`unique_program_id`, `avail_num`, `avails_expected` are as defined for
splice_schedule() above, except that times are PTS-based:

- **out_of_network_indicator** — '1' = opportunity to exit the network feed;
  the value of `splice_time()`, **as modified by `pts_adjustment`**, refers
  to an intended Out Point / Program Out Point. '0' = opportunity to return
  to the network feed; `splice_time()` as modified by `pts_adjustment`
  refers to an intended In Point / Program In Point.
- **splice_immediate_flag** — when '1', indicates the absence of the
  `splice_time()` field and that the splice mode shall be the Splice
  Immediate Mode, whereby the splicing device shall choose the nearest
  opportunity in the stream, relative to the splice information packet, to
  splice. When '0', indicates the presence of `splice_time()` in at least
  one location within the splice_insert() command.
- Timing constraints (§9.9.2.1): for a network Out Point at least one
  message shall arrive at least 4 seconds in advance of the signaled splice
  time. The intended Out Point shall be **before** the first presentation
  unit with presentation time ≥ the signaled `pts_time` as modified by
  `pts_adjustment`; the intended In Point shall **be** the first
  presentation unit with presentation time ≥ that value. In Component Splice
  Mode (deprecated), the first listed component shall carry a valid
  `pts_time` (the default pts_time), used by subsequent components that omit
  theirs.

## time_signal() — §9.7.4, Table 11, PDF pp. 52-53

Provides a time synchronized data delivery mechanism; the unique payload of
the message is carried in the descriptor loop (when used to signal splice
events it shall carry one or more segmentation descriptors). If
`time_specified_flag` is 0 (no `pts_time` in the message) the command shall
be interpreted as an immediate command (with an unspecified amount of
accuracy error).

| Syntax | Bits | Mnemonic |
|---|---|---|
| `time_signal() {` |  |  |
| &nbsp;&nbsp;splice_time() |  |  |
| `}` |  |  |

## bandwidth_reservation() — §9.7.5, Table 12, PDF p. 54

Provided for reserving bandwidth in a multiplex (e.g. keeping a PID present
at an intended repetition rate). Differs from splice_null() so receiving
equipment can handle it uniquely (e.g. remove it from the multiplex).
Descriptors sent with this command cannot be expected to be carried through
the entire transmission chain and should be private descriptors used only by
the bandwidth reservation process.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `bandwidth_reservation() {` |  |  |
| `}` |  |  |

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

## splice_time() — §9.8.1, Table 14, PDF pp. 56-57

The splice_time() structure, when modified by `pts_adjustment`, specifies
the time of the splice event.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `splice_time() {` |  |  |
| &nbsp;&nbsp;time_specified_flag | 1 | bslbf |
| &nbsp;&nbsp;`if(time_specified_flag == 1) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;pts_time | 33 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;`else` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| `}` |  |  |

- **time_specified_flag** — 1-bit flag; when '1', indicates the presence of
  the `pts_time` field and associated reserved bits.
- **pts_time** — 33 bits, time in ticks of the program's 90 kHz clock. When
  modified by `pts_adjustment`, represents the time of the intended Splice
  Point: the Splice Point shall be the first PES packet in the bit stream
  with a PTS time greater than or equal to `pts_time` as adjusted by
  `pts_adjustment`. Coding constraints for Splice Points are documented in
  SCTE 172.

## break_duration() — §9.8.2, Table 15, PDF pp. 57-58

Specifies the duration of the commercial Break(s); may be used to give the
splicer an indication of when the Break will be over and when the network In
Point will occur.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `break_duration() {` |  |  |
| &nbsp;&nbsp;auto_return | 1 | bslbf |
| &nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;duration | 33 | uimsbf |
| `}` |  |  |

- **auto_return** — 1-bit flag; when '1' (Auto Return Mode, §9.9.2.2), the
  duration shall be used by the splicing device to know when the return to
  the network feed (end of Break) is to take place; a splice_insert()
  command with `out_of_network_indicator` set to 0 is not intended to be
  sent to end this Break (though one may be sent to terminate it early, and
  shall always override the running duration). When '0', the duration field,
  if present, is not required to end the Break because a new splice_insert()
  command will be sent; its presence acts as a safety mechanism in the event
  that the end-of-Break splice_insert() command is lost.
- **duration** — 33 bits; elapsed time in ticks of the program's 90 kHz
  clock.

## Event id partitioning — §9.9.3, PDF pp. 60-61

Applies to `splice_event_id` in splice_insert()/splice_schedule() and to
`segmentation_event_id` carried in the segmentation_descriptor with a
time_signal() command. Only one occurrence of a given event id value shall be
active at any one time; once an event ends, the value becomes inactive and
eligible for reuse (but shall not be reused immediately if multiple messages
referencing the same Splice Point end some events and start new ones). The
32-bit event id value shall be partitioned:

| Syntax | Bits | Type |
|---|---|---|
| Event_source | 4 | uimsbf |
| Event_number | 28 | uimsbf |

- **Event_source** — a user assigned number for the source of the Cue
  Message.
- **Event_number** — a number chosen by the Event source to identify an
  instance of the Cue Message.

Assigned Event_source values:

| Source of Cue Message | Event_source value | Event id ranges |
|---|---|---|
| Cue embedded in original source material by network programming systems | 0 | 0x00000000, 0x00000001 … 0x0fffffff |
| Cue created by automation system switching | 4 | 0x40000000, 0x40000001 … 0x4fffffff |
| Cue created by manual event trigger system | 6 | 0x60000000, 0x60000001 … 0x6fffffff |
| Cue created by local content replacement system | 12 | 0xC0000000, 0xC0000001 … 0xCfffffff |

## Splice Descriptor Tags — §10.1, Table 16, PDF p. 62

Splice descriptors are only used within a splice_info_section (never in MPEG
syntax such as the PMT). Receiving equipment should skip descriptors with
unknown identifiers; for known identifiers it should skip descriptors with
an unknown splice_descriptor_tag. Multiple descriptors of the same or
different types in a single command are allowed and may be common; the only
limit on the number of descriptors is `section_length`.

| Tag | XML Element | Descriptors for Identifier "CUEI" |
|---|---|---|
| 0x00 | AvailDescriptor | avail_descriptor |
| 0x01 | DTMFDescriptor | DTMF_descriptor |
| 0x02 | SegmentationDescriptor | segmentation_descriptor |
| 0x03 | TimeDescriptor | time_descriptor |
| 0x04 | AudioDescriptor | audio_descriptor |
| 0x05 – 0xEF |  | Reserved for future SCTE splice_descriptors |
| 0xF0 – 0xFF |  | Reserved for DVB use (as specified in ETSI 103 752-1) |

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

## avail_descriptor() — §10.3.1, Table 18, PDF pp. 65-66

Optional extension to the splice_insert() command allowing an authorization
identifier to be sent for an avail (replicating analog cue-tone
functionality). Multiple copies may be included using the descriptor loop.
Intended only for use with a splice_insert() command, within a
splice_info_section.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `avail_descriptor() {` |  |  |
| &nbsp;&nbsp;splice_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;identifier | 32 | uimsbf |
| &nbsp;&nbsp;provider_avail_id | 32 | uimsbf |
| `}` |  |  |

- **splice_descriptor_tag** — shall be **0x00**.
- **descriptor_length** — shall be **0x08**.
- **identifier** — shall be **0x43554549** (ASCII "CUEI").
- **provider_avail_id** — 32-bit number that a receiving device may utilize
  to alter its behavior during or outside of an avail; may be used in a
  manner similar to analog cue tones (e.g. a network directing an
  affiliate/head-end to black out a sporting event).

## DTMF_descriptor() — §10.3.2, Table 19, PDF pp. 66-68

Optional extension to the splice_insert() command allowing a receiver device
to generate a legacy analog DTMF sequence based on a splice_info_section
being received.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `DTMF_descriptor() {` |  |  |
| &nbsp;&nbsp;splice_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;identifier | 32 | uimsbf |
| &nbsp;&nbsp;preroll | 8 | uimsbf |
| &nbsp;&nbsp;dtmf_count | 3 | uimsbf |
| &nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;`for(i=0; i<dtmf_count; i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;DTMF_char | 8 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

- **splice_descriptor_tag** — shall be **0x01**.
- **identifier** — shall be **0x43554549** (ASCII "CUEI").
- **preroll** — the time the DTMF is presented to the analog output of the
  device, in **tenths of seconds** (pre-roll range 0 to 25.5 seconds). The
  splice info section shall be sent at least two seconds earlier than this
  value; the minimum suggested pre-roll is 4.0 seconds.
- **dtmf_count** — the number of DTMF characters the device is to generate.
- **DTMF_char** — an ASCII value for the numerals '0' to '9', '*', '#'. The
  sequence shall complete with the last character sent being the timing mark
  for the pre-roll.

## segmentation_descriptor() — §10.3.3, Table 20, PDF pp. 68-79

Optional extension to the time_signal() and splice_insert() commands that
allows segmentation messages to be sent in a time/video accurate method.
Shall only be used with the time_signal(), splice_insert() and splice_null()
commands. The time_signal() or splice_insert() message should be sent at
least once, a minimum of 4 seconds in advance of the signaled splice_time().
Devices that do not recognize a value in any field shall ignore the message
and take no action. Component Segmentation Mode
(`program_segmentation_flag == '0'`) is **deprecated**.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `segmentation_descriptor() {` |  |  |
| &nbsp;&nbsp;splice_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;identifier | 32 | uimsbf |
| &nbsp;&nbsp;segmentation_event_id | 32 | uimsbf |
| &nbsp;&nbsp;segmentation_event_cancel_indicator | 1 | bslbf |
| &nbsp;&nbsp;segmentation_event_id_compliance_indicator | 1 | bslbf |
| &nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;`if(segmentation_event_cancel_indicator == '0') {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;program_segmentation_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_duration_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;delivery_not_restricted_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if(delivery_not_restricted_flag == '0') {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;web_delivery_allowed_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;no_regional_blackout_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;archive_allowed_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;device_restrictions | 2 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`} else {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`if(program_segmentation_flag == '0') {` (deprecated) |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;component_count | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for(i=0;i<component_count;i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;component_tag | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;pts_offset | 33 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`if(segmentation_duration_flag == '1')` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;segmentation_duration | 40 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_upid_type | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_upid_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_upid() |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_type_id | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segment_num | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segments_expected | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if(segmentation_type_id == '0x34' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x30' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x32' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x36' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x38' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x3A' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x44' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x46') {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;sub_segment_num | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;sub_segments_expected | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

> Transcription note: in the published PDF this conditional is typeset with
> two misprints — a stray closing parenthesis on the `'0x3A'` line
> (`== '0x3A') ||`) and the `8 / uimsbf` cells for `sub_segment_num`
> vertically drifted onto the `'0x38'` row (verified by word-coordinate
> inspection of PDF p. 69). The brace structure and the
> sub_segment_num/sub_segments_expected semantics (§10.3.3.1) make the
> intended reading above unambiguous: the condition closes after `'0x46'`,
> and both `sub_segment_num` and `sub_segments_expected` are 8-bit uimsbf.

Field semantics (§10.3.3.1):

- **splice_descriptor_tag** — shall be **0x02**.
- **descriptor_length** — length in bytes of the descriptor following this
  field. Note: `sub_segment_num` and `sub_segments_expected` can form an
  optional appendix to the segmentation descriptor; the presence or absence
  of this optional data block is determined by **descriptor_length**.
- **identifier** — shall be **0x43554549** (ASCII "CUEI").
- **segmentation_event_id** — a 32-bit unique segmentation event identifier.
  Only one occurrence of a given value shall be active at any one time (see
  §9.9.3 and §10.3.3.7).
- **segmentation_event_cancel_indicator** — 1-bit flag; when '1', a
  previously sent segmentation event identified by `segmentation_event_id`
  has been cancelled. The `segmentation_type_id` does not need to match
  between the original and cancelling messages; once cancelled, the
  `segmentation_event_id` may be reused for content identification or to
  start a new Segment.
- **segmentation_event_id_compliance_indicator** — when '0', the
  `segmentation_event_id` is compliant with §9.9.3; when '1', compliance is
  not specified.
- **program_segmentation_flag** — should be '1': Program Segmentation Mode,
  all PIDs/components of the program are to be segmented. '0' = Component
  Segmentation Mode (**deprecated**), each component to be segmented is
  listed separately. May be set to different states in different descriptor
  messages within a program.
- **segmentation_duration_flag** — should be '1', indicating the presence of
  the `segmentation_duration` field. The accuracy of the start time of this
  duration is constrained by the splice_command_type used (e.g. with
  splice_null() the precise position in the stream is not deterministic).
- **delivery_not_restricted_flag** — when '1', the next five bits are
  reserved. When '0', the following five bits
  (`web_delivery_allowed_flag` … `device_restrictions`) have the meanings
  below; they facilitate implementations that use out-of-scope methods to
  process and manage this Segment.
- **web_delivery_allowed_flag** — '1' = no restrictions with respect to web
  delivery of this Segment; '0' = restrictions related to web delivery are
  asserted.
- **no_regional_blackout_flag** — '1' = no regional blackout of this
  Segment; '0' = this Segment is restricted due to regional blackout rules.
- **archive_allowed_flag** — '1' = no assertion about recording this
  Segment; '0' = restrictions related to recording this Segment are
  asserted.
- **device_restrictions** — 2 bits, per Table 21 below; signals three
  pre-defined groups of devices whose population is independent and
  non-hierarchical (delivery/format of the group-defining messaging is out
  of scope).
- **component_count / component_tag** (Component mode, deprecated) — as in
  splice_insert(): count shall be ≥ 1 when `program_segmentation_flag` ==
  '0'; `component_tag` matches the PMT stream_identifier_descriptor() value
  and its presence denotes the presence of this component of the asset.
- **pts_offset** — 33-bit unsigned integer; an offset to be **added** to the
  `pts_time`, as modified by `pts_adjustment`, in the time_signal() message
  to obtain the intended splice time(s); zero = use `pts_time` without
  offset. If `time_specified_flag` = 0, or the command carrying this
  descriptor does not have a splice_time() field, this field shall be used
  to offset the derived immediate splice time.
- **segmentation_duration** — 40-bit unsigned integer; the duration of the
  Segment in ticks of the program's 90 kHz clock; may indicate when the
  Segment will be over and when the next segmentation message will occur.
  **Shall be 0 for end messages.**
- **segmentation_upid_type** — a value from Table 22 below.
- **segmentation_upid_length** — length in bytes of segmentation_upid() as
  indicated by Table 22; shall be set to **zero** if no segmentation_upid()
  is present. For UPID type MID() it reflects the total length of the nested
  UPID types structure.
- **segmentation_upid()** — contents and length determined by
  `segmentation_upid_type` and `segmentation_upid_length` (e.g. type 0x06
  ISAN with length 12 carries the ISAN identifier of the content this
  descriptor refers to).
- **segmentation_type_id** — 8 bits, one of the values in Table 23 below to
  designate the type of segmentation; all unused values are reserved. When
  `segmentation_type_id` is 0x01 (Content Identification), the value of
  `segmentation_upid_type` shall be non-zero. If `segmentation_upid_length`
  is zero, then `segmentation_type_id` shall be set to 0x00 (Not Indicated).
- **segment_num** — supports numbering Segments within a given collection of
  Segments (such as Chapters or Advertisements); when utilized, expected to
  reset to one at the beginning of a collection and to increment for each
  new Segment. Value as indicated in Table 23.
- **segments_expected** — a count of the expected number of individual
  Segments within the collection. Value as indicated in Table 23.
- **sub_segment_num** — optional, for the applicable segmentation_type_id
  values in Table 23; identifies a specific sub-Segment within a collection
  of sub-Segments, expected to be set to one for the first and to increment
  by one for each new sub-Segment. If present, `descriptor_length` shall
  include it in the byte count and serve as the indication that it is
  present in the descriptor.
- **sub_segments_expected** — shall be present if `sub_segment_num` is
  present; a count of the expected number of individual sub-Segments within
  the collection. Same `descriptor_length` rule as `sub_segment_num`.

### device_restrictions — §10.3.3.1, Table 21, PDF p. 73

| Segmentation Message | device_restrictions |
|---|---|
| Restrict Group 0 | 00 |
| Restrict Group 1 | 01 |
| Restrict Group 2 | 10 |
| None | 11 |

Restrict Group 0/1/2 — this Segment is restricted for a class of devices
defined by an out-of-band message that describes which devices are excluded.
None — this Segment has no device restrictions.

### segmentation_upid_type — §10.3.3.1, Table 22, PDF pp. 74-75

| segmentation_upid_type | segmentation_upid_length (bytes) | segmentation_upid() (Name) | Description |
|---|---|---|---|
| 0x00 | 0 | Not Used | The segmentation_upid is not defined and is not present in the descriptor. |
| 0x01 | variable | User Defined | **Deprecated: use type 0x0C**; the segmentation_upid does not follow a standard naming scheme. |
| 0x02 | 8 | ISCI | **Deprecated: use type 0x03**, 8 characters; 4 alpha characters followed by 4 numbers. |
| 0x03 | 12 | Ad-ID | Defined by the Advertising Digital Identification, LLC group. 12 characters; 4 alpha characters (company identification prefix) followed by 8 alphanumeric characters. (See [Ad-ID], [SMPTE 2092-1].) |
| 0x04 | 32 | UMID | See [SMPTE 330]. |
| 0x05 | 8 | ISAN | **Deprecated: use type 0x06**, ISO 15706 binary encoding. |
| 0x06 | 12 | ISAN | Formerly known as V-ISAN. ISO 15706-2 binary encoding ("versioned" ISAN). See [ISO 15706-2]. |
| 0x07 | 12 | TID | Tribune Media Systems Program identifier. 12 characters; 2 alpha characters followed by 10 numbers. |
| 0x08 | 8 | TI | AiringID (formerly Turner ID), used to indicate a specific airing of a Program that is unique within a network. |
| 0x09 | variable | ADI | CableLabs metadata identifier as defined in §10.3.3.2: `<element>:<providerID>/<assetID>`, where `<element>` is one of PREVIEW, MPEG2HD, MPEG2SD, AVCHD, AVCSD, HEVCSD, HEVCHD, SIGNAL, PO (PlacementOpportunity), BLACKOUT, BREAK, OTHER, in 7-bit printable ASCII (0x20–0x7E). |
| 0x0A | 12 | EIDR | An EIDR (see [EIDR]) represented in Compact Binary encoding as defined in Section 2.1.1 of the EIDR ID Format (see [EIDR ID FORMAT]). [SMPTE 2079] |
| 0x0B | variable | ATSC Content Identifier | ATSC_content_identifier() structure as defined in [ATSC A/57B]. |
| 0x0C | variable | MPU() | Managed Private UPID structure as defined in §10.3.3.3 (Table 24 below). |
| 0x0D | variable | MID() | Multiple UPID types structure as defined in §10.3.3.4 (Table 25 below). |
| 0x0E | variable | ADS Information | Advertising information as described in §10.3.3.5 (key=value pairs, e.g. `type=LA&dur=60&pos=90&tier=1`). |
| 0x0F | variable | URI | Universal Resource Identifier (see [RFC 3986]). |
| 0x10 | 16 | UUID | Universally Unique Identifier (see [RFC 4122]). This segmentation_upid_type can be used instead of a URI if it is desired to transfer the UUID payload only. |
| 0x11 | variable | SCR | Segment Content Reference as described in §10.3.3.6 (key=value pairs, e.g. `type=PI&tier=1`). |
| 0x12–0xFF | variable | Reserved | Reserved for future standardization. |

ADS (0x0E) and SCR (0x11) key-value conventions (§10.3.3.5 / §10.3.3.6):
key-value separator `=`, key-value delimiter `&`, serial separator `;`. ADS
keys: `type` (LA = Local Operator Availability, NA = National Addressable,
NR = National Replacement, CV = Creative Versioning, C3/C7 = Nielsen VOD
ratings credit for 3/7 days), `pos` (start point in milliseconds of the
referenced advertising relative to the start of the Segment), `dur`
(duration in milliseconds), `tier` (replacement authorization designation).

### segmentation_type_id — §10.3.3.1, Table 23, PDF pp. 76-79

| Segmentation Message | segmentation_type_id hex (decimal) | segment_num | segments_expected | sub_segment_num | sub_segments_expected |
|---|---|---|---|---|---|
| Not Indicated | 0x00 (00) | 0 | 0 | Not used | Not used |
| Content Identification | 0x01 (01) | 0 | 0 | Not used | Not used |
| Private | 0x02 (02) |  |  |  |  |
| Reserved | 0x03–0x0F |  |  |  |  |
| Program Start | 0x10 (16) | 1 | 1 | Not used | Not used |
| Program End | 0x11 (17) | 1 | 1 | Not used | Not used |
| Program Early Termination | 0x12 (18) | 1 | 1 | Not used | Not used |
| Program Breakaway | 0x13 (19) | 1 | 1 | Not used | Not used |
| Program Resumption | 0x14 (20) | 1 | 1 | Not used | Not used |
| Program Runover Planned | 0x15 (21) | 1 | 1 | Not used | Not used |
| Program Runover Unplanned | 0x16 (22) | 1 | 1 | Not used | Not used |
| Program Overlap Start | 0x17 (23) | 1 | 1 | Not used | Not used |
| Program Blackout Override | 0x18 (24) | 0 | 0 | Not used | Not used |
| Program Join | 0x19 (25) | 1 | 1 | Not used | Not used |
| Program Immediate Resumption | 0x1A (26) | 0 | 0 | Not used | Not used |
| Reserved | 0x1B–0x1F |  |  |  |  |
| Chapter Start | 0x20 (32) | Non-zero | Non-zero | Not used | Not used |
| Chapter End | 0x21 (33) | Non-zero | Non-zero | Not used | Not used |
| Break Start | 0x22 (34) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Break End | 0x23 (35) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Opening Credit Start (deprecated) | 0x24 (36) | 1 | 1 | Not used | Not used |
| Opening Credit End (deprecated) | 0x25 (37) | 1 | 1 | Not used | Not used |
| Closing Credit Start (deprecated) | 0x26 (38) | 1 | 1 | Not used | Not used |
| Closing Credit End (deprecated) | 0x27 (39) | 1 | 1 | Not used | Not used |
| Reserved | 0x28–0x2F |  |  |  |  |
| Provider Advertisement Start | 0x30 (48) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Provider Advertisement End | 0x31 (49) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Distributor Advertisement Start | 0x32 (50) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Distributor Advertisement End | 0x33 (51) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Provider Placement Opportunity Start | 0x34 (52) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Provider Placement Opportunity End | 0x35 (53) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Distributor Placement Opportunity Start | 0x36 (54) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Distributor Placement Opportunity End | 0x37 (55) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Provider Overlay Placement Opportunity Start | 0x38 (56) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Provider Overlay Placement Opportunity End | 0x39 (57) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Distributor Overlay Placement Opportunity Start | 0x3A (58) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Distributor Overlay Placement Opportunity End | 0x3B (59) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Provider Promo Start | 0x3C (60) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Provider Promo End | 0x3D (61) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Distributor Promo Start | 0x3E (62) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Distributor Promo End | 0x3F (63) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Unscheduled Event Start | 0x40 (64) | 0 | 0 | Not used | Not used |
| Unscheduled Event End | 0x41 (65) | 0 | 0 | Not used | Not used |
| Alternate Content Opportunity Start | 0x42 (66) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Alternate Content Opportunity End | 0x43 (67) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Provider Ad Block Start | 0x44 (68) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Provider Ad Block End | 0x45 (69) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Distributor Ad Block Start | 0x46 (70) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Distributor Ad Block End | 0x47 (71) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Reserved | 0x48–0x4F |  |  |  |  |
| Network Start | 0x50 (80) | 0 | 0 | Not used | Not used |
| Network End | 0x51 (81) | 0 | 0 | Not used | Not used |
| Reserved | 0x52–0xFF |  |  |  |  |

Notes (from Table 23):

1. Only one Program Overlap Start is allowed to be active at a time. A
   Program End shall occur before a subsequent Program Overlap Start can
   occur.
2. See [SCTE 223] for further usage of segmentation_type_id.
3. The opening credit and closing credit start/end types are deprecated. New
   implementations should use the Segment Content Reference for this
   purpose.

(The final reserved row is printed as "0x52-0FF" in the PDF; the intended
range is 0x52–0xFF.)

### MPU() — §10.3.3.3, Table 24, PDF p. 80

Managed Private UPID, `segmentation_upid_type` 0x0C.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `MPU() {` |  |  |
| &nbsp;&nbsp;format_identifier | 32 | uimsbf |
| &nbsp;&nbsp;private_data | N*8 | uimsbf |
| `}` |  |  |

- **format_identifier** — a 32-bit unique identifier as defined in ISO/IEC
  13818-1 and registered with the SMPTE Registration Authority.
- **private_data** — a variable length, byte-aligned set of data as defined
  by the registered owner of the `format_identifier` value. The length is
  defined by `segmentation_upid_length`, **which includes the
  format_identifier field length**.

### MID() — §10.3.3.4, Table 25, PDF pp. 80-81

Multiple UPID types structure, `segmentation_upid_type` 0x0D.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `MID() {` |  |  |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_upid_type | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_upid | N*8 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

- **segmentation_upid_type** — as defined in Table 22.
- **length** — segmentation_upid_length for the following
  segmentation_upid.
- **segmentation_upid** — segmentation_upid according to
  segmentation_upid_type as defined in Table 22.
- Note: the number of segmentation_upids present ("N") is not explicitly
  signaled; it is discovered by repeatedly parsing the fields above until
  the outer `segmentation_upid_length` is exhausted.

## time_descriptor() — §10.3.4, Table 27, PDF pp. 97-100

Optional extension to the splice_insert(), splice_null() and time_signal()
commands that allows a programmer's wall clock time to be sent to a client,
in the time format of the Precision Time Protocol [PTP] (TAI: no leap
seconds, unlike UTC; PTP uses the same epoch as Unix time, 00:00 January 1,
1970). For the highest accuracy, use with a command whose
`time_specified_flag` == 1. The repetition rate should be at least once
every 5 seconds; when it is the only descriptor in a time_signal() or
splice_null() command, the encoder should not insert a key frame.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `time_descriptor() {` |  |  |
| &nbsp;&nbsp;splice_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;identifier | 32 | uimsbf |
| &nbsp;&nbsp;TAI_seconds | 48 | uimsbf |
| &nbsp;&nbsp;TAI_ns | 32 | uimsbf |
| &nbsp;&nbsp;UTC_offset | 16 | uimsbf |
| `}` |  |  |

- **splice_descriptor_tag** — shall be **0x03**.
- **descriptor_length** — shall be **0x10**.
- **identifier** — shall be **0x43554549** (ASCII "CUEI").
- **TAI_seconds** — 48-bit TAI seconds value.
- **TAI_ns** — 32-bit TAI nanoseconds value.
- **UTC_offset** — 16 bits, used in the conversion from TAI time to UTC or
  NTP time per: `UTC seconds = TAI seconds − UTC_offset`;
  `NTP seconds = TAI seconds − UTC_offset + 2,208,988,800`.

## audio_descriptor() — §10.3.5, Table 28, PDF pp. 102-104

Implementation of a splice_descriptor() that dynamically signals the audios
actually in use in the stream, for programmers/MVPDs that do not support
dynamic signaling and for legacy audio formats that do not support it (see
[SCTE 248] §9.1.5). Shall only be used with a time_signal command and a
segmentation descriptor with the type Program_Start or
Program_Overlap_Start.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `audio_descriptor() {` |  |  |
| &nbsp;&nbsp;splice_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;identifier | 32 | uimsbf |
| &nbsp;&nbsp;audio_count | 4 | uimsbf |
| &nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;`for (i=0; i<audio_count; i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;component_tag | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;ISO_code | 24 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;Bit_Stream_Mode | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;Num_Channels | 4 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;Full_Srvc_Audio | 1 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

- **splice_descriptor_tag** — shall be **0x04**.
- **identifier** — shall be **0x43554549** (ASCII "CUEI").
- **audio_count** — the number of audio PIDs in the program.
- **component_tag** — an optional 8-bit value identifying the elementary PID
  stream containing the audio channel that follows; if used, the value shall
  be the same as in the stream_identifier_descriptor(). If not used, the
  value shall be **0xFF** and the stream order shall be inferred from the
  PMT audio order.
- **ISO_code** — a 3-byte language code defining the language of this audio
  service, corresponding to a registered language code in the Code column of
  the [ISO 639-2] registry.
- **Bit_Stream_Mode** — as per ATSC A/52 Table 5.7.
- **Num_Channels** — as per ATSC A/52 Table A4.5.
- **Full_Srvc_Audio** — 1 bit (from ATSC A/52 Annex A.4.3): '1' if this
  audio service is sufficiently complete to be presented to the listener
  without being combined with another audio service; '0' if it is not
  sufficiently complete (e.g. a visually impaired narrative service that
  must be combined with the music/effects/dialogue service).

## Encryption algorithm — §11.3, Table 29, PDF pp. 104-106

Encryption of the section (from `splice_command_type` through `E_CRC_32`) is
optional. Fixed key encryption (§11.2): the same key is provided to both
transmitter and receiver by unspecified means; up to 256 keys may be held,
selected by `cw_index`. If encryption is implemented, a receive device shall
implement all of the algorithms listed. All DES variants use a 64-bit key
(actually 56 bits plus a checksum) on 8-byte blocks; triple DES needs three
64-bit keys, one per pass ("standard" triple DES uses two keys, the first
and third identical). For DES-ECB and DES-CBC the encrypted data shall be a
multiple of 8 bytes from `splice_command_type` through `E_CRC_32` (pad with
the `alignment_stuffing` loop); DES-CBC uses an initial vector with a fixed
value of zero. NOTE (§11.3): FIPS Publication 46-3 was withdrawn on May 19,
2005, and implementers that require encryption may wish to use a user
private algorithm.

| Value | Encryption algorithm |
|---|---|
| 0 | No encryption |
| 1 | DES – ECB mode |
| 2 | DES – CBC mode |
| 3 | Triple DES EDE3 – ECB mode |
| 4–31 | Reserved |
| 32–63 | User private |

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

## Sample SCTE 35 Messages — §14, PDF pp. 112-121
_Informative. Transcribed verbatim from the vendored PDF via `pdftotext` (deterministic), 2026-06-13. Per the spec's §14 intro these decodes are "the output from the open source decoder available on github" (threefive). Each `Base64` string was machine-verified to decode to the stated byte length; the `Hex` shown is canonically re-derived from that base64 (identical to the spec's `Hex=` field for 14.1–14.7; for 14.8 the spec's printed hex line-wrapped, so the base64-derived hex is authoritative). These are the crate's known-vector test inputs._

### 14.1. time_signal – Placement Opportunity Start
This is an example of using the time_signal command with a segmentation descriptor. The programmer that created this message used the Web Delivery Allowed flag to indicate the broadcast Advertisements should be blacked out. This would tend to also indicate that digital ad insertion would be used to insert new Advertisements in their place. The programmer is also using the Segment number in a non-normative manner to indicate that there are Distributor Placement Opportunities within this Provider Placement Opportunity. A standardized method of doing this would be to use the MID format of the segmentation UPID type and insert a UPID type 0x0E (ADS) with this information. Also note the Tier value in this message is 0xfff as displayed on the output of a receiver. It is likely that the value of Tier in transmission had a different value that this receiver was authorized to receive, and the receiver obfuscated that by changing the value to 0xfff.

```text
2018-07-16 00:04:57 M274P29528596539
Hex=0xFC3034000000000000FFFFF00506FE72BD0050001E021C435545494800008E7FCF0001A599B00808000000002CA0A18A3402009AC9D17E
Base64=/DA0AAAAAAAA///wBQb+cr0AUAAeAhxDVUVJSAAAjn/PAAGlmbAICAAAAAAsoKGKNAIAmsnRfg==

Decoded length = 55
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 52
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x5
Time Signal
Time = 0x072bd0050 - 21388.766756
Descriptor Loop Length = 30
Segmentation Descriptor - Length=28
Segmentation Event ID = 0x4800008e
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 0
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
Segmentation Duration = 0x0001a599b0 = 307.000000 seconds
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca0a18a
Type = Placement Opportunity Start
Segment num = 2 Segments Expected = 0
CRC32 = 0x9ac9d17e
```

### 14.2. splice_insert
This is the legacy standard for a Distributor Placement Opportunity. As a significant number of existing ad servers will not respond to the newer time_signal command, it is likely that this message will be in use until the legacy components are removed and replaced. The programmer that generated this message uses the Break duration and auto return mode for the splice_insert. The Break duration at 60.293567 seconds is slightly longer than the contracted 60 second local avail; it is, however, the exact duration of the content that the local avail will overlay. This means that the encoder will generate a key frame at that specified Break duration from the splice time and the affiliate should fill the duration with a slate or black (in some countries a blue color is used). Some splicers or fragmented file delivery systems may be able to adjust the duration and boundaries to match the key frames as well.

```text
2018-07-16 00:06:59 M274P29540838841
Hex=0xFC302F000000000000FFFFF014054800008F7FEFFE7369C02EFE0052CCF500000000000A0008435545490000013562DBA30A
Base64=/DAvAAAAAAAA///wFAVIAACPf+/+c2nALv4AUsz1AAAAAAAKAAhDVUVJAAABNWLbowo=

Decoded length = 50
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 47
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x14
Splice Insert
Splice Event ID = 0x4800008f
Flags OON=1 Prog=1 Duration=1 Immediate=0
Splice time = 0x07369c02e - 21514.559089
Auto Return
break duration = 0x00052ccf5 = 60.293567 seconds
Unique Program ID = 0
Avail Num = 0
Avails Expected = 0
Descriptor Loop Length = 10
Avail Descriptor - Length=8
Avail Descriptor = 0x00000135 - 309
CRC32 = 0x62dba30a
```

### 14.3. time_signal – Placement Opportunity End

```text
2018-07-16 00:10:04 M274P29559224252
Hex=0xFC302F000000000000FFFFF00506FE746290A000190217435545494800008E7F9F0808000000002CA0A18A350200A9CC6758
Base64=/DAvAAAAAAAA///wBQb+dGKQoAAZAhdDVUVJSAAAjn+fCAgAAAAALKChijUCAKnMZ1g=

Decoded length = 50
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 47
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x5
Time Signal
Time = 0x0746290a0 - 21695.740089
Descriptor Loop Length = 25
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x4800008e
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca0a18a
Type = Placement Opportunity End
Segment num = 2 Segments Expected = 0
CRC32 = 0xa9cc6758
```

### 14.4. time_signal – Program Start/End

```text
2018-07-16 00:00:15 M274P29500484335
Hex=0xFC3048000000000000FFFFF00506FE7A4D88B60032021743554549480000187F9F0808000000002CCBC344110000021743554549480000197F9F0808000000002CA4DBA01000009972E343
Base64=/DBIAAAAAAAA///wBQb+ek2ItgAyAhdDVUVJSAAAGH+fCAgAAAAALMvDRBEAAAIXQ1VFSUgAABl/nwgIAAAAACyk26AQAACZcuND

Decoded length = 75
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 72
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x5
Time Signal
Time = 0x07a4d88b6 - 22798.906911
Descriptor Loop Length = 50
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x48000018
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ccbc344
Type = Program End
Segment num = 0 Segments Expected = 0
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x48000019
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca4dba0
Type = Program Start
Segment num = 0 Segments Expected = 0
CRC32 = 0x9972e343
```

### 14.5. time_signal – Program Overlap Start

```text
2018-07-16 02:59:52 M274P30575324060
Hex=0xFC302F000000000000FFFFF00506FEAEBFFF640019021743554549480000087F9F0808000000002CA56CF5170000951DB0A8
Base64=/DAvAAAAAAAA///wBQb+rr//ZAAZAhdDVUVJSAAACH+fCAgAAAAALKVs9RcAAJUdsKg=

Decoded length = 50
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 47
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x5
Time Signal
Time = 0x0aebfff64 - 32575.759333
Descriptor Loop Length = 25
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x48000008
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca56cf5
Type = Program Overlap Start
Segment num = 0 Segments Expected = 0
CRC32 = 0x951db0a8
```

### 14.6. time_signal – Program Blackout Override / Program End
Since the restriction flags are not evaluated on an End message, the use of the Program blackout override can be used in the case of an overlap start or other condition where the restrictions may need to be changed during a Program playback.

```text
2018-07-16 01:45:45 M274P30131806863
Hex=0xFC3048000000000000FFFFF00506FE932E380B00320217435545494800000A7F9F0808000000002CA0A1E3180000021743554549480000097F9F0808000000002CA0A18A110000B4217EB0
Base64=/DBIAAAAAAAA///wBQb+ky44CwAyAhdDVUVJSAAACn+fCAgAAAAALKCh4xgAAAIXQ1VFSUgAAAl/nwgIAAAAACygoYoRAAC0IX6w

Decoded length = 75
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 72
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x5
Time Signal
Time = 0x0932e380b - 27436.441722
Descriptor Loop Length = 50
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x4800000a
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca0a1e3
Type = Program Blackout Override
Segment num = 0 Segments Expected = 0
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x48000009
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca0a18a
Type = Program End
Segment num = 0 Segments Expected = 0
CRC32 = 0xb4217eb0
```

### 14.7. time_signal – Program End

```text
2018-07-16 03:00:28 M274P30578915636
Hex=0xFC302F000000000000FFFFF00506FEAEF17C4C0019021743554549480000077F9F0808000000002CA56C97110000C4876A2E
Base64=/DAvAAAAAAAA///wBQb+rvF8TAAZAhdDVUVJSAAAB3+fCAgAAAAALKVslxEAAMSHai4=

Decoded length = 50
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 47
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x5
Time Signal
Time = 0x0aef17c4c - 32611.795333
Descriptor Loop Length = 25
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x48000007
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca56c97
Type = Program End
Segment num = 0 Segments Expected = 0
CRC32 = 0xc4876a2e
```

### 14.8. time_signal – Program Start/End - Placement Opportunity End
This is a complex message, although one that can occur frequently as many ad Breaks are placed at the end of the Program. The implementer should take care though to find the length and current practice is to try and keep the message in a single transport packet.

```text
2018-07-16 03:00:33 M274P30579401569
Hex=0xFC3061000000000000FFFFF00506FEA8CD44ED004B021743554549480000AD7F9F0808000000002CB2D79D350200021743554549480000267F9F0808000000002CB2D79D110000021743554549480000277F9F0808000000002CB2D7B31000008A18869F
Base64=/DBhAAAAAAAA///wBQb+qM1E7QBLAhdDVUVJSAAArX+fCAgAAAAALLLXnTUCAAIXQ1VFSUgAACZ/nwgIAAAAACyy150RAAACF0NVRUlIAAAnf58ICAAAAAAsstezEAAAihiGnw==

Decoded length = 100
Table ID = 0xFC
MPEG Short Section
```
