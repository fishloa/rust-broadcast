# st12-1

[![Crates.io](https://img.shields.io/crates/v/st12-1.svg)](https://crates.io/crates/st12-1)
[![docs.rs](https://img.shields.io/docsrs/st12-1)](https://docs.rs/st12-1)

SMPTE ST 12-1:2014 "Time and Control Code" — the §9 Linear Time Code (LTC)
80-bit logical codeword, `no_std`.

- **[`LtcFrame`]** — the 80-bit LTC codeword (§9.2): BCD hours/minutes/
  seconds/frames, the drop-frame and color-frame flags, the four
  rate-dependent flag bits (polarity correction / BGF0 / BGF1 / BGF2 —
  resolved via [`FrameRate`], since the codeword itself carries no
  self-describing frame-rate field), the eight 4-bit binary groups ("user
  bits"), and the fixed synchronization word.
- **[`FrameRate`]** — which of ST 12-1 Table 3's three flag-bit-position
  columns (30-frame / 25-frame / 24-frame) applies.
- **[`BinaryGroupUsage`] / [`BinaryGroupFlags`]** — Table 1's classification
  of what the binary groups contain.

See `docs/st12-1.md` for the curated ST 12-1 §8/§9 transcription this crate
implements field-for-field, including a verified-against-the-rendered-PDF
note on Table 3's frame-rate-dependent bit-position swap.

## Scope

This crate models only the **already-demodulated logical 80-bit codeword** —
never the §9.3 biphase-mark-encoded physical/analog audio waveform LTC is
carried as on a wire. That line-encoding/clock-recovery layer is out of scope
for this project (the same way it never decodes PCM or AC-3 audio samples).

`#![no_std]`; depends only on `broadcast-common`.

## Quick start

```rust
use broadcast_common::{Parse, Serialize};
use st12_1::LtcFrame;

let frame = LtcFrame {
    hours: 1,
    minutes: 23,
    seconds: 45,
    frames: 13,
    drop_frame_flag: false,
    color_frame_flag: true,
    flag_bit_27: true,
    flag_bit_43: true,
    flag_bit_58: false,
    flag_bit_59: true,
    user_bits: [1, 2, 3, 4, 5, 6, 7, 8],
};
let mut bytes = [0u8; st12_1::FRAME_LEN];
frame.serialize_into(&mut bytes).unwrap();
assert_eq!(LtcFrame::parse(&bytes).unwrap(), frame);
```

## Examples

```sh
cargo run -p st12-1 --example build_frame
cargo run -p st12-1 --example parse_frame
```

## Features

| Feature   | Default | Description |
|-----------|---------|-------------|
| `std`     | yes     | Link the standard library. Without it the crate is `#![no_std]`. |
| `serde`   | no      | `serde::Serialize`/`Deserialize` derives on public types. |

## Minimum Supported Rust Version

1.86

## License

MIT OR Apache-2.0
