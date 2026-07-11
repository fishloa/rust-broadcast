# RTP fixed header + header extension — RFC 3550 §5.1 / §5.3.1

Curated transcription. This document is the implementation and audit oracle
for `rtp-packet` — cite it, not the raw RFC text, from module docs.

Source: [RFC 3550](https://www.rfc-editor.org/rfc/rfc3550.txt), "RTP: A
Transport Protocol for Real-Time Applications" (free IETF RFC).

## §5.1 — RTP Fixed Header Fields

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|P|X|  CC   |M|     PT      |       sequence number         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                           timestamp                           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|           synchronization source (SSRC) identifier            |
+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+
|            contributing source (CSRC) identifiers             |
|                             ....                              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

12-byte fixed header, followed by 0–15 32-bit CSRC identifiers
(`CC`-driven), followed by the header extension (§5.3.1) if `X=1`, followed
by the payload.

- **version (V), 2 bits** — "This field identifies the version of RTP. The
  version defined by this specification is two (2)." Reject any other value
  on parse.
- **padding (P), 1 bit** — "If the padding bit is set, the packet contains
  one or more additional padding octets at the end which are not part of the
  payload. The last octet of the padding contains a count of how many
  padding octets should be ignored, including itself." So when `P=1`: read
  `packet[len-1]` as the padding-octet count (inclusive of itself), and the
  real payload is `packet[..len - padding_count]`.
- **extension (X), 1 bit** — "If the extension bit is set, the fixed header
  MUST be followed by exactly one header extension, with a format defined in
  Section 5.3.1."
- **CSRC count (CC), 4 bits** — "The CSRC count contains the number of CSRC
  identifiers that follow the fixed header." Range 0–15 (4-bit field).
- **marker (M), 1 bit** — "The interpretation of the marker is defined by a
  profile. It is intended to allow significant events such as frame
  boundaries to be marked in the packet stream." Opaque bit at this layer —
  no profile-specific interpretation here.
- **payload type (PT), 7 bits** — "This field identifies the format of the
  RTP payload and determines its interpretation by the application."
  Profile-specific; opaque `u8` at this layer.
- **sequence number, 16 bits** — "The sequence number increments by one for
  each RTP data packet sent, and may be used by the receiver to detect
  packet loss and to restore packet sequence."
- **timestamp, 32 bits** — "The timestamp reflects the sampling instant of
  the first octet in the RTP data packet."
- **SSRC, 32 bits** — "The SSRC field identifies the synchronization
  source. This identifier SHOULD be chosen randomly."
- **CSRC list, 0 to 15 items, 32 bits each** — "The CSRC list identifies the
  contributing sources for the payload contained in this packet."

## §5.3.1 — RTP Header Extension

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|      defined by profile       |           length              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                        header extension                       |
|                             ....                              |
```

"If the X bit in the RTP header is one, a variable-length header extension
MUST be appended to the RTP header, following the CSRC list if present. The
header extension contains a 16-bit length field that counts the number of
32-bit words in the extension, excluding the four-octet extension header
(therefore zero is a valid length)."

So: a 16-bit `defined by profile` identifier (opaque to this crate — its
meaning is entirely profile-specific, e.g. defined by ST 2110 for its own
uses) + a 16-bit `length` (in 32-bit words, excluding the 4-byte
identifier+length header itself) + `length * 4` bytes of extension data
(also opaque — no further structure is defined by RFC 3550 itself).

## Wire layout this crate implements

`RtpPacket<'a>`:
1. 12-byte fixed header (V/P/X/CC/M/PT/seq/timestamp/SSRC)
2. `CC` × 4-byte CSRC identifiers
3. if `X=1`: 4-byte extension header (profile id + length) + `length*4` bytes
   of extension data
4. payload (the remaining bytes, minus the trailing padding-count byte(s) if
   `P=1`)

Byte-exact round-trip: `Serialize` must reproduce every byte above,
including deriving `CC` from the CSRC list length and `X` from whether an
extension is present — never trusting stray header bits independent of the
structured fields.
