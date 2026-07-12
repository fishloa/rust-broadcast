# SMPTE RDD 29:2019 — Dolby Atmos® Bitstream Specification

Curated transcription of the normative text, fetched directly from
`https://pub.smpte.org/pub/rdd29/rdd29-2019.pdf` (20pp, freely available —
the SMPTE catalog is public at pub.smpte.org per the 2026-06-17 catalog
release). This is the implementation/audit oracle for this crate — cite this
file, not the module doc comments, when checking field semantics. Page
numbers below are the document's own ("Page N of 20").

This is a **Registered Disclosure Document** (RDD), not a Standard/RP/EG: "It
has been examined... and is believed to contain adequate information to
satisfy the objectives defined in the Scope" (p.1) — i.e. SMPTE does not
warrant its completeness the way it would a Standard. Two fields
(`AudioDescription`'s wire semantics, and the `Plex(8)` escape-nesting
pseudocode) have gaps/inconsistencies the disclosure itself never resolves —
see "Scope decisions" below for how this crate handles each honestly.

## 1 Scope (p.4)

> This document defines the syntax of a frame-based Dolby Atmos bit stream.
> The bit stream carries audio essence and metadata necessary to reproduce a
> complete audio program.

Per the Introduction (p.3): "Dolby Atmos® is an advanced cinema sound format
comprising an audio essence and metadata stream played through specialized
renderers in the cinema."

## 2 Bitstream Organization (p.4-5)

The audio program is segmented into **Frames**, transmitted at 24, 25, 30,
48, 50, 60, 96, 100, or 120 Hz, aligned with the program edit units.

All audio data is encapsulated into **elements**, "similar in concept to
'chunks' in the RIFF format." Each element begins with a unique `ElementID`,
followed by `ElementSize` (the size **in bytes** of the entire element, not
including `ElementID`/`ElementSize` itself; includes the size of all sub-
elements). At the top level, the entire audio frame is a single `ATMOSFrame`
element; all audio essence and metadata elements for a frame are its sub-
elements.

Four element types exist:

### 2.1 ATMOSFrame Element
Contains everything common to the entire frame: Dolby Atmos version, audio
sample rate, audio bit depth, audio frame rate, and the maximum number of
rendered audio assets. All raw audio assets and metadata **must** be sub
elements of `ATMOSFrame`.

### 2.2 BedDefinition Element
A Dolby Atmos **bed** is a collection of audio channels played back with a
nominal location/function (e.g. "Left", or "LFE"). `BedDefinition` lists the
audio assets and their associated channel names.

### 2.3 ObjectDefinition Element
Audio assets ("objects") can be panned to any location independent of
loudspeaker configuration. Each `ObjectDefinition` element updates one
object's position at ~20ms intervals; the bit stream carries many
`ObjectDefinition` elements, each a direct sub-element of `ATMOSFrame` with a
unique `MetaID`.

### 2.4 AudioDataDLC Element
Contains the audio essence for one track (channel or object) — losslessly
compressed, 48kHz or 96kHz, 24-bit. Must be a direct sub-element of
`ATMOSFrame` and have a unique `AudioDataID`. Object tracks are typically
sparse (mostly digital silence between events).

## 3 Bitstream Conventions (p.5-6)

### 3.1 Position
Position is described relative to a unit-cube playback room, origin at the
front-left corner:

- `x`: lateral (0 = left wall, 1 = right wall)
- `y`: longitude (0 = front wall, 1 = back wall)
- `z`: elevation (0 = plane aligned with screen/side/rear loudspeakers, 1 =
  ceiling)

Examples: `(0,0,0)` = front-left corner, screen height; `(1,0,0)` =
front-right corner, screen height; `(0.5,0.5,1)` = middle of ceiling.

### 3.2 Relative distance coding (p.6)
An n-bit unsigned integer `Dn` maps linearly into `[0,1]`; the mapping
depends on the axis:

```
DistanceXY = Dn/2^(n-1) - (2^(n-1)-1)/2^(n-1),   2^(n-1)-1 <= Dn <= 2^n - 1
DistanceZ  = Dn/(2^n - 1),                        0 <= Dn <= 2^n - 1
```

Used for `ObjectPosX`/`ObjectPosY` (DistanceXY, n=16) and `ObjectPosZ`/
`ObjectSpread` (DistanceZ, n=16/8/12 depending on field).

### 3.3 Amplitude Gain (p.6)
A 10-bit mantissa `A10` (unsigned) maps to a linear gain:

```
gain = 0,                A10 = 2^10 - 1
gain = 2^(-A10/64),      0 <= A10 <= 2^10 - 2
```

Log-spaced 0dB..-96dB in ~0.094dB steps, plus a true-zero gain code. This
crate does not currently expose any field using this coding (no `A10` field
appears in the syntax tables actually transcribed below — the gain-coding
text is present but no wire field in §4/§5 cites it directly by name; kept
here for completeness/future use, not fabricated as an implemented field).

### 3.4 Plex Coding (p.6-7)

> Element size and some other fields are coded using Plex. Plex coding allows
> efficient coding of small numbers, with the capability to express
> arbitrarily large values... A data field of all ones is an escape code. If
> the desired symbol is not the escape code and fits within the allocated
> field, the symbol shall be placed therein... Otherwise, an escape code is
> used to signal that a replacement symbol follows... The field size for the
> replacement symbol is twice as long as the field it replaces... For
> example, 0x1234 is to be Plex(8) encoded as 0xFF1234 instead of
> 0xFFFFFF00001234. ... symbols to be Plex encoded shall have a value less
> than or equal to 0xFFFFFFFE.

**Plex(8) pseudocode as literally printed (p.7):**
```
Plex(8)
{
    PlexData ........................................... 8
    if (PlexData == 0xFF) {
        PlexData ....................................... 16
        if (PlexData == 0xFFFFFF) {
            PlexData ................................... 32
        }
    }
    return (PlexData)
}
```

**This pseudocode is internally inconsistent with its own prose** — see
"Scope decisions" #1 below for how this crate resolves it (verified via
`pdftotext -layout`, not an OCR artifact: the PDF's own text layer literally
reads `0xFFFFFF`).

`Plex(n)` generalizes this to any starting width `n` (used with `n=4` for
`ChannelCount`/`ChannelID` in `BedDefinition1`, `n=8` elsewhere): read `n`
bits; if all-ones, read `2n` bits; if *that* is all-ones, read `4n` bits (no
further escalation defined — consistent with the "value <= 0xFFFFFFFE" cap,
since a 32-bit field's only unrepresentable value is `0xFFFFFFFF`).

## 4 Bit Stream Syntax (p.7-12)

Pseudo-code, C-like, simplified. **Bold** field names below are wire fields;
`/* comments */` are non-normative.

### 4.1 ReadElement() (p.7)

```
ReadElement()
{
    ElementID ................................ Plex(8)
    ElementSize .............................. Plex(8)
    switch (ElementID) {
        case (ATMOS_FRAME):        ATMOSFrame();       break
        case (BED_DEFINITION1):    BedDefinition1();   break
        case (OBJECT_DEFINITION1): ObjectDefinition1();break
        case (AUDIO_DATA_DLC):     AudioDataDLC();     break
        default:                   UnknownData ....... ElementSize * 8
    }
}
```

**Table 1 — Dolby Atmos Element IDs** (p.12):

| ElementID Name       | Value | Meaning              |
|----------------------|-------|-----------------------|
| ATMOS_FRAME          | 0x08  | Frame Header          |
| BED_DEFINITION1      | 0x10  | Bed Definition Type 1 |
| RESERVED             | 0x20  | Reserved              |
| OBJECT_DEFINITION1   | 0x40  | Object Definition Type 1 |
| RESERVED             | 0x80  | Reserved              |
| RESERVED             | 0x100 | Reserved              |
| AUDIO_DATA_DLC       | 0x200 | Audio Data (DLC encoded) |

"If the ElementID is not defined in the system, then the decoder shall skip
the element" (§5.1.1) — i.e. unknown IDs (including the three reserved
values above) are not an error; they carry through as opaque data.

### 4.2 ATMOSFrame() (p.8)

```
ATMOSFrame()
{
    ATMOSVersion ............................. 8
    SampleRate ............................... 2
    BitDepth .................................. 2
    FrameRate .................................. 4
    MaxRendered .............................. Plex(8)
    ByteAlign()
    SubElementCount .......................... Plex(8)
    for (n = 0; n < SubElementCount; n++) { ReadElement() }
}
```

`ATMOSVersion`+`SampleRate`+`BitDepth`+`FrameRate` = 8+2+2+4 = 16 bits, so
`ByteAlign()` is a no-op given the fields transcribed here — kept for
parser symmetry with the other elements' genuine `AlignBits`.

### 4.3 BedDefinition1() (p.8)

```
BedDefinition1()
{
    MetaID ................................... Plex(8)
    Reserved (set to 0) ...................... 1
    ChannelCount (set to 10) ................. Plex(4)
    for (n = 0; n < ChannelCount; n++) {
        ChannelID[n] .......................... Plex(4)
        AudioDataID[n] ........................ Plex(8)
        Reserved (set to 0) ................... 3
    }
    Reserved (set to 0x180) .................. 10
    AlignBits ................................ VARIABLE
    Reserved (set to 0x5) .................... 8
    Reserved (set to 0) ...................... 8
}
```

"`ChannelCount` (set to 10)" is a worked-example annotation (a 9.1 bed has 10
channels), **not** a fixed/reserved constant — §2.2/§5.3.2 both describe
`ChannelCount` as a real, variable field ("the number of channels that make
up the bed"). This crate does not require `ChannelCount == 10`.

### 4.4 ObjectDefinition1() (p.9-10)

```
ObjectDefinition1()
{
    MetaID .................................... Plex(8)
    AudioDataID ................................ Plex(8)
    Reserved (set to 0x7FE) ................... 11
    /* NumPanSubBlocks is dependent on audio sample rate and frame rate */
    for (sb = 0; sb < NumPanSubBlocks; sb++) {
        if (sb == 0) { PanInfoExists = 1 }
        else         { PanInfoExists ........... 1 }
        if (PanInfoExists == 1) {
            Reserved (set to 0x1) .............. 5
            ObjectPosX[sb] ..................... 16
            ObjectPosY[sb] ..................... 16
            ObjectPosZ[sb] ..................... 16
            ObjectSnap[sb] ...................... 1
            if (ObjectSnap[sb] == 1) { Reserved (set to 0) .. 2 }
            ObjectZoneControl[sb] ............... 1
            if (ObjectZoneControl[sb] == 1) {
                for (n = 0; n < MAX_KNOWN_ZONES; n++) { ZoneGain .. 2 }
            }
            ObjectSpreadMode ..................... 2
            if (ObjectSpreadMode == OBJECT_SPREAD_LOWREZ) {
                ObjectSpread[sb] .................. 8
            } else if (ObjectSpreadMode == OBJECT_SPREAD_1D) {
                ObjectSpread[sb] .................. 12
            } else {
                ObjectSpread[sb] = 0.0
            }
            Reserved (set to 0) ................... 4
            ObjectDecorCoefPrefix .................. 2
            if (ObjectDecorCoefPrefix > 1) { ObjectDecorCoef[sb] .. 8 }
        }
    }
    AlignBits ................................. VARIABLE
    AudioDescription ............................ 8
    if (AudioDescription & 0x80) {
        while (AudioDescription != 0x00) {
            /* NULL terminated ASCII text entry */
            AudioDescription ...................... 8
        }
    }
    Reserved (set to 0) .......................... 8
}
```

**Table 7 — NumPanSubBlocks / PanSubBlockSize vs. Sample Rate and Frame
Rate** (p.16, §5.4.1, informative — derived from `SampleRate`+`FrameRate`,
not read from the bitstream):

| Frame Rate (fps) | NumPanSubBlocks |
|---|---|
| 24, 25, 30   | 8 |
| 48, 50, 60   | 4 |
| 96, 100, 120 | 2 |

(`PanSubBlockSize`/`Duration` columns govern `AudioDataDLC`'s internal sample
grouping only — irrelevant to metadata-element parsing, see "Scope
decisions" #3.)

**Table 8 — Zone IDs** (p.17, §5.4.5, informative — array position, not a
wire-coded ID):

| Zone ID | Description |
|---|---|
| 0 | All screen speakers left of center |
| 1 | Screen center speakers |
| 2 | All screen speakers right of center |
| 3 | All speakers on left wall |
| 4 | All speakers on right wall |
| 5 | All speakers on left half of rear wall |
| 6 | All speakers on right half of rear wall |
| 7 | All overhead speakers left of center |
| 8 | All overhead speakers right of center |

`MAX_KNOWN_ZONES` = 9 (the table has 9 rows, IDs 0-8).

**Table 9 — Zone Gain Code** (p.17):

| Code | Gain |
|---|---|
| 0x0 | Set gain to 0.0 |
| 0x1 | Set gain of 1.0 |
| 0x2, 0x3 | Reserved |

**Table 10 — Object Spread Mode Code** (p.17):

| Code | Identifier | Description |
|---|---|---|
| 0x0 | OBJECT_SPREAD_LOWREZ | Equal spreading in all dimensions, 8-bit coding |
| 0x1 | RESERVED | Reserved |
| 0x2 | OBJECT_SPREAD_1D | Equal spreading in all dimensions, 12-bit coding |
| 0x3 | RESERVED | Reserved |

**Table 11 — Decorrelation Coef Prefix Code** (p.18):

| Code | Gain |
|---|---|
| 0x0 | No decorrelation |
| 0x1 | Maximum decorrelation |
| 0x2 | Decorrelation coefficient follows in the bitstream |
| 0x3 | (Reserved) |

Per the literal pseudocode, `ObjectDecorCoef` follows whenever
`ObjectDecorCoefPrefix > 1` — i.e. for **both** code `0x2` and the reserved
code `0x3`, not only `0x2`. This crate implements the condition exactly as
printed.

`AudioDescription`'s wire semantics beyond "bit 7 set => NULL-terminated
ASCII text follows" are **not documented anywhere in §5.4** — there is no
`5.4.x AudioDescription` subsection at all, only the syntax-table appearance
on p.10. See "Scope decisions" #2.

### 4.5 AudioDataDLC() (p.10-12)

```
AudioDataDLC()
{
    AudioDataID ............................... Plex(8)
    DLCSize ..................................... 16
    DLCSampleRate ................................ 2
    ShiftBits .................................... 5
    /* Predictor information */
    NumPredRegions ............................... 2
    for (n = 0; n < NumPredRegions; n++) {
        RegionLength[n] .......................... 4
        FIROrder[n] .............................. 5
        for (m = 1; m <= FIROrder[n]; m++) { FIRPredictor[n][m] .. 10 }
    }
    /* Coded residual */
    for (n = 0; n < NumSubBlocks; n++) {
        CodeType .................................. 1
        if (CodeType == 0) {
            /* PCM residual */
            BitDepth ............................... 5
            for (l = 0; l < SubBlockSize; l++) { Residual[...] .. BitDepth }
        } else {
            /* Rice-Golomb residual */
            RiceRemBits ............................ 5
            for (i = 0; i < SubBlockSize; i++) {
                /* unary quotient + RiceRemBits remainder + sign, per-sample */
            }
        }
    }
    /* if SampleRate == 96kHz: repeat predictor-info + coded-residual blocks */
    AlignBits ................................. VARIABLE
}
```

(Full unary/Rice-Golomb residual pseudocode is on p.11 — reproduced in the
PDF, elided here since this crate never decodes it; see "Scope decisions" #3.)

§5.5.1: "`DLCSize` shall indicate the size in bytes of the remainder of the
`AudioDataDLC` Element, or equivalently, the size of the entire element, not
including `ElementID`, `ElementSize`, `AudioDataID`, and `DLCSize`." This is
the load-bearing sentence that makes the opaque-payload boundary (below)
possible: everything after `DLCSize` can be skipped as exactly `DLCSize`
opaque bytes without interpreting a single one of them.

**Table 12 — (DLC) Sample Rate code** (p.18): `0x0`=48000 sps, `0x1`=96000
sps, `0x2`/`0x3`=Reserved. (Not exposed as a typed field — see "Scope
decisions" #3.)

**Table 15 — Residual Coding Type** (p.20): `0x0`=Direct PCM, `0x1`=Rice-
Golomb Coding. (Codec-internal; not exposed.)

## 5 Bit Stream Field Description (p.12-20)

Referenced field-by-field above alongside each element's syntax. Notable
additional detail:

- **Table 2 — SampleRate** (p.13): `0x0`=48000 sps, `0x1`=96000 sps,
  `0x2`/`0x3`=Reserved.
- **Table 3 — BitDepth** (p.13): `0x0`=Reserved, `0x1`=24 bits/sample
  ("Only 24-bits per audio sample are currently supported"), `0x2`/`0x3`=
  Reserved.
- **Table 4 — FrameRate** (p.13): `0x0`..`0x8` = 24,25,30,48,50,60,96,100,120
  fps; `0x9`-`0xF` = Reserved.
- **Table 6 — Channel IDs** (p.14-15): `0x0` Left Screen, `0x1` Left Center
  Screen, `0x2` Center Screen, `0x3` Right Center Screen, `0x4` Right Screen,
  `0x5` Left Side Surround (7.1), `0x6` Left Surround, `0x7` Left Rear
  Surround (7.1), `0x8` Right Rear Surround (7.1), `0x9` Right Side Surround
  (7.1), `0xA` Right Surround, `0xB` Left Top Surround (9.1), `0xC` Right Top
  Surround (9.1), `0xD` LFE, otherwise Reserved.
- §5.3.4: `AudioDataID` of `0` ("NULL") means "no audio asset" for a given
  bed channel.
- §5.2.5 `MaxRendered` (Plex(8)): "the maximum audio assets that will be
  rendered during playback... for optimal target playback."
- §5.2.6 `SubElementCount` (Plex(8)): count of elements in the current
  element (i.e. direct sub-elements of `ATMOSFrame`).

## Scope decisions made by this crate (not verbatim spec text)

1. **`Plex(n)` escape condition implemented per the general algorithm's
   prose + worked example, not per the literally-printed nested-`if`
   pseudocode.** The pseudocode's inner check reads `if (PlexData ==
   0xFFFFFF)` (24 bits of ones) guarding escalation from a 16-bit field —
   but a 16-bit field's only all-ones value is `0xFFFF` (16 bits), and the
   surrounding prose is explicit ("the field size for the replacement symbol
   is twice as long as the field it replaces... doubling of field size")
   plus the worked example (`0x1234` encodes as `0xFF1234`, a 24-bit total:
   8-bit escape + 16-bit value — impossible if the 16-bit level's own escape
   were 24 bits wide). This crate treats `0xFFFFFF` in the printed pseudocode
   as a transcription defect in the RDD (verified against the PDF's native
   text layer via `pdftotext -layout`, not an OCR misread) and implements:
   read `n` bits; if all-ones for that width, read `2n` bits; if *that* is
   all-ones for *its* width, read `4n` bits (terminal — matches the
   "value <= 0xFFFFFFFE" cap, since `0xFFFFFFFF` is the one unrepresentable
   32-bit value). Round-trip is exercised at all three widths in
   `tests/round_trip.rs`.

2. **`AudioDescription` is preserved structurally, not semantically
   decoded**, because §5.4 has no subsection describing it (a genuine gap in
   the disclosure document, not an oversight in this transcription — compare
   to every other field in §4.4, each of which gets a `5.4.x` writeup).
   `AudioDescription` is modeled as `{ flag_byte: u8, text: Option<&[u8]> }`:
   `text` is `Some` (the bytes up to, but excluding, the NULL terminator)
   exactly when `flag_byte & 0x80 != 0`, matching the pseudocode's literal
   gate — with no invented interpretation of the low 7 bits of `flag_byte`
   when text is absent (the spec never says what they mean either way, so
   this crate keeps them as an opaque byte rather than fabricate semantics).

3. **`AudioDataDLC`'s payload past `AudioDataID`+`DLCSize` is opaque,
   never decoded.** `DLCSampleRate`, `ShiftBits`, `NumPredRegions`,
   `FIROrder`, `FIRPredictor` (quantized reflection coefficients), and the
   Rice-Golomb/PCM residual samples together **are** the Dolby Lossless
   Coding (DLC) audio codec's own bitstream — a linear-predictive-plus-
   entropy-coded compressed-PCM scheme, not a config header. Per this
   project's "parses containers/metadata, never decodes/re-implements a
   codec bitstream" boundary, this crate exposes only the two byte-aligned
   fields that sit *before* the codec's own bit-packed layout begins
   (`AudioDataID`, `DLCSize`), plus the remaining `DLCSize` bytes verbatim as
   an opaque `&[u8]` payload — mirroring `st337`'s treatment of
   `burst_payload`. `DLCSampleRate`/`ShiftBits` are documented above as real
   wire fields, but deliberately **not** individually exposed: they sit 7
   bits into the codec's own bit-packed region (2+5 bits, non-byte-aligned),
   so extracting just those two without also parsing (or re-emitting a
   bit-shifted copy of) the codec bitstream that immediately follows them
   would be a partial, awkward decode — worse for API honesty than exposing
   none of the codec-internal fields. `NumPredRegions`/`FIROrder`/
   `FIRPredictor`/`CodeType`/residual samples are never modeled at all: this
   is genuinely the audio-essence bitstream, and the `DLCSize` byte-count
   field (§5.5.1, quoted above) is exactly what makes skipping it — instead
   of parsing it — possible without becoming a codec decoder.

4. **All "Reserved (set to `0xNN`)" fields are hard-validated constants,
   not preserved-verbatim passthrough bits.** Unlike `st337`'s `Pf` (which
   the spec calls "reserved for future use" in prose, with no fixed value
   given, and is therefore round-tripped byte-for-byte to avoid corrupting a
   real future value), every reserved field in RDD 29 carries an explicit
   "(set to X)" instruction in the syntax tables themselves — closer to
   `st337`'s hard-validated `Pa`/`Pb` sync words than to its soft-preserved
   `Pf`. This crate validates each such field against its documented literal
   value on parse (`Error::InvalidReserved`) and always re-emits the literal
   value on serialize, rather than round-tripping an unexamined stored
   value — consistent with the "no raw-byte passthrough" project rule.

5. **`NumPanSubBlocks` is derived from `FrameRate` alone** (Table 7 collapses
   to the same count for both sample rates at each frame rate: 8 for
   24/25/30fps, 4 for 48/50/60fps, 2 for 96/100/120fps) — `SampleRate` only
   affects `PanSubBlockSize`/`Duration`, which are audio-essence-internal and
   out of scope per decision 3. `ObjectDefinition1::parse` therefore needs
   only the enclosing `ATMOSFrame`'s `FrameRate` as parsing context (not the
   full sample-rate/frame-rate pair), passed down explicitly rather than
   inferred.

6. **No `declare_elements!`-macro dispatch.** Unlike `dvb-si`'s ~100
   descriptors or `dvb-t2mi`'s ~15 payload types, RDD 29 has exactly three
   concrete element kinds (`BedDefinition1`, `ObjectDefinition1`,
   `AudioDataDLC`) plus an `Unknown` catch-all — small enough that a
   hand-written `AnyElement` enum with a manual dispatch `match` is clearer
   than the project's declarative `declare_*!` macro, whose main value (a
   drift-guard over a large, growing set) doesn't pay for itself at this
   size. Worth revisiting if a future RDD 29 revision adds more element
   types.

7. **§3.3 Amplitude Gain (`A10`) is transcribed but not implemented** — no
   field in the syntax tables this crate implements (§4.2-4.5) is documented
   as using this coding; it may apply to a field this RDD doesn't otherwise
   specify in enough detail to implement, or to a future/related Dolby
   format. Left as a documented gap rather than fabricated onto an
   unrelated field.

8. **`RESERVED` `ElementID`s (`0x20`, `0x80`, `0x100`) and any other
   unrecognized `ElementID`** round-trip through `AnyElement::Unknown {
   element_id, data }` exactly as `ReadElement()`'s own `default` case
   specifies (`UnknownData … ElementSize * 8`) — never an error, per
   §5.1.1's explicit "the decoder shall skip the element."
