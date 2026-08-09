# SMPTE ST 2110-20:2022 — Uncompressed Active Video

Source: SMPTE ST 2110-20:2022, "Professional Media Over Managed IP Networks:
Uncompressed Active Video" (revision of ST 2110-20:2017), 23 pages, approved
December 14, 2022. Transcribed from the vendored PDF
(`private/specs/smpte_st2110-20_2022_uncompressed_video_rtp.pdf`) via
`pdf2md` (verified, exit 0) with every bit-layout figure and table
cross-checked against a page-rendered image of the PDF (text-layer
extraction of ASCII-art bit diagrams and multi-column tables is unreliable
around glyph reflow — the figures below were re-read from page images to
confirm field widths/positions).

All bit-field diagrams in this document use the RFC-standard 32-bit-word
notation: row 1 numbers each of the 32 bit positions 0–31 within the word,
row 2 is the byte boundary ruler `+-+-+...+`. Field boxes are read
left-to-right, top-to-bottom, most-significant bit first per §6.1.1.

## 1. Scope (§1)

Real-time, RTP-based transport of uncompressed active video essence over IP
networks. Defines an RTP payload format based on IETF RFC 4175, plus an
SDP-based signaling method for the image technical metadata needed to
receive and interpret the stream.

Normative-precedence rule stated in §2: "Normative prose shall be the
authoritative definition; Tables shall be next; then formal languages; then
figures; and then any other language forms." (i.e. if a figure and the prose
appear to disagree, the prose wins — noted here since this transcription
leans on both.)

## 2. General provisions (§6.1.1)

- Sample arrays of active video essence are transported using RTP (RFC 3550).
- Multi-octet fields in the RTP Header, RTP Payload Header, and RTP Payload
  are Network Byte Order (MSB first) unless otherwise noted.
- In bit-field diagrams, the MSB of a multi-bit field occupies the
  lowest-numbered (left-most) bit position and is transmitted first.
- Image technical metadata is communicated via SDP (§7).
- Traffic shaping / transmission timing must comply with SMPTE ST 2110-21's
  Network Compatibility Model and (when the Media Clock is locked to the
  timestamping reference clock) the Virtual Receiver Buffer Model, for one of
  Narrow (Type N), Narrow Linear (Type NL), or Wide (Type W) senders.
  Receivers must conform to one of the types in ST 2110-21:2022 §7.2. (See
  `st2110-21-traffic.md`.)

## 3. RTP Header (§6.1.2)

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V|P|X|  CC   |M|     PT      |        Sequence Number         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                            Time Stamp                         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                              SSRC                              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

Figure 1 — RTP Header. Fields and their order are as defined in RFC 3550;
the following are the standard's additional constraints:

| Field | Width | Constraint |
|---|---|---|
| Payload Type (PT) | 7 bits | The dynamically allocated payload type per SMPTE ST 2110-10 §6.2 "Real-Time Transport Protocol (RTP)". |
| Sequence Number | 16 bits | The 16 **low-order** bits of the extended 32-bit RTP packet sequence counter. |
| Timestamp | 32 bits | The RTP Timestamp as specified in ST 2110-10. |
| SSRC | 32 bits | As specified in RFC 3550. |
| Marker bit (M) | 1 bit | Progressive: set to 1 on the last packet carrying video essence data for a video **frame**, 0 otherwise. Interlaced: set to 1 on the last packet carrying video essence data for a video **field**, 0 otherwise. |
| Extension bit (X) | 1 bit | When set, an RTP header extension immediately follows SSRC and must comply with RFC 8285. |

## 4. Media Clock, RTP Clock, RTP Timestamps (§6.1.3)

- Comply with SMPTE ST 2110-10.
- **RTP Clock rate is fixed at 90 kHz** for all ST 2110-20 streams.
- All RTP packets belonging to the same progressive frame carry the same RTP
  Timestamp. All RTP packets belonging to the same interlaced field carry the
  same RTP Timestamp.

## 5. RTP Payload Header — Extended Sequence Number + Sample Row Data (SRD) Headers (§6.1.4)

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|   Extended Sequence Number   |          SRD Length            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|F|     SRD Row Number         |C|          SRD Offset          |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|          SRD Length          |F|      SRD Row Number          |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|C|          SRD Offset        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

Figure 2 — RTP Payload Header with Extended Sequence Number and two Sample
Row Data (SRD) Headers (a third SRD Header, if present, repeats the same
16+1+15+1+15-bit shape as the second).

The RTP Payload Header = Extended Sequence Number field, followed by **one,
two, or three** SRD Headers (§6.2.1 caps a single RTP packet at three SRD
Headers — see below).

| Field | Width | Semantics |
|---|---|---|
| Extended Sequence Number | 16 bits | The 16 **high-order** bits of the extended 32-bit sequence number (low-order 16 bits are the RTP header's Sequence Number field, §6.1.2). |
| SRD Length | 16 bits | Octets of data from the indicated sample row, must be a multiple of the pgroup octet length. Zero is **forbidden** except when there is exactly one SRD Header, in which case SRD Length=0 signals that no sample row data follows this header at all. |
| Field Identification (F) | 1 bit | 0 = (temporally) first field, 1 = second field. Progressive: always 0, except Progressive segmented Frame (PsF) data, where F indicates the segment. |
| SRD Row Number | 15 bits | Sample Row Number within the Sample Array; starts at 0 at the top of the transmitted image (each field starts at 0 at its own top, for interlaced; each segment starts at 0, for PsF). Shall only increase within the field/frame (rows sent top-to-bottom). |
| Continuation (C) | 1 bit | 1 = another SRD Header follows this one in the RTP Payload Header (packet carries >1 sample row); 0 otherwise. |
| SRD Offset | 15 bits | Horizontal pixel position (in the full-bandwidth image pixel matrix) of the first full-bandwidth sample in the associated SRD Segment; 0 = left edge of image. Shall only increase within the same sample row (left-to-right). |

Note: the spec states these "shall" constraints on monotonicity but does not
specify receiver error-handling for a nonconformant sender (e.g. a
non-increasing SRD Offset) — left to the implementation.

## 6. Additional constraints on the payload format (§6.1.5)

- Successive RTP packets may carry parts of the same sample row (incremented
  sequence number, same timestamp) to fragment a long line.
- **4:2:0 progressive**: SRD Row Number is set to the first row of the
  paired rows; only every other Sample Row is signaled in SRD Row Number.
- Interlaced fields transmit in time order, first field first.
- Interlaced / PsF systems: if height is even, lines split evenly between
  fields/segments; if odd, the temporally first field/segment gets one more
  line than the second.
- A single RTP packet never spans more than one frame (progressive), one
  field (interlaced), or one segment (PsF).
- Image reconstruction: the temporally second field's (or PsF's second
  segment's) sample rows are spatially "below" the like-numbered rows of the
  first field/segment.

## 7. RTP Payload: Sample Row Data (SRD) Segments (§6.2.1)

- SRD Headers are followed by SRD Segments, except the single-header /
  SRD-Length-0 special case (§5, above) where none follow.
- SRD Segment order in the payload matches SRD Header order; one Segment per
  Header (except that special case).
- Packets at the end of a field/frame may carry padding octets after the
  last Segment (for GPM/BPM purposes, §6.3.2/6.3.3).
- **A single RTP packet shall not contain more than three SRD Headers.**
- Each Segment's length is an integer multiple of the pgroup size in octets.
  UDP size must not exceed the prevailing UDP Size Limit (Standard or
  Extended, ST 2110-10).
- If a sample row's length isn't evenly divisible by pgroup sample count, the
  sender zero-fills the final pgroup's remaining positions; the receiver
  ignores that fill data. The SRD Header's Length value **includes** the fill
  data.

## 8. pgroup — the octet-aligned sample group (§6.2.2)

A **pgroup** is the minimal group of samples that aligns to an octet
boundary; every pgroup is an integer number of octets. Pgroups:

- never fragment across packets;
- never represent samples from more than one image source array line — or
  two source lines, for 4:2:0 sampling (its pgroups straddle a
  luminance-row pair, §6.2.5).

Which samples appear in a pgroup, and their number/position/order, is
determined by the SDP `sampling` parameter (§7.4.1). If subsampled, the
sub-samples are shared only within the pgroup.

"pgroup coverage" (used in the construction tables below) = a contiguous
portion of the sample array in pixels, possibly spanning adjacent lines
within a field/frame.

Numbering convention: an unnumbered component symbol (e.g. `Y'`) means
exactly one sample of that component per pgroup; numeric indices (`C0'B`,
`C1'B`, …) appear when more than one sample of the same component shares a
pgroup, lowest index = left-most in the image.

### 8.1 4:4:4 sampling — Table 1 (§6.2.3)

| sampling | depth | pgroup size (octets) | pgroup coverage (pixels) | Sample Order |
|---|---|---|---|---|
| YCbCr-4:4:4 / CLYCbCr-4:4:4 | 8 | 3 | 1 | C'B, Y', C'R |
| | 10 | 15 | 4 | C0'B,Y0',C0'R, C1'B,Y1',C1'R, C2'B,Y2',C2'R, C3'B,Y3',C3'R |
| | 12 | 9 | 2 | C0'B,Y0',C0'R, C1'B,Y1',C1'R |
| | 16, 16f | 6 | 1 | C'B, Y', C'R |
| ICtCp-4:4:4 | 8 | 3 | 1 | CT, I, CP |
| | 10 | 15 | 4 | C0T,I0,C0P, C1T,I1,C1P, C2T,I2,C2P, C3T,I3,C3P |
| | 12 | 9 | 2 | C0T,I0,C0P, C1T,I1,C1P |
| | 16, 16f | 6 | 1 | CT, I, CP |
| RGB (linear) | 8 | 3 | 1 | R, G, B |
| | 10 | 15 | 4 | R0,G0,B0, R1,G1,B1, R2,G2,B2, R3,G3,B3 |
| | 12 | 9 | 2 | R0,G0,B0, R1,G1,B1 |
| | 16, 16f | 6 | 1 | R, G, B |
| RGB (non-linear) | 8 | 3 | 1 | R', G', B' |
| | 10 | 15 | 4 | R0',G0',B0', R1',G1',B1', R2',G2',B2', R3',G3',B3' |
| | 12 | 9 | 2 | R0',G0',B0', R1',G1',B1' |
| | 16, 16f | 6 | 1 | R', G', B' |
| XYZ | 12 | 9 | 2 | X0',Y0',Z0', X1',Y1',Z1' |
| | 16, 16f | 6 | 1 | X', Y', Z' |

Note: ICTCP nomenclature deliberately omits the prime symbol despite being a
non-linear signal, per ITU-R BT.2100 guidance (§7.4.1 note).

### 8.2 4:2:2 sampling — Table 2 (§6.2.4)

Applies to both `YCbCr-4:2:2`/`CLYCbCr-4:2:2` and `ICtCp-4:2:2`; identical
size/coverage numbers, only the component labels differ.

| depth | pgroup size (octets) | pgroup coverage (pixels) | Sample Order (Y'C'BC'R) | Sample Order (ICTCP) |
|---|---|---|---|---|
| 8 | 4 | 2 | C'B,Y0',C'R,Y1' | C'T,I0',C'P,I1' |
| 10 | 5 | 2 | C'B,Y0',C'R,Y1' | C'T,I0',C'P,I1' |
| 12 | 6 | 2 | C'B,Y0',C'R,Y1' | C'T,I0',C'P,I1' |
| 16, 16f | 8 | 2 | C'B,Y0',C'R,Y1' | C'T,I0',C'P,I1' |

Byte layout, 4:2:2 10-bit (5-octet pgroup), Figure 3:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|      C'B (10 bits)   |     Y0' (10 bits)    |     C'R (10 bits)    |    Y1' (10 bits)     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

The subsampling/co-siting details for a given colorimetry are defined by
that colorimetry's own signal specification, not by this document (§6.2.4
note).

### 8.3 4:2:0 sampling — Table 3 (§6.2.5)

4:2:0 applies **only to progressive-scan images transmitted progressively**
— it does not apply to PsF or interlaced essence.

| sampling | depth | pgroup size (octets) | pgroup coverage (pixels) | Sample Order (Y'C'BC'R) |
|---|---|---|---|---|
| YCbCr-4:2:0 / CLYCbCr-4:2:0 | 8 | 6 | 4 | Y'00-Y'01-Y'10-Y'11-CB'00-CR'00 |
| | 10 | 15 | 8 | Y'00-Y'01-Y'10-Y'11-CB'00-CR'00, Y'02-Y'03-Y'12-Y'13-CB'01-CR'01 |
| | 12 | 9 | 4 | Y'00-Y'01-Y'10-Y'11-CB'00-CR'00 |
| ICtCp-4:2:0 | 8 | 6 | 4 | I00-I01-I10-I11-CT00-CP00 |
| | 10 | 15 | 8 | I00-I01-I10-I11-CT00-CP00, I02-I03-I12-I13-CT01-CP01 |
| | 12 | 9 | 4 | I00-I01-I10-I11-CT00-CP00 |

The color-difference components are subsampled ×2 both horizontally and
vertically, so a chroma sample is shared by a 2×2 luma block. Sample
numbering example (Figure 4, Y'C'BC'R; ICTCP follows the same principle):

```
Y'00  Y'02  Y'04       (row 0, even cols)
CB'00 CB'01 CB'02       (chroma, one pair-row)
CR'00 CR'01 CR'02
Y'01  Y'03  Y'05       (row 0, odd cols — shares CB'00/CR'00 etc. with Y'00)
Y'10  Y'11 Y'12  Y'13 Y'14 Y'15   (row 1 — full res luma, shares row-0 chroma)
Y'20  Y'22  Y'24       (row 2, even cols — next chroma pair)
CB'10 CB'11 CB'12
CR'10 CR'11 CR'12
Y'21  Y'23  Y'25       (row 2, odd cols)
Y'30  Y'31 Y'32  Y'33 Y'34 Y'35   (row 3)
```

i.e. `CB'12`/`CR'12` sit at the position corresponding to `Y'24` — the Y and
C sample arrays have different dimensions due to subsampling (§6.2.5 Note 1).

When packetizing, samples from **two consecutive luminance rows** go into
each pgroup; the SRD Header's Row Number is the **first** row of the pair
(§6.2.5, §6.1.5). Byte layout, 4:2:0 10-bit (15-octet pgroup), Figure 5:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|    Y'00 (10 bits)    |    Y'01 (10 bits)    |    Y'10 (10 bits)    |    Y'11 (10 bits)    |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|   CB'00 (10 bits)    |   CR'00 (10 bits)    |    Y'02 (10 bits)    |    Y'03 (10 bits)    |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|    Y'12 (10 bits)    |    Y'13 (10 bits)    |   CB'01 (10 bits)   |   CR'01 (10 bits)    |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

i.e. the 15-octet pgroup packs *two* rows of 2×2-luma-plus-chroma blocks
(4 pixels of row pair 0 then 4 pixels of row pair 1), matching the 8-pixel
pgroup coverage.

### 8.4 Key (Alpha) signal — Table 4 (§6.2.6)

Key/Alpha signals (SMPTE RP 157) are a **single component** ("Key"),
represented at the signaled depth/width/height/exactframerate.

| sampling | depth | pgroup size (octets) | pgroup coverage (pixels) | Sample Order |
|---|---|---|---|---|
| Key | 8 | 1 | 1 | K |
| | 10 | 5 | 4 | K0, K1, K2, K3 |
| | 12 | 3 | 2 | K0, K1 |
| | 16, 16f | 2 | 1 | K |

## 9. Packing Modes (§6.3)

Senders operate in exactly one of two packing modes and signal it via the
`PM` SDP parameter; receivers must be able to receive either mode.

### 9.1 General Packing Mode — GPM (§6.3.2)

- The Continuation ("C") bit may pack samples from more than one row into a
  packet, to avoid undersized packets.
- Datagrams under 1000 octets should be avoided except at field/frame ends.
- Sender should target packets close to the prevailing UDP Size Limit.
- Last packet of a field/frame may be padded with zero-valued octets.
- Signaled via `PM=2110GPM`.

### 9.2 Block Packing Mode — BPM (§6.3.3)

A constrained subset of GPM:

- The sum of SRD Segment lengths in a packet **shall be a multiple of 180
  octets**; the largest 180-octet multiple consistent with the prevailing
  max UDP size is used.
- The C bit **shall** be used to pack multiple rows per packet, to hold a
  constant count of 180-octet blocks per packet.
- Per ST 2110-10 §5, with a 12-octet (minimum) RTP header, max available
  payload space is **1428 octets** → a payload of **7 × 180 = 1260 octets**
  per packet is used.
- The last packet of a field/frame is exempt from the 180-multiple rule; it
  may be truncated or zero-padded to match the size of the preceding
  packets.
- The **Extended UDP size limit shall not be used** in BPM.
- Signaled via `PM=2110BPM`.
- Informative Annex A (below) tabulates the resulting block sizes for the
  7×180 payload.

## 10. SDP considerations (§7)

Declared with Media Type `video`, Media Subtype `raw`; `a=rtpmap` states the
90 kHz clock. Per RFC 4566/8866 `a=fmtp` syntax: `;`-separated
`<name>=<value>` or bare `<name>` entries, no whitespace inside a name/value
or around `=`, no trailing `;`, terminated by CR.

### 10.1 Required parameters (§7.2)

| Parameter | Meaning |
|---|---|
| `sampling` | color-difference sub-sampling structure — see §10.3 below |
| `depth` | bits per sample — see §10.3 below |
| `width` | pixels per row; integer 1–32767 |
| `height` | full-bandwidth Sample Rows per frame; integer 1–32767 |
| `exactframerate` | fps: integer as a bare decimal (e.g. `25`), non-integer as `<num>/<den>` (e.g. `30000/1001`) using the numerically smallest numerator |
| `colorimetry` | system colorimetry — see §10.4 below |
| `PM` | Packing Mode, `2110GPM` or `2110BPM` (§9) |
| `SSN` | SMPTE Standard Number: `ST2110-20:2017` unless `colorimetry=ALPHA` or `TCS=ST2115LOGS3`, in which case `ST2110-20:2022` |

### 10.2 Parameters with default values (§7.3)

Included only when the default is wrong for the stream's content:

| Parameter | Meaning | Default when absent |
|---|---|---|
| `interlace` | presence = interlaced or PsF video | progressive |
| `segmented` | presence (with `interlace` also present) = PsF; **forbidden** without `interlace` | not PsF |
| `TCS` | Transfer Characteristic System — see §10.5 | `SDR`, unless `sampling` is `KEY` (then TCS is not meaningful) |
| `RANGE` | encoding range. With BT.2100 colorimetry: `NARROW` or `FULL` (BT.2100 Table 9 ranges). Otherwise: `NARROW`, `FULLPROTECT`, or `FULL` (RP 2077 ranges; `FULLPROTECT` = RP 2077:2013 §5 "permitted" range) | `NARROW` |
| `MAXUDP` | Maximum UDP Packet Size (ST 2110-10) | Standard UDP Size Limit |
| `PAR` | Pixel Aspect Ratio, `<width>:<height>` of a luminance sample, smallest integers | `1:1` |

### 10.3 `sampling` and `depth` values (§7.4)

`sampling` (§7.4.1):

| Signal format | 4:4:4 | 4:2:2 | 4:2:0 |
|---|---|---|---|
| Non-constant-luminance Y'C'BC'R (BT.601/709/2020/2100) | `YCbCr-4:4:4` | `YCbCr-4:2:2` | `YCbCr-4:2:0` |
| Constant-Luminance Y'CC'BCC'RC (BT.2020) | `CLYCbCr-4:4:4` | `CLYCbCr-4:2:2` | `CLYCbCr-4:2:0` |
| Constant-intensity ICTCP (BT.2100) | `ICtCp-4:4:4` | `ICtCp-4:2:2` | `ICtCp-4:2:0` |

Plus, 4:4:4-only: `RGB` (RGB or R'G'B', e.g. BT.601/709/2020/2100, ST
2065-1/2065-3), `XYZ` (X'Y'Z', e.g. ST 428-1), `KEY` (single-component Key
signal per RP 157 — colorimetry must be signaled as `ALPHA`, and TCS must
not be signaled for this stream).

`depth` (§7.4.2): `8`, `10`, `12`, `16` (16-bit integer, e.g. ST 2065-3 ADX),
`16f` (16-bit float, e.g. ST 2065-1 / BT.2100).

### 10.4 `colorimetry` values (§7.5)

| Value | Meaning |
|---|---|
| `BT601` | ITU-R BT.601-7 |
| `BT709` | ITU-R BT.709-6 |
| `BT2020` | ITU-R BT.2020-2 |
| `BT2100` | ITU-R BT.2100 Table 2 "System colorimetry" |
| `ST2065-1` | SMPTE ST 2065-1 (ACES) |
| `ST2065-3` | SMPTE ST 2065-3 (ADX) |
| `UNSPECIFIED` | not specified; manual coordination required |
| `XYZ` | CIE 1931 (ISO 11664-1) |
| `ALPHA` | Key signal per RP 157 |

BT.2100 colorimetry should also signal `RANGE` (§7.3).

### 10.5 `TCS` (Transfer Characteristic System) values (§7.6)

| Value | Meaning |
|---|---|
| `SDR` | Standard Dynamic Range; OETF of BT.709 or BT.2020; targets BT.1886 EOTF |
| `PQ` | HDR, BT.2100 Perceptual Quantization |
| `HLG` | HDR, BT.2100 Hybrid Log-Gamma |
| `LINEAR` | linear float samples (`depth=16f`), values in `[0..1.0]` |
| `BT2100LINPQ` | linear float samples normalized from PQ, per BT.2100-0 Table 10 |
| `BT2100LINHLG` | linear float samples normalized from HLG, per BT.2100-0 Table 10 |
| `ST2065-1` | linear float samples per ST 2065-1 |
| `ST428-1` | transfer characteristic of ST 428-1 §4.3 |
| `DENSITY` | density-encoded samples, e.g. ST 2065-3 |
| `ST2115LOGS3` | "Camera Log S3" per SMPTE ST 2115 |
| `UNSPECIFIED` | not specified; manual coordination required |

Default when `TCS` absent: `SDR` (unless the stream is `KEY`, where TCS is
not meaningful — see §10.2).

### 10.6 Worked example (§7.7, informative)

```
m=video 30000 RTP/AVP 112
a=rtpmap:112 raw/90000
a=fmtp:112 sampling=YCbCr-4:2:2; width=1280; height=720; exactframerate=60000/1001; depth=10; TCS=SDR; colorimetry=BT709; PM=2110GPM; SSN=ST2110-20:2017
```

1280×720, 10-bit 4:2:2, 60/1.001 fps, BT.709 colorimetry/TCS, GPM, dest UDP
port 30000, dynamic PT 112, 90 kHz clock.

## 11. Annex A — Block Packing Mode typical packet sizes (informative, §Annex A)

For the mandated 7×180=1260-octet BPM payload (§9.2):

| Sampling | Bit Depth | Octets/pgroup | Pixels/pgroup | pgroups per 180-octet block | 180-blocks/packet | Pixels/packet | Octets/packet |
|---|---|---|---|---|---|---|---|
| 4:2:2 | 8 | 4 | 2 | 45 | 7 | 630 | 1260 |
| 4:2:2 | 10 | 5 | 2 | 36 | 7 | 504 | 1260 |
| 4:2:2 | 12 | 6 | 2 | 30 | 7 | 420 | 1260 |
| 4:4:4 | 8 | 3 | 1 | 60 | 7 | 420 | 1260 |
| 4:4:4 | 10 | 15 | 4 | 12 | 7 | 336 | 1260 |
| 4:4:4 | 12 | 9 | 2 | 20 | 7 | 280 | 1260 |
| 4:4:4 | 16 | 6 | 1 | 30 | 7 | 210 | 1260 |
| 4:2:0 | 8 | 6 | 4 | 30 | 7 | 840 | 1260 |
| 4:2:0 | 10 | 15 | 8 | 12 | 7 | 672 | 1260 |
| 4:2:0 | 12 | 9 | 4 | 20 | 7 | 560 | 1260 |

(All rows verified against a rendered image of the source PDF page — the
text-layer extraction of this table had column misalignment.)

## 12. What this document leaves unstated

- No receiver error-handling is specified for violations of the "shall only
  increase" monotonicity rules on SRD Row Number / SRD Offset (§6.1.4/§6.1.5).
- The exact subsampling filter / co-siting positions for chroma samples are
  deferred entirely to "the applicable signal definition corresponding to
  the system colorimetry" (§6.2.4, §6.2.5 notes) — not specified here.
- No SDP parameter is defined for `TP` (sender type) or `TROFF`/`CMAX` — those
  belong to ST 2110-21 (see `st2110-21-traffic.md`), referenced but not
  reproduced here.
- Bibliography lists RFC 4175 and VSF TR-03/TR-04 as background reading,
  not normative dependencies of this document.
