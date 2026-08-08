# FLUTE — File Delivery over Unidirectional Transport

_Source: RFC 6726 §3.4 (Figures 1-4), transcribed_

FLUTE is built on ALC (RFC 5775, see `alc.md`) and LCT (RFC 5651, see `lct.md`).
This document specifies **FLUTE version 2** (the value carried in EXT_FDT's `V`
field). FLUTE adds a File Delivery Table (FDT) carried in-band as objects, plus two
fixed-length LCT header extensions: **EXT_FDT** and **EXT_CENC**.

## TOI = 0 semantics (§3.3, §3.4)

- The TOI value **`0` is reserved** for the delivery of FDT Instances. FDT Instances
  are carried in ALC packets with **TOI = 0** plus a REQUIRED EXT_FDT header
  extension.
- The TOI field MUST be included in all ALC packets sent within a FLUTE session
  (the sole exception being certain control packets, e.g. close-session, that carry
  no object information — those SHALL NOT carry a TOI).
- Each file (TOI > 0) is identified by a TOI and described in the FDT. A non-zero TOI
  not resolved by any FDT is an unmapped object and SHOULD generally be silently
  discarded by a pure FLUTE receiver.
- FDT Instances are uniquely identified by an **FDT Instance ID** (in EXT_FDT), not
  by the TOI (which is always 0 for all FDT Instances). Multiple FDT Instances are
  multiplexed on TOI = 0, distinguished by FDT Instance ID.

## Overall FDT packet (§3.4, Figure 1)

```
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                         UDP header                            |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                Default LCT header (with TOI = 0)              |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|          LCT Header Extensions (EXT_FDT, EXT_FTI, etc.)       |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       FEC Payload ID                          |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
              FLUTE Payload: Encoding Symbol(s)
~             (for FDT Instance in an FDT packet)               ~
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

The EXT_FDT extension is REQUIRED in every ALC packet carrying an FDT Instance, and
is identical across all packets of a given FDT Instance. The FDT Instance itself is
FEC-encoded for transmission like any other object (default FEC: Compact No-Code
FEC Encoding ID 0, RFC 5445 — which implies 16-bit Source Block Number and 16-bit
Encoding Symbol ID).

## EXT_FDT — FDT Instance Header (§3.4.1, Figure 2)

A new **fixed-length** (one 32-bit word) LCT Header Extension. HET = **192**.

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|   HET = 192   |   V   |          FDT Instance ID              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | No. of bits | Mnemonic | Meaning |
|-------|-------------|----------|---------|
| HET | 8 | uimsbf | = **192** (fixed-length form, HET ≥ 128 → no HEL). |
| V (FLUTE version) | 4 | uimsbf | = **2** for this specification. |
| FDT Instance ID | 20 | uimsbf | Uniquely identifies the FDT Instance within the session. |

Semantics:
- Being a fixed-length extension (HET ≥ 128), there is **no HEL byte**; the whole
  extension is exactly one 32-bit word.
- FDT Instance IDs start at 0 and increment by one per FDT Instance, wrapping at
  `2^20 − 1`. The 20-bit field bounds the supply of live IDs. Senders MUST NOT reuse
  an ID currently used by a non-expired FDT Instance.

## EXT_CENC — FDT Instance Content Encoding Header (§3.4.3, Figure 4)

A new **fixed-length** LCT Header Extension. HET = **193**. Used when the FDT
Instance payload is content-encoded (compressed). When present it MUST appear in all
packets carrying the same FDT Instance ID, alongside EXT_FDT.

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|   HET = 193   |     CENC      |          Reserved             |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | No. of bits | Mnemonic | Meaning |
|-------|-------------|----------|---------|
| HET | 8 | uimsbf | = **193** (fixed-length form, no HEL). |
| CENC (Content Encoding Algorithm) | 8 | uimsbf | Content encoding of the FDT Instance payload (see table). |
| Reserved | 16 | bslbf | MUST be all 0; MUST be ignored on reception. |

CENC algorithm values:

| Value | Algorithm |
|-------|-----------|
| 0 | null (none) |
| 1 | ZLIB (RFC 1950) |
| 2 | DEFLATE (RFC 1951) |
| 3 | GZIP (RFC 1952) |

If content encoding is not used for a given FDT Instance, EXT_CENC MUST NOT appear
in any packet for it.

## FDT Instance body — XML, OUT OF BINARY SCOPE

⚠ **The FDT Instance payload itself is an XML document and is OUT OF SCOPE of this
binary transcription.** (§3.4.2)

- The FDT Instance is an XML structure with a single root element `FDT-Instance`,
  containing `File` child elements (one file description entry each), with
  attributes such as `Expires`, `Complete`, `TOI`, `Content-Location`,
  `Content-Length`, `Content-Type`, FEC parameters, etc.
- It is carried as the FEC Encoding Symbol(s) / packet payload of the FDT packets
  shown above — i.e. it sits *after* the FEC Payload ID, in the payload region — and
  may span multiple ALC packets.
- The binary wire concern for this crate is the **LCT/ALC framing + EXT_FDT/EXT_CENC
  header extensions**; the XML parsing/semantics are a separate (text) layer and are
  not transcribed here.
- Note one binary-adjacent field carried *inside* the XML: the `Expires` attribute
  is a UTF-8 decimal representation of a 32-bit unsigned integer = the 32 most
  significant bits of a 64-bit NTP time (seconds since 1900-01-01, with epoch
  wraparound handling). This lives in the XML text, not in a binary header.

## IANA — LCT Header Extension Type registrations (§8.5)

| HET value | Name | Reference |
|-----------|------|-----------|
| 192 | EXT_FDT | RFC 6726 §3.4.1 |
| 193 | EXT_CENC | RFC 6726 §3.4.3 |

(Both are in the 192..255 fixed-length, Specification-Required range of the LCT
Header Extension Type namespace defined in RFC 5651 — see `lct.md`.)
