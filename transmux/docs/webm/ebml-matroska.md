# WebM / Matroska (EBML) element reference — for the WebM spoke (#471)

Sources: **EBML** = RFC 8794 (element framing / VINT). **Matroska** = RFC 9559
(element IDs, semantics). **WebM** = webmproject.org container guidelines (the
Matroska subset used with VP8/VP9/AV1 video + Vorbis/Opus audio).

WebM is a Matroska subset. A demuxer walks the EBML element tree by
(element-ID, size, data) triplets; both the ID and the size are **VINT**s.

## EBML framing (RFC 8794 §4)

- **VINT / element size**: the first byte's leading zero-count marks the width
  (1–8 bytes). The first `1` bit is the length marker; the remaining bits are the
  value. `0x81` = 1-byte VINT value 1; `0x40 0x02` = 2-byte VINT value 2. All-ones
  data bits = "unknown size" (used for live Segment/Cluster).
- **Element ID**: like a VINT but the marker bits are **kept** (the ID is the
  whole encoded value including the length-marker bit). So `Segment` is the 4-byte
  ID `0x18538067`, `TrackEntry` is the 1-byte ID `0xAE`, etc. — match IDs as their
  full stored value below.
- Master elements contain child elements; leaf elements carry uint / int / float
  / string / UTF-8 / binary / date payloads.

## Element IDs needed for demux (Matroska RFC 9559 §27 registry)

| ID | Element | Level / parent | Payload |
|---|---|---|---|
| `0x1A45DFA3` | EBML (header) | top | master |
| `0x4282` | DocType | EBML | string ("webm" / "matroska") |
| `0x18538067` | Segment | top | master |
| `0x114D9B74` | SeekHead | Segment | master (skippable) |
| `0x1549A966` | Info | Segment | master |
| `0x2AD7B1` | TimestampScale | Info | uint (ns per tick; default 1_000_000) |
| `0x4489` | Duration | Info | float (in TimestampScale ticks) |
| `0x1654AE6B` | Tracks | Segment | master |
| `0xAE` | TrackEntry | Tracks | master |
| `0xD7` | TrackNumber | TrackEntry | uint |
| `0x83` | TrackType | TrackEntry | uint (1=video, 2=audio) |
| `0x86` | CodecID | TrackEntry | string (see mapping) |
| `0x63A2` | CodecPrivate | TrackEntry | binary (codec setup — e.g. OpusHead) |
| `0x23E383` | DefaultDuration | TrackEntry | uint (ns per frame) |
| `0xE0` | Video | TrackEntry | master |
| `0xB0` | PixelWidth | Video | uint |
| `0xBA` | PixelHeight | Video | uint |
| `0xE1` | Audio | TrackEntry | master |
| `0xB5` | SamplingFrequency | Audio | float (Hz) |
| `0x9F` | Channels | Audio | uint |
| `0x1F43B675` | Cluster | Segment | master |
| `0xE7` | Timestamp | Cluster | uint (cluster base time, in TimestampScale ticks) |
| `0xA3` | SimpleBlock | Cluster | binary (see block layout) |
| `0xA0` | BlockGroup | Cluster | master |
| `0xA1` | Block | BlockGroup | binary (same layout as SimpleBlock, sans keyframe flag) |
| `0x9B` | BlockDuration | BlockGroup | uint |
| `0x1C53BB6B` | Cues | Segment | master (index — skippable) |

## (Simple)Block layout (RFC 9559 §12)

`SimpleBlock` / `Block` binary payload:
1. **Track number** — VINT (value; strip the length marker).
2. **Relative timestamp** — signed int16, big-endian, added to the Cluster
   `Timestamp` (both in TimestampScale ticks).
3. **Flags** — 1 byte. Bit 7 (`0x80`) = keyframe (SimpleBlock only). Bits 1–2
   (`0x06`) = lacing: 00 none, 01 Xiph, 11 EBML, 10 fixed-size.
4. **Lacing** (if any): a frame count byte then per-frame sizes; else the rest of
   the element is one frame.
5. **Frame data**.

A demuxer without lacing support can assert lacing==none for this fixture (VP9 /
Opus WebM from ffmpeg use no lacing — one frame per block).

## CodecID → transmux codec mapping

| Matroska CodecID | codec | IR `CodecConfig` |
|---|---|---|
| `V_VP9` | VP9 | `Vp9` (build `vpcC` — profile/level/bit-depth; if not derivable, VP9 profile 0 8-bit defaults, documented) |
| `V_VP8` | VP8 | (out of scope) |
| `V_AV1` | AV1 | `Av1` (`av1C` from CodecPrivate) |
| `A_OPUS` | Opus | `Opus` (`dOps` from the `OpusHead` in CodecPrivate) |
| `A_VORBIS` | Vorbis | (out of scope) |

**Opus CodecPrivate** = the `OpusHead` identification header (RFC 7845 §5.1):
magic "OpusHead", version, channel count, pre-skip, input sample rate, output
gain, channel mapping — the same bytes that populate the `dOps` box.

## Timestamps

Presentation time (ns) = `(Cluster.Timestamp + block.rel_ts) * TimestampScale`.
Convert to the IR's timescale (e.g. 90 kHz or the track's own) consistently.
WebM/Matroska carries **presentation** timestamps; VP9/Opus here have no B-frame
reorder, so DTS == PTS.
