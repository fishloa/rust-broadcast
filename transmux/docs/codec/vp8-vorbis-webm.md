# VP8 + Vorbis (WebM) — config for the transmux IR

Adds the last two WebM/Matroska codecs (alongside VP9/Opus, see
`docs/webm/ebml-matroska.md`). CodecID mapping:

| Matroska CodecID | codec | IR `CodecConfig` |
|---|---|---|
| `V_VP8` | VP8 | `Vp8` |
| `A_VORBIS` | Vorbis | `Vorbis` |

## VP8 (RFC 6386)

VP8 has **no** out-of-band config box — dimensions come from the **key-frame
header** (RFC 6386 §9.1, §19.1). A VP8 frame:
- 3-byte uncompressed frame tag (little-endian 24-bit): bit0 `key_frame` (0 = key),
  bits1-3 `version`, bit4 `show_frame`, bits5-23 `first_part_size`.
- **Key frames only:** then the 3-byte start code `0x9D 0x01 0x2A`, then
  `width` = 14 bits + 2-bit horizontal scale, `height` = 14 bits + 2-bit vertical
  scale (little-endian 16-bit each; mask `& 0x3FFF` for the dimension).

So `CodecConfig::Vp8 { width, height }` — parse the first key frame's tag +
start code + the two 16-bit little-endian size words. (`show_frame`/`version`
are informational.) WebM samples are raw VP8 frames; a `SimpleBlock` keyframe
flag corresponds to `key_frame == 0`.

## Vorbis (Vorbis I spec, xiph.org)

WebM carries the Vorbis setup in **`CodecPrivate`** as the three Vorbis headers
packed with **Xiph lacing**:
- byte 0: `numPackets - 1` = **2** (i.e. 3 headers).
- then the lengths of the first two headers, each as a series of bytes summed
  while == 255 (Xiph lacing); the third header's length is the remainder.
- **Header 1 — Identification** (Vorbis I §4.2.2): after the 7-byte packet type +
  "vorbis" signature — `vorbis_version(u32)`, `audio_channels(u8)`,
  `audio_sample_rate(u32 LE)`, bitrate maximum/nominal/minimum (u32 each),
  `blocksize` byte, framing bit. → **channels + sample_rate**.
- Header 2 = Comment, Header 3 = Setup (codebooks) — carried opaque.

So `CodecConfig::Vorbis { codec_private, channels, sample_rate }` — store the
`CodecPrivate` verbatim (needed to reconstruct/mux) and decode channels +
sample_rate from the identification header for the IR track spec.

## Notes
- Both round-trip through `WebmDemux` → IR. Output-side muxing of VP8/Vorbis into
  fMP4 is out of scope here (WebM is their native container); the IR carries them
  so `{WebM} → IR → {WebM}` and inspection work.
