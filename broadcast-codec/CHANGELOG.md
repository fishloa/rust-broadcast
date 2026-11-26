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
- `pcm_passthrough`, `drain_contract` and `registry` examples.
- Providers and the registry: `CodecProvider` (the object-safe trait one
  adapter implements), `CodecDescriptor`, `CodecId`, `Direction`,
  `SampleFormat`, `EncoderConfig` (typed settings plus ordered
  provider-specific options), `Registry` (a value the application builds, with
  `prefer`, `fallback` and `select_with`), `Preference` applied to one or both
  directions, `Fallback::{Strict, NextCandidate}`, and the text form
  `Preference: FromStr`/`Display` with `parse_preferences` for a
  comma-separated list. The whole selection rule is stated at the crate root
  and pinned by one test per clause.
- `CodecId::from_name`, the textual inverse of `CodecId::name`: `h264` and
  `H.264` name the same codec.
- A run-time inventory: `ProviderInfo` (the adapter's name and version, the
  underlying library and its version, the library's licence as the library or
  its manifest states it, and free-text notes) returned by
  `CodecProvider::info`, and `Registry::providers` for every provider in
  registration order; `CodecDescriptor::profiles` for the codec variants an
  implementation handles; `Availability` and `Probe` for whether an
  implementation opens here, `CodecProvider::probe` (defaulting to unknown)
  and `Registry::inventory`, which never probes unless asked; `InventoryEntry`
  with its one-line `Display`; and `render_inventory`, which renders a whole
  inventory as a plain table. The `registry` example prints it.
- `CodecId::Other` now has a text form, `other:<8 hex digits>` (lower case),
  accepted by `CodecId::from_name` and written by the preference text form, so
  a format the enum does not name can be preferred and listed. `other` alone is
  still an unknown codec.

### Breaking

- `CodecProvider` gained a required `info` method: every adapter must say who
  it is and under what terms. `CodecProvider::probe` has a default answer of
  `Availability::Unknown`, so probing is opt-in.
- `CodecError`'s message-bearing payloads are `Cow<'static, str>` rather than
  `&'static str`, so the registry can name the provider, implementation or
  codec it refused at run time. `CodecError` is no longer `Copy`; the
  `refused`/`failed`/`unsupported` constructors take `impl Into<Cow<'static,
  str>>` and are no longer `const`.
