# media-plane

[![Crates.io](https://img.shields.io/crates/v/media-plane.svg)](https://crates.io/crates/media-plane)
[![docs.rs](https://img.shields.io/docsrs/media-plane)](https://docs.rs/media-plane)

The media-plane integration layer: ingress -> byte stages -> demux -> IR
transforms -> `Trunk` -> egress, per
[`docs/superpowers/specs/2026-07-26-media-plane-architecture.md`](https://github.com/fishloa/rust-broadcast/blob/main/docs/superpowers/specs/2026-07-26-media-plane-architecture.md)
in the workspace.

This first release (0.1.0, plan step 3a-i) delivers only the byte-stage
piece: [`ByteStage`], the pre-demux byte-to-byte drive contract, built
directly on `broadcast_common::Stage` rather than a second trait. `Trunk`,
cursors, ingress/egress traits, `ByteTap`, and `ByteMerge` are later steps
of the same plan and are not in this release.

`no_std` + `alloc`, with a `std` feature. See the crate-root docs for the
full architecture context and the `ByteStage` design rationale.

## License

MIT OR Apache-2.0
