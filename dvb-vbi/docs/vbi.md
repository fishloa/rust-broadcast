# VBI data carriage in DVB (PES data field)

_Source: ETSI EN 301 775 V1.2.1 (2003-05) §4, Tables 1–13 (PDF pp. 8–13), render-verified_

ETSI EN 301 775 specifies how Vertical Blanking Information (VBI) data is carried
in MPEG-2 / DVB Transport Streams, to be transcoded into the VBI of the companion
decoded video (or interpreted directly). It extends EN 300 472 (EBU Teletext
carriage) with **Inverted Teletext**, **VPS** (EN 300 231), **WSS** (EN 300 294),
**Closed Captioning** (line 21, EIA-608 Rev A), and a generic **monochrome 4:2:2
luminance-sample** transport.

The data uses the private PES packet mechanism, `stream_id = private_stream_1`
(`1011 1101` = `0xBD`), `stream_type = 0x06` (PES carrying private data). The
stream is identified in the PMT by the `VBI_data_descriptor` (EN 300 468); a
`VBI_teletext_descriptor` is additionally present iff the stream also carries EBU
Teletext. EBU Teletext (EN 300 706) itself is out of scope here — the value of
this spec is the carriage layer plus the VPS / WSS / CC / monochrome data units.

> §4.5 (PDF p. 9): "The `txt_data_block` in this definition is a combination of
> magazine_and_packet_address and data_block. Inverted Teletext is coded using the
> same definition as EBU Teletext."

## PES packet constraints (§4.3, PDF p. 7)

A VBI PES packet contains data of one and only one video frame and always carries
a PTS. The PES header length is fixed:

| Field | Value |
|-------|-------|
| `stream_id` | `1011 1101` = `private_stream_1` (`0xBD`) |
| `PES_packet_length` | `(N × 184) − 6`, N integer, so the PES packet ends at a Transport packet boundary |
| `data_alignment_indicator` | `1` (VBI access units aligned with PES packets) |
| `PES_header_data_length` | `0x24` |
| `stuffing_byte` | PES header padded with stuffing to make the header 45 bytes long |
| `PES_packet_data_byte` | coded per the `PES_data_field` syntax (Table 1) |

`data_identifier` (§4.1, PDF p. 7) shall be in `0x10`–`0x1F` or `0x99`–`0x9B`.

## Table 1 — Syntax for PES data field (§4.4.1, PDF p. 8)

| Syntax | No. of bits | Mnemonic | Valid Range |
|--------|-------------|----------|-------------|
| `PES_data_field () {` | | | |
| &nbsp;&nbsp;data_identifier | 8 | uimsbf | See table 2 |
| &nbsp;&nbsp;`for (I = 0; i < N; i++) {` | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;data_unit_id | 8 | uimsbf | See table 3 |
| &nbsp;&nbsp;&nbsp;&nbsp;data_unit_length | 8 | uimsbf | 0x00 .. 0xFF |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (data_unit_id == 0x02 \|\| data_unit_id == 0x03 \|\| data_unit_id == 0xC0 \|\| data_unit_id == 0xC1) {` | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;txt_data_field (); | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`} else if (data_unit_id == 0xC3) {` | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;vps_data_field(); | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`} else if (data_unit_id == 0xC4) {` | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;wss_data_field (); | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`} else if (data_unit_id == 0xC5) {` | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;closed_captioning_data_field (); | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`} else if (data_unit_id == 0xC6) {` | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;monochrome_data_field (); | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`} else if (data_unit_id == 0xFF) {` | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`/* No data field */` | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i = 0; i < N; i++) {` | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;stuffing_byte | 8 | bslbf | "11111111" |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | | |
| &nbsp;&nbsp;`}` | | | |
| `}` | | | |

### Semantics (§4.4.2, PDF p. 8)

- **data_identifier** — 8-bit, identifies the type of data carried in the PES
  packet. Coded per Table 2.
- **data_unit_id** — 8-bit, identifies the type of this data unit. Based on
  Table 4 of EN 300 472 with added reserved values. Coded per Table 3.
- **data_unit_length** — 8-bit, number of bytes in this data unit immediately
  following the length field. **If `data_identifier` is between `0x10` and `0x1F`
  inclusive, this field shall always be set to `0x2C`** (= 44 bytes).
- **stuffing_byte** — shall in any case be discarded by the decoder.

## Table 2 — data_identifier for PES data field (§4.4.2, PDF p. 8)

| data_identifier | Value | Action of decoders (see note) |
|-----------------|-------|-------------------------------|
| `0x00`–`0x0F` | reserved for future use | discard |
| `0x10`–`0x1F` | EBU Teletext only **or** EBU Teletext combined with VPS and/or WSS and/or Closed Captioning and/or VBI sample data | transcode EBU Teletext, VPS, WSS, Closed Captioning, VBI sample data |
| `0x20`–`0x7F` | reserved for future use | discard |
| `0x80`–`0x98` | user defined | discard |
| `0x99`–`0x9B` | EBU Teletext and/or VPS and/or WSS and/or Closed Captioning and/or VBI sample data | transcode EBU Teletext, VPS, WSS, Closed Captioning, VBI sample data |
| `0x9C`–`0xFF` | user defined | discard |

> NOTE: If a decoder is not capable of decoding the data or if the specific
> decoding option is switched off, the data shall be discarded.

For `data_identifier` `0x10`–`0x1F`, decoders compliant only to EN 300 472 may be
able to decode the Teletext data; they must be robust against the non-teletext
data, i.e. robust against data units other than `0x02`, `0x03`, and `0xFF`
(§4.1, PDF p. 7).

## Table 3 — data_unit_id for data_identifier 0x10–0x1F or 0x99–0x9B (§4.4.2, PDF p. 9)

| data_unit_id | Value | Action of decoders (see note) |
|--------------|-------|-------------------------------|
| `0x00`–`0x01` | reserved for future use | discard |
| `0x02` | EBU Teletext non-subtitle data | transcode as EBU Teletext |
| `0x03` | EBU Teletext subtitle data | transcode as EBU Teletext |
| `0x04`–`0x7F` | reserved for future use | discard |
| `0x80`–`0xBF` | user defined | discard |
| `0xC0` | Inverted Teletext | transcode as EBU Teletext with an inverted framing code |
| `0xC1` | reserved | discard |
| `0xC2` | reserved | discard |
| `0xC3` | VPS | transcode as VPS |
| `0xC4` | WSS | transcode as WSS |
| `0xC5` | Closed Captioning | transcode as Closed Captioning |
| `0xC6` | monochrome 4:2:2 samples | transcode as raw VBI data |
| `0xC7`–`0xFE` | user defined | discard |
| `0xFF` | stuffing | discard |

> NOTE: If a decoder is not capable of decoding the data or if the specific
> decoding option is switched off, the data shall be discarded.

⚠ PDF p. 8 / p. 9 (Table 1 vs Table 3): Table 1's `if` branch routes `data_unit_id`
`0x02`, `0x03`, `0xC0`, **and `0xC1`** to `txt_data_field()`, but Table 3 marks
`0xC1` as **reserved → discard**. The spec is internally inconsistent here:
`0xC1` is reserved per the value table yet appears in the parse branch. Treat the
value table (Table 3) as authoritative — `0xC1` is reserved.

---

## Data unit 0x02 / 0x03 / 0xC0 — EBU and Inverted Teletext (§4.5, PDF pp. 9–10)

### Table 4 — Syntax of data field for EBU and Inverted Teletext (§4.5.1, PDF p. 9)

| Syntax | No. of bits | Mnemonic | Valid Range |
|--------|-------------|----------|-------------|
| `txt_data_field () {` | | | |
| &nbsp;&nbsp;reserved_future_use | 2 | bslbf | "11" |
| &nbsp;&nbsp;field_parity | 1 | bslbf | "0", "1" |
| &nbsp;&nbsp;line_offset | 5 | uimsbf | 0, 7 .. 22 |
| &nbsp;&nbsp;framing_code | 8 | bslbf | "11100100", "00011011" |
| &nbsp;&nbsp;txt_data_block | 336 | bslbf | see [3] |
| `}` | | | |

First byte bit-packing (MSB→LSB): `[7:6]` reserved_future_use = `11`,
`[5]` field_parity, `[4:0]` line_offset.

### Semantics (§4.5.2, PDF pp. 9–10)

- **field_parity** — 1 bit. `1` = first field of a frame; `0` = second field of a
  frame.
- **line_offset** — 5 bits, the VBI line on which the Teletext packet is presented
  if transcoded. Within a field the numbering follows a progressive incremental
  order except for the undefined value 0. Toggling `field_parity` indicates a new
  field. Coded per Table 5; only values 0 and 7–22 are permitted for EBU and
  Inverted Teletext data units.
- **framing_code** — 8 bits. If transcoded into the VBI: for EBU Teletext it shall
  be `11100100`; for Inverted Teletext it shall be `00011011`.
- **txt_data_block** — 336 bits = the 42 bytes following the clock-run-in and
  framing-code sequence of an EBU/Inverted Teletext data packet (EN 300 706 [3]).
  Packets inserted in the order they arrive at the Teletext decoder / are
  transcoded; data bits inserted in the order they appear in the VBI.

### Table 5 — line_offset for EBU and Inverted Teletext (§4.5.2, PDF p. 10)

| line_offset | field_parity == '1' | field_parity == '0' |
|-------------|---------------------|---------------------|
| 0 | line number undefined | line number undefined |
| 1 to 6 | reserved for future use | reserved for future use |
| 7 | line number = 7 | line number = 320 |
| 8 | line number = 8 | line number = 321 |
| : | : | : |
| 22 | line number = 22 | line number = 335 |
| 23 to 31 | reserved for future use | reserved for future use |

(Field 0 maps line_offset L to line number 313 + L for L in 7..22, i.e. 320..335.)

---

## Data unit 0xC3 — VPS (§4.6, PDF pp. 10–11)

VPS (Video Programme System, EN 300 231 [4]) packets, 625-line video.

### Table 6 — Syntax of data field for VPS (§4.6.1, PDF p. 10)

| Syntax | No. of bits | Mnemonic | Valid Range |
|--------|-------------|----------|-------------|
| `vps_data_field () {` | | | |
| &nbsp;&nbsp;reserved_future_use | 2 | bslbf | '11' |
| &nbsp;&nbsp;field_parity | 1 | bslbf | '1' |
| &nbsp;&nbsp;line_offset | 5 | uimsbf | 16 |
| &nbsp;&nbsp;vps_data_block | 104 | bslbf | see [4] |
| `}` | | | |

First byte: `[7:6]` = `11`, `[5]` field_parity, `[4:0]` line_offset.

### Semantics (§4.6.2, PDF p. 11)

- **field_parity** — coding equipment generates only `1`; decoders need only
  implement `1`, may ignore packets with `0`.
- **line_offset** — coding equipment generates only `16`; decoders need only
  implement `16`, may ignore other lines. Coded per Table 7.
- **vps_data_block** — 104 bits = the 13 data bytes of a VPS line per §8.2.2.2 of
  EN 300 231 [4], **excluding the run-in and start code byte** (so bytes 3 up to
  and including 15 are coded). Data bits inserted in the order they appear in the
  VBI.

### Table 7 — line_offset for VPS (§4.6.2, PDF p. 11)

| line_offset | field_parity == '1' | field_parity == '0' |
|-------------|---------------------|---------------------|
| 0 to 15 | reserved for future use | reserved for future use |
| 16 | line number = 16 | reserved for future use |
| 17 to 31 | reserved for future use | reserved for future use |

---

## Data unit 0xC4 — WSS (§4.7, PDF p. 11)

Wide Screen Signalling (WSS, EN 300 294 [5]) packets, 625-line video.

### Table 8 — Syntax of data_field for WSS (§4.7.1, PDF p. 11)

| Syntax | No. of bits | Mnemonic | Valid Range |
|--------|-------------|----------|-------------|
| `wss_data_field () {` | | | |
| &nbsp;&nbsp;reserved_future_use | 2 | bslbf | '11' |
| &nbsp;&nbsp;field_parity | 1 | bslbf | '1' |
| &nbsp;&nbsp;line_offset | 5 | uimsbf | 23 |
| &nbsp;&nbsp;wss_data_block | 14 | bslbf | see [5] |
| &nbsp;&nbsp;reserved_future_use | 2 | bslbf | '11' |
| `}` | | | |

Total = 24 bits = 3 bytes. First byte: `[7:6]` = `11`, `[5]` field_parity,
`[4:0]` line_offset. The 14 wss bits then a trailing 2-bit reserved_future_use
`11` (so byte layout is: byte0 = header, byte1 = wss bits `[13:6]`, byte2 = wss
bits `[5:0]` followed by the 2-bit reserved tail).

### Semantics (§4.7.2, PDF p. 11)

- **field_parity** — coding equipment generates only `1`; decoders need only
  implement `1`, may ignore packets with `0`.
- **line_offset** — coding equipment generates only `23`; decoders need only
  implement `23`, may ignore other lines. Coded per Table 9. Decoders shall
  generate WSS bits only in the first half of the line (§4.1 of EN 300 294 [5]).
- **wss_data_block** — 14 bits = the 14 wide-screen-signalling bits defined in
  Table 1 of EN 300 294 [5]. WSS bit 0 corresponds to the left-most bit of
  wss_data_block, so data bits are inserted in the PES packet in the same order
  they appear in the VBI.

### Table 9 — line_offset for WSS (§4.7.2, PDF p. 11)

| line_offset | field_parity == '1' | field_parity == '0' |
|-------------|---------------------|---------------------|
| 0 to 22 | reserved for future use | reserved for future use |
| 23 | line number = 23 | reserved for future use |
| 24 to 31 | reserved for future use | reserved for future use |

---

## Data unit 0xC5 — Closed Captioning (line 21) (§4.8, PDF p. 12)

Closed Captioning packets (EIA-608 Revision A [6]), 525-line video.

### Table 10 — Syntax of data_field for Closed Captioning (§4.8.1, PDF p. 12)

| Syntax | No. of bits | Mnemonic | Valid Range |
|--------|-------------|----------|-------------|
| `closed_captioning_data_field () {` | | | |
| &nbsp;&nbsp;reserved_future_use | 2 | bslbf | '11' |
| &nbsp;&nbsp;field_parity | 1 | bslbf | '0', '1' |
| &nbsp;&nbsp;line_offset | 5 | uimsbf | 21 |
| &nbsp;&nbsp;closed_captioning_data_block | 16 | bslbf | see [6] |
| `}` | | | |

Total = 24 bits = 3 bytes. First byte: `[7:6]` = `11`, `[5]` field_parity,
`[4:0]` line_offset; then 2 bytes of CC data.

### Semantics (§4.8.2, PDF p. 12)

- **line_offset** — coding equipment generates only `21`; decoders need only
  implement `21`, may ignore other lines. Coded per Table 11.
- **closed_captioning_data_block** — 16 bits = the 16 Closed Captioning data bits
  defined in EIA-608 Revision A [6]. CC bit b0 of character one corresponds to the
  left-most bit of closed_captioning_data_block, so data bits are inserted in the
  PES packet in the same order they appear in the VBI.

### Table 11 — line_offset for Closed Captioning (§4.8.2, PDF p. 12)

| line_offset | field_parity == '1' | field_parity == '0' |
|-------------|---------------------|---------------------|
| 0 to 20 | reserved for future use | reserved for future use |
| 21 | line number = 21, first field | line number = 21, second field |
| 22 to 31 | reserved for future use | reserved for future use |

---

## Data unit 0xC6 — Monochrome 4:2:2 samples (§4.9, PDF pp. 12–13)

Generic luminance-only VBI sample transport for standards not otherwise covered.
Coded using ITU-R BT.601-1 [7] / BT.656 [8]; a video line contains 720 luminance
samples and 360 samples per colour-difference signal. Only luminance is coded; all
co-sited chrominance is the value `0x80` (zero chrominance) by definition. Samples
not coded (start/end of line) take a decoder-defined value. A single line may be
split across several segments, contiguously coded into adjacent data units within
the PES packet.

Buffer-model constraint (§4.9, PDF p. 13): to be compliant with standard DVB
Teletext decoders the stream must comply with the standard Teletext buffer model
(EN 300 472 [2]). An encoder shall generate at most one monochrome VBI data line
per field if any other VBI data is encoded in that field; if no other VBI data is
in a field, up to two monochrome lines may be generated.

### Table 12 — Syntax of data_field for monochrome 4:2:2 samples (§4.9.1, PDF p. 13)

| Syntax | No. of bits | Mnemonic | Valid Range |
|--------|-------------|----------|-------------|
| `monochrome_data_field () {` | | | |
| &nbsp;&nbsp;first_segment_flag | 1 | bslbf | '0', '1' |
| &nbsp;&nbsp;last_segment_flag | 1 | bslbf | '0', '1' |
| &nbsp;&nbsp;field_parity | 1 | bslbf | '0', '1' |
| &nbsp;&nbsp;line_offset | 5 | uimsbf | 7 .. 23 |
| &nbsp;&nbsp;first_pixel_position | 16 | uimsbf | 0 .. 719 |
| &nbsp;&nbsp;n_pixels | 8 | uimsbf | 1 .. 251 |
| &nbsp;&nbsp;`for (i = 0; i < n_pixels; i++) {` | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;Y_value | 8 | uimsbf | 0x10 .. 0xEB |
| &nbsp;&nbsp;`}` | | | |
| `}` | | | |

First byte bit-packing (MSB→LSB): `[7]` first_segment_flag, `[6]`
last_segment_flag, `[5]` field_parity, `[4:0]` line_offset. (Note: unlike the
Teletext/VPS/WSS/CC units, there is **no** 2-bit reserved_future_use prefix here —
the two flags occupy `[7:6]` instead.)

### Semantics (§4.9.2, PDF p. 13)

- **first_segment_flag** — `1` for the first segment of a line, `0` for all others.
- **last_segment_flag** — `1` for the last segment of a line, `0` for all others.
- **field_parity** — `1` first field, `0` second field (per clause 4.5 semantics);
  toggling indicates a new field.
- **line_offset** — 5 bits, the VBI line; within a field follows progressive
  incremental order. Coded per Table 13.
- **first_pixel_position** — position of the first coded luminance sample of this
  segment, in units of one pixel (0..719 inclusive). If this segment is followed
  by another (i.e. last_segment_flag == 0), the next segment's
  first_pixel_position shall equal current first_pixel_position + n_pixels.
- **n_pixels** — number of luminance samples coded in this segment; shall be > 0.
- **Y_value** — value of a single luminance sample, per ITU-R BT.601-1 [7].

### Table 13 — line_offset for monochrome 4:2:2 samples (§4.9.2, PDF p. 13)

| line_offset | field_parity == '1' | field_parity == '0' |
|-------------|---------------------|---------------------|
| 0 to 6 | reserved for future use | reserved for future use |
| 7 | line number = 7, first field | line number = 7, second field |
| 8 | line number = 8, first field | line number = 8, second field |
| : | : | : |
| 23 | line number = 23, first field | line number = 23, second field |
| 24 to 31 | reserved for future use | reserved for future use |

---

## Data unit 0xFF — Stuffing (§4.4.1, PDF p. 8)

`data_unit_id == 0xFF` carries no data field (`/* No data field */`). Per Table 3
its meaning is "stuffing" and the decoder action is "discard". `data_unit_length`
bytes follow and are skipped.

---

## §4.10 — Combining VBI data with EN 300 472 Teletext data (PDF p. 14)

VBI data units and EBU Teletext data units may share one elementary stream.
Backwards compatibility with EN 300 472 is guaranteed because EN 300 472 EBU
Teletext and EN 301 775 VBI data can co-exist in the same service on separate
PIDs. Where a mixed variety of IRDs must be supported, VBI is broadcast on two or
more PIDs (e.g. one PID for the EN 300 472 Teletext standard and another for the
VBI standard, with Teletext broadcast on both PIDs).

---

## Notes / ⚠ flags

- ⚠ §4.4 Table 1 vs Table 3 (PDF pp. 8–9): `data_unit_id == 0xC1` appears in the
  `txt_data_field()` parse branch of Table 1, yet Table 3 lists `0xC1` as
  *reserved → discard*. Internal spec inconsistency. Candidate resolutions:
  (a) treat `0xC1` as reserved per the value table (recommended); (b) honour the
  parse branch and decode `0xC1` as Teletext. Table 3 (value table) is taken as
  authoritative.
- The WSS data unit (`0xC4`) total length is 3 bytes (24 bits): an 8-bit header,
  14 wss bits, then a trailing 2-bit reserved_future_use `11` — confirmed from the
  Table 8 field list (2 + 1 + 5 + 14 + 2 = 24).
- For `data_identifier` `0x10`–`0x1F`, `data_unit_length` is fixed at `0x2C`
  (44 bytes) — the txt_data_field body (1 header + 1 framing_code + 42 data =
  44 bytes), giving EN 300 472-only decoder compatibility (§4.4.2, PDF p. 8).
