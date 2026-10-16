# dvb-csa

Pure-Rust DVB Common Scrambling Algorithm (CSA2): the cipher underneath conditional access on DVB-S, DVB-T and DVB-C.

[![crates.io](https://img.shields.io/crates/v/dvb-csa.svg)](https://crates.io/crates/dvb-csa)
[![docs.rs](https://docs.rs/dvb-csa/badge.svg)](https://docs.rs/dvb-csa)

## What it does / what it does not

**Does.** Scrambles and descrambles MPEG-2 TS payloads (or any byte buffer) with
an 8-byte control word using CSA2: a 56-round block cipher chained CBC-style
plus an LFSR stream cipher. It also handles the TS-packet level: locating the
payload past the adaptation field and setting `transport_scrambling_control`
(even/odd key). An optional bitsliced path processes 64 independent payloads at
once. Users are descramblers, head-end scramblers, test-stream generators and
conformance tools.

**Does not.**
- It does not obtain control words: ECM/EMM handling, CAS and CAM interaction
  are out of scope (see `dvb-ci-runtime` for the CAM route, `dvb-simulcrypt`
  for head-end CW/ECM messaging).
- It does not implement DVB-CSA3 (AES-based, ETSI TS 100 289) or any other
  scrambling scheme (AES-CBC, CENC: see `transmux`).
- It does not parse the TS beyond what scrambling needs (use `mpeg-ts`), and
  does not identify which PIDs to descramble.
- It is not a normative implementation: DVB-CSA has no public specification
  (see below).

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| DVB-CSA (version 1/2) | none public: the algorithm was confidential, licensed through ETSI | Block cipher, stream cipher, chained mode, short-payload pass-through | Full (oracle-validated) | Correctness is established by byte-exact agreement with libdvbcsa 1.1.0: 18 committed golden vectors (`tests/fixtures/libdvbcsa-vectors.hex`, 13 full 184-byte TS payloads plus 8, 16, 32, 64 and 100 byte payloads) |
| ISO/IEC 13818-1 (MPEG-2 Systems) | as cited in `src/ts.rs` | §2.4.3.3 `transport_scrambling_control`, §2.4.3.4 adaptation field: payload location in a 188-byte packet | Full | Packets with no payload return `Error::NoPayload` |
| ETSI TS 100 289 V1.1.1 (2011-09), Support for use of the DVB Scrambling Algorithm version 3 | V1.1.1 (`etsi_ts_100_289_v01.01.01_dvb_scrambling_ca`, in `private/specs-md/INDEX.md`) | §5.1 Table 1 even/odd `transport_scrambling_control` values (`KeyParity`) | Partial | Only the even/odd signalling is used; the CSA3 cipher it describes is Not implemented |

Reverse-engineered, not spec-cited: a round-trip test proves nothing for a
cipher (any invertible function passes), so the libdvbcsa vectors are the gate.
A TSDuck-scrambled capture (`france-tnt-scrambled-0x02d0.ts`) is committed but
its control word was never recorded, so no test uses it.

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `std` | yes | `std::error::Error` on `Error`; enables `mpeg-ts/std`. |
| `bitsliced` | no | Adds `dvb_csa::bitsliced`, the 64-lane batch fast path. |

`no_std` without default features, with or without `bitsliced`.

## Quick start

```rust
use dvb_csa::{ControlWord, scramble, descramble};

let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);

let mut payload = vec![0xAA; 184];
scramble(&cw, &mut payload);
descramble(&cw, &mut payload);
// payload is back to all 0xAA
assert_eq!(payload, vec![0xAA; 184]);
```

At TS-packet level (skips the header and adaptation field, sets and checks
`transport_scrambling_control`):

```rust
use dvb_csa::ts::{self, KeyParity};
use dvb_csa::ControlWord;

let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
let mut packet = [0xAA_u8; 188];
packet[..4].copy_from_slice(&[0x47, 0x01, 0x00, 0x10]); // sync, PID 0x100, payload only, clear
let clear = packet;

ts::scramble_ts_packet(&cw, KeyParity::Even, &mut packet).unwrap();
assert_ne!(packet[4..], clear[4..]); // payload scrambled
ts::descramble_ts_packet(&cw, KeyParity::Even, &mut packet).unwrap();
assert_eq!(packet, clear); // and back
```

Runnable examples: `cargo run -p dvb-csa --example scramble_file` and
`--example descramble_file`.

## What is implemented

- **Core:** `scramble` / `descramble` over a payload (payloads under 8 bytes
  pass through unchanged, matching libdvbcsa); `ControlWord` (8 bytes,
  zeroized on drop, constant-time equality, redacted `Debug`).
- **TS level:** `ts::scramble_ts_packet` / `ts::descramble_ts_packet`,
  `ts::KeyParity` (`Even` = `10`, `Odd` = `11`). Refuses to scramble an
  already-scrambled packet (`Error::AlreadyScrambled`) and a parity/packet
  mismatch on descramble (`Error::ParityMismatch`).
- **Batch:** `bitsliced::scramble_batch` / `descramble_batch` over up to
  `bitsliced::LANES` (64) independent payloads (feature `bitsliced`).
- **Errors:** `Error` (`BufferTooShort`, `NoPayload`, `AlreadyScrambled`,
  `ParityMismatch`), `#[non_exhaustive]`.

### Algorithm

Both ciphers are keyed by the same control word. The block cipher is a 56-round
substitution/permutation network on 8-byte blocks in a CBC-like chain over all
complete blocks; the stream cipher is an LFSR byte generator seeded from the
nibble-swapped control word and the encrypted first block, XORed over bytes
8..end. Encrypt is block CBC then stream XOR; decrypt is stream XOR then block
CBC undo.

### Bitsliced batch fast path

```toml
[dependencies]
dvb-csa = { version = "0.3", features = ["bitsliced"] }
```

```rust
use dvb_csa::{ControlWord, bitsliced};

let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
let mut payloads: Vec<Vec<u8>> = (0..64).map(|_| vec![0xAA; 184]).collect();

let mut batch: Vec<&mut [u8]> = payloads.iter_mut().map(|p| p.as_mut_slice()).collect();
bitsliced::scramble_batch(&cw, &mut batch);
bitsliced::descramble_batch(&cw, &mut batch);
```

The unit of parallelism is the payload, not the block: scrambling's block
cipher is a reverse CBC and the stream cipher a chained LFSR, both strictly
sequential (the stream cipher is about two thirds of the work), so the API
batches independent payloads and has no single-payload bitsliced entry point.
A batch well under 64 leaves lanes idle. Measured by `benches/throughput.rs`
on an Apple M2 Ultra (rustc 1.86.0), 64 x 184-byte TS payloads:

| Operation | Scalar | Bitsliced | Speed-up |
|---|---|---|---|
| scramble | 15.7 MiB/s | 91.3 MiB/s | 5.8x |
| descramble | 15.1 MiB/s | 99.8 MiB/s | 6.6x |

The fast path is bit-exact with the scalar path, held by three gates: every
generated circuit is checked against the table it came from over its entire
input domain; a differential test requires identical output on randomised
payloads and lengths; and the libdvbcsa vectors run through the batch path
too. Circuits are generated by `tools/gen_circuits.py` from `src/tables.rs`,
the single source of truth for the cipher.

## Design notes

- Round-trip is not the correctness gate for a cipher; the external libdvbcsa
  vectors are.
- Control-word-derived state is zeroed with the `zeroize` crate and compared
  with `subtle` constant-time equality; `Debug` never prints key bytes
  (`tests/secret_debug_guard.rs`).
- `#[non_exhaustive]` enums, `name()` + `Display` labels, structured errors.
- `no_std`, no `alloc` needed for the scalar path.

## Related crates

Depends on `mpeg-ts` (TS header and adaptation-field decode), `zeroize`,
`subtle`, `thiserror`. Pairs with `dvb-simulcrypt` (head-end CW/ECM message
framing) and `dvb-ci` / `dvb-ci-runtime` (CAM descrambling); `dvb-si` locates
the CA descriptors and PIDs.

## MSRV and licence

Rust 1.95.0. MIT OR Apache-2.0.
