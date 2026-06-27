# SMPTE ST 291-1 ANC Data Packet — parity & checksum (via RFC 8331)

_Source: RFC 8331 "RTP Payload for SMPTE ST 291-1 Ancillary Data" (T. Edwards,
February 2018) §2 + §2.1 (text pp. 4–10), render-verified from the vendored
`specs/rfc8331_anc_rtp.txt`._

This document grounds the **per-ANC-packet field set, the 10-bit-word parity
rule, and the `Checksum_Word` computation** that SMPTE ST 2038 (see
[`st_2038.md`](st_2038.md)) defers to SMPTE ST 291-1 ("reinserted as ancillary
data, per SMPTE ST 291-1"). ST 291-1 itself is paid (not vendored), but RFC 8331
**§2.1 reproduces** the ST 291-1 ANC-packet semantics — including the parity and
checksum derivations — and is freely vendorable. RFC 8331 §1 states its data
model "is based on the data model of SMPTE ST 2038 [ST2038]", so the field set
maps directly onto the ST 2038 per-ANC-packet fields.

> Scope note: RFC 8331 wraps the ANC packets in an **RTP payload** (RTP header +
> a payload header: Extended Sequence Number, Length, `ANC_Count`, `F`,
> reserved). The RTP/payload-header framing is RFC-8331-specific and **not**
> part of ST 2038. Only the **per-ANC-packet** fields (§2.1, from `C` through
> `word_align`) and the **parity/checksum math** are the ST 291-1 material that
> ST 2038 defers; the RTP framing is transcribed below for context but flagged
> as not-ST-2038.

---

## §2 Packet diagram (RFC 8331 Figure 1, p. 4) — render-verified

The example RTP packet (two ANC data packets, on lines 9 and 10):

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|P|X| CC    |M|    PT       |        sequence number        |   } RTP header
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+   } (RFC 3550)
|                           timestamp                           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|           synchronization source (SSRC) identifier            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|   Extended Sequence Number    |           Length=32           |   } payload
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+   } header
| ANC_Count=2   | F |                reserved                   |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|C|   Line_Number=9     |   Horizontal_Offset   |S| StreamNum=0 |   } ANC packet
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+   } header #1
|         DID       |        SDID       |  Data_Count=0x84  |       } 10-bit words
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
                         User_Data_Words...
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
            |   Checksum_Word   |         word_align            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|C|   Line_Number=10    |   Horizontal_Offset   |S| StreamNum=0 |   } ANC packet #2
...
```

> The 10-bit ANC words are packed contiguously (not byte-aligned within an ANC
> packet); each ANC packet is padded to a 32-bit boundary by `word_align`.

---

## §2.1 Per-ANC-packet fields (pp. 6–10), render-verified

For each ANC data packet, the header fields below **MUST** be present, in this
order. (These map directly to the ST 2038 per-ANC-packet fields in
[`st_2038.md`](st_2038.md) Table 2.)

### C (1 bit) — color-difference channel flag
Set to `1` ⇒ ANC data corresponds to the **color-difference data channel (C)**.
Set to `0` ⇒ ANC data corresponds to the **luma (Y) data channel**, **or** the
source is an SD signal, **or** the source has no specific luma/color-difference
channels. For a multi-stream source, `C` refers to the channel of the stream
used to transport the packet. (Mirrors ST 2038 `c_not_y_channel_flag`.)

### Line_Number (11 bits)
Digital-interface line number of the ANC data packet, unsigned, network byte
order. Special generic-vertical-location values (RFC 8331 p. 7):

| Line_Number | Generic vertical location |
|-------------|---------------------------|
| `0x7FF` | Without specific line location within the field or frame |
| `0x7FE` | On any line from the 2nd line after the line specified for switching (SMPTE RP 168) to the last line before active video, inclusive |
| `0x7FD` | On a line number larger than can be represented in 11 bits (future formats) |

### Horizontal_Offset (12 bits)
Location relative to **start of active video (SAV)**, unsigned, network byte
order; `0` ⇒ the ADF begins immediately following SAV. Measured in 10-bit words
of the indicated data stream and channel. Special values (RFC 8331 p. 8):

| Horizontal_Offset | Generic horizontal location |
|-------------------|-----------------------------|
| `0xFFF` | Without specific horizontal location |
| `0xFFE` | Within horizontal ancillary data space (HANC), per SMPTE ST 291-1 |
| `0xFFD` | Within the ancillary data space between SAV and EAV markers |
| `0xFFC` | Horizontal offset larger than can be represented in 12 bits (future formats / certain low-frame-rate 720p) |

> Note (p. 8): the 12-bit width is kept to maintain easy conversion to/from
> SMPTE ST 2038, which also has a 12-bit Horizontal_Offset field. ST 296 systems
> 7/8 (1280×720p @ 24 / 23.98) have a luma sample max of 4124; an offset beyond
> 4091 (0xFFB) is unlikely, and `0xFFC` signals an offset too large to represent.

### S (Data Stream Flag) (1 bit)
Indicates whether the data stream number of a multi-stream mapping is specified.
`S = 0` ⇒ `StreamNum` gives no guidance on the source data stream number.
`S = 1` ⇒ `StreamNum` carries source-data-stream-number information.

### StreamNum (7 bits)
If `S = 1`, **MUST** carry source-data-stream-number identification. If the data
stream is numbered, `StreamNum` = (source data stream number − 1). For
unnumbered multi-stream interfaces: `0` = link A / left-eye stream, `1` = link B
/ right-eye stream.

> An ANC packet with `Line_Number = 0x7FF` **and** `Horizontal_Offset = 0xFFF`
> **SHALL** be considered carried without any specific location within the
> field/frame (RFC 8331 p. 10).

Immediately after the header fields, the following 10-bit-word data fields
**MUST** be present (per SMPTE ST 291-1):

### DID (10 bits)
Data identification word.

### SDID (10 bits)
Secondary data identification word. Used only for a **"Type 2"** ANC packet.
In a **"Type 1"** ANC packet this word actually carries the **data block number
(DBN)**.

### Data_Count (10 bits)
The **lower 8 bits** (b7 MSB … b0 LSB) contain the actual count of 10-bit words
in `User_Data_Words`. **b8 is the even parity for bits b7..b0, and b9 is the
inverse (logical NOT) of b8.** (See the parity rule below.)

### User_Data_Words (integer number of 10-bit words)
The UDW convey data identified by DID (or DID+SDID). The number of 10-bit words
is given by `Data_Count`. Words are carried MSB-first to LSB-last.

### Checksum_Word (10 bits)
Validity check over the DID word through the UDW. (See the checksum computation
below.)

At the end of each ANC packet:

### word_align (bits as needed to complete a 32-bit word)
Enough **`0`** bits to complete the last 32-bit word of the ANC packet in the
RTP payload. If the packet already ends on a 32-bit boundary, no bits are added.
`word_align` **SHALL** be used even for the last ANC packet in an RTP packet,
and **SHALL NOT** be used if zero ANC packets are carried.

> ⚠ **`word_align` padding bit value is `0`** in RFC 8331 (p. 10). This differs
> from ST 2038, where the in-PES `while (!bytealigned)` padding emits **`1`**
> bits (see [`st_2038.md`](st_2038.md) §4.2 / flag 3). The padding *purpose*
> also differs: RFC 8331 word-aligns to **32 bits**; ST 2038 byte-aligns to
> **8 bits**. Do not carry the `0`-bit rule over to the ST 2038 parser.

---

## The 10-bit-word parity rule (ST 291-1, via RFC 8331)

Every 10-bit ANC word that carries an 8-bit value (DID, SDID, Data_Count, each
User_Data_Word) uses its top two bits as parity, exactly as RFC 8331 states for
`Data_Count` (p. 10):

> "Bit b8 is the even parity for bits b7 through b0, and bit b9 is the inverse
> (logical NOT) of bit b8."

Formally, for the payload value byte `b7..b0`:

```
b8 = even_parity(b0..b7)        # b8 = XOR of b0..b7  (set so that b0..b8 has even # of 1s)
b9 = NOT b8
```

So a valid 10-bit word is `{ b9=!b8, b8=parity, b7..b0=value }`.

> ⚠ "Even parity of b0..b7" here means: `b8` is chosen so that the count of `1`
> bits among `b0..b7` **plus `b8`** is even — i.e. `b8 = b0 XOR b1 XOR … XOR b7`.
> RFC 8331 states the rule explicitly only for `Data_Count`'s b8/b9, and ST 291-1
> applies the same b9=NOT(b8) construction to DID/SDID/UDW. The "even parity over
> b0..b7" phrasing is RFC 8331's wording for Data_Count; the per-word value width
> (8 data bits) is the ST 291-1 convention these fields follow.

---

## The `Checksum_Word` computation (RFC 8331 p. 10, render-verified)

RFC 8331 defines `Checksum_Word` (10 bits) verbatim:

> "It consists of 10 bits, where bits b8 (MSB) through b0 (LSB) define the
> checksum value and bit b9 is the inverse (logical NOT) of bit b8. The checksum
> value is equal to the **nine least significant bits of the sum of the nine
> least significant bits** of the DID word, the SDID word, the Data_Count word,
> and all User_Data_Words in the ANC data packet. The checksum is initialized to
> zero before calculation, and any 'end carry' resulting from the checksum
> calculation is ignored."

Formally:

```
sum = ( DID[8:0] + SDID[8:0] + Data_Count[8:0]
        + Σ User_Data_Word[i][8:0] )      # each term is its low 9 bits (b8..b0)

checksum_value = sum & 0x1FF               # low 9 bits; end carry discarded
b9             = NOT (checksum_value >> 8) # b9 = inverse of b8 (the MSB of the 9-bit value)
Checksum_Word  = (b9 << 9) | checksum_value
```

Key points:

- The summed terms are the **low 9 bits (b8..b0)** of DID, SDID, Data_Count, and
  every UDW — **not** the full 10-bit words (the b9 inverse-parity bit is
  **excluded** from the sum).
- The accumulator starts at **zero**; **end carry is discarded** (i.e. the sum is
  taken modulo 2⁹).
- The result occupies `Checksum_Word` bits **b8..b0**; **b9 = NOT b8** (same
  inverse-MSB construction as the data words — but note `Checksum_Word` uses a
  **9-bit** value field, b8..b0, whereas the data words use an 8-bit value field
  b7..b0 with b8 as the parity bit).

> ⚠ Subtle but load-bearing: the parity/checksum constructions are **not
> identical**. For DID/SDID/Data_Count/UDW, the *value* is **8 bits** (b7..b0),
> **b8 = even parity**, **b9 = NOT b8**. For `Checksum_Word`, the *value* is
> **9 bits** (b8..b0, the summed checksum), and only **b9 = NOT b8** — there is
> no separate "even parity" bit in the checksum word; b8 is simply the MSB of the
> 9-bit checksum value.

---

## §7 Validation guidance (RFC 8331 p. 16, render-verified)

To avoid buffer-overflow attacks, receivers **SHOULD** validate that ANC packets
are of the appropriate length (using `Data_Count`) for the DID/SDID-specified
type, and the `Checksum_Word` **SHOULD** be checked to ensure the data has not
been damaged in transit (noting the checksum is unlikely to provide integrity
against a directed attack).

---

## ⚠ Flags summary

1. **`word_align` padding bit is `0` (RFC 8331), vs `1` in ST 2038.** Different
   bit value *and* different alignment width (32-bit word vs 8-bit byte). Don't
   cross-apply.

2. **Parity ≠ checksum construction.** Data words (DID/SDID/Data_Count/UDW):
   8-bit value + b8 even-parity + b9=NOT(b8). `Checksum_Word`: 9-bit value
   (b8..b0) + b9=NOT(b8), no separate parity bit. Easy to conflate.

3. **Checksum sums the low *9* bits of each term, mod 2⁹, end-carry discarded.**
   The b9 inverse-parity bit of each summed word is excluded from the sum.

4. **"Even parity of b0..b7"** is RFC 8331's wording (stated for Data_Count's
   b8/b9). ST 291-1 applies the same b9=NOT(b8) construction to DID/SDID/UDW;
   the underlying ST 291-1 standard is paid/not vendored, but RFC 8331 §2.1
   reproduces these derivations — this is free, vendorable footing.

5. **RTP/payload-header framing** (Extended Sequence Number, Length, `ANC_Count`,
   `F`, reserved, RTP header) is **RFC-8331-specific**, not part of ST 2038.
   Only the per-ANC-packet fields + parity/checksum are the ST-291-1 material
   ST 2038 defers.
