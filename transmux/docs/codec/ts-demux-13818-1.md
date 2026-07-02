# MPEG-2 TS demux — PSI tables + stream_type→codec map

Sources: ITU-T H.222.0 (= ISO/IEC 13818-1) for the PSI section syntax and the
stream_type registry; ETSI TS 101 154 for the DVB codec-carriage assignments used
by `TsDemux` to map `stream_type` → hub codec.

## Program Association Table (PAT) — ISO/IEC 13818-1 §2.4.4.3, table_id `0x00`

Carried on PID `0x0000`. Body after the 8-byte long-form section header
(`table_id`, `section_syntax_indicator`, `section_length`, `transport_stream_id`,
`version_number`/`current_next_indicator`, `section_number`, `last_section_number`)
is a loop of 4-byte entries, ending 4 bytes before the section end (CRC_32):

```
for (i = 0; i < N; i++) {
    program_number            16
    reserved                   3
    program_map_PID / network_PID  13
}
CRC_32                        32
```

`program_number == 0` → the 13-bit PID is the `network_PID` (NIT), skipped by the
demuxer. All other entries give a `program_map_PID` (a PMT PID).

## Program Map Table (PMT) — ISO/IEC 13818-1 §2.4.4.8, table_id `0x02`

Body after the 8-byte long-form header:

```
reserved                       3
PCR_PID                       13
reserved                       4
program_info_length           12
descriptor()                  program_info_length bytes   (skipped)
for (i = 0; i < N; i++) {
    stream_type                8
    reserved                   3
    elementary_PID            13
    reserved                   4
    ES_info_length            12
    descriptor()              ES_info_length bytes         (skipped)
}
CRC_32                        32
```

## stream_type → codec (ISO/IEC 13818-1 Table 2-34 + ETSI TS 101 154)

| stream_type | meaning | hub codec |
|---|---|---|
| `0x1B` | AVC video (H.264, ISO/IEC 14496-10) — 13818-1 Table 2-34 | H.264 |
| `0x24` | HEVC video (H.265, ISO/IEC 23008-2) — 13818-1 Table 2-34 | HEVC |
| `0x0F` | ISO/IEC 13818-7 AAC in ADTS — 13818-1 Table 2-34 | AAC (ADTS) |
| `0x81` | ATSC/DVB AC-3 (user private; ETSI TS 101 154 §G) | AC-3 |
| `0x87` | E-AC-3 (user private; ETSI TS 101 154 §G) | E-AC-3 |
| `0x82` | DTS (user private) | DTS |
| `0x85` | DTS-HD (user private) | DTS |
| `0x8A` | DTS (user private) | DTS |

Unknown stream_types are skipped (not carried, not fatal) per issue #467.

## PES-over-TS reassembly — ISO/IEC 13818-1 §2.4.3.6/§2.4.3.7

No `pointer_field`: a TS packet with `payload_unit_start_indicator = 1` begins a
PES packet; continuation packets append. A PES runs from one PUSI to the next
(unbounded video, `PES_packet_length = 0`, flushed at the next unit start / EOS).
PTS/DTS are 33-bit @ 90 kHz; DTS defaults to PTS when absent. See `mpeg-pes`.

## Random-access / sync samples

For H.264 an access unit is a sync sample (IDR) iff it contains a NAL of type 5
(`coded slice of an IDR picture`, ISO/IEC 14496-10 Table 7-1); for HEVC iff it
contains a NAL of type 19/20 (IDR_W_RADL/IDR_N_LP) or 21 (CRA), ISO/IEC 23008-2
Table 7-1. Audio access units are all sync samples.
