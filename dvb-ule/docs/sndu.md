# SNDU — SubNetwork Data Unit format

_Source: RFC 4326 §4 (Figures 1–6), transcribed_

A PDU (IP datagram, Ethernet frame, or other network-layer packet) is encapsulated
using ULE to form an **SNDU** (SubNetwork Data Unit). Each SNDU is an MPEG-2 Payload
Unit. The base encapsulation (Figure 1) is:

```
< ----------------------------- SNDU ----------------------------- >
+-+-------------------------------------------------------+--------+
|D| Length | Type | Dest Address* |           PDU         | CRC-32 |
+-+-------------------------------------------------------+--------+
                Figure 1 (* optional Destination Address)
```

All multi-byte values are transmitted in **network byte order** (most significant byte
first). The most significant bit of each byte is placed in the left-most position of
the 8-bit field (§4).

## SNDU base header layout (§4, §4.7.2 Figure 3)

The base header is the first 4 bytes (D + Length + Type), optionally followed by the
6-byte Destination Address when `D=0`:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|D|        Length  (15b)        |             Type (16b)        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|            Receiver Destination NPA Address  (6B)  [if D=0]   |
+                               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                               |                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+                               +
|                                                               |
=                              PDU                              =
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                            CRC-32  (4B)                        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Width | Mnemonic | Source | Meaning |
|-------|-------|----------|--------|---------|
| `D` (Destination Address Absent) | 1 b | bslbf | §4.1 | Most significant bit of byte 0. `0` = Destination Address Field present; `1` = absent. |
| `Length` | 15 b | uimsbf | §4.2 | Length in bytes of the SNDU, counted from the byte **following the Type field** up to and including the CRC. Includes any extension headers. See End Indicator special case (§4.3). |
| `Type` | 16 b | uimsbf | §4.4 | Payload type or presence of a Next-Header (see `ext-headers.md`). |
| `Dest Address` (NPA/MAC) | 6 B | — | §4.5 | Present only when `D=0`. Directly follows the fourth byte of the SNDU header. |
| `PDU` | variable | — | §4.7 | The encapsulated PDU (or extension-header chain + PDU). |
| `CRC-32` | 4 B (32 b) | — | §4.6 | Integrity check; last four bytes of the SNDU. |

### D — Destination Address Absent (§4.1)

- The D-bit is the **most significant bit of the Length field** (bit 0 of byte 0).
- `D=0` indicates the presence of the Destination Address Field (§4.5).
- `D=1` indicates that a Destination Address Field is **not** present.
- An End Indicator (§4.3) MUST be sent with `D=1`. Other SNDUs MAY be sent with `D=0`
  or `D=1`. The default method SHOULD use `D=0`.

### Length (§4.2)

- A 15-bit value: length in bytes of the SNDU, counted from the byte **following the
  Type field** of the base header, **up to and including the CRC**.
- Includes the size of any extension headers (`ext-headers.md`).
- Note the End-Indicator special case (§4.3) — see below.

### End Indicator (§4.3, §4.7.1)

- When the first two bytes following an SNDU have the value **0xFFFF**, this denotes an
  End Indicator: all-ones Length combined with a `D=1`.
- Bit layout: `D=1`, `Length = 0x7FFF` (all 15 length bits set) → the two-byte value
  reads **0xFFFF**.
- Indicates to the Receiver that there are no further SNDUs in the current TS Packet,
  and that no Destination Address Field is present.
- The End Indicator MUST carry a D-bit value of 1. Its format (Figure 2):

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|1|            0x7FFF           |                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+                               +
|   A sequence of zero or more bytes with a value 0xFF filling  |
=           the remainder of the TS Packet Payload             =
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
                Figure 2: ULE End Indicator
```

The value 0xFF has specific MPEG-2 framing semantics, where it is used to indicate
Padding. The End Indicator is followed by zero or more bytes of value 0xFF until the
end of the TS Packet payload.

## Type field (§4.4)

The 16-bit Type field indicates the payload type **or** the presence of a Next-Header.
The value space is divided at decimal 1536 (0x0600):

- **Type < 1536** → Next-Header (Extension Header), IANA-assigned (§4.4.1).
- **Type >= 1536** → EtherType for the carried payload, per the IANA EtherType registry
  (§4.4.2).

Examples (§4.4.1 / §4.4.2):

| Type | Meaning | Source |
|------|---------|--------|
| `0x0000` | Test SNDU | §5.1 |
| `0x0001` | Bridged Frame | §5.2 |
| `0x0100` | Extension-Padding | §5.3 |
| `0x0800` | IPv4 Payload | §4.7.2 |
| `0x86DD` | IPv6 Payload | §4.7.3 |

The full Type-field interpretation (Next-Header H-LEN/H-Type split, mandatory vs
optional extension headers) is detailed in `ext-headers.md`.

## SNDU Destination Address Field — NPA/MAC (§4.5)

When `D=0`, a Network Point of Attachment (NPA) field directly follows the fourth byte
of the SNDU header.

| Field | Width | Source |
|-------|-------|--------|
| `Receiver Destination NPA Address` | 6 B | §4.5 |

- NPA destination addresses are 6-byte numbers (resembling an IEEE MAC address) that
  identify the Receiver(s) in the MPEG-2 transmission network that should process the
  SNDU.
- The value `0x00:00:00:00:00:00` MUST NOT be used as a destination address.
- The least significant bit of the **first byte** is set to `1` for multicast frames;
  the remaining bytes specify the link-layer multicast address.
- `0xFF:FF:FF:FF:FF:FF` is the link broadcast address (deliver to all Receivers).
- IPv4 subnetwork-broadcast packets, when carried with `D=0`, MUST use the NPA link
  broadcast address `0xFF:FF:FF:FF:FF:FF`.
- The field MUST be carried (`D=0`) for IP unicast packets destined to routers on
  shared links. A sender MAY omit it (`D=1`) where the Receiver can use a discriminator
  (the IP destination address or a bridged MAC address) combined with the PID as a
  link-level address.
- IP multicast packets carried with `D=0` MUST map the IP group destination address to
  the multicast SNDU Destination Address (RFC 1112 for IPv4; RFC 2464 for IPv6).

## SNDU Trailer CRC-32 (§4.6)

Each SNDU MUST carry a 32-bit CRC field in the **last four bytes** of the SNDU.

- **Generator polynomial** (hexadecimal): `0x104C11DB7` (the standard CRC-32 polynomial,
  as used by Ethernet, DSM-CC section syntax, and AAL5):

  `x^32 + x^26 + x^23 + x^22 + x^16 + x^12 + x^11 + x^10 + x^8 + x^7 + x^5 + x^4 + x^2 + x^1 + x^0`

- **Initial value**: the CRC-32 accumulator register is initialised to `0xFFFFFFFF`.
- **Coverage**: all bytes from the **start of the SNDU header** to the **end of the
  SNDU, excluding the 32-bit CRC trailer itself**.
- Bytes are processed in order of increasing position within the SNDU; the order of
  processing bits is **NOT reversed**. (This use resembles, but differs from, SCTP /
  RFC 3309.)
- The Receiver independently recomputes the CRC and compares it with the transmitted
  trailer. SNDUs with an invalid CRC are discarded, and the Receiver enters the Idle
  State (§7.2).

## Defined base SNDU formats (§4.7)

| Format | D | Type | Layout | Source |
|--------|---|------|--------|--------|
| End Indicator | 1 | (0x7FFF length) | header only + 0xFF padding | §4.7.1 |
| IPv4 (L2 filtering) | 0 | 0x0800 | header + NPA + IPv4 datagram + CRC | §4.7.2 Fig 3 |
| IPv4 (L3 filtering) | 1 | 0x0800 | header + IPv4 datagram + CRC | §4.7.2 Fig 4 |
| IPv6 (L2 filtering) | 0 | 0x86DD | header + NPA + IPv6 datagram + CRC | §4.7.3 Fig 5 |
| IPv6 (L3 filtering) | 1 | 0x86DD | header + IPv6 datagram + CRC | §4.7.3 Fig 6 |
| Test SNDU | 0/1 | 0x0000 | header (+NPA if D=0) + discarded data + CRC | §5.1 |
| Bridged SNDU | 0/1 | 0x0001 | see `ext-headers.md` | §5.2 |

> ⚠ The carried Destination Address is labelled "Receiver Destination NPA Address" in
> the §4.7 figures and "NPA" / "MAC address" in the prose; it is the same 6-byte field.
> Distinct from the bridged-frame inner MAC Destination/Source addresses (`ext-headers.md`).
