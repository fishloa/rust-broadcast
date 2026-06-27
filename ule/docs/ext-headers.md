# ULE Type field & Extension Headers

_Source: RFC 4326 §4.4 / §5, RFC 5163 §3, transcribed_

## Type field interpretation (RFC 4326 §4.4)

The 16-bit Type field in the SNDU base header (`sndu.md`) is split at decimal **1536
(0x0600)**, mirroring the Ethernet length/EtherType convention:

- **Type < 1536 (0x0000–0x05FF)** — Next-Header. Indicates a link-specific protocol
  and/or the presence of an Extension Header. IANA-assigned (§4.4.1).
- **Type >= 1536 (0x0600–0xFFFF)** — EtherType of the carried PDU, identical to the IANA
  EtherType registry values for Ethernet (§4.4.2). Used here purely as a type code, not
  as a frame-length indicator.

ULE always carries an explicit Length field in the SNDU header, so the Ethernet
LLC-length mode of identification is not needed; values below 1536 are repurposed as
Next-Header codes.

## Next-Header / Extension-Header structure (RFC 4326 §5)

A ULE Extension Header is identified by a 16-bit Type-field value < 1536, organised as a
5-bit zero prefix, a 3-bit `H-LEN`, and an 8-bit `H-Type` (Figure 7):

```
 0                   1
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|0 0 0 0 0|H-LEN|    H-Type     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
        Figure 7: ULE Next-Header Field
```

| Field | Bits | Mnemonic | Meaning |
|-------|------|----------|---------|
| zero prefix | `[15:11]` (5 b) | — | Always `00000` for a Next-Header (this is what keeps the value < 1536 = 0x0600). |
| `H-LEN` | `[10:8]` (3 b) | uimsbf | Extension-header class/length selector (table below). |
| `H-Type` | `[7:0]` (8 b) | uimsbf | One of 256 Mandatory or 256 Optional extension-header types (IANA registry). |

### H-LEN assignment (RFC 4326 §5)

| H-LEN | Meaning |
|-------|---------|
| 0 | **Mandatory** Extension Header (length not in H-LEN; pre-defined per H-Type, unbounded). |
| 1 | Optional Extension Header, length **2 B** (Type only). |
| 2 | Optional Extension Header, length **4 B** (Type + 2 B). |
| 3 | Optional Extension Header, length **6 B** (Type + 4 B). |
| 4 | Optional Extension Header, length **8 B** (Type + 6 B). |
| 5 | Optional Extension Header, length **10 B** (Type + 8 B). |
| >= 6 | The combined H-LEN and H-Type values indicate the **EtherType** of a PDU that directly follows this Type field. |

- For Optional headers, **H-LEN is the total number of bytes in the extension, including
  the 2-byte Type field**.
- **Mandatory** (H-LEN = 0) headers have a pre-defined length per H-Type, not signalled
  in H-LEN; there is no maximum length limit. A Mandatory Extension Header MAY modify the
  format/encoding of the enclosed PDU (e.g. encryption/compression).
- The H-Type field selects one of 256 Mandatory or 256 Optional headers. Optional
  extensions are registered in the form H=1 (decimal 256–511) but may be used with any
  H-LEN value 1–5.
- Note that for H-LEN >= 6 the 16-bit field is itself the EtherType (>= 1536), i.e. the
  same value space as a Type-2 (EtherType) Type field.

### Chained extension headers (RFC 4326 §5, Figures 8–9)

Extension Headers may be chained. Each header begins with a Type field; if that Type is
< 1536 it indicates a further Extension Header, otherwise it is the EtherType of the PDU.

```
< --------------------------   SNDU   ------------------------- >
+---+--------------------------------------------------+--------+
|D=0| Length | T1 | NPA Address | H1 | T2 |    PDU     | CRC-32 |
+---+--------------------------------------------------+--------+
< ULE base header >             <  ext 1  >
        Figure 8: one Extension Header (D=0)

< --------------------------   SNDU   ------------------------- >
+---+---------------------------------------------------+--------+
|D=1| Length | T1 | H1 | T2 | H2 | T3 |       PDU       | CRC-32 |
+---+---------------------------------------------------+--------+
< ULE base header >< ext 1  >< ext 2  >
        Figure 9: two Extension Headers (D=1)
```

- `T1` = base-header Type (a Next-Header value).
- `H1`/`H2` = the fields defined for that header type (0 or more bytes).
- The final Type field (`T2`/`T3` above) holds either the next Next-Header, or an
  EtherType (>= 1536) giving the PDU type.
- When `D=0`, the NPA Address (6 B) follows the base header before the first extension's
  fields.

## Defined Next-Header / Extension-Header type registry

Values assigned across RFC 4326 §4.4.1, §5 and RFC 5163 §3/§4.

| H-Type | Name | Class | H-LEN | Source |
|--------|------|-------|-------|--------|
| `0x0000` (0) | Test SNDU | Mandatory | 0 | RFC 4326 §5.1 |
| `0x0001` (1) | Bridged Frame | Mandatory | 0 | RFC 4326 §5.2 |
| `0x0002` (2) | MPEG-2 TS-Concat | Mandatory | 0 | RFC 5163 §3.1 |
| `0x0003` (3) | PDU-Concat | Mandatory | 0 | RFC 5163 §3.2 |
| `257` (0x0101) | TimeStamp | Optional | 3 | RFC 5163 §3.3 |
| `0x0100` (256) | Extension-Padding | Optional | 1–5 | RFC 4326 §5.3 |

> The Type-field examples `0x0800` (IPv4) and `0x86DD` (IPv6) are EtherTypes (>= 1536),
> not extension headers — see `sndu.md`.

### Test SNDU — Mandatory, Type 0x0000 (RFC 4326 §5.1)

Must be the final (or only) extension header in the chain. The structure of the Data
portion is undefined; Receivers MAY log reception but MUST then discard the Test SNDU.
The D-bit MAY be set. (Figure 10: base header + Data + CRC-32.)

### Bridged Frame — Mandatory, Type 0x0001 (RFC 4326 §5.2)

Must be the final (or only) extension header. The extension carries the bridged MAC
frame's addressing, followed by the bridged frame contents (Figures 11/12):

| Field | Width | Notes |
|-------|-------|-------|
| `MAC Destination Address` | 6 B | Inner bridged-frame destination MAC (distinct from the SNDU NPA). |
| `MAC Source Address` | 6 B | Encapsulator's MAC source. |
| `EtherType/LLC-Length` | 2 B | EtherType (DIX) if >= 1536, else an LLC Length field (IEEE 802.3). |
| `Contents of bridged MAC frame` | variable | The bridged payload. |

- With `D=0` (Figure 11) the SNDU NPA precedes these fields; with `D=1` (Figure 12) the
  bridged MAC Destination Address follows the base header directly.
- A frame Type < 1536 introduces an LLC Length field; the Receiver MUST check it and
  discard frames whose length exceeds the SNDU payload size.
- The inbound Ethernet LAN FCS MUST be checked then removed (not forwarded); the ULE
  CRC-32 is appended instead.

### MPEG-2 TS-Concat — Mandatory, Type 0x0002 (RFC 5163 §3.1)

Transports one or more whole 188-byte MPEG-2 TS Packets within a ULE SNDU (Figures 1/3).

```
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|0|           Length  (15b)     |         Type = 0x0002         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|            Receiver Destination NPA Address  (6B)             |  [D=0 only]
+                               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                               |   TS-Packet 1 ...             |
=        TS-Packet 1, TS-Packet 2 (if Length > 2*188), etc.     =
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                             (CRC-32)                          |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
       Figures 1 (D=0) / 3 (D=1): TS-Packet payload
```

- Number of TS Packets = `(Length - N - 10 + D*6) / 188`, where `N` = bytes of any
  extension headers preceding the TS-Concat extension (0 if none) and `D` = the D-bit.
  - For ULE D=0, no other extensions: `(Length - 10) / 188`.
  - For ULE D=1, no other extensions: substitute D=1 → `(Length - 10 + 6) / 188 =
    (Length - 4) / 188`.  ⚠ (Derived from the §3.1 formula; the RFC states the GSE
    equivalent as `(Length - 6) / 188`, not the ULE D=1 value, so the D=1 ULE figure is
    inferred from the general formula, not stated verbatim.)
- A valid Length corresponds to an integral number of TS Packets; a non-zero remainder
  mod 188 MUST cause discard of all encapsulated TS Packets (TS-Concat size mismatch).
- NULL TS Packets SHOULD NOT be sent this way.

### PDU-Concat — Mandatory, Type 0x0003 (RFC 5163 §3.2)

Carries a sequence of (usually short) PDUs of a common ULE Type within one SNDU Payload
(Figures 4/6):

```
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|0|           Length  (15b)     |         Type = 0x0003         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|            Receiver Destination NPA Address  (6B)             |  [D=0 only]
+                               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                               |        PDU-Concat-Type        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|R|      PDU-1-Length  (15b)    |          PDU-1 ...            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+                               +
                          ... more PDUs as required
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                             (CRC-32)                          |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Width | Mnemonic | Notes |
|-------|-------|----------|-------|
| `PDU-Concat-Type` | 16 b | uimsbf | Common ULE Type of every concatenated PDU (any registry value except a recursive PDU-Concat type). |
| `R` | 1 b | bslbf | Reserved, MUST be 0; Receivers MUST ignore. MSB of each PDU-x-Length. |
| `PDU-x-Length` | 15 b | uimsbf | Length in bytes of the x-th PDU (MUST be < 32758). |
| `PDU-x` | variable | — | The x-th PDU; not necessarily aligned to 16/32-bit boundaries. |

- The base-header Length is the combined length of all PDUs (including each PDU's length
  prefix). The Receiver MUST verify that the sum of PDU lengths equals the SNDU Length;
  inconsistency SHOULD discard the whole SNDU (PDU-Concat size mismatch). A receiver MUST
  NOT forward a partial PDU whose indicated length exceeds the remaining payload.
- Must follow any other Extension Headers (e.g. TimeStamp) that apply to the composite
  PDU.

### TimeStamp — Optional, Type 257 decimal, H-LEN 3 (RFC 5163 §3.3)

Adds a 32-bit timestamp to an SNDU. Figure 7 shows the H-LEN=3 form (4-byte extension =
2-byte Type + 4-byte value... see note):

```
 0               7               15              23              31
+---------------+---------------+---------------+---------------+
|     0x03      |      0x01     |        TimeStamp HI           |
+---------------+---------------+---------------+---------------+
|          TimeStamp LO         |            Type               |
+---------------+---------------+---------------+---------------+
        Figure 7: 32-bit TimeStamp Extension Header
```

| Field | Width | Notes |
|-------|-------|-------|
| Type field (`0x03 0x01` = 0x0301) | 16 b | H-LEN=3 (0x03 high nibble region), H-Type=0x01 → decimal 257. |
| `TimeStamp HI` + `TimeStamp LO` | 32 b | 1-microsecond ticks past the hour in Universal Time at encapsulation. Right-justified; unused LSBs padded with 0. |
| `Type` (trailing) | 16 b | Type of the carried PDU or a further Next-Header (RFC 4326 §4.4). |

- Optional header → Receivers that do not support it MAY skip it (using H-LEN to find the
  next field) but MUST continue processing the rest of the SNDU and forward the PDU.

> ⚠ Figure 7's left two bytes are the Next-Header Type `0x0301` (H-LEN=3, H-Type=0x01 =
> 257). For an Optional header, H-LEN counts the total extension bytes including the
> 2-byte Type; H-LEN=3 → 6 bytes total = 2-byte Type + 4-byte TimeStamp. The trailing
> 16-bit Type field in Figure 7 is the *next* header/EtherType, drawn as part of the same
> diagram. RFC 5163 §3.3 / §4 state "TimeStamp of length 4B with a Type field"; the byte
> count is reconciled as 2 (Type) + 4 (TimeStamp) = 6 = H-LEN 3 × 2B words.

### Extension-Padding — Optional, H-Type 0x100, H-LEN 1–5 (RFC 4326 §5.3)

- IANA H-Type value `0x100`. Total length is given by H-LEN (in 16-bit words); the field
  is one to five 16-bit words.
- Only the **last** 16-bit word carries a value: it forms the Next-Header Type field. The
  sender SHOULD set all preceding words to `0x0000`.
- A Receiver MUST interpret the (last) Type field but MUST ignore all other words of the
  extension. The effect is that the following H-LEN 16-bit words of option header are
  ignored.

## Recommended ordering of Extension Headers (RFC 5163 §3, Table 1)

| Fields the Extension Header operates on | Example Extension Headers |
|------------------------------------------|---------------------------|
| Link framing and transmission | TimeStamp Extension |
| Entire remaining SNDU Payload | Encryption Extension |
| Group of encapsulated PDUs | PDU-Concat or TS-Concat |
| Specific encapsulated PDU | IEEE-defined type, Test or MAC bridging Extension |

Ordered first→last within an SNDU; a guideline only (not all types appear together).
