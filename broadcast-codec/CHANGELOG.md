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
  `PixelFormat`, `ColourDescription`, `CodecError`, and the object-safe
  sans-IO `Decoder`/`Encoder` traits.
- `pcm_passthrough` and `drain_contract` examples.
