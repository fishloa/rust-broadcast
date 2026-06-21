# dvb-emsg

[![Crates.io](https://img.shields.io/crates/v/dvb-emsg.svg)](https://crates.io/crates/dvb-emsg)
[![docs.rs](https://img.shields.io/docsrs/dvb-emsg)](https://docs.rs/dvb-emsg)

MPEG-DASH Event Message Box (`emsg`) — inband DASH/CMAF timed events (SCTE 35
splice signalling, ID3 metadata, ad/tracking triggers): version 0/1 `FullBox`
parse + serialize.

Implements:

- **`EmsgBox`** — the `'emsg'` ISOBMFF `FullBox` (`size` / `'emsg'` / `version`
  / `flags`) plus both version bodies: the two null-terminated UTF-8 strings
  (`scheme_id_uri`, `value`), the integer fields (`timescale`, `event_duration`,
  `id`), the version-discriminated presentation-time field, and the opaque
  `message_data[]`. `size` and `version` are **recomputed/derived on serialize**
  from the typed fields (no raw passthrough).
- **`PresentationTime`** — the version-discriminated timing field:
  `presentation_time_delta` (u32, version 0, segment-relative) vs
  `presentation_time` (u64, version 1, representation-relative). Selecting a
  variant *is* selecting the box version.
- **`EmsgVersion`** — the `version` byte (0 / 1) with its spec label.
- **`EmsgBox::is_scte35`** — recognises the SCTE 35 scheme
  (`SCTE35_SCHEME_PREFIX`, `urn:scte:scte35…`), in which case `message_data`
  carries a SCTE 35 `splice_info_section`.

Note the **v0/v1 field ordering differs**: version 0 places the two strings
*first* (before the integers); version 1 places the integers first and the
strings last. Both orderings are parsed and serialized.

## ⚠ Source footing — softer than the fully-free crates

The `emsg` **field semantics and types** are render-verified from a **free**
source: **DASH-IF IOP Part 10 V5.0.0, §6.1 + Table 6-2** (transcribed in
[`docs/emsg.md`](docs/emsg.md)). But the normative ISOBMFF box syntax — the
`aligned(8) class EventMessageBox extends FullBox('emsg', version, flags = 0)`
declaration, the exact byte-level field ordering, the `version`-gated branch,
and the null-terminated-string layout — lives in **ISO/IEC 23009-1 §5.10.3.3**,
which is **paid and NOT vendored** in this repo. The box layout here is
implemented from the **well-known public `emsg` structure** (widely reproduced
in MPEG-DASH / CMAF) combined with the free DASH-IF Part 10 semantics, with
ISO/IEC 23009-1 §5.10.3.3 cited as the formal (paid) normative source. This is
**softer footing** than the fully-free crates in this workspace — flagged per
project policy.

`#![no_std]` + `alloc`; depends only on `dvb-common`.

## Quick start

```rust
use dvb_emsg::{EmsgBox, PresentationTime};

let scte35 = [0xFCu8, 0x30, 0x11]; // start of a splice_info_section
let b = EmsgBox {
    scheme_id_uri: "urn:scte:scte35:2013:bin",
    value: "",
    timescale: 90_000,
    presentation_time: PresentationTime::Delta(0),
    event_duration: 0xFFFF_FFFF,
    id: 1,
    message_data: &scte35,
};
assert!(b.is_scte35());
let bytes = b.to_vec().unwrap();
assert_eq!(EmsgBox::parse(&bytes).unwrap(), b);
```

## Examples

```sh
cargo run -p dvb-emsg --example build_emsg
cargo run -p dvb-emsg --example parse_emsg
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no      | `serde::Serialize` derives on public types. |

## Minimum Supported Rust Version

1.81

## License

MIT OR Apache-2.0
