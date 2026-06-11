# ISO/IEC 13818-1:2007 (MPEG-2 Systems) — adaptation field, PCR, and PSI section framing

**Provenance.** ISO/IEC 13818-1:2007 (identical text: ITU-T Rec. H.222.0
(05/2006)) is a paid ISO standard and cannot be vendored into this repository
— the PDF consulted for this transcription lives locally at
`specs/iso_iec_13818-1_2007_systems.pdf` and is deliberately **not** committed
(gitignored under `specs/iso_iec_*.pdf`). The syntax tables and semantics
below were hand-transcribed from that PDF on 2026-06-09. They are
cross-checkable against the live broadcast captures in `dvb-si/tests/fixtures/`
— both `.ts` fixtures carry adaptation fields with PCRs. PDF page numbers
cited below are the PDF file's own page indices (the printed spec page is
PDF page minus 12).

## Contents

- [Table 2-2 — Transport Stream packet (§2.4.3.2–2.4.3.3)](#table-2-2--transport-stream-packet)
- [Table 2-6 — Adaptation field (§2.4.3.4)](#table-2-6--adaptation-field)
- [Adaptation field semantics (§2.4.3.5)](#adaptation-field-semantics)
- [PCR arithmetic (§2.4.2.1–2.4.2.2) and coding frequency (§2.7.2)](#pcr-arithmetic-and-coding-frequency)
- [PSI section carriage and pointer_field (§2.4.4–2.4.4.2)](#psi-section-carriage-and-pointer_field)
- [Table 2-30 — Program association section (§2.4.4.3–2.4.4.4)](#table-2-30--program-association-section)
- [Table 2-33 — TS program map section (§2.4.4.8)](#table-2-33--ts-program-map-section)

## Table 2-2 — Transport Stream packet
_§2.4.3.2–2.4.3.3, PDF pp. 31-33_

Transport Stream packets shall be 188 bytes long (§2.4.3).

| Syntax | Bits | Mnemonic |
|---|---|---|
| transport_packet() { |  |  |
| sync_byte | 8 | bslbf |
| transport_error_indicator | 1 | bslbf |
| payload_unit_start_indicator | 1 | bslbf |
| transport_priority | 1 | bslbf |
| PID | 13 | uimsbf |
| transport_scrambling_control | 2 | bslbf |
| adaptation_field_control | 2 | bslbf |
| continuity_counter | 4 | uimsbf |
| if(adaptation_field_control = = '10' \|\| adaptation_field_control = = '11'){ |  |  |
| adaptation_field() |  |  |
| } |  |  |
| if(adaptation_field_control = = '01' \|\| adaptation_field_control = = '11') { |  |  |
| for (i = 0; i < N; i++){ |  |  |
| data_byte | 8 | bslbf |
| } |  |  |
| } |  |  |
| } |  |  |

Cross-check notes (§2.4.3.3):

- `sync_byte` is fixed `'0100 0111'` (0x47).
- `payload_unit_start_indicator` for PSI: `'1'` iff the packet carries the
  first byte of a PSI section, in which case the first payload byte is the
  pointer_field; `'0'` means no pointer_field is present. For null packets it
  shall be `'0'`. (Full PSI rules under
  [pointer_field](#psi-section-carriage-and-pointer_field) below.)
- `adaptation_field_control` (Table 2-5): `'00'` reserved (decoders shall
  discard such packets), `'01'` payload only, `'10'` adaptation_field only,
  `'11'` adaptation_field followed by payload. Null packets shall use `'01'`.
- `continuity_counter` increments per packet of the same PID, wraps to 0; it
  shall **not** be incremented when adaptation_field_control is `'00'` or
  `'10'`. Duplicate packets: at most two consecutive packets of the same PID
  with the same continuity_counter, adaptation_field_control `'01'` or `'11'`,
  byte-identical except that a PCR, if present, shall carry a valid value.
- `data_byte` count N = 184 minus the bytes of the adaptation_field().
- PID 0x0000 = PAT, 0x0001 = CAT, 0x0002 = TSDT, 0x0003 = IPMP control
  information, 0x0004–0x000F reserved, 0x1FFF = null packets (Table 2-3).
  Packets with PID 0x0000, 0x0001 and 0x0010–0x1FFE are allowed to carry a
  PCR (Table 2-3 NOTE).

## Table 2-6 — Adaptation field
_§2.4.3.4, PDF pp. 33-34_

| Syntax | Bits | Mnemonic |
|---|---|---|
| adaptation_field() { |  |  |
| adaptation_field_length | 8 | uimsbf |
| if (adaptation_field_length > 0) { |  |  |
| discontinuity_indicator | 1 | bslbf |
| random_access_indicator | 1 | bslbf |
| elementary_stream_priority_indicator | 1 | bslbf |
| PCR_flag | 1 | bslbf |
| OPCR_flag | 1 | bslbf |
| splicing_point_flag | 1 | bslbf |
| transport_private_data_flag | 1 | bslbf |
| adaptation_field_extension_flag | 1 | bslbf |
| if (PCR_flag = = '1') { |  |  |
| program_clock_reference_base | 33 | uimsbf |
| reserved | 6 | bslbf |
| program_clock_reference_extension | 9 | uimsbf |
| } |  |  |
| if (OPCR_flag = = '1') { |  |  |
| original_program_clock_reference_base | 33 | uimsbf |
| reserved | 6 | bslbf |
| original_program_clock_reference_extension | 9 | uimsbf |
| } |  |  |
| if (splicing_point_flag = = '1') { |  |  |
| splice_countdown | 8 | tcimsbf |
| } |  |  |
| if (transport_private_data_flag = = '1') { |  |  |
| transport_private_data_length | 8 | uimsbf |
| for (i = 0; i < transport_private_data_length; i++) { |  |  |
| private_data_byte | 8 | bslbf |
| } |  |  |
| } |  |  |
| if (adaptation_field_extension_flag = = '1') { |  |  |
| adaptation_field_extension_length | 8 | uimsbf |
| ltw_flag | 1 | bslbf |
| piecewise_rate_flag | 1 | bslbf |
| seamless_splice_flag | 1 | bslbf |
| reserved | 5 | bslbf |
| if (ltw_flag = = '1') { |  |  |
| ltw_valid_flag | 1 | bslbf |
| ltw_offset | 15 | uimsbf |
| } |  |  |
| if (piecewise_rate_flag = = '1') { |  |  |
| reserved | 2 | bslbf |
| piecewise_rate | 22 | uimsbf |
| } |  |  |
| if (seamless_splice_flag = = '1') { |  |  |
| splice_type | 4 | bslbf |
| DTS_next_AU`[32..30]` | 3 | bslbf |
| marker_bit | 1 | bslbf |
| DTS_next_AU`[29..15]` | 15 | bslbf |
| marker_bit | 1 | bslbf |
| DTS_next_AU`[14..0]` | 15 | bslbf |
| marker_bit | 1 | bslbf |
| } |  |  |
| for (i = 0; i < N; i++) { |  |  |
| reserved | 8 | bslbf |
| } |  |  |
| } |  |  |
| for (i = 0; i < N; i++) { |  |  |
| stuffing_byte | 8 | bslbf |
| } |  |  |
| } |  |  |
| } |  |  |

(The PDF prints `Reserved`/`Splice_type` with inconsistent capitalization in
Table 2-6; lowercased here per the field semantics in §2.4.3.5.)

## Adaptation field semantics
_§2.4.3.5, PDF pp. 34-38_

**adaptation_field_length** — number of bytes in the adaptation_field
immediately following the adaptation_field_length field. Value 0 inserts a
single stuffing byte. When adaptation_field_control is `'11'`, the value
shall be in the range **0 to 182**; when `'10'`, it shall be **183**. For
packets carrying PES packets, stuffing is accomplished by defining an
adaptation field longer than the sum of the lengths of its data elements and
filling the extra space with stuffing bytes — this is the **only** stuffing
method allowed for TS packets carrying PES packets. (PSI packets use the
alternative 0xFF stuffing of §2.4.4 instead.)

**discontinuity_indicator** — `'1'` means the discontinuity state is true for
the current packet (`'0'` or absent = false). Two discontinuity types:

- *System time-base discontinuity* — indicated in packets of a PID designated
  as a PCR_PID (§2.4.4.9). When true, the **next PCR** in a packet of that
  PID is a sample of a **new system time clock** for the program. The
  discontinuity point is the arrival instant (T-STD input) of the first byte
  of the packet containing the new time-base PCR. The indicator shall be
  `'1'` in the packet in which the discontinuity occurs; it may also be `'1'`
  in earlier packets of the same PCR_PID, in which case it shall stay `'1'`
  in every packet of that PID up to and including the packet carrying the
  first PCR of the new time base. After a discontinuity, **no fewer than two
  PCRs** of the new time base shall be received before another time-base
  discontinuity can occur; except in trick mode, data from no more than two
  time bases may be present in the T-STD buffers of one program at any time.
  No PTS/DTS of the new time base may arrive before the discontinuity, and
  none of the old time base after it.
- *Continuity-counter discontinuity* — may be signalled in any packet. For a
  non-PCR_PID, when the state is true the continuity_counter may be
  discontinuous with respect to the previous packet of that PID. For a
  PCR_PID, the counter may only be discontinuous in the packet where the
  time-base discontinuity occurs. At most **one** continuity-counter
  discontinuity point per discontinuity state. For non-PCR_PIDs, the
  indicator may be `'1'` in the next packet of the same PID, but shall not be
  `'1'` in **three consecutive** packets of the same PID. After a
  continuity-counter discontinuity in an elementary-stream PID, the first
  byte of elementary stream data shall be the first byte of an elementary
  stream access point (video: sequence header / visual object sequence header
  / AVC access unit, optionally preceded by a sequence_end_code; audio: first
  byte of an audio frame).
- While the discontinuity state is true, if two consecutive packets of the
  same PID have the same continuity_counter and adaptation_field_control
  `'01'`/`'11'`, the second may be discarded; the stream shall not be
  constructed so that discarding it loses PES payload or PSI data.
- PSI: after a discontinuity_indicator `'1'` in a packet carrying PSI, a
  single version_number discontinuity may occur; at it, a
  TS_program_map_section with **section_length == 13**,
  current_next_indicator == 1 (no descriptors, no streams) shall be sent,
  followed by a complete program definition with version_number incremented
  by one.

**random_access_indicator** — `'1'`: the next PES packet to start in this
PID's payload shall contain an elementary stream access point (and for video,
a PTS for the first picture after it; for audio, the PTS shall be in the PES
packet containing the first byte of the audio frame). In the PCR_PID it may
only be set to `'1'` in packets containing the PCR fields.

**elementary_stream_priority_indicator** — `'1'`: this payload has higher
priority among packets of the same PID (MPEG-2 video: only if the payload
contains bytes of an intra-coded slice; AVC: only slice_type 2, 4, 7 or 9).

**PCR_flag / OPCR_flag** — `'1'` indicates that the adaptation field contains
a PCR / OPCR field coded in two parts.

**splicing_point_flag** — `'1'` indicates a splice_countdown field is present,
specifying the occurrence of a splicing point.

**transport_private_data_flag** — `'1'` indicates one or more private_data
bytes are present.

**adaptation_field_extension_flag** — `'1'` indicates the presence of the
adaptation field extension.

**program_clock_reference_base; program_clock_reference_extension** — the
PCR is a 42-bit field coded in two parts (base per equation 2-2, extension
per equation 2-3 — see [PCR arithmetic](#pcr-arithmetic-and-coding-frequency)).
The PCR indicates the intended time of arrival of the byte containing the
**last bit of program_clock_reference_base** at the input of the system
target decoder.

**original_program_clock_reference_base/_extension (OPCR)** — coded
identically to the PCR; shall be coded only in packets in which the PCR is
present. Assists reconstruction of a single-program TS from another TS (copy
OPCR → PCR, valid only if the original stream is reconstructed exactly in its
entirety). `OPCR(i) = OPCR_base(i) × 300 + OPCR_ext(i)` (2-8). Ignored by
decoders; shall not be modified by any multiplexor or decoder.

**splice_countdown** — 8-bit signed (tcimsbf). Positive: the number of
remaining packets of the same PID until the splicing point (duplicates and
adaptation-field-only packets excluded); the splicing point is immediately
after the last byte of the packet in which splice_countdown reaches zero,
whose last payload byte shall be the last byte of a coded audio frame or
coded picture. The next payload-bearing packet of the PID shall start with
the first byte of a PES packet whose payload commences with an access point
(video: or a sequence_end_code followed by one). Value −n: this packet is the
n-th packet following the splicing point.

**transport_private_data_length** — number of private_data bytes immediately
following this field; private data shall not extend beyond the adaptation
field.

**adaptation_field_extension_length** — number of bytes of extended
adaptation field data immediately following this field, including reserved
bytes if present.

**ltw_flag / ltw_valid_flag / ltw_offset** — ltw_offset (15 bits, defined
only when ltw_valid_flag = `'1'`) is the legal-time-window offset in units of
(300/f_s) seconds (f_s = the program's system clock frequency): the upper
bound t1(i) of the Legal Time Window for this packet minus the packet's
T-STD arrival time t(i). Intended for remultiplexers reconstructing MBn
buffer state.

**piecewise_rate** — 22-bit positive integer; defined only when both ltw_flag
and ltw_valid_flag are `'1'`. Hypothetical bitrate R used to extrapolate the
Legal Time Window end times of following packets of the same PID that carry
no ltw_offset: t1(A_{i+j}) = t1(A_i) + j × 188 × 8 / R.

**seamless_splice_flag** — `'1'` indicates splice_type and DTS_next_AU are
present. Shall not be `'1'` where splicing_point_flag is not `'1'`; once set
in a packet with positive splice_countdown, it shall remain set in all
subsequent packets of the PID with splicing_point_flag `'1'` until the
splice_countdown-zero packet inclusive. splice_type shall be `'0000'` unless
the PID carries H.262 video (then it indexes the splice constraint Tables
2-7…2-20 and shall keep the same value until splice_countdown reaches zero).
DTS_next_AU (33 bits across three marker-delimited parts) is the decoding
time of the first access unit after the splicing point.

**stuffing_byte** — fixed 8-bit value `'1111 1111'` (0xFF), discarded by the
decoder.

## PCR arithmetic and coding frequency
_§2.4.2.1–2.4.2.2, PDF pp. 23-24; §2.7.2, PDF p. 106_

The PCR field is encoded in two parts: program_clock_reference_base in units
of 1/300 of the system clock frequency, and program_clock_reference_extension
in units of the system clock frequency. The value encoded indicates the time
t(i), where i is the index of the byte containing the last bit of the
program_clock_reference_base field:

```
PCR(i)      = PCR_base(i) × 300 + PCR_ext(i)                          (2-1)
PCR_base(i) = ((system_clock_frequency × t(i)) DIV 300) % 2^33        (2-2)
PCR_ext(i)  = ((system_clock_frequency × t(i)) DIV 1)   % 300         (2-3)
```

System clock frequency constraints (§2.4.2.1):

```
27 000 000 − 810 ≤ system_clock_frequency ≤ 27 000 000 + 810   (Hz)
rate of change of system_clock_frequency with time ≤ 75 × 10⁻³ Hz/s
```

i.e. the system clock is nominally **27 MHz**; PCR_base ticks at 90 kHz
(27 MHz / 300) and PCR_ext counts the 0–299 remainder of 27 MHz cycles.
Between PCRs, byte arrival times are interpolated linearly at the transport
rate (equations 2-4/2-5). The **PCR tolerance** — the maximum inaccuracy
allowed in received PCRs (imprecision or remultiplexing modification, not
network jitter) — is **± 500 ns** (§2.4.2.2).

**§2.7.2 Frequency of coding the program clock reference.** The Transport
Stream shall be constructed such that the time interval between the bytes
containing the last bit of program_clock_reference_base fields in successive
occurrences of the PCRs in TS packets of the PCR_PID for each program shall
be **less than or equal to 0.1 s**:

```
t(i) − t(i′) ≤ 0.1 s
```

for all consecutive PCR pairs of the PCR_PID. There shall be **at least two
(2) PCRs** from the specified PCR_PID between consecutive PCR
discontinuities (refer to §2.4.3.4) to facilitate phase locking and
extrapolation of byte delivery times.

## PSI section carriage and pointer_field
_§2.4.4–2.4.4.2, PDF pp. 54-55_

Carriage rules for PSI sections in TS packets (§2.4.4 intro — these ground a
section→TS packetizer):

- Sections may be variable in length; the beginning of a section is indicated
  by a **pointer_field** in the TS packet payload. Adaptation fields may
  occur in TS packets carrying PSI sections.
- **Stuffing**: packet stuffing bytes of value **0xFF** may appear in the
  payload of packets carrying PSI and/or private_sections **only after the
  last byte of a section**; in that case **all** bytes until the end of the
  packet shall also be 0xFF (decoders may discard them), and the payload of
  the **next** packet with the same PID shall begin with a pointer_field of
  value **0x00** (next section starts immediately thereafter).
- Maximum section sizes: **1024 bytes** for an ISO/IEC 13818-1-defined PSI
  table section, **4096 bytes** for a private_section.
- PAT: every TS shall contain packets with PID 0x0000 which together carry a
  complete PAT; only table_id 0x00 sections are permitted on PID 0x0000. CAT
  (PID 0x0001, table_id 0x01) is required whenever any elementary stream is
  scrambled. Each program in the PAT shall be described in a unique
  TS_program_map_section (table_id 0x02); a program definition shall not span
  more than one TS_program_map_section, and the program_map_PID shall not
  change during the continuous existence of a program. A new table version
  becomes valid when the last byte of the needed section(s), with new
  version_number and current_next_indicator `'1'`, exits B_sys (T-STD).
- The NIT is optional and private; if present it is listed in the PAT under
  reserved program_number 0x0000 and takes the form of private_sections.
- There are no restrictions on the occurrence of start codes, sync bytes or
  other bit patterns in PSI data.

### Table 2-29 — Program specific information pointer
_§2.4.4.1, PDF p. 55_

| Syntax | Bits | Mnemonic |
|---|---|---|
| pointer_field | 8 | uimsbf |

**pointer_field** (§2.4.4.2) — the number of bytes, immediately following the
pointer_field, until the first byte of the **first section that is present**
in the payload of the TS packet (0x00 = the section starts immediately after
the pointer_field). When at least one section begins in a given packet, the
payload_unit_start_indicator shall be `'1'` and the first byte of the payload
shall contain the pointer. When no section begins in the packet, the
payload_unit_start_indicator shall be `'0'` and no pointer shall be sent.

## Table 2-30 — Program association section
_§2.4.4.3–2.4.4.4, PDF pp. 55-57_

| Syntax | Bits | Mnemonic |
|---|---|---|
| program_association_section() { |  |  |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | bslbf |
| '0' | 1 | bslbf |
| reserved | 2 | bslbf |
| section_length | 12 | uimsbf |
| transport_stream_id | 16 | uimsbf |
| reserved | 2 | bslbf |
| version_number | 5 | uimsbf |
| current_next_indicator | 1 | bslbf |
| section_number | 8 | uimsbf |
| last_section_number | 8 | uimsbf |
| for (i = 0; i < N; i++) { |  |  |
| program_number | 16 | uimsbf |
| reserved | 3 | bslbf |
| if (program_number = = '0') { |  |  |
| network_PID | 13 | uimsbf |
| } |  |  |
| else { |  |  |
| program_map_PID | 13 | uimsbf |
| } |  |  |
| } |  |  |
| CRC_32 | 32 | rpchof |
| } |  |  |

Cross-check notes (§2.4.4.5): table_id = 0x00; section_syntax_indicator =
`'1'`; section_length's first two bits `'00'`, value shall not exceed
**1021 (0x3FD)**; version_number increments by 1 modulo 32 whenever the PAT
definition changes; program_number 0x0000 ⇒ network_PID, otherwise
program_map_PID; CRC_32 gives zero output of the Annex A decoder registers
over the entire section.

table_id assignments (§2.4.4.4, Table 2-31): 0x00 PAT, 0x01 CA_section,
0x02 TS_program_map_section, 0x03 TS_description_section, 0x04
ISO_IEC_14496_scene_description_section, 0x05
ISO_IEC_14496_object_descriptor_section, 0x06 Metadata_section, 0x07
IPMP_Control_Information_section, 0x08–0x3F ISO/IEC 13818-1 reserved,
0x40–0xFE user private, 0xFF forbidden.

## Table 2-33 — TS program map section
_§2.4.4.8, PDF pp. 58-59_

| Syntax | Bits | Mnemonic |
|---|---|---|
| TS_program_map_section() { |  |  |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | bslbf |
| '0' | 1 | bslbf |
| reserved | 2 | bslbf |
| section_length | 12 | uimsbf |
| program_number | 16 | uimsbf |
| reserved | 2 | bslbf |
| version_number | 5 | uimsbf |
| current_next_indicator | 1 | bslbf |
| section_number | 8 | uimsbf |
| last_section_number | 8 | uimsbf |
| reserved | 3 | bslbf |
| PCR_PID | 13 | uimsbf |
| reserved | 4 | bslbf |
| program_info_length | 12 | uimsbf |
| for (i = 0; i < N; i++) { |  |  |
| descriptor() |  |  |
| } |  |  |
| for (i = 0; i < N1; i++) { |  |  |
| stream_type | 8 | uimsbf |
| reserved | 3 | bslbf |
| elementary_PID | 13 | uimsbf |
| reserved | 4 | bslbf |
| ES_info_length | 12 | uimsbf |
| for (i = 0; i < N2; i++) { |  |  |
| descriptor() |  |  |
| } |  |  |
| } |  |  |
| CRC_32 | 32 | rpchof |
| } |  |  |

Cross-check notes (§2.4.4.9): table_id = 0x02; section_syntax_indicator =
`'1'`; section_length first two bits `'00'`, value shall not exceed
**1021 (0x3FD)**; one program definition per TS_program_map_section (so a
program definition is never longer than **1016 (0x3F8)** bytes);
**section_number and last_section_number shall both be 0x00**; version_number
refers to the definition of a single program (single section), incremented by
1 modulo 32 on change; PCR_PID = **0x1FFF** when no PCR is associated with
the program definition (private streams); program_info_length and
ES_info_length each have their first two bits `'00'`, remaining 10 bits give
the descriptor-loop byte count.

## Table 2-34 — Stream type assignments

> **Source provenance:** transcribed from **Rec. ITU-T H.222.0 (06/2021), Table
> 2-34** — the free-of-charge ITU-T text that is technically identical to the
> paid ISO/IEC 13818-1 (8th edition). ITU-T Recommendations are published at no
> cost (<https://www.itu.int/rec/T-REC-H.222.0>); the PDF is consulted locally
> (gitignored under `specs/iso_iec_*.pdf` per the ISO non-redistribution
> posture) and only this transcription is committed. The 2021 edition assigns
> through `0x35` (EVC); `0x36`–`0x7E` are reserved, so it is authoritative for
> every codec stream_type in current broadcast use (HEVC, VVC, …).

| stream_type | Description |
|---|---|
| 0x00 | ITU-T \| ISO/IEC Reserved |
| 0x01 | ISO/IEC 11172-2 Video (MPEG-1 video) |
| 0x02 | Rec. ITU-T H.262 \| ISO/IEC 13818-2 Video, or ISO/IEC 11172-2 constrained-parameter video |
| 0x03 | ISO/IEC 11172-3 Audio (MPEG-1 audio) |
| 0x04 | ISO/IEC 13818-3 Audio (MPEG-2 audio) |
| 0x05 | Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 private_sections |
| 0x06 | Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 PES packets containing private data |
| 0x07 | ISO/IEC 13522 MHEG |
| 0x08 | Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 Annex A DSM-CC |
| 0x09 | Rec. ITU-T H.222.1 |
| 0x0A | ISO/IEC 13818-6 type A |
| 0x0B | ISO/IEC 13818-6 type B |
| 0x0C | ISO/IEC 13818-6 type C |
| 0x0D | ISO/IEC 13818-6 type D |
| 0x0E | Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 auxiliary |
| 0x0F | ISO/IEC 13818-7 Audio with ADTS transport syntax (AAC) |
| 0x10 | ISO/IEC 14496-2 Visual (MPEG-4 part 2) |
| 0x11 | ISO/IEC 14496-3 Audio with the LATM transport syntax (AAC LATM) |
| 0x12 | ISO/IEC 14496-1 SL-packetized / FlexMux stream carried in PES packets |
| 0x13 | ISO/IEC 14496-1 SL-packetized / FlexMux stream carried in ISO/IEC 14496 sections |
| 0x14 | ISO/IEC 13818-6 Synchronized Download Protocol |
| 0x15 | Metadata carried in PES packets |
| 0x16 | Metadata carried in metadata_sections |
| 0x17 | Metadata carried in ISO/IEC 13818-6 Data Carousel |
| 0x18 | Metadata carried in ISO/IEC 13818-6 Object Carousel |
| 0x19 | Metadata carried in ISO/IEC 13818-6 Synchronized Download Protocol |
| 0x1A | IPMP stream (ISO/IEC 13818-11, MPEG-2 IPMP) |
| 0x1B | AVC video stream (Rec. ITU-T H.264 \| ISO/IEC 14496-10, Annex A profiles) |
| 0x1C | ISO/IEC 14496-3 Audio, without additional transport syntax (DST, ALS, SLS) |
| 0x1D | ISO/IEC 14496-17 Text |
| 0x1E | Auxiliary video stream (ISO/IEC 23002-3) |
| 0x1F | SVC video sub-bitstream of an AVC video stream (H.264 Annex G) |
| 0x20 | MVC video sub-bitstream of an AVC video stream (H.264 Annex H) |
| 0x21 | Video stream conforming to Rec. ITU-T T.800 \| ISO/IEC 15444-1 (JPEG 2000) |
| 0x22 | Additional view Rec. ITU-T H.262 \| ISO/IEC 13818-2 video for service-compatible stereoscopic 3D |
| 0x23 | Additional view Rec. ITU-T H.264 \| ISO/IEC 14496-10 video for service-compatible stereoscopic 3D |
| 0x24 | Rec. ITU-T H.265 \| ISO/IEC 23008-2 video (HEVC) or an HEVC temporal video sub-bitstream |
| 0x25 | HEVC temporal video subset (Annex A profiles) |
| 0x26 | MVCD video sub-bitstream of an AVC video stream (H.264 Annex I) |
| 0x27 | Timeline and External Media Information Stream (TEMI, Annex U) |
| 0x28 | HEVC enhancement sub-partition incl. TemporalId 0 (H.265 Annex G) |
| 0x29 | HEVC temporal enhancement sub-partition (H.265 Annex G) |
| 0x2A | HEVC enhancement sub-partition incl. TemporalId 0 (H.265 Annex H) |
| 0x2B | HEVC temporal enhancement sub-partition (H.265 Annex H) |
| 0x2C | Green access units carried in MPEG-2 sections |
| 0x2D | ISO/IEC 23008-3 Audio with MHAS transport syntax — main stream |
| 0x2E | ISO/IEC 23008-3 Audio with MHAS transport syntax — auxiliary stream |
| 0x2F | Quality access units carried in sections |
| 0x30 | Media Orchestration Access Units carried in sections |
| 0x31 | Substream of an H.265 \| ISO/IEC 23008-2 video stream containing a Motion-Constrained Tile Set (MCTS) |
| 0x32 | JPEG XS video stream (ISO/IEC 21122-2 profiles) |
| 0x33 | VVC video stream (Rec. ITU-T H.266 \| ISO/IEC 23090-3) or a VVC temporal video sub-bitstream |
| 0x34 | VVC temporal video subset (H.266 Annex A profiles) |
| 0x35 | EVC video stream or an EVC temporal video sub-bitstream (ISO/IEC 23094-1) |
| 0x36–0x7E | Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 reserved |
| 0x7F | IPMP stream |
| 0x80–0xFF | User Private (incl. by industry convention: 0x81 ATSC AC-3, 0x86 SCTE-35 splice_info, 0x87 ATSC E-AC-3 — these are NOT assigned by H.222.0 and must be cited to their own specs) |
