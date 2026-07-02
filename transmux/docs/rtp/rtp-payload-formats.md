# RTP + payload formats — for the RTP spoke (#469)

Sources: **RFC 3550** (RTP), **RFC 6184** (H.264/AVC payload), **RFC 3640**
(MPEG-4/AAC payload, mode `AAC-hbr`), **RFC 4566** (SDP). Scope for this spoke:
H.264 video + AAC-LC audio de/packetization (the IR's `Avc`/`Aac` tracks), with
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

NAL unit octet: `F(1) | NRI(2) | Type(5)`. Packetization modes; use
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

## Mapping to transmux (Package ⇄ Unpackage)

- **`RtpPacketizer` : Package** — IR `Media` → RTP packets (per track: single-NAL
  / STAP-A / FU-A for video; AU-hbr for audio) + an SDP string.
- **`RtpDepacketizer` : Unpackage** — RTP packets → IR (reassemble FU-A, split
  STAP-A, strip AU-headers). Round-trips to the original coded samples.
