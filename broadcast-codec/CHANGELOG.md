# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) with the
0.x caveat that the seam is expected to change.

## [Unreleased]

### Added

- Initial contents: `CodecId`, `CodecParameters`, `AudioParameters`,
  `VideoParameters`, `Packet`, `AudioFrame`, `AudioSamples`, `ChannelLayout`,
  `SampleLayout`, `Frame`, `VideoFrame`, `VideoPlane`, `VideoStorage`,
  `PixelFormat`, `ColourDescription`, `CodecError`, the object-safe sans-IO
  `Decoder`/`Encoder` traits, and a `Timescale` re-export so an adapter builds
  `CodecParameters` without a separate `broadcast-core` dependency.
- `PixelFormat` variants for 12-bit 4:2:0/4:2:2/4:4:4 and 10/12-bit monochrome,
  with their plane geometry.
- `ContentLightLevel`, `MasteringDisplay` and `ItuT35` metadata, carried by
  `VideoFrame` through `with_content_light`, `with_mastering_display` and
  `with_itu_t35`.
- `pcm_passthrough` and `drain_contract` examples.
