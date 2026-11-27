# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) with the
0.x caveat that the seam is expected to change.

## [Unreleased]

### Added

- `Decoder::reconfigure`, which adopts new `CodecParameters` mid-stream after
  the caller has drained: a decoder that still holds a frame refuses with
  `CodecError::Full`, and one that cannot adopt the new parameters returns
  `CodecError::CannotReconfigure` so the caller can build a fresh decoder. The
  default implementation is that refusal, so existing implementations keep
  compiling. `CodecError` gains the `CannotReconfigure` variant with a
  `cannot_reconfigure` constructor, and `TrimStage`'s wrapper forwards the call
  and rebuilds its trim from the new parameters.
- `CodecParameters::configuration_record` and `ConfigurationRecord`
  (`Absent`/`OutOfBand`/`InBand`), with `with_in_band_configuration`: an
  encoder that writes its configuration record inside its first access unit
  says so instead of reporting empty extradata. `with_extradata` now states
  `OutOfBand`. The per-`CodecId` form of the record is documented on
  `CodecParameters` and on `Encoder::parameters`.
- `CodecParameters::initial_padding`/`with_initial_padding` and
  `trailing_padding`/`with_trailing_padding`: a lossy audio stream's gapless
  trim, in samples per channel. `initial_padding` is 0 when there is none or the
  container does not say; `trailing_padding` is `None` when the container does
  not say. Both builders refuse video parameters with `PaddingError::Video`.
- `Packet::trim_start`/`trim_end` and `AudioFrame::trim_start`/`trim_end` with
  their builders and accessors: per-packet gapless trim, and its provenance on
  the frame the packet decoded to.
- `CodecDescriptor::with_trims`/`applies_trim`: whether an implementation
  applies the container's trim itself. A decoder returns every decoded sample,
  untrimmed, unless this is true; the rule is stated at the crate root and on
  `Decoder::receive`.
- `TrimStage`, which applies a stream's initial and trailing padding and each
  frame's own trim once, for every adapter and caller; it adjusts each frame's
  timestamp by the samples removed from its front, drops frames that become
  empty, and holds back at most the trailing padding plus one frame until
  `finish`. `Registry::trimmed_decoder` returns the selected decoder unchanged
  when its descriptor trims, and otherwise wraps it so `receive` goes through a
  `TrimStage`.
- `EncoderConfig::pixel_format`/`with_pixel_format` and
  `sample_format`/`sample_layout`/`with_sample_format`: the storage a caller
  asks an encoder to produce (a picture's pixel format, or an audio frame's
  sample format and layout). `None` leaves the choice to the provider, which
  states the format it produces.
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
- `CodecProfile`, the typed codec-variant key the stream and the implementation
  share: H.264, HEVC, VVC, AV1, VP9, MPEG-2 video and AAC profiles named as
  their specifications name them, plus `Other { codec, tag }`. `codec()` says
  which codec a profile belongs to, `token()`/`from_token` are the text form
  (`h264/high10`, `aac/lc`, `other:00000007/deadbeef`) and `profile_token()` is
  the profile half for a table that already names the codec. No profile
  identifier number is in this crate: mapping one to a variant is the bitstream
  parser's and the adapter's job.
- `CodecParameters::profile`/`with_profile` and
  `EncoderConfig::profile`/`with_profile`: the profile a stream is in and the
  profile an encoder is asked to produce, refused with `ProfileError` when the
  profile belongs to another codec.
- `CodecProvider::profiles` (defaulting to the union over the provider's
  descriptors, first-seen order, no duplicates) and `Registry::profiles`, which
  attributes each profile to the providers that state it.
- `Registry::candidates_for` and `Registry::encoder_candidates_for`, which
  apply the new profile clause, and `Registry::rejected_for` and
  `Registry::encoder_rejected_for`, which report each removed candidate and why
  (`ProfileRejection`).
- The preference text form gained an optional profile: `h264/high10=libav`,
  `dec:aac/he=oxideav:fdk`. `Preference::with_profile` builds one, and
  `PreferenceError::UnknownProfile` names a profile token that names none of
  the codec's profiles. A profiled preference applies only to streams of that
  profile and outranks one for the bare codec.

### Breaking

- `CodecDescriptor::profiles` is `&[CodecProfile]` rather than `&[String]`, and
  `with_profiles` takes an iterator and returns `Result<Self, ProfileError>`, so
  a descriptor cannot state a profile another codec owns. Empty still means
  "not stated", not "all".
- The selection rule gained a clause (now rule 4, ahead of the fallback rules):
  a candidate that states profiles without the stream's profile is removed, and
  the remaining rules number 5 to 7. `Registry::candidates` keeps its signature
  and reports the order alone; `candidates_for`/`encoder_candidates_for` apply
  the clause. A selector or preference that picks a removed candidate is an
  error naming it, never a silent switch to another.
- `Registry::prefer` replaces an earlier preference for the same codec, profile
  and direction, so a profiled preference and a bare one for the same codec now
  coexist instead of replacing each other.
- `CodecProvider` gained a required `info` method: every adapter must say who
  it is and under what terms. `CodecProvider::probe` has a default answer of
  `Availability::Unknown`, so probing is opt-in.
- `CodecError`'s message-bearing payloads are `Cow<'static, str>` rather than
  `&'static str`, so the registry can name the provider, implementation or
  codec it refused at run time. `CodecError` is no longer `Copy`; the
  `refused`/`failed`/`unsupported` constructors take `impl Into<Cow<'static,
  str>>` and are no longer `const`.
