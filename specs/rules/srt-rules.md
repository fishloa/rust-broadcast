# SRT (draft-sharabayko-srt-01) — packet structure rules

Curated field tables for the `srt-runtime` packet codecs. Source:
`specs/ietf_draft_sharabayko_srt_01.txt` (line cites below), §3 "Packet
Structure" only — this is the free, redistributable IETF Internet-Draft, so
(unlike the gitignored-copyrighted-PDF `specs/fulltext/` convention used
elsewhere in this repo) the source text is committed directly in `specs/`.

This document is packet **structure** only. The handshake exchange (§4.3),
loss/ARQ handling, TSBPD (§4.5), congestion control (§5), and the AES
key-wrap/unwrap crypto detail (§6) are out of scope for this note and for the
current `srt-runtime` release — see the crate root docs for the follow-up
list.

## SRT header — §3 (L317-373)

Every SRT packet is one UDP datagram's payload. The first 32 bits always
start with `F` (Packet Type Flag): `0` = data packet, `1` = control packet
(L362-363). Both packet kinds share the fixed **16-byte header** shape: two
type-specific words, then:

- **Timestamp**: 32 bits, microseconds relative to connection establishment;
  packet send time or origin time depending on transmission mode (L365-368).
- **Destination Socket ID**: 32 bits; `0` is the special "connection request"
  value (L370-373).

## Data packet — §3.1, Figure 3 (L397-454)

| Field | Bits | Notes |
|---|---|---|
| `F` | 1 | `0` for a data packet |
| Packet Sequence Number | 31 | sequential per data packet |
| `PP` (Packet Position Flag) | 2 | `10b` first · `00b` middle · `01b` last · `11b` solo (whole message in one packet) (L418-422) |
| `O` (Order Flag) | 1 | in-order delivery required (`1`) or not (`0`) (L424-427) |
| `KK` (Key-based Encryption Flag) | 2 | `00b` not encrypted · `01b` even key · `10b` odd key · `11b` **reserved — control packets only** (L429-433) |
| `R` (Retransmitted Packet Flag) | 1 | set when this is a retransmission (L435-437) |
| Message Number | 26 | sequence number of the message this packet belongs to (L439-440) |
| Timestamp | 32 | see §3 |
| Destination Socket ID | 32 | see §3 |
| Data | variable | rest of the UDP datagram (L453-454) |

## Control packet — §3.2, Figure 4, Table 1 (L456-533)

| Field | Bits | Notes |
|---|---|---|
| `F` | 1 | `1` for a control packet |
| Control Type | 15 | Table 1 below |
| Subtype | 16 | `0x0` for every Table 1 type except User-Defined |
| Type-specific Information | 32 | meaning depends on Control Type (see per-type notes) |
| Timestamp | 32 | see §3 |
| Destination Socket ID | 32 | see §3 |
| CIF (Control Information Field) | variable | meaning depends on Control Type |

**Table 1 — Control Type values (L509-533):**

| Packet Type | Control Type | Section |
|---|---|---|
| HANDSHAKE | `0x0000` | §3.2.1 |
| KEEPALIVE | `0x0001` | §3.2.3 |
| ACK | `0x0002` | §3.2.4 |
| NAK (Loss Report) | `0x0003` | §3.2.5 |
| Congestion Warning | `0x0004` | §3.2.6 |
| SHUTDOWN | `0x0005` | §3.2.7 |
| ACKACK | `0x0006` | §3.2.8 |
| DROPREQ | `0x0007` | §3.2.9 |
| PEERERROR | `0x0008` | §3.2.10 |
| User-Defined Type | `0x7FFF` | reserved; carries Key Material (§3.2.2) via `Subtype` |

### Handshake — §3.2.1, Figure 5 (L535-759)

CIF is a 48-byte fixed core (`Type-specific Information` word unused/`0`)
followed by zero or more extension blocks:

| Field | Bits | Notes |
|---|---|---|
| Version | 32 | `4` or `5`; `>5` reserved |
| Encryption Field | 16 | Table 2 (L621-634): `0` none · `2` AES-128 · `3` AES-192 · `4` AES-256 |
| Extension Field | 16 | Table 3 bitmask (L644-655) on CONCLUSION: `0x0001` HSREQ · `0x0002` KMREQ · `0x0004` CONFIG; opaque echo on INDUCTION response |
| Initial Packet Sequence Number | 32 | first data packet's sequence number |
| Maximum Transmission Unit Size | 32 | typically 1500 |
| Maximum Flow Window Size | 32 | max in-flight (unacked) data packets |
| Handshake Type | 32 | Table 4 (L681-695): `0xFFFFFFFD` DONE · `0xFFFFFFFE` AGREEMENT · `0xFFFFFFFF` CONCLUSION · `0x00000000` WAVEHAND · `0x00000001` INDUCTION |
| SRT Socket ID | 32 | source socket issuing this handshake |
| SYN Cookie | 32 | randomized, meaning per handshake type |
| Peer IP Address | 128 (4×32) | IPv4: only word 0 non-zero |
| *(repeated)* Extension Type | 16 | Table 5 (L733-753) |
| *(repeated)* Extension Length | 16 | in 4-byte blocks |
| *(repeated)* Extension Contents | `Extension Length × 4` bytes | |

**Table 5 — Extension Type values (L733-753):** `1` `SRT_CMD_HSREQ` ·
`2` `SRT_CMD_HSRSP` · `3` `SRT_CMD_KMREQ` · `4` `SRT_CMD_KMRSP` ·
`5` `SRT_CMD_SID` · `6` `SRT_CMD_CONGESTION` · `7` `SRT_CMD_FILTER` ·
`8` `SRT_CMD_GROUP`.

**Handshake Extension Message** (§3.2.1.1, Figure 6, L760-799) — the contents
of an `SRT_CMD_HSREQ`/`SRT_CMD_HSRSP` block, always 12 bytes:

| Field | Bits |
|---|---|
| SRT Version | 32 (`major*0x10000 + minor*0x100 + patch`) |
| SRT Flags | 32 — Table 6 bitmask (L802-823): `0x01` TSBPDSND · `0x02` TSBPDRCV · `0x04` CRYPT (MUST be set) · `0x08` TLPKTDROP · `0x10` PERIODICNAK · `0x20` REXMITFLG (MUST be set) · `0x40` STREAM (buffer mode when set, else message mode) · `0x80` PACKET_FILTER |
| Receiver TSBPD Delay | 16 (ms) |
| Sender TSBPD Delay | 16 (ms) |

**Stream ID Extension** (§3.2.1.3, Figure 7, L875-919) — the contents of an
`SRT_CMD_SID` block: a UTF-8 string, max 512 bytes, **stored as 32-bit
little-endian words** (L919) — i.e. each 4-byte word's byte order is reversed
relative to every other big-endian field in the protocol. Padded with `0x00`
to the declared `Extension Length`.

**Group Membership Extension** (§3.2.1.4, Figures 8-9, L921-981) — the
contents of an `SRT_CMD_GROUP` block, 8 bytes; reserved for future multipath
use:

| Field | Bits | Notes |
|---|---|---|
| Group ID | 32 | |
| Type | 8 | `SRT_GTYPE_*`: `0` undefined · `1` broadcast · `2` main/backup · `3` balancing · `4` multicast (reserved) |
| Flags | 8 | only bit 0 (`M`) defined: message-number sync (`1`) vs sequence-number sync (`0`) (Figure 9, L973-981) |
| Weight | 16 | link priority (main/backup) or reserved |

### Key Material message — §3.2.2, Figures 10-11 (L859-874, L983-1178)

Carried as a Handshake Extension (`SRT_CMD_KMREQ`/`SRT_CMD_KMRSP` contents,
§3.2.1.2) **or** as the CIF of a User-Defined control packet (Control Type
`0x7FFF`, `Subtype` `SRT_CMD_KMREQ`/`SRT_CMD_KMRSP`, L996-1000).

| Field | Bits | Fixed value | Notes |
|---|---|---|---|
| `S` | 1 | `0` | reserved |
| `V` (Version) | 3 | `1` | initial version |
| `PT` (Packet Type) | 4 | `2` | `0` reserved · `1` Media Stream Message · `2` **Keying Material Message** · `7` reserved (MPEG-TS sync-byte discriminator) |
| `Sign` | 16 | `0x2029` | `'HAI'` PnP Vendor ID, big-endian |
| `Resv1` | 6 | `0` | reserved |
| `KK` | 2 | — | `00b` no SEK (invalid) · `01b` even · `10b` odd · `11b` both — **different field from the data-packet `KK`** despite the same 2-bit shape |
| KEKI | 32 | — | Key Encryption Key Index; `0` = default stream key |
| Cipher | 8 | — | `0` none/KEKI-indexed · `2` AES-CTR (SP800-38A) |
| Auth | 8 | — | `0` none/KEKI-indexed (only defined value) |
| SE (Stream Encapsulation) | 8 | — | `0` unspecified/KEKI-indexed · `1` MPEG-TS/UDP · `2` MPEG-TS/SRT |
| `Resv2` | 8 | `0` | reserved |
| `Resv3` | 16 | `0` | reserved |
| `SLen/4` | 8 | — | salt length ÷ 4; `0` or `4` (128-bit salt is the only defined length) |
| `KLen/4` | 8 | — | SEK length ÷ 4; `{4,6,8}` → 16/24/32 bytes (AES-128/192/256); MUST match the handshake's Encryption Field |
| Salt | `SLen` bytes | — | |
| ICV | 64 | — | AES key-wrap Integrity Check Vector |
| xSEK | `KLen` bytes | — | present iff `KK != 00b`; the even key if `KK=11b` |
| oSEK | `KLen` bytes | — | present iff `KK = 11b` (both keys); the odd key |

Wrap-field length formula (L1139-1141): `n = (KK + 1) / 2` (integer
division: `0` for no-SEK, `1` for a single key, `2` for both); Wrap length =
`(n * KLen) + 8` bytes (the `+8` is the ICV).

### Keep-Alive — §3.2.3, Figure 12 (L1181-1223)

No CIF. `Type-specific Information` reserved/`0`.

### ACK — §3.2.4, Figure 13 (L1237-1355)

`Type-specific Information` = **Acknowledgement Number** (sequential,
starting from 1). Three CIF shapes, selected by length (L1326-1354):

| Variant | CIF length | Fields present |
|---|---|---|
| Full (sent every 10 ms) | 28 bytes | Last Acknowledged Packet Sequence Number, RTT, RTT Variance, Available Buffer Size, Packets Receiving Rate, Estimated Link Capacity, Receiving Rate (all 32-bit) |
| Small | 16 bytes | first four of the above (up to and including Available Buffer Size) |
| Light | 4 bytes | Last Acknowledged Packet Sequence Number only |

Only Full ACKs are themselves acknowledged, via ACKACK (L1339-1340).

### NAK (Loss Report) — §3.2.5, Figure 14 (L1356-1413); loss-list coding — Appendix A (L4356-4394)

`Type-specific Information` reserved/`0`. CIF is a sequence of 31-bit
sequence-number entries:

- **Single** (Figure 21, L4373-4379): one 32-bit word, top bit `0`, low 31
  bits = the lost sequence number.
- **Range** (Figure 22, L4386-4394): two consecutive 32-bit words — the first
  with its top bit set to `1` (low 31 bits = range start `a`), the second
  with its top bit `0` (low 31 bits = range end `b`).

### Congestion Warning — §3.2.6, Figure 15 (L1415-1459)

Reserved for future use. No CIF; `Type-specific Information` = `0`.

### Shutdown — §3.2.7, Figure 16 (L1461-1497)

No CIF. `Type-specific Information` reserved/`0`.

### ACKACK — §3.2.8, Figure 17 (L1498-1547)

`Type-specific Information` = the Acknowledgement Number of the Full ACK
being acknowledged. No CIF.

### Message Drop Request — §3.2.9, Figure 18 (L1548-1618)

`Type-specific Information` = Message Number (`0` if the sender no longer has
the packet(s) and cannot restore it). CIF (8 bytes): First Packet Sequence
Number (32), Last Packet Sequence Number (32).

### Peer Error — §3.2.10, Figure 19 (L1629-1667)

`Type-specific Information` = Error Code (only `4000`, file-system error, is
currently defined). No CIF. Sender-side File Transfer Congestion Control only
(L1639-1640).

## Reserved-bit / fixed-value policy adopted by `srt-runtime`

Per this repo's convention of validating (rather than silently accepting)
documented reserved/fixed fields: `Subtype` (must be `0` except on
User-Defined), the header `Type-specific Information` word where a given
Control Type does not use it (Handshake, Keep-Alive, NAK, Congestion Warning,
Shutdown), and the Key Material message's `S`/`V`/`PT`/`Sign`/`Resv1`/`Resv2`/
`Resv3` fields are all checked against their spec-mandated value on parse and
are **not stored** in the typed structs (reconstructed on serialize). A
non-compliant value is a structured parse error, never a panic or silent
truncation.

## Deviations / clarifications from a literal reading

- **Table 3's bitmask column formatting**: the draft prints the `HSREQ` /
  `KMREQ` / `CONFIG` values as 8-hex-digit numbers (`0x00000001` etc.) even
  though the `Extension Field` they live in is only 16 bits (Figure 5). This
  is table-formatting consistency with the 32-bit `SRT Flags` table (Table 6)
  elsewhere in the same section, not a claim that the field is wider — all
  three values (`1`/`2`/`4`) fit comfortably in 16 bits. `srt-runtime` treats
  `Extension Field` as a `u16` bitmask.
- **Key Material `PT` field**: the draft documents `PT` as a general
  4-bit discriminator (values `0`/`1`/`2`/`7`) but only specifies the wire
  layout for `PT=2` ("Keying Material Message") in §3.2.2. `srt-runtime`'s
  `KeyMaterial::parse` therefore requires `PT=2` and treats any other value
  as `Error::InvalidKeyMaterial` — decoding a Media Stream Message (`PT=1`)
  is out of scope (undefined by this draft).
