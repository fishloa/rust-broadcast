# RTP + payload formats — for the RTP spoke (#469)

Sources: **RFC 3550** (RTP), **RFC 6184** (H.264/AVC payload), **RFC 3640**
(MPEG-4/AAC payload, mode `AAC-hbr`), **RFC 4566** (SDP). Scope for this spoke:
H.264 video + AAC-LC audio de/packetisation (the IR's `Avc`/`Aac` tracks), with
SDP generation.

## RTP fixed header (RFC 3550 §5.1) — 12 bytes

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|P|X|  CC   |M|     PT      |       sequence number         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                           timestamp                           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|           synchronization source (SSRC) identifier            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```
- `V`=2, `P`=0, `X`=0, `CC`=0 (no CSRC) for this spoke.
- `M` (marker): set on the **last packet of an access unit** (video); for audio,
  set per RFC 3640 (typically on the last packet carrying a frame).
- `PT` (payload type): dynamic, 96+ (e.g. 96 video, 97 audio) — must match the
  SDP `rtpmap`.
- `sequence number`: increments by 1 per packet (random start ok; use a fixed
  start for deterministic tests).
- `timestamp`: media clock. **H.264 → 90 kHz**; AAC → the audio sample rate.
  All packets of one access unit share the same timestamp.
- `SSRC`: a fixed per-stream random-ish id.

## H.264 payload (RFC 6184)

NAL unit octet: `F(1) | NRI(2) | Type(5)`. Packetisation modes; use
**packetization-mode=1** (non-interleaved).

- **Single NAL unit packet** (NAL Type 1–23): the whole NAL is the RTP payload
  (drop the Annex-B start code; the IR carries length-prefixed NALs — strip the
  4-byte length).
- **STAP-A** (Type **24**): aggregate several small NALs in one packet —
  `STAP-A NAL hdr (F|NRI|24)` then per-NAL `size(u16) | NAL`. Common for sending
  SPS+PPS together.
- **FU-A** (Type **28**): fragment ONE large NAL over multiple packets when it
  exceeds the MTU. Layout per packet:
  - FU indicator octet: `F | NRI | 28` (F/NRI copied from the original NAL).
  - FU header octet: `S(1) | E(1) | R(1) | Type(5)` — `S`=start (first frag),
    `E`=end (last frag), `R`=0, `Type`=the original NAL type.
  - fragment payload (the original NAL **without** its first octet; that octet's
    F/NRI/Type is reconstructed on reassembly from the FU indicator+header).
  Reassembly: concat fragments from `S` to `E`, prepend the reconstructed NAL
  octet.

Pick an MTU (e.g. 1400) so large VCL NALs (IDR slices) force FU-A. The marker bit
is set on the packet carrying the last NAL of the access unit.

## AAC payload (RFC 3640, mode `AAC-hbr`)

RTP payload = **AU Header Section** + **AU Data**:
- AU-headers-length: `u16`, the length **in bits** of the AU-header section.
- For `AAC-hbr` with `sizeLength=13; indexLength=3` (and `indexDeltaLength=3`):
  each AU-header is 2 bytes = `AU-size(13 bits) | AU-Index(-delta)(3 bits)`. First
  AU uses AU-Index=0; subsequent use AU-Index-delta=0. One AU per packet is fine.
- AU Data: the raw AAC access unit(s) (the IR's audio frame bytes, no ADTS
  header — RTP carries raw AUs).

## SDP (RFC 4566 + fmtp)

```
m=video 0 RTP/AVP 96
a=rtpmap:96 H264/90000
a=fmtp:96 packetization-mode=1; profile-level-id=<6 hex from SPS>; sprop-parameter-sets=<b64(SPS)>,<b64(PPS)>
m=audio 0 RTP/AVP 97
a=rtpmap:97 mpeg4-generic/<rate>/<channels>
a=fmtp:97 streamtype=5; mode=AAC-hbr; config=<hex(ASC)>; sizeLength=13; indexLength=3; indexDeltaLength=3
```
`profile-level-id` = SPS bytes[1..4] (profile_idc, constraint flags, level_idc)
as 6 hex; `sprop-parameter-sets` = base64 of the raw SPS and PPS NALs;
`config` = the AudioSpecificConfig as hex.

## Streaming timing recovery — per-sample duration from RTP-timestamp deltas

When depayloading RTP incrementally (as opposed to a bulk demux of a pre-recorded
file), a caller consuming real-time packets needs accurate per-sample `duration`
to build correctly-timed track samples. RTP carries only a **presentation
timestamp** (32-bit wire field, RFC 3550 §5.1), which wraps every ~13.6 hours at
90 kHz. The streaming depayloader (`RtpStreamDepacketiser`):

- **Unwraps** the 32-bit RTP timestamp to a monotonic 64-bit value (via the
  standard increment-and-wrap detection idiom: the signed 32-bit delta of the
  wire value against the low 32 bits of the previous unwrapped value).
- **Emits samples with real per-AU duration**: `duration` is the RTP-timestamp
  delta to the *next* access unit's timestamp (i.e., one-AU buffering latency;
  `flush` uses the last-computed duration for the final AU). The IR timescale
  equals the RTP `clock_rate` (video: 90 kHz; AAC: the sample rate), so
  timestamp deltas directly become sample durations.
- **Sets `is_sync`** (keyframe marker) from reassembled access-unit content: IDR
  detection for H.264 video; always `true` for audio (no inter-frame dependencies).
- **Constrains `composition_offset = 0`** (no reorder offset): v1 assumes **low-delay
  H.264 with no B-frame reorder**. RTP carries only PTS; recovering a separate
  DTS when B-frames are present (and therefore computing a non-zero composition
  offset) is future work. Cross-track A/V synchronization via RTCP Sender Report
  NTP/RTP correlation is also out of scope.

## SDP fmtp → CodecConfig mapping (`rtp_sdp`)

The SDP `fmtp` (format-specific media parameters) carries codec-configuration
NALs and parameters. The helper module `rtp_sdp` provides two entry points per
codec: a **full-fmtp-line function** (takes the entire `a=fmtp` attribute value
and parses it) and a **value-level building block** (takes just the extracted
parameter value). Both are described per RFC below.

### Helper functions

- **`fmtp_param(fmtp: &str, key: &str) -> Option<&str>`** — Anchored
  `;`-separated parameter lookup (RFC 4566 §5.14). Accepts the full `a=fmtp`
  line (with optional leading payload-type token) or just the parameter list.
  Matches `key` as a whole parameter name (never a substring), returns the
  trimmed value of the first match, or `None` if absent/empty. Operates on
  `char` boundaries to preserve multibyte UTF-8 values.
- **`rtpmap_clock_rate(rtpmap: &str) -> Option<u32>`** — Parses the clock rate
  from an `a=rtpmap` value (RFC 4566 §6). Handles the optional leading
  payload-type token. Returns `None` on missing or malformed input.

### H.264: `sprop-parameter-sets` → avcC (RFC 6184 §8.1)

**SDP fmtp attribute:**
```
a=fmtp:96 packetization-mode=1; sprop-parameter-sets=<b64(SPS)>,<b64(PPS)>
```

**Full-line entry point:**
- `avc_config_from_fmtp(fmtp: &str)` — Takes the entire `a=fmtp` line, extracts
  `sprop-parameter-sets` via `fmtp_param`, and delegates to
  `avc_config_from_sprop`.

**Value-level entry point:**
- `avc_config_from_sprop(sprop: &str)` — Takes the extracted
  `sprop-parameter-sets` value (comma-separated base64 NAL units: nal_unit_type
  7 for SPS, 8 for PPS). Parses each NAL, extracts profile/level from the first
  SPS bytes `[1..4]` (`profile_indication` / `profile_compatibility` /
  `level_indication`), and returns an `AVCConfigurationBox` (avcC). NAL
  length-prefix size is fixed to 4 bytes (NAL-length-size-minus-one = 3), per
  transmux convention.

### AAC: `config` fmtp → esds (RFC 3640 §4.1)

**SDP fmtp attribute:**
```
a=fmtp:97 streamtype=5; mode=AAC-hbr; config=<hex(ASC)>;
  sizeLength=13; indexLength=3; indexDeltaLength=3
```

**Full-line entry point:**
- `aac_config_from_fmtp(fmtp: &str)` — Takes the entire `a=fmtp` line, extracts
  `config` via `fmtp_param`, and delegates to `aac_config_from_asc_hex`.

**Value-level entry point:**
- `aac_config_from_asc_hex(config_hex: &str)` — Takes the extracted `config`
  value (hex-encoded `AudioSpecificConfig`; ISO/IEC 14496-3 §1.6.2.1). Decodes
  the ASC to recover `samplingFrequencyIndex` (indices 0–12 map to standard
  rates via ISO/IEC 14496-3 Table 1.10; index 15 is an explicit-rate escape)
  and `channelConfiguration`, then constructs a `CodecConfig::Aac` holding an
  `EsdsBox` with the ASC bytes in `DecoderSpecificInfo`,
  ObjectTypeIndication = 0x40 (AAC), and StreamType = 5 (audio). Sample size is
  fixed to 16 bits, per CMAF/fMP4 convention.

## Mapping to transmux (Package ⇄ Unpackage)

- **`RtpPacketiser` : Package** — IR `Media` → RTP packets (per track: single-NAL
  / STAP-A / FU-A for video; AU-hbr for audio) + an SDP string.
- **`RtpDepacketiser` : Unpackage** — RTP packets → IR (reassemble FU-A, split
  STAP-A, strip AU-headers). Round-trips to the original coded samples.
- **`RtpStreamDepacketiser` : streaming Unpackage** — Real-time RTP packet feed
  (`push`) → timed `Sample`s with real per-AU `duration` (from RTP-timestamp
  deltas) and `is_sync` (from IDR detection), carrying the actual `CodecConfig`
  (e.g. from `rtp_sdp` helpers). See [RFC 6184](https://tools.ietf.org/html/rfc6184),
  [RFC 3640](https://tools.ietf.org/html/rfc3640).
