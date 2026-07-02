# FLV (Flash Video) container — for the FLV spoke (#513)

Source: **Adobe Flash Video File Format Specification v10.1, Annex E** (public;
PDF in the private `rust-broadcast-specs`). transmux carries H.264 video + AAC
audio (the FLV mainstream); coded samples pass through opaque.

Byte order: **big-endian**.

## FLV header (E.2) — 9 bytes + first PreviousTagSize

| field | bytes | value |
|---|---|---|
| Signature | 3 | `"FLV"` (0x46 0x4C 0x56) |
| Version | 1 | 0x01 |
| TypeFlags | 1 | bit0 `audio present`, bit2 `video present` (rest 0) |
| DataOffset | 4 | header size = 9 |
| PreviousTagSize0 | 4 | 0 |

## FLV tag (E.4.1) — repeated: `[Tag][PreviousTagSize]`

| field | size | note |
|---|---|---|
| TagType | UI8 | 8 = audio, 9 = video, 18 = script data |
| DataSize | UI24 | bytes after StreamID to end of tag body |
| Timestamp | UI24 | ms |
| TimestampExtended | UI8 | high byte (24+8 = 32-bit ms) |
| StreamID | UI24 | always 0 |
| Data | DataSize | AudioTagHeader / VideoTagHeader + payload, or SCRIPTDATA |
| *(then)* PreviousTagSize | UI32 | size of the tag just written, incl. its 11-byte header |

## Video tag (E.4.3) — `VideoTagHeader` + body

`VideoTagHeader`: `FrameType` (UB[4]) + `CodecID` (UB[4]).
- FrameType: 1 = keyframe (seekable), 2 = inter, 3 = disposable inter, 5 = video info/command.
- CodecID: **7 = AVC (H.264)** (others: 2 Sorenson, 4 VP6, …).

For CodecID == 7 → **AVCVIDEOPACKET** (E.4.3.2):
| field | size | note |
|---|---|---|
| AVCPacketType | UI8 | 0 = AVC sequence header (**`AVCDecoderConfigurationRecord` = `avcC`**), 1 = NALU, 2 = end of sequence |
| CompositionTime | SI24 | if type==1: `(PTS − DTS)` in ms; else 0 |
| Data | rest | type 0 → the avcC bytes; type 1 → one or more length-prefixed NAL units (4-byte lengths) |

→ tag Timestamp = **DTS** (ms); PTS = DTS + CompositionTime. Map to
`CodecConfig::Avc` (avcC from the AVCPacketType-0 tag); samples = the type-1 NALU
payloads (already length-prefixed, as the IR wants).

## Audio tag (E.4.2) — `AudioTagHeader` + body

`AudioTagHeader`: `SoundFormat` (UB[4]) + `SoundRate` (UB[2]) + `SoundSize`
(UB[1]) + `SoundType` (UB[1]).
- SoundFormat: **10 = AAC** (others: 2 MP3, 11 Speex, …). SoundRate 3 = 44 kHz;
  for AAC always 3 (real rate is in the ASC). SoundType: 0 mono, 1 stereo.

For SoundFormat == 10 → **AACAUDIODATA** (E.4.2.2):
| field | size | note |
|---|---|---|
| AACPacketType | UI8 | 0 = AAC sequence header (**`AudioSpecificConfig`**), 1 = raw AAC frame |
| Data | rest | type 0 → the ASC bytes; type 1 → one raw AAC access unit |

→ Map to `CodecConfig::Aac` (esds/ASC from the type-0 tag); samples = type-1 frames.

## Script data (E.4.1, TagType 18)
`onMetaData` — an AMF0 `ECMA array` of `duration`/`width`/`height`/`framerate`/
`audiocodecid`/`videocodecid`/… Informational; optional to emit on mux, parse
leniently on demux.

## Mapping
- `FlvDemux` (`Unpackage`): read header → tags; the two sequence-header tags
  (AVCPacketType 0 / AACPacketType 0) give the configs; subsequent tags are
  samples in DTS order with CompositionTime → composition offset.
- `FlvMux` (`Package`): header (flags from track kinds) → metadata tag →
  sequence-header tags (avcC / ASC) → interleaved A/V tags → PreviousTagSize
  after each.
