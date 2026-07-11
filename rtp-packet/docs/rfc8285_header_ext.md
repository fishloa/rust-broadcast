# RFC 8285 — one-byte/two-byte RTP header-extension multiplexing

Curated transcription. This document is the implementation and audit oracle
for the `rfc8285` feature of `rtp-packet` — cite it, not the raw RFC text,
from module docs.

Source: [RFC 8285](https://www.rfc-editor.org/rfc/rfc8285.txt), "A General
Mechanism for RTP Header Extensions" (free IETF RFC, October 2017). RFC 8285
is a profile-specific interpretation of the RFC 3550 §5.3.1 generic header
extension's opaque `data` — see `rtp-packet/docs/rtp-header.md` for that
layer.

## §4.1 — General

> The RTP header extension is formed as a sequence of extension elements,
> with possible padding. Each extension element has a local identifier and a
> length. The local identifiers MAY be mapped to a larger namespace in the
> negotiation (e.g., session signaling).

## §4.1.2 — Header Extension Type Considerations (the ID space + parsing algorithm)

> Each extension element in a packet has a local identifier (ID) and a
> length. The local identifiers present in the stream MUST have been
> negotiated or defined out of band. There are no static allocations of
> local identifiers. Each distinct extension MUST have a unique ID. **The ID
> value 0 is reserved for padding and MUST NOT be used as a local
> identifier.**
>
> An extension element with an ID value equal to 0 MUST NOT have an
> associated length field greater than 0. If such an extension element is
> encountered, its length field MUST be ignored, processing of the entire
> extension MUST terminate at that point, and only the extension elements
> present prior to the element with ID 0 and a length field greater than 0
> SHOULD be considered.

> There are two variants of the extension: one-byte and two-byte headers.
> ... Each RTP packet with an RTP header extension following this
> specification will indicate whether it contains one-byte or two-byte
> header extensions through the use of the "defined by profile" field.

> A sequence of extension elements, possibly with padding, forms the header
> extension defined in the RTP specification. There are as many extension
> elements as will fit in the RTP header extension, as indicated by the RTP
> header extension length. Since this length is signaled in full 32-bit
> words, padding bytes are used to pad to a 32-bit boundary. **The entire
> extension is parsed byte by byte to find each extension element (no
> alignment is needed)**, and parsing stops (1) at the end of the entire
> header extension or (2) in the "one-byte headers only" case, on
> encountering an identifier with the reserved value of 15 — whichever
> happens earlier.

> In both forms, padding bytes have the value of 0 (zero). They MAY be
> placed between extension elements, if desired for alignment, or after the
> last extension element, if needed for padding. A padding byte does not
> supply the ID of an element, nor does it supply the length field. **When a
> padding byte is found, it is ignored, and the parser moves on to
> interpreting the next byte.**

> Note carefully that the one-byte header form allows for data lengths
> between 1 and 16 bytes, by adding 1 to the signaled length value (thus, 0
> in the length field indicates that one byte of data follows). ... This
> addition is not performed for the two-byte headers, where the length
> field signals data lengths between 0 and 255 bytes.

§5 (SDP signaling) restates the shared ID namespace explicitly:

> `<value>` is the local identifier (ID) of this extension and is an integer
> in the valid range (**0 is reserved for padding in both forms**, and 15 is
> reserved in the one-byte header form, as noted above).

So the byte-by-byte scan algorithm, common to both forms:

1. If the next byte is `0x00`, it is one padding byte: consume it, continue.
2. Otherwise it begins a real extension element header (form-specific
   layout below).
3. **One-byte form only**: if the element's 4-bit ID nibble is `15`
   (`0b1111`), this is the reserved "stop" marker — ignore its length
   nibble, **terminate parsing of the entire extension**, keeping only the
   elements already accumulated.
4. **Malformed-but-specified case (§4.1.2, both forms)**: an element whose
   *ID is 0* but whose length field is `> 0` (which, per the byte-by-byte
   scan, can only arise in the one-byte form as a byte like `0x_5` — a
   nonzero byte whose upper nibble is still `0`, since a literal `0x00`
   byte is caught by rule 1 as plain padding first) — the length field
   **MUST be ignored** and processing **MUST terminate at that point**,
   same as rule 3. This crate treats it identically to the one-byte stop
   marker: stop, keep prior elements.
5. Otherwise, decode ID + length per the form-specific layout below, read
   that many data bytes as the element body, and continue the scan.

## §4.2 — One-Byte Header

> In the one-byte header form of extensions, the 16-bit value required by
> the RTP specification for a header extension, labeled in the RTP
> specification as "defined by profile", **MUST have the fixed bit pattern
> `0xBEDE`**.

```
 0
 0 1 2 3 4 5 6 7
+-+-+-+-+-+-+-+-+
|  ID   |  len  |
+-+-+-+-+-+-+-+-+
```

> The 4-bit ID is the local identifier of this element in the range **1-14
> inclusive**. ... The local identifier value **15** is reserved for a
> future extension and MUST NOT be used as an identifier. ...
>
> The 4-bit length is the number, minus one, of data bytes of this header
> extension element following the one-byte header. Therefore, the value
> zero (0) in this field indicates that one byte of data follows, and a
> value of 15 (the maximum) indicates element data of 16 bytes.

So: `id` valid range is **1..=14**; `data.len()` valid range is **1..=16**
(wire `len` nibble = `data.len() - 1`).

Worked example (§4.2), three elements + padding, `length=3` (3 words = 12
bytes of body after the 4-byte profile/length header):

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       0xBE    |    0xDE       |           length=3            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|  ID   | L=0   |     data      |  ID   |  L=1  |   data...
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
      ...data   |    0 (pad)    |    0 (pad)    |  ID   | L=3   |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                          data                                 |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

Byte-by-byte this is: `elem(id=A, L=0 → 1 data byte)`, `elem(id=B, L=1 → 2
data bytes)`, `0x00` pad, `0x00` pad, `elem(id=C, L=3 → 4 data bytes)` — 12
bytes total. The RFC diagram gives concrete hex only for the profile id
(`0xBEDE`) and `length`; the element IDs and data payloads are left
abstract. `tests/round_trip_8285.rs` instantiates this exact
element/padding *structure* with concrete (but RFC-non-specified) ID/data
values, documented there as spec-structure-derived rather than verbatim RFC
bytes.

## §4.3 — Two-Byte Header

> In the two-byte header form, the 16-bit value defined by the RTP
> specification for a header extension, labeled in the RTP specification as
> "defined by profile", is defined as shown below.

```
 0                   1
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|         0x100         |appbits|
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

> The appbits field is 4 bits that are application dependent and MAY be
> defined to be any value or meaning; this topic is outside the scope of
> this specification. ... If no extension has been specified through
> configuration or signaling for this local identifier value (256), the
> appbits field SHOULD be set to all 0s (zeros) by the sender and MUST be
> ignored by the receiver.

So the 16-bit `defined by profile` value is `0x100` (12 bits) in the top 12
bits + a 4-bit `appbits` field in the bottom 4 bits: on the wire,
`profile_id & 0xFFF0 == 0x1000` identifies the two-byte form, with
`profile_id & 0x000F` the (opaque, application-defined) `appbits`.

```
 0                   1
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       ID      |     length    |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

> The 8-bit ID is the local identifier of this element in the range **1-255
> inclusive**. ... Note that there is one ID space for both the one-byte
> form and the two-byte form. This means that the lower values (1-14) can
> be used in the 4-bit ID field in the one-byte header format with the same
> meanings.
>
> The 8-bit length field is the length of extension data in bytes, not
> including the ID and length fields. The value zero (0) indicates that
> there is no subsequent data.

So: `id` valid range is **1..=255** (0 excluded — reserved for padding, per
§4.1.2/§5 above, same as the one-byte form); `data.len()` valid range is
**0..=255**, stored directly (no `length - 1` bias, unlike the one-byte
form).

Worked example (§4.3), three elements + padding, `length=3` (12 bytes of
body):

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       0x10    |    0x00       |           length=3            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|      ID       |     L=0       |     ID        |     L=1       |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       data    |    0 (pad)    |       ID      |      L=4      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                          data                                 |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

Byte-by-byte: `elem(id=A, L=0 → 0 data bytes)`, `elem(id=B, L=1 → 1 data
byte)`, `0x00` pad, `elem(id=C, L=4 → 4 data bytes)` — 12 bytes total.
Again instantiated with concrete ID/data values in
`tests/round_trip_8285.rs`, documented as spec-structure-derived.

## Wire layout this crate implements (`rfc8285` feature)

Given an already-parsed RFC 3550 `HeaderExtension { profile_id, data }`
(see `docs/rtp-header.md`), this feature adds a profile-specific decode of
`data`:

- `profile_id == 0xBEDE` → **one-byte form**: `data` is a byte-by-byte
  sequence of `OneByteElement { id: OneByteId (1..=14), data: &[u8]
  (1..=16 bytes) }`, `0x00` padding bytes, and an optional trailing
  reserved-ID-15 (or malformed-ID-0-with-length) stop point after which any
  remaining bytes are not parsed as elements.
- `profile_id & 0xFFF0 == 0x1000` → **two-byte form**: `data` is a
  byte-by-byte sequence of `TwoByteElement { id: TwoByteId (1..=255),
  data: &[u8] (0..=255 bytes) }` and `0x00` padding bytes.
- Anything else: `data` is not an RFC 8285 extension this crate knows how
  to decode — a distinct "not an RFC 8285 profile" error/result, **not** a
  malformed-packet error (RFC 8285 interpretation is opt-in/profile-scoped,
  per RFC 3550 §5.3.1: "the actual format of the extension is specified by
  the profile").

Byte-exact round trip: `Serialize` on the parsed element list reproduces
every element byte-for-byte, in order, followed by whatever trailing
zero-padding is needed to keep the overall extension a multiple of 4 bytes
(so it can be placed straight back into a `HeaderExtension.data` slot,
whose own `Serialize`/parse already enforces 4-byte word alignment — see
`docs/rtp-header.md`). Padding position is **canonicalized to a single
trailing run**, not preserved verbatim from the original wire bytes — see
judgment call 4 below.

## Judgment calls / deviations from a literal paraphrase (worth double-checking)

1. **ID 0 is reserved in *both* forms, not just the one-byte form.** A
   naive reading of §4.3 alone ("the 8-bit ID ... in the range 1-255
   inclusive") could be mistaken for "any byte 0-255 is a valid two-byte
   ID," but §4.1.2 and §5's SDP signaling text state plainly: "0 is
   reserved for padding **in both forms**." This matters for round-trip
   correctness, not just spec purity: since padding bytes and two-byte
   element headers share the same byte-by-byte scan, a hypothetical
   `TwoByteElement { id: 0, .. }` would serialize with a leading `0x00`
   byte that a conformant parser (including this crate's own) would
   consume as an ordinary padding byte instead of the start of that
   element — desynchronizing every subsequent byte in the scan. This crate
   therefore rejects `id == 0` for **both** `OneByteId` and `TwoByteId`,
   via validating newtypes.
2. **One-byte form has a second malformed-input case beyond ID 15**: a
   byte whose upper (ID) nibble is `0` but whose lower (length) nibble is
   nonzero (e.g. `0x05`) is not a plain `0x00` padding byte, yet still
   carries the reserved ID 0. Per §4.1.2 this must also terminate parsing
   (keeping prior elements), identically to encountering ID 15. This
   crate's one-byte parser treats both cases the same way.
3. **Stream-level "don't mix one-byte and two-byte forms" policy (§4.1.2)**
   is a *per-stream* signaling/negotiation concern (SDP Offer/Answer or
   out-of-band knowledge across a whole RTP session), not something
   visible from a single `HeaderExtension`. This crate does not — and
   structurally cannot — enforce it; each `HeaderExtension` is decoded
   independently by its own `profile_id`.
4. **Padding position is canonicalized on serialize, not preserved
   verbatim.** §4.1.2 explicitly permits padding "between extension
   elements, if desired for alignment, or after the last extension
   element" — both of RFC 8285's own worked examples (§4.2/§4.3) place
   their padding *between* elements, not at the end. Since padding carries
   no semantic content (it is skipped, not decoded, by definition), this
   crate's parsed element list does not record *where* padding fell, only
   which elements were present and in what order. Consequently,
   `Serialize` always emits a single trailing padding run: parsing one of
   RFC 8285's own worked-example byte sequences and re-serializing does
   **not** reproduce those exact bytes (the padding moves to the tail),
   even though it decodes to the identical element list, and re-parsing
   the re-serialized form yields that same list again (a genuine
   byte-identical round trip *from this crate's own canonical output*,
   just not from arbitrary legal RFC 8285 input with interspersed
   padding). `tests/round_trip_8285.rs` exercises both: decoding the
   RFC's own inter-element-padding layout correctly, and a true
   byte-identical `Serialize` round trip on the canonical (trailing-pad)
   form.
