# RTCP — RTP Control Protocol (RFC 3550 §6)

Curated transcription. This document is the implementation and audit oracle
for `rtcp-packet` — cite it, not the raw RFC text, from module docs.

Source: [RFC 3550](https://www.rfc-editor.org/rfc/rfc3550.txt), "RTP: A
Transport Protocol for Real-Time Applications" (free IETF RFC), §6.

## §6.1 — RTCP Packet Format (common header + compound packet)

Every RTCP packet begins with a fixed part similar to RTP data packets,
followed by structured elements that MAY be variable length but MUST end on a
32-bit boundary — "The alignment requirement and a length field in the fixed
part of each packet are included to make RTCP packets 'stackable'." Multiple
RTCP packets are concatenated with no separators to form a **compound
packet**, sent in a single lower-layer packet (e.g. one UDP datagram); there
is no explicit count of individual packets, since the lower layer provides an
overall length.

Compound packet rules (§6.1):

- **SR or RR**: "The first RTCP packet in the compound packet MUST always be
  a report packet... even if no data has been sent or received, in which
  case an empty RR MUST be sent."
- **SDES**: an SDES packet containing a CNAME item MUST be included in each
  compound packet (usage/bandwidth guidance, not a wire-layout constraint
  this crate enforces).
- **BYE or APP**: MAY follow in any order; BYE SHOULD be the last packet sent
  for a given SSRC/CSRC; packet types MAY appear more than once.

This crate's [`CompoundPacket`] enforces only the wire-structural
constraint that is actually checkable from bytes alone: the first packet
must be SR or RR. The SDES-CNAME-inclusion and BYE-last rules are session
*policy*, not a framing invariant, and are left to the caller.

Common header, shared by every RTCP packet type:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|P|   RC/SC/subtype |     PT        |            length     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

- **version (V), 2 bits** — "identifies the version of RTP, which is the
  same in RTCP packets as in RTP data packets. The version defined by this
  specification is two (2)." Rejected if not 2 on parse (§6.4.1).
- **padding (P), 1 bit** — "If the padding bit is set, this individual RTCP
  packet contains some additional padding octets at the end which are not
  part of the control information but are included in the length field. The
  last octet of the padding is a count of how many padding octets should be
  ignored, including itself (it will be a multiple of four)." (§6.4.1)
- **RC / SC / subtype, 5 bits** — reception report count (SR/RR, §6.4.1: "A
  value of zero is valid"), source count (SDES/BYE, §6.5/§6.6: "A count value
  of zero is valid, but useless"), or APP subtype (§6.7: "May be used as a
  subtype... or for any application-dependent data").
- **packet type (PT), 8 bits** — SR=200, RR=201, SDES=202, BYE=203, APP=204.
- **length, 16 bits** — "The length of this RTCP packet in 32-bit words minus
  one, including the header and any padding. (The offset of one makes zero a
  valid length and avoids a possible infinite loop in scanning a compound
  RTCP packet, while counting 32-bit words avoids a validity check for a
  multiple of 4.)" (§6.4.1)

## §6.4.1 — SR: Sender Report (PT=200)

```
header |V=2|P|    RC   |   PT=SR=200   |             length            |
       +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
       |                         SSRC of sender                        |
sender |              NTP timestamp, most significant word             |
info   |             NTP timestamp, least significant word             |
       |                         RTP timestamp                         |
       |                     sender's packet count                     |
       |                      sender's octet count                     |
report |                 SSRC_1 (SSRC of first source)                 |
block  | fraction lost |       cumulative number of packets lost       |
  1    |           extended highest sequence number received           |
       |                      interarrival jitter                      |
       |                         last SR (LSR)                         |
       |                   delay since last SR (DLSR)                  |
       :                    (RC report blocks total)                   :
       |                  profile-specific extensions                  |
```

Three sections (the header is 8 octets), possibly followed by a fourth
profile-specific extension section "if defined":

1. **Header** (8 bytes): `V/P/RC | PT=200 | length` + `SSRC` (32 bits, "The
   synchronization source identifier for the originator of this SR packet").
2. **Sender info** (20 bytes, always present): `NTP timestamp` (64 bits, MSW
   + LSW — "wallclock time... when this report was sent"), `RTP timestamp`
   (32 bits, "corresponds to the same time as the NTP timestamp... but in the
   same units and with the same random offset as the RTP timestamps in data
   packets"), `sender's packet count` (32 bits), `sender's octet count` (32
   bits, "not including header or padding").
3. **Zero or more report blocks** (24 bytes each, RC of them) — see below.
4. **Profile-specific extensions** (optional, undefined by RFC 3550 itself —
   opaque and profile-dependent). **Not decoded by this crate**: see
   "Implementation notes" below.

## §6.4.1 — Report block (24 bytes, shared by SR §6.4.1 and RR §6.4.2)

- **SSRC_n, 32 bits** — "The SSRC identifier of the source to which the
  information in this reception report block pertains."
- **fraction lost, 8 bits** — "expressed as a fixed point number with the
  binary point at the left edge of the field... If the loss is negative due
  to duplicates, the fraction lost is set to zero."
- **cumulative number of packets lost, 24 bits** — "the number of packets
  expected less the number of packets actually received... the loss may be
  negative if there are duplicates." **Signed** two's-complement — this
  crate sign-extends bit 23 to a full `i32` on parse and masks back to 24
  bits on serialize.
- **extended highest sequence number received, 32 bits** — low 16 bits =
  highest sequence number received; high 16 bits = the sequence-number-cycle
  count (§A.1).
- **interarrival jitter, 32 bits** — "an estimate of the statistical
  variance of the RTP data packet interarrival time... expressed as an
  unsigned integer." (The jitter *algorithm*, §6.4.1/§A.8, is caller
  responsibility — this crate only carries the sampled value.)
- **last SR timestamp (LSR), 32 bits** — "The middle 32 bits out of 64 in
  the NTP timestamp... received as part of the most recent RTCP sender
  report (SR) packet... If no SR has been received yet, the field is set to
  zero."
- **delay since last SR (DLSR), 32 bits** — "expressed in units of 1/65536
  seconds... If no SR packet has been received yet... set to zero."

## §6.4.2 — RR: Receiver Report (PT=201)

"The format of the receiver report (RR) packet is the same as that of the SR
packet except that the packet type field contains the constant 201 and the
five words of sender information are omitted (these are the NTP and RTP
timestamps and sender's packet and octet counts)." So: header (8 bytes,
`SSRC of packet sender` immediately after `V/P/RC | PT=201 | length`) + RC ×
report block + optional profile-specific extensions (not decoded, as SR).
"An empty RR packet (RC = 0) MUST be put at the head of a compound RTCP
packet when there is no data transmission or reception to report."

## §6.5 — SDES: Source Description (PT=202)

```
header |V=2|P|    SC   |  PT=SDES=202  |             length            |
chunk  |                          SSRC/CSRC_1                          |
  1    |                           SDES items                          |
chunk  |                          SSRC/CSRC_2                          |
  2    |                           SDES items                          |
```

"A three-level structure composed of a header and zero or more chunks, each
of which is composed of items describing the source identified in that
chunk." `SC` (5 bits): "The number of SSRC/CSRC chunks contained in this SDES
packet. A value of zero is valid but useless."

Each chunk: `SSRC/CSRC` (32 bits) + a list of zero or more items, **starting
on a 32-bit boundary**. Each item: an 8-bit `type`, an 8-bit `length`
("describing the length of the text (thus, not including this two-octet
header)"), and the text itself — "Items are contiguous, i.e., items are not
individually padded to a 32-bit boundary." Text encoding is UTF-8 (RFC 2279).

"The list of items in each chunk MUST be terminated by one or more null
octets, the first of which is interpreted as an item type of zero to denote
the end of the list. No length octet follows the null item type octet, but
additional null octets MUST be included if needed to pad until the next
32-bit boundary. ... A chunk with zero items (four null octets) is valid but
useless." This padding is **separate** from the header's `P` bit.

### §6.5.1–§6.5.8 — SDES item types

| Type | Name  | §     |
|------|-------|-------|
| 1    | CNAME | 6.5.1 — Canonical End-Point Identifier (mandatory item; `user@host`-style, globally unique per source for cross-media/cross-tool association) |
| 2    | NAME  | 6.5.2 — User Name |
| 3    | EMAIL | 6.5.3 — Electronic Mail Address (RFC 822 `user@host`) |
| 4    | PHONE | 6.5.4 — Phone Number |
| 5    | LOC   | 6.5.5 — Geographic User Location |
| 6    | TOOL  | 6.5.6 — Application or Tool Name |
| 7    | NOTE  | 6.5.7 — Notice/Status |
| 8    | PRIV  | 6.5.8 — Private Extensions (see below) |

### §6.5.8 — PRIV internal sub-structure

```
|     PRIV=8    |     length    | prefix length |prefix string...
...             |                  value string               ...
```

A PRIV item's `text` (the bytes after the 2-byte type+length item header) is
**itself** structured: an 8-bit `prefix length`, that many bytes of `prefix`
string (an application-chosen unique name), then a `value` string filling
the rest of the item. **Not decoded by this crate**: see "Implementation
notes" below — a PRIV item's `text` is exposed as the flat, undivided item
payload.

## §6.6 — BYE: Goodbye (PT=203)

```
      |V=2|P|    SC   |   PT=BYE=203  |             length            |
      |                           SSRC/CSRC                           |
      :                              ...                              :
(opt) |     length    |               reason for leaving            ...
```

Header (`SC`, 5 bits: "A count value of zero is valid, but useless") + `SC` ×
`SSRC/CSRC` (32 bits each) + an *optional* 8-bit length + that many octets of
UTF-8 reason text ("e.g., 'camera malfunction' or 'RTP loop detected'"). "If
the string fills the packet to the next 32-bit boundary, the string is not
null terminated. If not, the BYE packet MUST be padded with null octets to
the next 32-bit boundary." This padding is separate from the header `P` bit.

## §6.7 — APP: Application-Defined (PT=204)

```
   |V=2|P| subtype |   PT=APP=204  |             length            |
   |                           SSRC/CSRC                           |
   |                          name (ASCII)                         |
   |                   application-dependent data                ...
```

Header (`subtype`, 5 bits, "May be used as a subtype to allow a set of APP
packets to be defined under one unique name, or for any application-dependent
data") + `SSRC/CSRC` (32 bits) + `name` (4 octets, "interpreted as a sequence
of four ASCII characters, with uppercase and lowercase characters treated as
distinct") + application-dependent data — "It MUST be a multiple of 32 bits
long."

## Implementation notes (known fidelity gaps, documented per project
discipline rather than silently glossed over)

- **SR/RR profile-specific extensions are not decoded or preserved.** RFC
  3550 defines no structure for them at this layer ("if defined" — entirely
  profile-specific), and this crate's parser does not retain any bytes past
  the `RC` report blocks even though the header `length` field may cover
  them. A wire packet that carries a non-empty profile-specific extension
  section will **not** round-trip byte-identically through this crate
  (`serialize` reproduces only the header + sender-info + report blocks). No
  RFC 3550-profile in this workspace emits one today, so this is a
  documented gap, not a silent behavioral regression — a future extension
  would need its own typed field (opaque `&[u8]` tail) to close it.
- **SDES PRIV (type 8) sub-structure is not decoded.** [`SdesItem`] exposes
  every item's `text` as a flat `String`, including PRIV's, so the
  `prefix_length`/`prefix`/`value` split described in §6.5.8 is not
  separately typed — callers that need it must re-parse `text`'s bytes
  themselves. Round-trip is still byte-identical (the flat text is preserved
  verbatim), so this is a decode-completeness gap, not a wire-fidelity bug.

## Wire layout this crate implements

Each packet type above maps to a struct with symmetric `Parse`/`Serialize`
(byte-identical round-trip for every field this crate decodes, per the
caveats above); [`RtcpPacket`] dispatches on the header `PT` byte;
[`CompoundPacket`] is a `Vec<RtcpPacket>` enforcing the §6.1 leading-SR/RR
rule on both parse and construction.
