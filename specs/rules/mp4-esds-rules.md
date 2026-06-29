# ISO/IEC 14496-1 (Systems) + 14496-14 (MP4) — `esds` / ES_Descriptor rules

Curated **semantic** rules for the MPEG-4 elementary-stream descriptor (`esds`) carried in MP4
sample entries (`mp4a`/`mp4v`/`mp4s`). Sources:
`specs/fulltext/iso_iec_14496-1_systems_es_descriptor_2010.md` and
`specs/fulltext/iso_iec_14496-14_mp4_2003.md` (gitignored pdf2md of the copyrighted PDFs;
regenerate with pdf2md). Each rule cites the spec § and line. Consumers: the planned transmux /
MP4-mux crate (extracting AAC `AudioSpecificConfig`, demuxing `mp4a`/`mp4v` to ES). Decisions
cite here.

## Expandable descriptor framing — 14496-1 §8.3.3 (fulltext L3986) — the parse primitive

- Every descriptor is a **self-describing expandable class**: `tag`(8) then a **variable-length
  size** (`sizeOfInstance`, the byte count of the body, **excluding** tag + size bytes) (L3992).
- **Size encoding** (L3996) — a 7-bit-per-byte varint, high bit = "another byte follows":
  ```
  bit(1) nextByte; bit(7) sizeByte; sizeOfInstance = sizeByte;
  while (nextByte) { bit(1) nextByte; bit(7) sizeByte; sizeOfInstance = (sizeOfInstance<<7)|sizeByte; }
  ```
  Common writers emit a fixed 4-byte form (`0x80 0x80 0x80 NN`) but the parser must accept 1–4 bytes.
- **Unknown tags must be skipped using the size** (L3988) — a parser walks the descriptor list by
  tag+size, ignoring tags it doesn't model (forward-compat). `maxClassSize` (e.g. `2^28-1`) bounds it.
- A serializer **computes** `sizeOfInstance` from the body — never stores-and-echoes it (workspace
  no-raw-passthrough invariant); round-trip must reproduce the writer's chosen size-byte width.

## Descriptor tags — 14496-1 Table 1 (fulltext L948) + 14496-14 §3.1.3 (L241)

`0x03` ES_DescrTag · `0x04` DecoderConfigDescrTag · `0x05` DecSpecificInfoTag · `0x06` SLConfigDescrTag ·
`0x0E` ES_ID_IncTag · `0x0F` ES_ID_RefTag · `0x10` MP4_IOD_Tag · `0x11` MP4_OD_Tag (file-format-only
tags, L243/L247). Tags `0x00`/forbidden; `0x80–0xFE` user-private in most sub-namespaces.

## ES_Descriptor — 14496-1 §7.2.6.5 (fulltext L1502)

Layout (L1506): `ES_ID`(16) + `streamDependenceFlag`(1) `URL_Flag`(1) `OCRstreamFlag`(1)
`streamPriority`(5); then `if streamDependenceFlag` `dependsOn_ES_ID`(16); `if URL_Flag` `URLlength`(8)
+ `URLstring`; `if OCRstreamFlag` `OCR_ES_Id`(16); then **`DecoderConfigDescriptor`**, then
`SLConfigDescriptor`, then optional descriptor arrays (lang/QoS/registration/extension).
- `ES_ID` 0 and 0xFFFF reserved (L1530). Three parts: identity, component descriptors, extensions (L1522).
- **In MP4 the ES_Descriptor is stored in the sample entry with constraints** (14496-14 §3.1.2, L209):
  `ES_ID = 0` as stored (built from low 16 bits of `track_ID` at stream time), `streamDependenceFlag=0`
  (deps via `dpnd` track reference), `OCRStreamFlag=0`, `SLConfigDescriptor = predefined type 2`
  (unless URL-referenced). So a demux reads codec config from the descriptor, **not** these stored zeros.

## DecoderConfigDescriptor — 14496-1 §7.2.6.6 (fulltext L1570)

Layout (L1574): `objectTypeIndication`(8) + `streamType`(6) `upStream`(1) `reserved=1`(1) +
`bufferSizeDB`(24) + `maxBitrate`(32) + `avgBitrate`(32) + optional `DecoderSpecificInfo` + optional
`profileLevelIndicationIndexDescriptor[]`.
- **`objectTypeIndication`** (Table 5, L1584) — the codec id. Key values for transmux:
  `0x20` MPEG-4 Visual (14496-2) · **`0x21` H.264/AVC (14496-10)** · `0x22` AVC parameter sets ·
  **`0x40` MPEG-4 Audio (14496-3 / AAC)** · `0x60–0x65` MPEG-2 Video (13818-2) profiles ·
  `0x66/67/68` MPEG-2 AAC (13818-7) · `0x69` MPEG-2 audio (13818-3) · `0x6B` MP1 audio (11172-3) ·
  `0x6C` JPEG · `0x6E` JPEG2000. `0xC0–0xFE` user-private; `0xFF` no object type. `0x00` forbidden.
- **`streamType`** (Table 6, L1664): `0x04` VisualStream · `0x05` AudioStream · `0x01` ObjectDescriptor ·
  `0x02` ClockReference · `0x03` SceneDescription · `0x20–0x3F` user-private. `0x00` forbidden.
- `avgBitrate = 0` for VBR streams (L1688). `maxBitrate` is the peak over any 1-s window.

## DecoderSpecificInfo — 14496-1 §7.2.6.7 (fulltext L1694)

- An **opaque container** whose meaning depends on `streamType`+`objectTypeIndication` (L1702). For a
  transmux it is the bytes to lift into the codec record:
  - **`0x40` (AAC, 14496-3)**: the **`AudioSpecificConfig`** — parse per 14496-3 §1.6
    (`iso_iec_14496-3_aac_audiospecificconfig_2001.pdf`); carries audioObjectType / samplingFrequencyIndex /
    channelConfiguration. This is what feeds an ADTS header or a `mp4a` re-mux.
  - **`0x20` (MPEG-4 Visual)**: VOS/VOL headers per 14496-2 Annex K (L1704).
  - **13818-7 (MPEG-2 AAC)**: an `adif_header()`; access unit = `raw_data_block()` (L1712).
  - **11172-3 / 13818-3 (MP1/MP2 audio)**: **empty** — all config is in the frame itself (L1714).
  - Treat the body as `&[u8]` only when genuinely opaque (unknown OTI); otherwise type it per the codec.

## MP4 storage / `esds` box — 14496-14 §5.6 (fulltext L452) + §3 (L195)

- `ESDBox` = `FullBox('esds', 0, 0)` wrapping one **`ES_Descriptor`** (L454). Sample entries:
  `mp4v`→VisualSampleEntry+esds, `mp4a`→AudioSampleEntry+esds, `mp4s`→SampleEntry+esds (L456).
- **A sample = one access unit**, stored natural/unfragmented, contiguous bytes (L199). For AAC/MP4
  audio this is one `raw_data_block`; the transmux maps sample↔access-unit directly.
- For visual streams, **config headers live in the ES_Descriptor, not in the samples** (L450) — a
  TS→MP4 transmux must lift sequence headers into `esds`/`avcC`, not leave them inline.
- `stsd` may hold **multiple entries** (imply `ES_DescriptorUpdate`); a URL-referenced ES allows only
  one entry (L446). `compressorname` set to 0 (L468).
- **track_ID**: low 2 bytes = ES_ID for media tracks, high 2 bytes zero (L257). IDs unique per file;
  `next_track_ID` in `mvhd` is the allocator (search needed once it hits ≥65535 / all-1s) (L259).

## Code-conformance notes (tracked — NOT yet applied; future transmux crate)

1. Descriptor walker: tag+expandable-size loop, skip-unknown (§8.3.3 L3988); serializer computes the
   size varint from the body (no stored-and-echoed length); round-trip preserves the size-byte width.
2. Codec lift: switch on `objectTypeIndication` (Table 5 L1584) + `streamType` (Table 6 L1664) to type
   the `DecoderSpecificInfo` (AAC AudioSpecificConfig for 0x40; opaque only for unknown OTI).
3. ES→TS / TS→ES: honour the stored-vs-streamed constraints (ES_ID=0 stored, low 16 bits of track_ID
   at stream time; config in descriptor not samples) (§3.1.2 L209, §5.6 L450).
