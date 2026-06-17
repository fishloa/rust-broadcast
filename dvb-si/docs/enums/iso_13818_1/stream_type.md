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
| 0x80–0xFF | User Private |

> NOTE (not part of Table 2-34): within the `0x80–0xFF` User Private range,
> industry conventions assign `0x81` = ATSC AC-3 (ATSC A/52), `0x86` =
> SCTE-35 splice_info (ANSI/SCTE 35), `0x87` = ATSC E-AC-3 (ATSC A/52B). These
> are **not** assigned by Rec. ITU-T H.222.0 | ISO/IEC 13818-1 and are cited to
> their own specifications; `dvb-si` decodes them as a documented convenience.

