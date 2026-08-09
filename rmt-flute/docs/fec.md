# FEC Building Block — Object Transmission Information, Payload ID, and Block Partitioning

_Source: RFC 5052 (FEC Building Block, all sections), RFC 6726 §3.4.2 + §5 (FLUTE's
use of FEC OTI), RFC 5053 §3 (Raptor FEC Scheme — FEC Payload ID + OTI sections
only), RFC 6330 §3 (RaptorQ FEC Scheme — FEC Payload ID + OTI sections only),
transcribed_

RFC 5052 is the **building block** ALC (`alc.md`), FLUTE (`flute.md`) and NORM
(`norm.md`) all sit on for FEC. It does not itself define any wire bytes — it
defines a *vocabulary* (FEC Encoding ID, FEC Object Transmission Information,
FEC Payload ID) and delegates the actual bit layout of each to whichever
concrete **FEC Scheme** (identified by the FEC Encoding ID) is in use. Each of
ALC/FLUTE/NORM's own docs already says this and points here; this document is
where the RFC 5052 vocabulary itself, plus its one CDP-level consumer already in
this crate's scope (FLUTE §3.4.2/§5) and two illustrative concrete FEC Scheme
examples (Raptor, RaptorQ), are transcribed together.

## 0. Scope — what this is and is NOT

- **Is**: the FEC Encoding ID / FEC Instance ID vocabulary (§1 below); the three
  FEC Object Transmission Information element classes and how a CDP is required
  to transport them (§2); what a FEC Payload ID is and who defines its bytes
  (§3); the CDP obligations RFC 5052 imposes (§4); the Block Partitioning
  Algorithm (§5) — the one substantive *algorithm* RFC 5052 defines, and the
  natural basis for any "split an object into source blocks" helper; the current
  IANA FEC Encoding ID registry (§6); FLUTE's own concrete transport of FEC OTI
  via FDT attributes (§7); and, purely as **illustrative, scheme-specific
  examples** of how two real FEC Schemes fill in the FEC Payload ID / OTI
  formats RFC 5052 leaves opaque, the relevant sections of RFC 5053 (Raptor) and
  RFC 6330 (RaptorQ) (§8).
- **Is NOT**: a decoder/encoder for any FEC scheme. RFC 5052 itself does not
  specify encoding/decoding procedures (that is each FEC Scheme's own
  specification, e.g. RFC 5053 §5 for Raptor or RFC 6330 §5 for RaptorQ —
  the *coding* mathematics of those documents is out of scope here and not
  transcribed; only their FEC Payload ID / OTI *framing* sections are, per the
  brief this document was written to).
- **Is NOT** A/331 ROUTE's own FEC framework (`atsc3/docs/a331-route.md` §6-7:
  "FEC transport object", "FEC super-object", `RepairFlow` XML) — that is
  ATSC-specific and layers *on top of* the RFC 5052 vocabulary transcribed here
  (plus RFC 6363 FECFRAME concepts A/331 cites but this crate does not
  implement). See §9 below for how the two relate.

⚠ **Critical invariant this document exists to reinforce, not undermine**: the
FEC Payload ID and the Scheme-specific FEC Object Transmission Information are
**opaque to the CDP** (§2.4, §3) — their bit layout is defined by whichever FEC
Scheme (FEC Encoding ID) is in use, not by ALC, FLUTE, NORM, or this document.
`alc.md`/`norm.md` already document this and provide [`FecPayloadId128`] (the
RFC 5445 Small Block Systematic layout, `fec_id`=128/129) as *one* concrete
illustrative example. §8 below adds two more illustrative examples — Raptor's
4-byte `SBN`(16)+`ESI`(16) and RaptorQ's 4-byte `SBN`(8)+`ESI`(24) — which are
**different bit-widths from each other and from `FecPayloadId128`**, precisely
to make the scheme-dependence concrete rather than implying any one universal
FEC Payload ID shape. A super-object/transport-object helper built from this
document's Block Partitioning Algorithm (§5) must stay scheme-agnostic for the
same reason: it operates on symbol *counts*, not on any encoded FEC Payload ID
byte format.

## 1. FEC Encoding ID and FEC Instance ID (§6.1)

Every FEC Scheme is identified by an **FEC Encoding ID** — an 8-bit integer
(0-255) assigned by IANA. Receivers use it to select the correct FEC decoder.

| FEC Scheme class | FEC Encoding ID range | FEC Instance ID used? |
|---|---|---|
| **Fully-Specified** — encoder/decoder implementable from an IETF RFC alone | 0-127 | No — MUST NOT be used |
| **Under-Specified** — no public spec, or a party controls the algorithm and won't disclose it | 128-255 | Yes — a 16-bit integer (0-65535) scoped *within* the FEC Encoding ID, identifying which specific instance of the Under-Specified scheme is in use |

The FEC Encoding ID (and, for Under-Specified schemes, the FEC Instance ID) is
essential for the decoder to even attempt decoding, so both are part of the
**Mandatory**/**Common** FEC Object Transmission Information (§2). An
already-registered Under-Specified FEC Encoding ID MUST be reused for a new
instance if the existing FEC Payload ID/OTI fields and formats already fit —
new Under-Specified `(Encoding ID, Instance ID)` tuples are only for genuinely
new formats.

Every FEC scheme (Fully- or Under-Specified) MUST define: the type, semantics
and an encoding format for its FEC Payload ID and its FEC Object Transmission
Information, and MUST have a reserved FEC Encoding ID value associated with
that format.

## 2. FEC Object Transmission Information (§6.2)

FEC Object Transmission Information (**FEC OTI**) is the information a decoder
needs to decode a *whole* object (as opposed to the FEC Payload ID, §3, which
identifies what a single *packet* carries). RFC 5052 defines three element
classes:

### 2.1 Mandatory (§6.2.3)

Exactly one element, always required:

| Element | Encoding | Meaning |
|---|---|---|
| FEC Encoding ID | integer, 0-255 | Identifies the FEC Scheme (§1). Its *encoding format* is defined by the CDP (not the FEC scheme), since the receiver needs to read this field before it knows which FEC scheme's rules to apply to everything else. |

### 2.2 Common (§6.2.4)

Optional-per-scheme; each FEC scheme picks which of these it uses and how their
values are derived. RFC 5052 defines only the abstract *type*, not a universal
encoding — each FEC scheme defines its own encoding for the subset it uses
(§8 below shows Raptor's and RaptorQ's concrete encodings side by side).

| Element | Type | Meaning |
|---|---|---|
| FEC Instance ID | integer, 0-65535 | Only for Under-Specified schemes (§1). |
| Transfer-Length | non-negative integer | Length of the object, in octets. |
| Encoding-Symbol-Length | non-negative integer | Length of each encoding symbol, in octets. |
| Maximum-Source-Block-Length | non-negative integer | Max number of source symbols per source block. |
| Max-Number-of-Encoding-Symbols | non-negative integer | Max number of encoding symbols (source + repair, for a systematic code). |

If a CDP defines its own per-element encoding (rather than using the FEC
scheme's encoding of the whole Common OTI block), that encoding MUST be able to
represent values up to `2^16 - 1` for FEC Instance ID, `2^48 - 1` for
Transfer-Length, and `2^32 - 1` for the other three.

### 2.3 Scheme-specific (§6.2.5)

At most one such element per FEC scheme, but that element MAY itself pack
multiple sub-fields (§8 shows Raptor's `Z`/`N`/`Al` and RaptorQ's `Z`/`N`/`Al`
as examples). From a CDP's point of view this is always just an **opaque,
variable-length octet string** whose internal structure only FEC-scheme-aware
code may interpret.

### 2.4 Transport and opacity (§6.2.1, §6.2.2)

- The CDP is responsible for reliably transporting FEC OTI to the receiver(s),
  by data-packet header, session-control packet, or "some other means".
- The Mandatory element's *encoding format* is defined by the CDP (so the
  receiver can identify the scheme before parsing anything scheme-specific).
- For Common and Scheme-specific OTI, a CDP has two options: (a) use the
  encoding format the FEC scheme itself defines (in which case the CDP may
  simply carry the concatenation of encoded-Common + encoded-Scheme-specific
  as one opaque blob — RFC 5052 calls this concatenation "the encoded FEC
  Object Transmission Information"), or (b) define its own per-element
  encoding for the Common elements (Scheme-specific is never (b) — it is
  always the FEC scheme's own opaque encoding).
- **Opacity**: the encoded Scheme-specific OTI, and any FEC-scheme-defined
  encoding of the Common OTI, are opaque to the CDP — inspecting them requires
  FEC-scheme-specific logic. A CDP's *own* per-element encoding of Common OTI
  (option (b)) is not opaque in this sense, but different FEC schemes may use
  different subsets of the Common elements even then.

FLUTE is RFC 5052's own example of a CDP using option (b) for the FDT (it
defines its own XML-attribute encoding for each Common element — §7 below) while
also supporting the FEC scheme's native encoding via EXT_FTI.

## 3. FEC Payload ID (§6.3)

The **FEC Payload ID** is per-*packet* (not per-object) information: it tells
the decoder which encoding symbols (source or repair) a given packet carries,
and how they relate to the FEC encoding transformation — e.g. which source
symbols of the object a source packet carries, or how a packet's repair
symbols were constructed. It MAY also carry a symbol grouping above the
individual symbol — most commonly, which **source block** the symbols belong
to (see §5 — the source-block partitioning is exactly what the FEC Payload ID
usually needs to reference).

- A data packet carrying encoding symbols MUST include a FEC Payload ID.
- **Its encoding format, including size, is defined entirely by the FEC
  Scheme** — RFC 5052 imposes no shape on it whatsoever. The CDP's only job is
  to specify *where in the packet* the FEC Payload ID goes and how it
  associates with the symbols that follow (for ALC: immediately after the LCT
  header, before the encoding symbols — see `alc.md`'s packet-composition
  diagram; for NORM: the `fec_id` field then `fec_payload_id`, per `norm.md`).
- **Two-format systematic codes**: a systematic FEC scheme (one where the
  original source data is included verbatim in the encoded data) MAY define
  *two* FEC Payload ID formats — one for source-symbol-only packets, one for
  packets carrying at least one repair symbol. A CDP that wants to support such
  a scheme MUST provide some indication of which format a given packet uses.
  This is exactly what ALC's PSI "SPI" bit is for (`alc.md` §"PSI bits used by
  ALC": SPI=1 ⇒ source-data format, SPI=0 ⇒ repair-data format).
- Some FEC schemes MAY instead specify that the FEC Payload ID be derived
  *implicitly* from other packet information already present (e.g. a
  transport-layer header) rather than carried explicitly — usable only with a
  CDP that exposes the right information for that derivation.

## 4. CDP (Content Delivery Protocol) requirements (§8)

Any CDP that layers on this building block (ALC/RFC 5775, FLUTE/RFC 6726,
NORM/RFC 5740, or A/331 ROUTE — §9) MUST define:

1. An encoding format for the Mandatory FEC OTI element (the FEC Encoding ID).
2. A means to reliably communicate it, using that format.
3. A means to reliably communicate the Common FEC OTI elements — via the FEC
   scheme's own encoding format, the CDP's own per-element format, or both.
4. A means to reliably communicate the Scheme-specific FEC OTI element, in the
   encoding format the FEC scheme itself defines (there is no CDP-defined
   alternative for this one — it is always opaque, §2.4).
5. A means to associate a FEC Payload ID with each data packet.

A CDP MAY additionally define a means to indicate which of the two systematic
FEC Payload ID formats (§3) a packet is using.

## 5. Block Partitioning Algorithm (§9.1)

The one concrete *algorithm* RFC 5052 specifies: how to split an object of `L`
octets into `N` source blocks of as-equal-as-possible length, given a maximum
source block length `B` (symbols) and a symbol length `E` (octets). This is the
natural common substrate for a scheme-agnostic "transport-object → source
blocks" helper (§9), since it operates purely on symbol counts and produces no
FEC-scheme-specific bytes.

### First step (§9.1.1) — how many symbols, how many blocks

Input: `B` (max source symbols per source block), `L` (transfer length, octets),
`E` (encoding symbol length, octets).

```
T = ceil(L / E)      -- number of source symbols in the object
N = ceil(T / B)      -- number of source blocks to partition into
```

### Second step (§9.1.2) — the partition itself

Input: `T` (source symbols), `N` (source blocks, from the first step).

```
A_large = ceil(T / N)      -- symbols per "larger" source block
A_small = floor(T / N)     -- symbols per "smaller" source block
I       = T - A_small * N  -- number of "larger" blocks
```

The first `I` source blocks each have `A_large` source symbols; the remaining
`N - I` source blocks each have `A_small` source symbols. Each source symbol is
`E` octets, **except** the very last source symbol of the very last source
block, whose length is `L - floor((L-1)/E)*E` octets (i.e. the object's actual
trailing remainder, not a full `E`-octet symbol — objects are not generally an
exact multiple of the symbol length).

This is the algorithm FEC schemes and CDPs are told to prefer over inventing
their own equivalent (§9's chapeau: "FEC Schemes and CDPs SHOULD use these
algorithms in preference to scheme- or protocol-specific algorithms, where
appropriate"). Raptor's and RaptorQ's own `Z`/`N`/`Al` Scheme-specific OTI
(§8) are additional partitioning parameters layered on top of this same
`T`-source-symbols / `N`-source-blocks structure, not a replacement for it.

## 6. IANA — FEC Encoding ID registry

RFC 5052 §12 establishes the "Reliable Multicast Transport (RMT) FEC Encoding
IDs and FEC Instance IDs" IANA registry and reserves 0-127 for Fully-Specified
schemes, 128-255 for Under-Specified schemes (§1), but does not itself assign
any concrete values — those come from each FEC scheme's own RFC registering
against this registry. The values relevant to ALC/FLUTE/NORM (this crate's
scope) currently registered, per IANA (`rmt-fec-parameters` registry):

| FEC Encoding ID | Name | Defined by |
|---|---|---|
| 0 | Compact No-Code | RFC 5445 (not vendored in this repo — see `alc.md`/`flute.md`'s own caveat) |
| 1 | Raptor | RFC 5053 — §8.1 below |
| 2 | Reed-Solomon Codes over GF(2^m) | RFC 5510 |
| 3 | LDPC Staircase Codes | RFC 5170 |
| 4 | LDPC Triangle Codes | RFC 5170 |
| 5 | Reed-Solomon Codes over GF(2^8) | RFC 5510 |
| 6 | RaptorQ Code | RFC 6330 — §8.2 below |
| 128 | Small Block, Large Block and Expandable FEC Codes | RFC 5445 |
| 129 | Small Block Systematic FEC Codes | RFC 5445 — the layout already reproduced as [`FecPayloadId128`] in `norm.md`/`alc.md` |
| 130 | Compact FEC | RFC 5445 |

(This table is a factual cross-reference to the current IANA registry state,
consulted 2026-08, not itself part of RFC 5052's normative text — flagged as
such per this project's "every value traceable" discipline. FLUTE's default FEC
Encoding ID for the FDT is 0 (Compact No-Code) per `flute.md`.)

## 7. FLUTE's use of FEC OTI (RFC 6726 §3.4.2, §5)

FLUTE is the one CDP already fully in this crate's scope, and RFC 6726 §5
("Delivering FEC Object Transmission Information") is RFC 5052's own worked
example of CDP requirement 3 above (§4) — option (b), a CDP-defined per-element
encoding — layered *alongside* option (a) (the FEC scheme's native encoding via
EXT_FTI, `alc.md`).

- FLUTE **inherits** the FEC building block from ALC. The FEC OTI for a FLUTE
  session MUST be delivered in-band, by one (or both) of: ALC's EXT_FTI
  extension (`alc.md`), or the FDT (this section).
- **Priority when both are present**: EXT_FTI takes priority over the FDT (RFC
  6726 §5, receiver SHOULD prefer it) — the FDT is the required fallback when
  the object's TOI isn't yet known to use EXT_FTI, or for redundancy.
- The FDT MUST support delivering FEC OTI for TOI=0 (FDT Instances themselves)
  via EXT_FTI; for other TOIs, a receiver MUST support both EXT_FTI and the FDT
  attribute path.
- **The FEC OTI delivered via EXT_FTI and via the FDT MUST be identical** for a
  given TOI (RFC 6726 §5) — the FDT-attribute encoding below is a CDP-defined
  *alternative transport* of the same information, not a different value.

### FDT attribute encoding of Common/Scheme-specific FEC OTI (§3.4.2, §5)

Each attribute may appear on the `FDT-Instance` element (session-wide default)
or on an individual `File` element (per-file override — `File` takes
precedence when both are present, per `flute.md`'s existing FDT description):

| FDT attribute | XML type | Carries (RFC 5052 element) |
|---|---|---|
| `Transfer-Length` | `xs:unsignedLong` | Transfer-Length (§2.2) |
| `FEC-OTI-FEC-Encoding-ID` | `xs:unsignedByte` | FEC Encoding ID (§2.1, Mandatory) — as carried in the Codepoint field of the ALC/LCT header (see below) |
| `FEC-OTI-FEC-Instance-ID` | `xs:unsignedLong` | FEC Instance ID (§2.2, Under-Specified schemes only) |
| `FEC-OTI-Maximum-Source-Block-Length` | `xs:unsignedLong` | Maximum-Source-Block-Length (§2.2), if the FEC scheme requires it |
| `FEC-OTI-Encoding-Symbol-Length` | `xs:unsignedLong` | Encoding-Symbol-Length (§2.2), if required |
| `FEC-OTI-Max-Number-of-Encoding-Symbols` | `xs:unsignedLong` | Max-Number-of-Encoding-Symbols (§2.2), if required |
| `FEC-OTI-Scheme-Specific-Info` | `xs:base64Binary` | the encoded Scheme-specific FEC OTI (§2.3) verbatim, if required — this one is **always** the FEC scheme's own opaque encoding, base64-wrapped; FLUTE never re-encodes its internal structure |

**Codepoint = FEC Encoding ID, base FLUTE rule**: in base FLUTE, the FEC
Encoding ID (8 bits) for a given TOI MUST be carried in the Codepoint (`CP`)
field of the ALC/LCT header itself (`lct.md`'s `CP` field), and whenever
`FEC-OTI-FEC-Encoding-ID` is also present in the FDT for that TOI, the two MUST
agree. (ROUTE/A/331 overrides this specific rule — its `CP` values instead
select a delivery-object format via Table A.3.6, per `a331-route.md` §4 — a
CDP-specific deviation the base FLUTE spec does not anticipate but does not
forbid either, since RFC 6726 leaves Codepoint semantics otherwise to the CDP
context. This crate does not need to resolve that tension: `AlcPacket` never
interprets `CP` itself.)

## 8. Illustrative concrete FEC Payload ID / OTI layouts (opaque to the CDP)

⚠ As emphasized in §0/§3: the following are **two more examples**, in addition
to the RFC 5445 Small Block Systematic layout `norm.md`/`alc.md` already give
as [`FecPayloadId128`]. All three are mutually different bit layouts,
registered against different FEC Encoding IDs. None of the three is "the" FEC
Payload ID format — which one applies to a given packet is determined entirely
by that packet's FEC Encoding ID (§1, §6), out-of-band information the CDP
communicates via FEC OTI (§2), never something ALC/FLUTE/NORM frame bytes
dictate on their own.

### 8.1 Raptor — FEC Encoding ID 1 (RFC 5053 §3)

**FEC Payload ID** (§3.1, Figure 1) — a fixed 4-octet field:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|     Source Block Number       |      Encoding Symbol ID       |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Bits | Meaning |
|---|---|---|
| Source Block Number (SBN) | 16 | Identifies the source block the packet's encoding symbols relate to. |
| Encoding Symbol ID (ESI) | 16 | Identifies the encoding symbol(s) within the packet. |

**FEC Object Transmission Information** (§3.2):

- **Mandatory** (§3.2.1): FEC Encoding ID MUST be **1**.
- **Common** (§3.2.2, Figure 2) — Transfer Length (`F`, < 2^45) + Encoding
  Symbol Length (`T`, < 2^16), encoded as a **10-octet** field (48 + 16 + 16
  bits — note this is a different, older layout from RaptorQ's below, not the
  same shape with different names):

  ```
   0                   1                   2                   3
   0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  |                      Transfer Length                          |
  +                               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  |                               |           Reserved            |
  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  |    Encoding Symbol Length     |
  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  ```

  Bit accounting: word 1 (32 bits) + the top 16 bits of word 2 = the 48-bit
  Transfer Length; the bottom 16 bits of word 2 = Reserved; the top 16 bits of
  word 3 = the 16-bit Encoding Symbol Length (word 3's bottom 16 bits are
  unused by this field — the diagram's own ruler only spans the 16 bits it
  draws). Total: 32 + 32 + 16 = 80 bits = **10 octets**. RFC 5053 encodes
  Transfer Length as a full 48-bit field "for simplicity" even though the
  *value range* is limited to 2^45 by the symbol/block-count limits noted in
  its own NOTE 1.
- **Scheme-specific** (§3.2.3, Figure 3) — a 4-octet field: `Z` (number of
  source blocks, 2 octets) + `N` (number of sub-blocks, 1 octet) + `Al` (symbol
  alignment parameter, 1 octet):

  ```
   0                   1                   2                   3
   0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  |             Z                 |      N        |       Al      |
  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  ```

- **Encoded FEC OTI** = the 10-octet Common field concatenated with the
  4-octet Scheme-specific field = **14 octets** total (§3.2.3's own summary
  states this explicitly).

### 8.2 RaptorQ — FEC Encoding ID 6 (RFC 6330 §3)

**FEC Payload ID** (§3.2, Figure 1) — also a fixed 4-octet field, but with a
**different SBN/ESI split** than Raptor's:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|     SBN       |               Encoding Symbol ID              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Bits | Meaning |
|---|---|---|
| SBN | 8 | Source Block Number — non-negative integer identifying the source block. |
| ESI | 24 | Encoding Symbol ID — non-negative integer identifying the encoding symbol(s) in the packet. |

⚠ Note for readers of `a331-route.md` §3.2: A/331's own ROUTE-repair FEC
Payload ID figure (Figure A.3.4) states **16-bit SBN + 16-bit Encoding Symbol
ID** for its RaptorQ repair flows — a different split from RFC 6330's own
**8-bit SBN + 24-bit ESI** shown above. Both are 4 octets total, so the
discrepancy is in how the 32 bits are divided between the two sub-fields, not
in the overall FEC Payload ID size. This transcription reports RFC 6330's own
figure faithfully; reconciling it with A/331's figure (a ROUTE-specific
redefinition, an erratum in one of the two documents, or a version difference)
is a question for whoever implements the ROUTE repair-flow FEC Payload ID —
not resolved here, and not invented.

**FEC Object Transmission Information** (§3.3):

- **Mandatory** (§3.3.1): FEC Encoding ID MUST be **6**.
- **Common** (§3.3.2, Figure 2) — Transfer Length `F` (40 bits, ≤ 946,270,874,880)
  + Symbol Size `T` (16 bits), encoded as an 8-octet field:

  ```
   0                   1                   2                   3
   0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  |                      Transfer Length (F)                      |
  +               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  |               |    Reserved   |           Symbol Size (T)     |
  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  ```

  (Transfer Length occupies the first 32 bits plus the top 8 bits of the second
  word; an 8-bit Reserved field follows; Symbol Size is the low 16 bits of the
  second word.)
- **Scheme-specific** (§3.3.3, Figure 3) — a 4-octet field: `Z` (number of
  source blocks, 8 bits) + `N` (number of sub-blocks, 16 bits) + `Al` (symbol
  alignment parameter, 8 bits) — same field *names* as Raptor, **different bit
  widths**:

  ```
   0                   1                   2                   3
   0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  |       Z       |              N                |       Al      |
  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  ```

- **Encoded FEC OTI** = the 8-octet Common field concatenated with the 4-octet
  Scheme-specific field = **12 octets** total (§3.3.3's own summary).

## 9. What this means for a scheme-agnostic FEC transport-object/super-object helper

Both `atsc3-route` (A/331 Annex A) and `dvb-mabr` (ETSI TS 103 769) need to
construct/reassemble a delivery object's FEC-protectable unit from source
blocks and encoding symbols — the shape recorded as needed (but not yet built)
in this crate's own tracking issue. This document supplies the RFC 5052 layer
that construction can be built on without hardcoding a FEC scheme:

- The **Block Partitioning Algorithm** (§5) — `L`/`E`/`B` in, `N` source blocks
  (with `A_large`/`A_small`/`I`) out — is exactly "how many source blocks does
  this object split into, and how big is each", entirely in terms of symbol
  *counts*, never FEC-scheme-specific bytes. This is the natural core of a
  transport-object/source-block helper.
- What such a helper must **not** do is bake in a FEC Payload ID layout (§3, §8)
  or a Scheme-specific OTI layout (§2.3, §8) — those stay the caller's problem,
  passed through as opaque byte slices (mirroring how `AlcPacket::fec_payload_id`
  and `NormData::fec_payload_id` already work, and how [`FecPayloadId128`] is
  offered as an opt-in concrete helper rather than baked into `AlcPacket`
  itself).
- A/331 ROUTE's own "FEC transport object" (one delivery object + padding +
  trailing size field) and "FEC super-object" (concatenation of `N` FEC
  transport objects) constructions — already transcribed in
  `atsc3/docs/a331-route.md` §6 — are ATSC-specific *composition* on top of
  this same RFC 5052 source-block/encoding-symbol vocabulary (via RFC 6363
  FECFRAME, which this crate does not implement). Whether DVB-MABR needs
  exactly that composition or something looser is a question for whoever scopes
  the DVB-MABR crate — recorded here only as the reason this document stops at
  RFC 5052 + FLUTE's consumption of it, rather than reproducing A/331's
  ROUTE-specific super-object framing a second time.
