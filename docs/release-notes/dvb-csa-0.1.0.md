# dvb-csa 0.1.0

Released 2026-08-07.

### Added

Initial release — pure-Rust DVB Common Scrambling Algorithm (CSA2):
56-round block cipher (SPN on 8-byte blocks) with key-permutation schedule,
LFSR stream cipher with dual 40-bit shift registers and S-box feedback,
CBC-like chaining combining block + stream ciphers, and TS-packet-level
`scramble()`/`descramble()`.

- `ControlWord` key type with `expand_block()` / `expand_stream()` derivations.
- `ts` module for TS-packet-level scramble/descramble (payload extraction).
- Oracle validation against libdvbcsa 1.1.0: 11 golden vectors (184-byte
  payloads) + 4 inline multi-size vectors (8B, 16B, 32B, 64B).
- Criterion benchmarks for 184-byte scramble/descramble throughput.
- CLI examples: `scramble_file`, `descramble_file`.
- `no_std` support (default); optional `std` feature.
