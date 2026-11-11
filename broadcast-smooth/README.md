# broadcast-smooth

[![Crates.io](https://img.shields.io/crates/v/broadcast-smooth.svg)](https://crates.io/crates/broadcast-smooth)
[![docs.rs](https://img.shields.io/docsrs/broadcast-smooth)](https://docs.rs/broadcast-smooth)

Microsoft Smooth Streaming (MS-SSTR) client Manifest syntax: a typed model with parse and serialize.

```toml
[dependencies]
broadcast-smooth = "0.1"
```

## What it does / what it does not

**Does:** parse an MS-SSTR client Manifest (the `SmoothStreamingMedia` document a Smooth server serves at `/Manifest`) into `SmoothManifest`/`StreamIndex`/`QualityLevel`/`C`, and render it back with `to_xml()`. It covers every element and attribute of the client manifest: `MajorVersion`/`MinorVersion`/`Duration`/`TimeScale`, the live/DVR attributes (`IsLive`, `DVRWindowLength`, `LookAheadFragmentCount`), `Protection`/`ProtectionHeader`, each `StreamIndex` (`Type`, `Name`, `Subtype`, `Chunks`, `QualityLevels`, `TimeScale`, `Url`), its `QualityLevel`s (bitrate, FourCC, `CodecPrivateData`, geometry and audio parameters) and its `c` timeline (`t`/`d`/`r`/`n`). Fragment-URL templates resolve through `StreamIndex::resolve_fragment_url`, and the `c` timeline expands (bounded) through `StreamIndex::enumerate_chunks`. It was extracted from `transmux` (issue #2356) so a caller that only needs the manifest does not pull in the container-muxing hub; `transmux` keeps the Smooth fragments and the codec-config glue (`track_spec_from_quality_level`).

**Does not:** fetch anything or run a client (`broadcast-client` drives Smooth pull, `multimux` serves it), produce fragments (`transmux`'s `SmoothPackager`), synthesise an init segment from `CodecPrivateData` (`transmux::track_spec_from_quality_level`), or interpret the `ProtectionHeader` text (it is carried opaquely; PlayReady parsing belongs to a DRM crate).

## How output differs from input

The serializer builds the document from the typed model alone; nothing is echoed from the parsed source. Byte-identity with a parsed document is therefore not promised. In practice a real manifest round-trips to the same model, with these differences:

- attribute order is the writer's, not the source's; the declaration is always `<?xml version="1.0" encoding="utf-8"?>`, indentation is two spaces, and elements without children are written self-closing;
- attributes the model has no field for are dropped, as are unknown child elements and foreign-namespace content (`ProtectionHeader@SystemID` is such an attribute);
- `IsLive` is written `"TRUE"` when set (a source spelling it another way, e.g. `"1"`, is read as false);
- a duplicate attribute keeps its first value.

Every `c` element is written exactly as the model holds it — one element per entry, each attribute only when set (`n`, `t`, `d`, `r`) — so a model re-parses to itself; a writer that numbers its chunks or expands a `t` + `r` run does that in the model it builds.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| Microsoft Smooth Streaming Protocol, MS-SSTR | client Manifest §2.2.2 (transcription: `private/specs-verified/mpeg/transmux/smooth/ms-sstr.md`) | Client Manifest structure, live attributes, `c` timeline, fragment-URL token substitution | Full for the client manifest | Fragment responses (§2.2.4) are `transmux`'s |

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Links the standard library. Without it the crate is `#![no_std]` and needs only `alloc`. |
| `manifest` | yes | The client Manifest surface: parse and serialize. Needs `std`, because the shared XML module it reads through (`broadcast-common/xml`) is std-only, so it builds no further surface for a bare-metal target. |

## Quick start

Parse the committed fixture ffmpeg's `smoothstreaming` muxer wrote (`fixtures/smooth/av/Manifest`) and render it back:

```rust
use broadcast_smooth::SmoothManifest;

// A committed fixture, resolved at run time (here: the workspace path).
let xml = std::fs::read_to_string(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/smooth/av/Manifest"
))?;
let manifest = SmoothManifest::parse(&xml)?;
for stream in &manifest.streams {
    println!(
        "{}: {} quality level(s)",
        stream.stream_type.name(),
        stream.qualities.len()
    );
}
assert_eq!(SmoothManifest::parse(&manifest.to_xml())?, manifest);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Runnable examples

- `cargo run -p broadcast-smooth --example parse_manifest` parses the committed fixture and prints its streams.
- `cargo run -p broadcast-smooth --example build_manifest` builds a live/DVR manifest with a `Protection` header from the typed model.

## Testing

- Round trip on real data: every manifest under `fixtures/smooth/**` and `fixtures/golden/smooth/*.xml` (the byte goldens recorded from `transmux`'s writer) must serialize and re-parse to an equal model, and the serializer must be a fixed point.
- The `smooth_manifest_parse` fuzz target parses arbitrary input and round-trips every accepted document.

Fixtures are resolved with the workspace `test-fixtures` crate.

## Licence

MIT OR Apache-2.0.
