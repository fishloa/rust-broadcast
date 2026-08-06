# dvb-csa

Pure-Rust implementation of the **DVB Common Scrambling Algorithm (CSA2)** —
the cipher underneath conditional access on DVB-S, DVB-T, and DVB-C.

## Status: oracle-validated, not spec-cited

DVB-CSA has no public normative specification — the algorithm was confidential
and licensed through ETSI. Every open implementation is reverse-engineered.
Correctness is established by byte-exact agreement with **libdvbcsa 1.1.0**
(VideoLAN's reference free implementation): 11 golden vectors (184-byte TS
payloads) plus inline multi-size vectors (8 B, 16 B, 32 B, 64 B).

## Algorithm overview

DVB-CSA2 combines a **block cipher** and a **stream cipher**, both keyed by the
same 8-byte control word:

- **Block cipher** — 56-round substitution/permutation network on 8-byte blocks,
  applied in a CBC-like chained mode across all complete blocks.
- **Stream cipher** — LFSR-based byte-stream generator seeded from the
  nibble-swapped control word and the encrypted first block as IV, XOR'd with
  bytes 8..end.

Encrypt = block CBC then stream XOR; decrypt = stream XOR then block CBC undo.
Payloads shorter than 8 bytes pass through unchanged.

## Usage

```rust
use dvb_csa::{ControlWord, scramble, descramble};

let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);

let mut payload = vec![0xAA; 184];
scramble(&cw, &mut payload);
descramble(&cw, &mut payload);
// payload is back to all 0xAA
```

For TS-packet-level operation (extracts the payload, respects the adaptation
field):

```rust
use dvb_csa::{ControlWord, ts};

let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
let mut packet = [0u8; 188];
// ... fill packet ...
let _ = ts::scramble_ts_packet(&cw, &mut packet);
```

## Features

| Feature | Default | What it does |
|---------|---------|--------------|
| `std`   | yes     | Enables `std::error::Error` on `Error` |

Builds `no_std` without default features.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
