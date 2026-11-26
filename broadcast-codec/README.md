# broadcast-codec

The codec seam for `rust-broadcast`: raw audio/video frame types and two
sans-IO traits that an outside codec plugs into.

**This crate holds no codec.** It defines the vocabulary a decoder or encoder
adapter exchanges with the rest of the workspace:

- [`CodecId`] and [`CodecParameters`] say what a stream is and what a decoder
  needs before its first packet.
- [`Packet`] is one coded access unit; [`Frame`] is one decoded audio or video
  frame. Video frames carry planar, semi-planar or packed storage at 8, 10 or
  12 bits, plus colour description, static HDR metadata and opaque ITU-T T.35
  payloads.
- [`Decoder`] and [`Encoder`] are object-safe, sans-IO traits. A real codec
  library (`symphonia`, `rav1d`, `rav1e`, `openh264`, …) is wrapped in a type
  that implements one of them, without changing this crate or `transmux`.

## Contract

The caller drives both traits the same way:

1. `send` one input unit.
2. Call `receive` until it returns `Ok(None)`; that means "send more".
3. Call `finish` at end of input.
4. Keep calling `receive` until it returns `Ok(None)` again; that means
   "drained".

`receive` returning `Ok(None)` is never an error, and a decoder or encoder may
hold input for as long as it needs before producing output. `reset` drops all
held state so the next input starts a new stream.

Every public struct has private fields, a constructor or builder, and
accessor methods. Every public struct and enum is `#[non_exhaustive]`.

The API is **0.x and expected to change** while real codecs are brought through
it. Nothing here participates in the workspace's DVB lockstep; the crate is
unpublished.

## Features

- `std` (default) — forwards `std` to `broadcast-core`. The crate is
  `#![no_std]` plus `alloc` either way.
