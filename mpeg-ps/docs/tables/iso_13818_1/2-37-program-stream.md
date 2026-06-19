# Table 2-37 — Program stream / Table 2-38 — Program stream pack

_Source: ISO/IEC 13818-1:2021 (Rec. ITU-T H.222.0 06/2021) §2.5.3.1–2.5.3.3 (PDF p. 90).
Verified against the PDF render (blaze garbled the `pack()` line)._

## Table 2-37 — Program stream

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| MPEG2_program_stream() { |  |  |
| do { |  |  |
| pack() |  |  |
| } while (nextbits() == pack_start_code) |  |  |
| MPEG_program_end_code | 32 | bslbf |
| } |  |  |

- **MPEG_program_end_code**: `0x000001B9`. Terminates the program stream.

## Table 2-38 — Program stream pack

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| pack() { |  |  |
| pack_header() |  |  |
| while (nextbits() == packet_start_code_prefix) { |  |  |
| PES_packet() |  |  |
| } |  |  |
| } |  |  |

A pack is a `pack_header()` followed by zero or more `PES_packet()`s (each opening with
the 24-bit `packet_start_code_prefix` `0x000001`). See Table 2-39 for the pack header.
