# Asynchronous Layered Coding (ALC) Packet Format

_Source: RFC 5775 §2, §4 (Figures 1-3), transcribed_

ALC is a **protocol instantiation** of LCT for massively scalable, reliable
content delivery over IP multicast. ALC version 1 MUST use LCT version 1
(RFC 5651, see `lct.md`). ALC is carried as the payload of UDP.

## Packet composition (§2, §4.1, Figure 3)

An ALC packet = **UDP header + LCT header + FEC Payload ID + Encoding Symbol(s)**.

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                         UDP Header                            |
|                                                               |
+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+
|                         LCT Header                            |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       FEC Payload ID                          |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     Encoding Symbol(s)                        |
|                           ...                                 |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Portion | Defined by | Notes |
|---------|------------|-------|
| LCT Header | RFC 5651 (`lct.md`) | The default LCT header. CCI carries the multiple-rate congestion-control info. |
| FEC Payload ID | RFC 5052 / the FEC Scheme in use | Format depends on the FEC Encoding ID; **out of scope of RFC 5775** (see below). |
| Encoding Symbol(s) | the FEC Scheme | The packet payload — encoding symbols identified by the FEC Payload ID. |

Semantics / constraints:
- **TSI is REQUIRED to be non-zero** for ALC. This means the sender MUST NOT set both
  LCT flags `S` and `H` to 0 (otherwise TSI length = 0). (§4.1)
- **Data-less packets**: a sender MAY emit ALC packets with no payload (e.g. to close
  a session or convey CC info). Such packets contain **neither** the FEC Payload ID
  **nor** the payload — only the LCT header. Receivers detect this from the
  IP/UDP total datagram length. (§4.1)
- The TOI MAY be omitted if only one object is carried; MUST be used (in all packets)
  if more than one object is carried. (§2.1)
- The Codepoint (CP) MAY carry the FEC Encoding ID per object (mapping out-of-band).

## PSI bits used by ALC (§2.1, Figure 2)

ALC defines the two-bit LCT PSI field (bits 6-7 of the first LCT word):

```
              +-+-+
          ...|X|Y|...
              +-+-+
```

| PSI bit | Name | Meaning |
|---------|------|---------|
| X | SPI (Source Packet Indicator) | With systematic FEC Schemes that use a different FEC Payload ID format for source-only vs repair packets: SPI=1 → source-data Payload ID format in use; SPI=0 → repair-data Payload ID format in use. |
| Y | Reserved | — |

For FEC Schemes defining a single FEC Payload ID format, SPI MUST be 0 (sender) and
ignored (receiver).

## FEC Payload ID

⚠ **The concrete FEC Payload ID bit layouts are NOT defined in RFC 5775.** RFC 5775
only states that the FEC Payload ID immediately follows the LCT header, that it
"uniquely identifies the encoding symbol(s)", and that "the FEC Payload ID is
described in the FEC building block [RFC5052]" — its size and format depend on the
FEC Scheme / FEC Encoding ID in use and are specified by the corresponding FEC
Scheme document (e.g. RFC 5445 FEC Basic Schemes, RFC 6865 Reed-Solomon, etc.).
Those documents are **not in this transcription set** — do not invent the layouts.

(For a concrete example of one such layout — Small Block Systematic codes,
`fec_id`=129, a 32-bit `source_block_number` + 16-bit `source_block_len` + 16-bit
`encoding_symbol_id` — see `norm.md`, which reproduces it from RFC 5445 as an
illustrative example. ALC does not reproduce any FEC Payload ID layout itself.)

## EXT_FTI Header Extension (§4.2)

ALC defines one new LCT Header Extension, **EXT_FTI**, to carry the FEC Object
Transmission Information in-band with an object's data packets.

| Property | Value |
|----------|-------|
| Name | EXT_FTI |
| HET (Header Extension Type) | **64** (variable-length form, HET 0..127 → carries an HEL) |
| HEC content | The encoded FEC Object Transmission Information per RFC 5052. |

⚠ The format of the encoded FEC Object Transmission Information carried in the
EXT_FTI HEC is **dependent on the FEC Scheme and is out of scope of RFC 5775**
(specified by RFC 5052 / the FEC Scheme). It is registered at HET 64 (the
Specification-Required variable-length range). NORM defines its own concrete
EXT_FTI body layout (also HET 64) — see `norm.md`.

## IANA registration (§6)

| HET value | Name | Reference |
|-----------|------|-----------|
| 64 | EXT_FTI | RFC 5775 |
