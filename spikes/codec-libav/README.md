# codec-libav

A spike: FFmpeg's libavcodec behind [`broadcast_codec`]'s `Decoder` and
`Encoder` traits, offered through a `CodecProvider` so an application reaches
every codec libav has, on this machine, without naming libav at its call sites.

It lives in its own Cargo workspace with its own `Cargo.lock`. It is not a
member of the root workspace, is never built by CI there, is not published, and
no crate in the root workspace, `fuzz/`, `demo/`, `tools/` or `bindings/`
depends on it or on any FFmpeg binding. The dependency runs one way: this
spike depends on `broadcast-codec`.

## Licensing

**This spike links FFmpeg dynamically.** The terms of the FFmpeg build on the
machine that runs the binary are the terms of the distributed result: FFmpeg is
LGPL version 2.1 or later, and GPL when it was built with GPL parts enabled
(`--enable-gpl`). Whoever distributes a binary linked against that libav is the
distributor and carries those obligations, including source-offer duties if
their build is GPL.

Codec patent licences are a separate matter and the distributor's, not this
spike's: no patent licence for H.264, HEVC, AAC or any other codec carried by
libav is granted here or implied by linking a user's own build.

This crate links no FFmpeg statically and builds no FFmpeg from source: it uses
the system library through `pkg-config`, with no `static`, `build` or
vendoring feature enabled.

This spike is not published and carries no release version.

## What it provides

- `LibavProvider`: every decoder and encoder the linked libav reports whose
  codec maps to a `broadcast_codec::CodecId`, including hardware codecs and
  hardware acceleration modes.
- `LibavDecoder` and `LibavEncoder`: `broadcast_codec::Decoder` and
  `broadcast_codec::Encoder` over one libav context, with no libav type in
  their API.

## Running it

The system needs FFmpeg's development files visible to `pkg-config`:

```sh
pkg-config --modversion libavcodec
cargo run --example avcodec_version
cargo test --locked
```

Tests skip loudly when libav is not usable on the host. Fixtures come from the
`test-fixtures` crate.

[`broadcast_codec`]: ../../broadcast-codec
