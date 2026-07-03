# KLV Framing + MISB ST 0601 UAS Datalink Local Set

> Sources:
> - MISB ST 0601 (public via Wikipedia Commons): https://upload.wikimedia.org/wikipedia/commons/1/19/MISB_Standard_0601.pdf
>   (direct NGA URL 403 Forbidden; Wikipedia mirror is public)
> - klvdata Python library (MIT, open source): https://github.com/paretech/klvdata
> - KLV Wikipedia: https://en.wikipedia.org/wiki/KLV
> - RFC 6597 "RTP Payload Format for SMPTE ST 336 Encoded Data": https://datatracker.ietf.org/doc/html/rfc6597
> - jmisb API docs (Java MISB library): https://westridgesystems.github.io/jmisb/
> - SMPTE ST 336 (PAID — NOT freely available): https://pub.smpte.org/pub/st336/st0336-2017.pdf (paywall)
>
> NOTE: SMPTE ST 336:2017 "Data Encoding Protocol Using Key-Length-Value" is the authoritative KLV standard.
> It is **NOT freely downloadable** (paywalled at SMPTE). However, sufficient framing details are recoverable
> from MISB ST 0601, RFC 6597, and open-source implementations to implement a conformant parser/encoder.
> See §7 Gap Assessment.

---

## 1. KLV Framing Basics (SMPTE ST 336 / ISO/IEC 8825-1)

KLV = Key, Length, Value triplets. Each triplet carries one metadata item.

### 1.1 Key (K)

Keys can be **1, 2, 4, or 16 bytes**. The size is agreed upon by the Local Set definition.

- **16-byte keys** are SMPTE-administered **Universal Labels (UL)**. They serve as globally registered unique identifiers for Global Sets and the outermost Local Set wrapper.
- **1-byte keys** (BER-OID encoded) are used for items within a Local Set.

The UAS Datalink Local Set (MISB ST 0601) uses:
- 16-byte UL key for the outer LS wrapper
- 1-byte BER-OID tags for inner items (tags 0x01–0x7F for values ≤ 127; tags requiring multi-byte OID encoding for larger values)

### 1.2 Length (L) — BER Encoding

BER length encoding from ISO/IEC 8825-1 §8.1.3:

**Short form** (value 0–127):
```
[1 byte] where bit 7 = 0, bits 6:0 = length
```
Example: length 42 → `0x2A`

**Long form** (value 128–∞):
```
[1 byte] 0x80 | N   — N = number of subsequent length bytes (1–126)
[N bytes]           — unsigned big-endian integer = actual length
```
Examples:
- Length 200 → `0x81 0xC8` (N=1, 0xC8=200)
- Length 1000 → `0x82 0x03 0xE8` (N=2, 0x03E8=1000)
- Indefinite form (`0x80` alone) is defined in BER but NOT used in KLV/MISB

**Decoding algorithm (Python-style pseudocode):**
```python
first_byte = read(1)
if first_byte < 128:              # short form
    length = first_byte
else:
    n = first_byte & 0x7F         # long form: n = number of length bytes
    length = int.from_bytes(read(n), 'big')
```

### 1.3 Value (V)

Raw bytes of the encoded field value, exactly `length` bytes long.

### 1.4 BER-OID Tag Encoding (for Local Set item keys)

Within the UAS Local Set, each item's key is a 1-byte BER-OID integer (0x01–0x7F for the current MISB 0601 tag range). Tags ≤ 127 fit in one byte. If future tags exceed 127, multi-byte BER-OID encoding would apply (continuation bytes with high bit set), but all current ST 0601 tags fit in one byte.

---

## 2. UAS Datalink Local Set — Universal Label (16-byte key)

The outer KLV packet for MISB ST 0601 is identified by this 16-byte Universal Label:

```
06 0E 2B 34 – 02 0B 01 01 – 0E 01 03 01 – 01 00 00 00
```

In hex groups:
```
Byte  0: 0x06   SMPTE Universal Label prefix
Byte  1: 0x0E   
Byte  2: 0x2B   
Byte  3: 0x34   
Byte  4: 0x02   Category
Byte  5: 0x0B   
Byte  6: 0x01   
Byte  7: 0x01   
Byte  8: 0x0E   
Byte  9: 0x01   
Byte 10: 0x03   
Byte 11: 0x01   
Byte 12: 0x01   
Byte 13: 0x00   
Byte 14: 0x00   
Byte 15: 0x00   
```

(Source: klvdata library `misb0601.py` constant, confirmed by multiple open-source implementations.)

Note: Haivision documentation mentions the first 5 preamble bytes `06 0E 2B 34 02` are used for KLV packet framing sync.

---

## 3. UAS Local Set Packet Structure

A complete MISB ST 0601 packet:

```
[16 bytes] Universal Label Key    — 06 0E 2B 34 02 0B 01 01 0E 01 03 01 01 00 00 00
[L bytes]  BER-encoded length     — total byte count of Value section
[V bytes]  Value                  — sequence of KLV items (Local Set items)
```

The Value section is a concatenated sequence of inner KLV items:

```
[1 byte]  tag key (BER-OID)
[L bytes] BER length
[V bytes] item value bytes
[1 byte]  tag key ...
```

**Ordering rules (MISB ST 0601):**
- Tag 2 (Precision Time Stamp) MUST appear **first** in the Local Set.
- Tag 1 (Checksum) MUST appear **last** in the Local Set.
- All other tags may appear in any order between those two.

---

## 4. Key Tag Definitions

Source: MISB ST 0601, klvdata library, jmisb API docs.

### Tag 1 — Checksum

| Field | Value |
|-------|-------|
| Tag byte | `0x01` |
| Length | 2 bytes |
| Encoding | CRC-16/CCITT (polynomial 0x1021, initial value 0xFFFF) |
| Coverage | Lower 16 bits of summation over the **entire LS packet** including the 16-byte UL key and the 1-byte checksum length field |
| Position | MUST be the last item in the Local Set |

Layout:
```
01             -- tag key
02             -- length (always 2)
HH LL          -- 16-bit CRC, big-endian
```

### Tag 2 — Precision Time Stamp

| Field | Value |
|-------|-------|
| Tag byte | `0x02` |
| Length | 8 bytes |
| Encoding | unsigned 64-bit integer, big-endian |
| Units | microseconds elapsed since POSIX epoch (midnight UTC, 1970-01-01), **excluding leap seconds** |
| Position | MUST be the first item in the Local Set |
| UDS Key | `06 0E 2B 34 01 01 01 03 07 02 01 01 01 05 00 00` |

Layout:
```
02             -- tag key
08             -- length (always 8)
TT TT TT TT TT TT TT TT  -- u64 BE microseconds since epoch
```

Example: 2024-01-01 00:00:00.000000 UTC = 1704067200000000 µs = `0x00 06 0D 34 86 C1 C0 00`

### Tag 3 — Mission ID

| Tag | Length | Encoding |
|-----|--------|---------|
| `0x03` | variable | UTF-8 string |

### Tag 4 — Platform Tail Number

| Tag | Length | Encoding |
|-----|--------|---------|
| `0x04` | variable | UTF-8 string |

### Tag 5 — Platform Heading Angle

| Tag | Length | Encoding | Range |
|-----|--------|---------|-------|
| `0x05` | 2 bytes | u16 BE | 0–360° mapped to 0–65535 |

### Selected tag table (tags 1–25, representative)

| Tag | Hex | Name | Length (bytes) | Type/Notes |
|-----|-----|------|----------------|------------|
| 1 | 0x01 | Checksum | 2 | CRC-16/CCITT, last in LS |
| 2 | 0x02 | Precision Time Stamp | 8 | u64 BE, µs since POSIX epoch, first in LS |
| 3 | 0x03 | Mission ID | var | UTF-8 string |
| 4 | 0x04 | Platform Tail Number | var | UTF-8 string |
| 5 | 0x05 | Platform Heading Angle | 2 | u16 BE, 0–360° |
| 6 | 0x06 | Platform Pitch Angle | 2 | i16 BE (signed), ±20° |
| 7 | 0x07 | Platform Roll Angle | 2 | i16 BE (signed), ±50° |
| 8 | 0x08 | Platform True Airspeed | 1 | u8, m/s |
| 9 | 0x09 | Platform Indicated Airspeed | 1 | u8, m/s |
| 10 | 0x0A | Platform Designation | var | UTF-8 string |
| 11 | 0x0B | Image Source Sensor | var | UTF-8 string |
| 12 | 0x0C | Image Coordinate System | var | UTF-8 string |
| 13 | 0x0D | Sensor Latitude | 4 | i32 BE (signed), ±90° |
| 14 | 0x0E | Sensor Longitude | 4 | i32 BE (signed), ±180° |
| 15 | 0x0F | Sensor True Altitude | 2 | u16 BE, meters (offset IMSL) |
| 16 | 0x10 | Sensor Horizontal FOV | 2 | u16 BE, 0–180° |
| 17 | 0x11 | Sensor Vertical FOV | 2 | u16 BE, 0–180° |
| 18 | 0x12 | Sensor Relative Azimuth Angle | 4 | u32 BE, 0–360° |
| 19 | 0x13 | Sensor Relative Elevation Angle | 4 | i32 BE (signed), ±180° |
| 20 | 0x14 | Sensor Relative Roll Angle | 4 | u32 BE, 0–360° |
| 21 | 0x15 | Slant Range | 4 | u32 BE, meters |
| 22 | 0x16 | Target Width | 2 | u16 BE, meters |
| 23 | 0x17 | Frame Center Latitude | 4 | i32 BE (signed), ±90° |
| 24 | 0x18 | Frame Center Longitude | 4 | i32 BE (signed), ±180° |
| 25 | 0x19 | Frame Center Elevation | 2 | u16 BE, meters (offset) |

The standard defines 93+ tags through tag 0x69 (105) and beyond in later revisions. Full enumeration: https://westridgesystems.github.io/jmisb/org/jmisb/api/klv/st0601/UasDatalinkTag.html

---

## 5. Complete Example KLV Packet (annotated)

Minimal valid packet containing only Precision Timestamp + Checksum:

```
UL key (16 bytes):
  06 0E 2B 34 02 0B 01 01 0E 01 03 01 01 00 00 00

Length of Value section: 13 bytes total (tag2=10 bytes + tag1=4 bytes)
BER short form: 0D

Value section (13 bytes):
  -- Tag 2: Precision Time Stamp --
  02           tag = 2
  08           length = 8
  00 04 59 F4 A6 AA 4A 00   example timestamp (µs since epoch)

  -- Tag 1: Checksum (last) --
  01           tag = 1
  02           length = 2
  XX XX        CRC-16/CCITT over bytes [0..28] (entire packet up to CRC field)
```

Full packet hex (30 bytes, excluding the actual CRC):
```
06 0E 2B 34 02 0B 01 01 0E 01 03 01 01 00 00 00  <- UL (16 bytes)
0D                                                <- BER length 13
02 08 00 04 59 F4 A6 AA 4A 00                    <- tag 2, len 8, timestamp
01 02 XX XX                                       <- tag 1, len 2, CRC
```

---

## 6. KLV-over-RTP (RFC 6597)

Source: https://datatracker.ietf.org/doc/html/rfc6597

RFC 6597 defines the RTP payload format for SMPTE ST 336 encoded KLV data.

### 6.1 RTP Payload Structure

**No custom payload header.** The RTP payload begins directly with the first byte of the KLV data (the first byte of the UL key for a complete packet, or a continuation fragment for fragmented packets). Standard RTP header only.

```
RTP Packet:
  [RTP Header — 12 bytes standard]
  [KLV data — raw bytes, no framing overhead]
```

### 6.2 KLV Unit Mapping

A **KLVunit** = one logical group of KLV items presented at a specific time (corresponds to one MISB LS packet).

Mapping rules:
- One RTP packet payload contains exactly **one KLVunit or a fragment thereof**.
- If the KLVunit fits in a single RTP packet, it is sent complete.
- For large KLVunits: fragment sequentially across multiple RTP packets.

### 6.3 Fragmentation

When a KLVunit must be split across multiple RTP packets:
- Fragment in sequential byte order: reassembly = concatenate payloads in RTP sequence number order.
- All fragment packets of one KLVunit share **the same RTP timestamp**.
- The KLVunit MUST NOT be split within a compound item (a nested set/pack must stay in one KLVunit).

### 6.4 Marker Bit (M bit)

| Condition | M bit |
|-----------|-------|
| Final (or only) packet of a KLVunit | 1 |
| All other packets (intermediate fragments) | 0 |

The M bit = 1 signals the receiver that a complete KLVunit has been received and can be processed.

### 6.5 RTP Timestamp and Clock

- The RTP timestamp encodes the **presentation time** of the KLVunit.
- All fragments of a KLVunit have the **identical** RTP timestamp.
- Clock rate: specified in the SDP `a=rtpmap` `rate` parameter (typically 90000 Hz for video-synchronised KLV, or application-defined).
- For MISB 0601, the Precision Time Stamp (tag 2) inside the KLV provides higher-precision timing independent of RTP timestamp resolution.

### 6.6 SDP Media Type

```
m=application <port> RTP/AVP <pt>
a=rtpmap:<pt> smpte336m/<clockrate>
```

MIME type: `application/smpte336m`

---

## 7. Gap Assessment: SMPTE ST 336 vs Free Sources

### What SMPTE ST 336 (paid) would add

SMPTE ST 336 is the authoritative specification for:
- The formal 16-byte SMPTE Universal Label structure (16-byte UL bit-field breakdown, designation bytes, category/registry fields)
- Formal BER encoding normative reference
- Global Set, Local Set, Variable Length Pack, Defined Length Pack definitions
- The SMPTE Universal Label register

**Cost:** ~$90 USD from shop.smpte.org. Not freely available.

### What is available without SMPTE ST 336

From MISB ST 0601 (public Wikipedia mirror), klvdata (MIT open source), RFC 6597 (free IETF), jmisb API docs, and Wikipedia, the following is fully determined:

| Item | Available? | Source |
|------|-----------|--------|
| 16-byte UL key for UAS LS | YES | klvdata source code |
| BER short-form length encoding | YES | klvdata source + Wikipedia |
| BER long-form length encoding | YES | klvdata source + Wikipedia |
| BER-OID 1-byte tag encoding (for ST 0601 range) | YES | klvdata source |
| Inner KLV triplet structure | YES | multiple open sources |
| Outer packet = UL + BER length + Value | YES | klvdata + RFC 6597 |
| Tag ordering rules (tag 2 first, tag 1 last) | YES | MISB ST 0601 public docs |
| Tag 1 (Checksum) byte layout + CRC algorithm | YES | klvdata + MISB public |
| Tag 2 (Precision Timestamp) byte layout | YES | klvdata + MISB public |
| Tags 3–105: names, sizes, encoding types | MOSTLY | jmisb API docs + klvdata |
| RTP framing (fragmentation, M bit, timestamp) | YES | RFC 6597 (free) |
| Formal UL bit-field registry semantics | NO | Requires SMPTE ST 336 |
| Multi-byte BER-OID tags (tags > 127) | PARTIAL | General BER; MISB doesn't currently use them |

### Conclusion

For implementing a **KLV parser/encoder for MISB ST 0601** (UAS datalink metadata) and **KLV-over-RTP** carriage, the free public sources (MISB ST 0601 public PDF + klvdata open source + RFC 6597) provide sufficient byte-level detail. The paid SMPTE ST 336 adds formal normative authority and the UL bit-field registry, but **does not block implementation** — the actual encoding rules are fully recoverable from open sources.

The only capability genuinely gated behind SMPTE ST 336 would be correctly implementing or registering **new Universal Labels** for non-MISB applications. For consuming/producing standard MISB ST 0601 metadata, SMPTE ST 336 is not required.
