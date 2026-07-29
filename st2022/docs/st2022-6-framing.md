# SMPTE ST 2022-6:2012 — HBRMT framing (RTP header, payload header, media payload)

_Source: SMPTE ST 2022-6:2012, "Transport of High Bit Rate Media Signals over IP
Networks (HBRMT)", 16 pages, approved 2012-10-09. Fetched free from
`https://pub.smpte.org/pub/st2022-6/st2022-6-2012.pdf` (see
[`README.md`](README.md) for provenance) and render-verified locally via the
`pdf2md` tool's `textlayer` engine (exit code 1 with 18 diagnostics — every
diagnostic is on p.12/Figure 5, the **informative** worked TRS-bit-packing
example, and every flagged `0x…` token's engine value and text-layer value
are identical (see [`README.md`](README.md)); nothing in the normative
header tables below triggered a diagnostic)._

Clause numbers below are ST 2022-6's own (§6.3/§6.4/§6.5/§7.1). "Normative"
below follows the standard's own conformance notation (§2): all text is
normative except the Introduction, text explicitly marked "Informative", and
paragraphs starting "Note:".

---

## 1. Media Datagram structure (§6.2, Figure 1)

A **Media Datagram** = one UDP/IP datagram containing:

```
+----------------------------------------------------------------+
|                    RTP Header (12 octets)                      |
+----------------------------------------------------------------+
|       Payload Header (4, 8, 12 or greater if extended)          |
+----------------------------------------------------------------+
|                       Media Payload                             |
+----------------------------------------------------------------+
```

Normative size constraint (§6.2): the transmitted Media Datagram **shall** be
≤ 1500 octets total, and the IP "don't fragment" bit **shall** be set. The
video luminance/color-difference payload **shall** be packed into 1376-octet
Media Payloads (§6.2); the final (partial) datagram of a frame is padded with
additional null octets to reach 1376 octets, and this padding is **not**
RTP-layer padding — the RTP header's `P` bit **shall** be 0 (§6.2).

## 2. RTP/UDP/IP Header (§6.3, Figure 2)

RFC 3550's generic RTP header is used unmodified in shape; ST 2022-6
constrains specific field values.

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|P|X| CC     |M|     PT      |       sequence number         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                            timestamp                           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|            synchronization source (SSRC) identifier            |
+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+
```

| Field | Width | ST 2022-6 constraint (§6.3) |
|---|---|---|
| `V` (Version) | 2 bits | shall be `2` |
| `P` (Padding) | 1 bit | `0` = no padding, `1` = padding present (trailing count octet). Note: the 1376-octet fill of a partial final payload is **not** this padding — `P` shall be `0` for that case (§6.2). |
| `X` (Extension) | 1 bit | shall be `0` — this is the **RTP-level** extension bit (RFC 3550), distinct from the Payload Header's own `Ext` field in §6.4 below, which is a completely separate extension mechanism one layer up. |
| `CC` (CSRC count) | 4 bits | shall be `0` (no CSRC list is present in Media Datagrams) |
| `M` (Marker) | 1 bit | shall be `1` to mark the **last** Media Datagram of the video frame; `0` for all other Media Datagrams |
| `PT` (Payload Type) | 7 bits | dynamically allocated. This standard specifies: `PT=98` = "High bit rate media transport / 27-MHz Clock"; `PT=99` = "High bit rate media transport FEC / 27-MHz Clock". A receiver shall ignore datagrams whose payload type it does not understand. |
| sequence number | 16 bits | low-order RTP sequence counter; shall increment by exactly 1 for each RTP data datagram sent |
| timestamp | 32 bits | sampling instant of the first octet in the datagram; derived from a clock incrementing monotonically/linearly; frequency is indicated by the Payload Header `CF` field (§6.4). Note: sequence number *and* timestamp together can be used to identify Media Datagrams across sequence-number rollover. |
| SSRC | 32 bits | synchronization source identifier; shall comply with RFC 3550 |

Transport-level port assignment (§6.3, normative): the UDP port of the Media
Datagram stream **shall** be unique and different from the FEC ports. When
FEC is used, the Column FEC stream's UDP port **shall** be the media port +
2; the Row FEC stream's UDP port **shall** be the media port + 4.

## 3. High Bit Rate Media Payload Header (§6.4, Figure 3)

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|Ext    |F|VSID | FRCount        | R | S | FEC | CF    | RESERVE |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
| MAP |       FRAME      |     FRATE     |SAMPLE | FMT-RESERVE |     <- only if F=1 (always, per this standard)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                    Video timestamp (only if CF>0)              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   Header extension (only if Ext>0)             |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

### 3.1 Fixed first 32 bits — always present

| Field | Width | Values (§6.4) |
|---|---|---|
| `Ext` (Extension) | 4 bits | `0000` = no extension. `0001`–`1111` = payload header is extended by **this value × 4 octets** beyond the fixed 8-octet header. |
| `F` (Video source format flag) | 1 bit | `0` = Video Source Format row absent, `1` = present. **This standard requires `F=1`** (video source format shall be transmitted) and requires the `F` setting to be constant for the session. |
| `VSID` (Video source ID / protection profile) | 3 bits | `000` = primary stream, `001` = protect stream, `010`–`111` = reserved. Per ST 2022-7 (see [`st2022-7-hitless.md`](st2022-7-hitless.md)), this field's value is expected to be identical across ST 2022-7 datagram copies of the same content. |
| `FRCount` (Frame count) | 8 bits | video frame counter; increments to a new value on the datagram immediately after the frame's `M`-marked (end-of-frame) datagram; rolls over after 256 frames. |
| `R` (Timestamp reference) | 2 bits | `00` = not locked, `01` = reserved, `10` = locked to UTC time/frequency reference, `11` = locked to a private time/frequency reference. |
| `S` (Video payload scrambling) | 2 bits | `00` = not scrambled, `01`/`10`/`11` = reserved for future use. Any non-`00` value is explicitly out of this standard's scope. |
| `FEC` (FEC usage) | 3 bits | `000` = no FEC stream, `001` = L (Column) FEC utilized, `010` = L & D (Column & Row) FEC utilized, all other values reserved. |
| `CF` (Clock Frequency) | 4 bits | `0000` = no timestamp, `0001` = 27 MHz, `0010` = 148.5 MHz, `0011` = 148.5/1.001 MHz, `0100` = 297 MHz, `0101` = 297/1.001 MHz, `0110`–`1111` = reserved. Non-zero `CF` **requires** the sender to include the 32-bit Video Timestamp row; `CF=0000` **requires** the sender to omit it. A compliant receiver shall handle both cases. |
| `RESERVE` | **5 bits** (see note) | reserved for future use; shall be set to `0` by the sender. |

> **Note on `RESERVE`'s width**: §6.4's prose never states a bit count for
> this field directly (unlike every neighboring field, each of which has an
> explicit "N bits" in the running text). The 5-bit figure above is **derived
> by arithmetic**, not read verbatim: Figure 3 fixes the second 16-bit half of
> this row's 32 bits as `R(2) + S(2) + FEC(3) + CF(4) + RESERVE(?)`, and
> 16 − (2+2+3+4) = 5. This is arithmetically forced by the explicitly-stated
> widths of the other four fields against the diagram's 32-bit row boundary,
> so it is not a guess about an unknown quantity — but it is a computed value
> rather than a directly-quoted one, and is flagged as such in
> [`README.md`](README.md) for a second pair of eyes before an implementation
> relies on the exact bit offset.

### 3.2 Video Source Format row (present because `F` shall be 1 in this standard)

**`MAP`** — 4 bits, "shall indicate the top-level structure of the data stream":

| MAP code | Structure |
|---|---|
| `0x00` | Direct sample structure per SMPTE ST 292-1 / SMPTE ST 425-1 Level A, etc. |
| `0x01` | SMPTE ST 425-1 Level B-DL mapping of ST 372 Dual-Link |
| `0x02` | SMPTE ST 425-1 Level B-DS mapping of two ST 292-1 streams |
| `0x03`–`0x0F` | Reserved |

**`FRAME`** — 8 bits, "shall indicate the luminance active pixel structure":

| FRAME code | Horiz. Active | Vert. Active | Vert. Total | Sampling Structure | Transport Structure |
|---|---|---|---|---|---|
| `0x00` | — | — | — | Unknown/Unspecified Frame Structure | — |
| `0x01`–`0x0F` | | | | Reserved | |
| `0x10` | 720 | 486 | 525 | Interlace | Interlace |
| `0x11` | 720 | 576 | 625 | Interlace | Interlace |
| `0x12`–`0x1F` | | | | Reserved | |
| `0x20` | 1920 | 1080 | 1125 | Interlace | Interlace |
| `0x21` | 1920 | 1080 | 1125 | Progressive | Progressive |
| `0x22` | 1920 | 1080 | 1125 | Progressive | Interlace (Segmented) |
| `0x23` | 2048 | 1080 | 1125 | Progressive | Progressive |
| `0x24` | 2048 | 1080 | 1125 | Progressive | Interlace (Segmented) |
| `0x25`–`0x2F` | | | | Reserved | |
| `0x30` | 1280 | 720 | 750 | Progressive | Progressive |
| `0x31`–`0xFF` | | | | Reserved | |

**`FRATE`** — 8 bits, "shall indicate the frame rate of the payload":

| FRATE code | Frame Rate |
|---|---|
| `0x00` | Unknown/Unspecified frame rate, 2.970 GHz signal |
| `0x01` | Unknown/Unspecified frame rate, 2.970/1.001 GHz signal |
| `0x02` | Unknown/Unspecified frame rate, 1.485 GHz signal |
| `0x03` | Unknown/Unspecified frame rate, 1.485/1.001 GHz signal |
| `0x04` | Unknown/Unspecified frame rate, 0.270 GHz signal |
| `0x05`–`0x0F` | Reserved (the PDF prints the boundary as `0x04-0x0f`; `0x04` itself is the defined code directly above, so the reserved range is read as starting at `0x05`) |
| `0x10` | 60 Hz |
| `0x11` | 60/1.001 Hz |
| `0x12` | 50 Hz |
| `0x13` | Reserved |
| `0x14` | 48 Hz |
| `0x15` | 48/1.001 Hz |
| `0x16` | 30 Hz |
| `0x17` | 30/1.001 Hz |
| `0x18` | 25 Hz |
| `0x19` | Reserved |
| `0x1A` | 24 Hz |
| `0x1B` | 24/1.001 Hz |
| `0x1C`–`0xFF` | Reserved |

**`SAMPLE`** — 4 bits, "shall indicate the component pixel sampling structure and bit depth":

| SAMPLE code | Sampling Structure | Bit Depth |
|---|---|---|
| `0x00` | Unknown/Unspecified | — |
| `0x01` | 4:2:2 | 10 bits |
| `0x02` | 4:4:4 | 10 bits |
| `0x03` | 4:4:4:4 | 10 bits |
| `0x04` | Reserved | — |
| `0x05` | 4:2:2 | 12 bits |
| `0x06` | 4:4:4 | 12 bits |
| `0x07` | 4:4:4:4 | 12 bits |
| `0x08` | 4:2:2:4 | 12 bits |
| `0x09`–`0x0F` | Reserved for future use | — |

**`FMT-RESERVE`** — 8 bits: reserved for future use, shall be set to `0` by the sender.

### 3.3 Video Timestamp (present only if `CF` ≠ `0000`)

32 bits. Value of a free-running counter synchronous with the interface word
clock of the encapsulated video, at the frequency indicated by `CF`. Fixed at
the transmitter to the first pixel whose complete data word is contained in
the current datagram (if a pixel value straddles two datagrams, the
timestamp applies to the first pixel *fully* contained in the current one).

### 3.4 Header Extension (present only if `Ext` ≠ `0000`)

`Ext × 4` octets of **TLV** (Tag/Length/Value):

| Sub-field | Width | Meaning |
|---|---|---|
| `T` (Tag) | 1 byte | `0` = special PAD tag (no `L` field follows — this copes with padding an odd byte count). `1`–`0xFF` = free to allocate. |
| `L` (Length) | 1 byte | number of subsequent bytes forming `V` (absent when `T=0`) |
| `V` (Value) | `L` bytes | opaque payload for this tag |

## 4. High Bit Rate Media Payload (§6.5, Figure 4)

Nominal payload = 1376 octets of Media Data. The **last** payload of a media
frame carries `Media data payload` followed by `Reserved` fill octets to
reach 1376 total (§6.2/§6.5) — this reserved fill is distinct from RTP-level
padding (§6.3's `P` bit stays `0`).

Normative packing rules (§6.5):
- 10-bit or 12-bit luminance/color-difference samples (per `FRAME`/`FRATE`/`SAMPLE`) are carried in **original form** — no NRZI encoding/scrambling.
- **All TRS, VANC, and HANC are included in the media data payload** — i.e. the entire serial-digital-interface payload (video + embedded audio + embedded ancillary data, all as raw SDI HANC/VANC) is encapsulated as one opaque byte stream. ST 2022-6 defines **no separate audio-specific framing or mapping** — embedded audio arrives exactly as it sits in the SDI signal's HANC space, and this standard does not parse or describe that embedded-audio sub-structure itself (see [`README.md`](README.md) "could not establish").
- The start of a digital frame is the EAV sequence immediately preceding the `0H` datum of line 1 (HD formats and 625 systems) or line 4 (525 systems), per ITU-R BT.656-5.
- The first sample of each frame is encapsulated as the first octets of the datagram immediately following the end-of-frame datagram; its 8 most-significant bits go directly into the first octet, remaining bits left-justified into subsequent octets.

Last-payload sizing (§6.5, normative equations — `PL`=pixels/line from `FRAME`/`FRATE`, `BS`=bits/sample from `SAMPLE`, `OL`=octets/line, `LF`=lines/frame from `FRAME`, `OF`=octets/frame, `DPF`=datagram payloads/frame, `LPO`=last-payload media octets):

```
4:2:2:      OL = PL * BS * 2/8
4:4:4/4:4:4:4: OL = PL * BS * 4/8
OF = OL * LF
DPF = floor(OF / 1376) + 1
LPO = OF - (1376 * (DPF - 1))
```

## 5. Frame/field structure summary

- One video frame = a run of consecutive Media Datagrams, RTP sequence
  number incrementing by 1 each; the last datagram of the frame has RTP `M=1`.
- `FRCount` in the Payload Header increments once per frame (mod 256),
  independent of/in addition to the RTP sequence number, so a frame boundary
  is identifiable even if the `M` bit or a datagram is lost.
- Interlaced formats are signalled structurally through the `FRAME` table
  (Sampling Structure / Transport Structure columns: e.g. `0x20` is a fully
  interlaced 1125-line signal; `0x22`/`0x24` are progressive-segmented-frame
  variants) rather than via a separate field/frame flag in the payload
  header — ST 2022-6 does not define a distinct "field" framing construct
  beyond what the `FRAME` code implies.

## 6. FEC interoperability (§7.1, normative limits only — not a wire format)

ST 2022-6 only bounds the FEC matrix parameters; the actual FEC Header/FEC
Payload wire format is defined in the separate SMPTE ST 2022-5 standard
(not obtained — see [`README.md`](README.md)).

- Column-only FEC: `1 ≤ L ≤ 1020`, `4 ≤ D ≤ 255`.
- Column-and-row FEC: `4 ≤ L ≤ 1020`, `4 ≤ D ≤ 255`.
- `L × D ≤ 1500` for SD (270 Mb/s); `≤ 3000` for HD (1.485 Mb/s); `≤ 6000` for 3G (2.97 Gb/s).
- Both Block-Aligned and Non-Block-Aligned FEC matrix interleaves shall be supported by a compliant device.
