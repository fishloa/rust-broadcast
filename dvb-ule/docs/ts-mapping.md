# SNDU → MPEG-2 TS Packet mapping

_Source: RFC 4326 §3, §4.3, §6, §7, transcribed_

PDUs are encapsulated to form SNDUs (`sndu.md`). Each SNDU is transmitted over an MPEG-2
transmission network either inside the payload of a single 188-byte TS Packet, or by
being fragmented across a series of TS Packets. Where space permits, a single TS Packet
MAY carry more than one SNDU (or part thereof) — **Packing**. All TS Packets comprising
an SNDU MUST carry the same PID (same TS Logical Channel) (§3).

ULE is limited to TS private streams only. TS Packets from a ULE Encapsulator MUST be
sent with an Adaptation Field Control (AFC) value of `01` (no adaptation field; payload
only). Receivers MUST discard TS Packets carrying other AFC values. Adaptation-field
stuffing is NOT used (§3).

## TS Packet header fields relevant to ULE (§2 "TS Header")

The 188-byte TS Packet has a 4-byte header; ULE references the following fields:

| Field | Width | Source | Use in ULE |
|-------|-------|--------|------------|
| Sync byte | 8 b | §2 | Fixed 0x47 |
| Transport Error Indicator | 1 b | §2 | — |
| `PUSI` (Payload Unit Start Indicator) | 1 b | §2 | Signals start of an SNDU + presence of the Payload Pointer |
| Transport Priority | 1 b | §2 | — |
| `PID` (Packet Identifier) | 13 b | §2 | Identifies the TS Logical Channel; same for all packets of an SNDU |
| Transport Scrambling Control | 2 b | §2 | — |
| `AFC` (Adaptation Field Control) | 2 b | §2 | MUST be `01` for ULE |
| `CC` (Continuity Counter) | 4 b | §2 | Incremented by 1 (mod 16) per TS Packet on the channel |
| `Payload Pointer` (PP) | 8 b | §2 | Present **only when PUSI=1** — the first byte after the TS header |

## Payload Unit Start Indicator (PUSI) (§3)

The semantics of PUSI for ULE follow MPEG-2 PSI packets:

- **PUSI = 0**: the TS Packet does NOT contain the start of an SNDU; it carries the
  continuation, or end, of an SNDU. No Payload Pointer is present.
- **PUSI = 1**: the TS Packet contains the start of at least one SNDU, and a one-byte
  **Payload Pointer** follows the last byte of the TS Packet header.

## Payload Pointer (PP) (§2, §6.1)

- A one-byte pointer present immediately after the 4-byte TS header **when PUSI = 1**.
- It contains the number of bytes that follow the Payload Pointer, counted from the
  first byte of the TS Packet payload field and **excluding the PP field itself**, up
  to the start of the first Payload Unit (SNDU).
- The first TS Packet of a new SNDU (after Idle, or starting a fresh channel) MUST carry
  PUSI = 1 and a Payload Pointer value of **0x00** — i.e. the SNDU starts immediately
  after the PP (§6.1).
- A PP value greater than 0 points past the tail of a continuing/previous SNDU to the
  start byte of the first new SNDU in this packet.

## Fragmentation and concatenation (§3, §6, Figures 13–15)

- **Fragmentation**: an SNDU larger than the available payload is segmented into a series
  of TS Packet payloads, sent on one TS Logical Channel (Figure 13). The first packet
  carries PUSI=1; continuation/final packets carry PUSI=0. The Continuity Counter MUST
  increment by 1 (mod 16) per successive TS Packet on the channel (§6.1).
- **Packing (concatenation)**: when a TS Packet has sufficient remaining payload, the
  Encapsulator MAY follow one SNDU directly with the next SNDU using the next available
  byte (Figure 15). If the packet's PUSI was not already set, the PUSI MUST be set to 1
  and a Payload Pointer inserted, pointing to the start of the newly packed SNDU.

## Padding / Stuffing (§3, §4.3, §6.1, §6.2)

When an SNDU finishes before the end of a TS Packet payload and no further SNDU is to be
started, the remainder is filled with bytes of value **0xFF** (Padding). The End
Indicator (0xFFFF, §4.3) marks "no more SNDUs in this packet"; it is followed by zero or
more 0xFF bytes to the end of the payload (Figure 14).

A Receiver that sees `0xFF` in the first byte where a Table Section's `table_id` would be
(i.e. the next SNDU's first byte) interprets it as Padding/Stuffing and silently discards
the remainder of the TS Packet payload (§3).

## Padding-and-Packing procedure — five end-of-SNDU cases (§6.2)

After completing an SNDU, exactly one of five actions occurs:

| Case | Remaining payload | Action |
|------|-------------------|--------|
| (i) | none | Transmit the packet. Start next SNDU in a new packet (PUSI=1, PP=0x00). |
| (ii) | exactly **1 byte** | MUST place `0xFF` in that final byte and transmit (a PP would consume the last byte). Start next SNDU in a new packet (PUSI=1, PP=0x00). |
| (iii) | exactly **2 bytes** and PUSI was **not** already set | MUST place `0xFFFF` (End Indicator) in the final two bytes and transmit (prevents fragmenting the Length field). Start next SNDU in a new packet (PUSI=1, PP=0x00). |
| (iv) | **> 2 bytes** | MAY transmit the partially full packet, but MUST first place `0xFF` in all remaining unused bytes (End Indicator followed by Padding). Start next SNDU in a new packet (PUSI=1, PP=0x00). |
| (v) | at least **2 bytes** for SNDU data (i.e. **3 bytes** if PUSI was not previously set, **2 bytes** if it was) | MAY pack a further SNDU starting at the next available byte. If PUSI was not already set, PUSI MUST be set to 1 and an 8-bit Payload Pointer inserted in the first byte after the TS header (reducing available data by one byte). |

## Reassembly rules at the Receiver (§7)

- **Idle State** (§7.1): the Receiver waits for the start of a new SNDU. It resumes only
  on a TS Packet with PUSI=1; the Payload Pointer then locates the first SNDU start.
- **Idle-state Payload-Pointer check** (§7.1.1): on entering reassembly the PP must be
  validated against the payload size.
- **Processing a received SNDU** (§7.2): the Receiver reads the Length, accumulates bytes
  across successive same-PID packets (PUSI=0 continuations), and on completion verifies
  the CRC-32 (`sndu.md` §4.6). An SNDU with an invalid CRC is discarded and the Receiver
  re-enters the Idle State.
- **Reassembly Payload-Pointer check** (§7.2.1): a packet with PUSI=1 arriving mid-SNDU
  carries a PP; the PP value must point past the tail of the SNDU being reassembled and
  be consistent with the remaining length, else the partial SNDU is discarded.
- The Continuity Counter is used to detect lost/duplicated TS Packets on the channel
  (§6.1, §7.3).

> ⚠ §7.3 "Other Error Conditions" and the detailed receiver state machine extend beyond
> the field layouts transcribed here; consult RFC 4326 §7 directly for the full
> error-handling rules.
