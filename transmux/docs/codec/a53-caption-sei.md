# ATSC A/53 caption carriage (`GA94` / `MPEG_cc_data()`) — #599

**Source:** ATSC A/53 Part 4:2009 "MPEG-2 Video System Characteristics"
(`private/specs/atsc_a53_part4_2009_mpeg2_video.pdf`) §6.2.2/§6.2.3, Tables
6.6-6.10 (pdftotext extraction — plain running text + simple two-column
tables, no bit-syntax rendering issues). This part defines the **MPEG-2**
picture `user_data()` embedding only; MPEG-2 has no SEI mechanism, so it
never wraps the payload in an ITU-T T.35 country/provider header. The H.264
(and by extension HEVC) **SEI** wrapping used by `transmux::nal::caption_cc_data`
is a well-established, non-A/53 industry convention layered on top of the same
`ATSC_user_data()`/`cc_data()` payload — see the note below the tables.

## Table 6.6 — `user_data()` (MPEG-2 picture user data)

| Syntax                | Bits | Format |
|-----------------------|------|--------|
| `user_data_start_code`| 32   | bslbf  |
| `user_data_identifier`| 32   | bslbf  |
| `user_structure()`    | —    | —      |

`user_data_start_code` = `0x0000 01B2` (ISO/IEC 13818-2). Not relevant to the
H.264/HEVC SEI path (SEI has its own `payloadType`/`payloadSize` framing
instead), included only for provenance of the tables below.

## Table 6.7 — `user_data_identifier` value assignments

| `user_data_identifier` | `user_structure()`                    |
|-------------------------|---------------------------------------|
| `0x47413934` (`"GA94"`) | `ATSC_user_data()` (Table 6.8)        |
| `0x44544731` (`"DTG1"`) | `afd_data()` (§6.2.4)                 |
| all other values         | not defined in this Standard          |

## Table 6.8 — `ATSC_user_data()`

| Syntax                        | Bits | Format |
|--------------------------------|------|--------|
| `user_data_type_code`          | 8    | uimsbf |
| `user_data_type_structure()`   | —    | —      |

## Table 6.9 — `user_data_type_code` value assignments

| value          | `user_data_type_structure()`        |
|-----------------|--------------------------------------|
| `0x00`-`0x02`   | ATSC Reserved                        |
| **`0x03`**      | **`MPEG_cc_data()`** (Table 6.10)     |
| `0x04`-`0x05`   | ATSC Reserved                        |
| `0x06`          | `bar_data()` (§6.2.3.2)              |
| `0x07`-`0xFF`   | ATSC Reserved                        |

## Table 6.10 — `MPEG_cc_data()` (Captioning Data, §6.2.3.1)

| Syntax          | Bits | Format         |
|-----------------|------|----------------|
| `cc_data()`     | —    | —              |
| `marker_bits`   | 8    | `'1111 1111'`  |

`cc_data()` is CEA-708 Table 2 — the same structure the `cc-data` crate
decodes (ETSI TS 101 154 §B.5 Table B.9 is the DVB-normative re-statement of
the identical CEA-708 wire form: `process_cc_data_flag` + `cc_count`, then
`cc_count` × (`cc_valid`, `cc_type`, `cc_data_1`, `cc_data_2`)).

## H.264/HEVC SEI wrapping (not in A/53 Part 4 — industry convention)

H.264 (ITU-T H.264 Annex D.1.6/D.2.6) and HEVC (ITU-T H.265 Annex D, same
`payloadType` registry) define an SEI message `user_data_registered_itu_t_t35`
(`payloadType` 4):

```
user_data_registered_itu_t_t35(payloadSize) {
    itu_t_t35_country_code           // 8 bits — 0xB5 = "United States" (ITU-T T.35)
    itu_t_t35_provider_code          // 16 bits — 0x0031 identifies ATSC (well-known,
                                      //   not itself an A/53 Part 4 value — Part 4
                                      //   has no SEI/T.35 concept)
    user_identifier                  // 32 bits — 0x47413934 "GA94" (A/53 Table 6.7)
    user_data_type_code              // 8 bits — 0x03 (A/53 Table 6.9) selects
                                      //   MPEG_cc_data() (Table 6.10) below
    MPEG_cc_data()                   // cc_data() + marker_bits, remaining payloadSize-8 bytes
}
```

This wrapper (country/provider/`GA94`/type-code signature, `payloadType` 4)
matches what ffmpeg (`ff_alloc_a53_sei`), and every other broadcast/OTT
encoder that embeds ATSC captions in H.264/HEVC, produces. It is what
`transmux::nal::caption_cc_data` recognises; the `MPEG_cc_data()` bytes it
returns feed straight into `cc_data::CcData::parse` exactly like the
PES-carried `cc_data()` path (issue #568).

**Real-capture confirmation:** the exact byte sequence
`B5 00 31 47 41 39 34 03` (country + provider + `GA94` + type code) appears
401 times in the first 15 MB of the real
`samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2885/transformers_EIA608_H264.ts`
capture (fetched on demand into the gitignored `.test-streams/`, see
`tools/fetch-test-streams.sh`), confirming this is the layout real encoders
emit, not just a documented-but-unused convention. One such SEI NAL, extracted
byte-for-byte from that capture, is `transmux::nal`'s
`REAL_A53_SEI_NAL` test fixture.
