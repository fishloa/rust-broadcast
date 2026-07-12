# st337

[![Crates.io](https://img.shields.io/crates/v/st337.svg)](https://crates.io/crates/st337)
[![docs.rs](https://img.shields.io/docsrs/st337)](https://docs.rs/st337)

SMPTE ST 337:2015 ("Format for Non-PCM Audio and Data in an AES3 Serial
Digital Audio Interface") burst-preamble/burst-payload framing —
spec-complete parse/serialize, `no_std`.

ST 337 defines how compressed/non-PCM audio (AC-3, E-AC-3, DTS, etc.) and
other data is carried inside the AES3 professional digital audio
interface's sample words, via a repeating **burst-preamble** structure
(`Pa`/`Pb` sync words, a `Pc` info word, a `Pd` length word, and — for the
"extended" data-type escape code — `Pe`/`Pf`) followed by the opaque
compressed-audio (or other) payload.

- **[`Burst`]** — one complete data burst: [`BurstPreamble`] + the opaque
  `burst_payload` bytes (§7.1/§7.2).
- **[`BurstPreamble`]** — `data_type`, `data_mode`, `error_flag`,
  `data_type_dependent`, `data_stream_number`, `length_code`, and (when the
  six-word "extended" form is used) `Pe`/`Pf` (§7.2.4).
- **[`DataMode`]** — the `data_mode` field (§7.2.4.3 Table 8). Only
  `DataMode::Mode16` is supported for parse/build — see `docs/st337.md`'s
  "Scope decisions" for why.

See `docs/st337.md` for the curated SMPTE ST 337:2015 §7 transcription this
crate implements field-for-field (fetched directly from
`pub.smpte.org/latest/st337/st0337-2015.pdf`), and
`docs/st337-PROVENANCE.md` for a real-fixture + independent-oracle
(`ffmpeg -f spdif`) cross-check of the sync-word constants and `Pc` bit
layout — including a genuine, documented discrepancy it surfaced between
ST 337's own `length_code` semantics (bits) and IEC 61937's for the same
nominal data type (bytes).

**What this crate is not**: an AES3 physical-layer (biphase-mark line code,
subframe/timeslot bit placement) codec, and not a `data_type` -> codec
registry (that mapping is SMPTE ST 338, not independently verified here —
`data_type` is a plain validated `u8`, not an enum with invented codec
names). It parses/builds the *logical* burst word sequence as a plain byte
stream (`&[u8]`, 2 bytes per 16-bit preamble word) — the same
"container-not-codec" scope this workspace's `transmux` crate uses for media
containers.

`#![no_std]` + `alloc`; depends only on `broadcast-common`.

## Quick start

```rust
use broadcast_common::{Parse, Serialize};
use st337::{Burst, DataMode};

let payload = [0xDE, 0xAD, 0xBE, 0xEF];
let burst = Burst::new(1, DataMode::Mode16, false, 0, 0, None, &payload).unwrap();

let mut bytes = vec![0u8; burst.serialized_len()];
burst.serialize_into(&mut bytes).unwrap();
assert_eq!(Burst::parse(&bytes).unwrap(), burst);
```

## Examples

```sh
cargo run -p st337 --example build_burst
cargo run -p st337 --example parse_burst
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no      | `serde::Serialize`/`Deserialize` derives on public types. |

## Minimum Supported Rust Version

1.86

## License

MIT OR Apache-2.0
