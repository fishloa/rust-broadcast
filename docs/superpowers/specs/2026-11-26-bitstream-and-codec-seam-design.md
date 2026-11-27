# Bitstream parsers and the codec seam

Status: approved by the owner, 2026-11-26. Follows the streaming-first programme
(`2026-11-24-streaming-first-containers-design.md`, §10 "a future `codec` crate").

## 1. Why

The aim is for these crates to be what a Rust project reaches for where it would otherwise link ffmpeg's
container and packaging side. ffmpeg is five things. We already cover the container, protocol and
packaging part, and go further on broadcast signalling. Two gaps remain between that and "a Rust ffmpeg":

1. **Nothing below `transmux` can read a codec bitstream.** About 16 000 lines of parsers (H.264/HEVC
   parameter sets, AC-3/E-AC-3 and DTS frame headers, AAC configuration, NAL and access-unit handling) live
   inside the muxing hub. `isobmff` cannot tell the resolution of its own `avcC`. `container-probe` and
   `media-doctor` carry their own ADTS and MP3 header parsers because depending on the hub is too heavy.
   An outside project that only wants "parse this SPS" has to take all of `transmux`.
2. **There is no place for a decoder or encoder to attach.** We will never write one. Other projects have
   (`rav1d`, `rav1e`, `symphonia`, `openh264`). Today nothing in our crates could hold one without a
   breaking change to add the place.

This design closes both, before the next release, so the workspace breaks once.

## 2. Goals, non-goals, anti-goals

**Goals**

- A crate, `broadcast-bitstream`, that an outside project can depend on alone to parse codec headers,
  find access-unit boundaries, convert between stream formats, and derive facts (resolution, frame rate,
  RFC 6381 codec string, channel layout).
- A crate, `broadcast-codec`, that defines how a decoder or encoder plugs in: raw frame types and two
  traits. No codec in it.
- Proof, in `transmux`, that a stage which changes a track's codec fits the pipeline.
- Adding transcoding later is additive: new adapter crates, no breaking release of `broadcast-bitstream`,
  `isobmff`, `transmux` or `media-plane`.

**Non-goals**

- Decoding or encoding of any codec in our own code.
- Filters, scaling, resampling (ffmpeg's `libavfilter`, `swscale`, `swresample`).
- A working transcode in this round. The hooks are built; the first real adapter is later work.
- Lossless parse and writers for parameter sets in v1 (roadmap, §8).

**Anti-goals** (would make this a failure even if it works)

- A codec dependency entering the main workspace graph. `symphonia` is MPL-2.0 and `deny.toml` does not
  allow it; the same will be true of others.
- A decode trait inside a crate we intend to freeze, designed without a real adapter behind it.
- A second copy of anything: one bit reader, one ADTS parser, one home per codec.

## 3. Constraints

- `broadcast-bitstream`: `no_std` + `alloc`, depends on `broadcast-core`, `thiserror` and `memchr` (the
  Annex B start-code scan finds each `01` byte rather than testing every offset), optional `serde`,
  optional `isobmff` (§5.4). MSRV as the workspace. Nothing was added to `broadcast-core` for this work:
  the bit extraction it already had was enough, and `broadcast-common` is not a dependency.
- `broadcast-codec`: `no_std` + `alloc`, depends on `broadcast-core` only.
- Neither depends on the other, on `transmux`, or on `broadcast-common`.
- Every existing golden and fixture round trip stays byte-identical. This is a move plus an API tidy; no
  parser's behaviour changes.
- No release and no version change in this work: breaking changes are recorded under each crate's
  `[Unreleased]` and versions move at the release. Both new crates start at `0.1.0`, `publish = false`.
  At the release they are published before any crate that depends on them (`broadcast-core`, then
  `broadcast-bitstream` and `broadcast-codec`, then the rest).
- New crates clear `docs/CRATE-ACCEPTANCE.md`, with one recorded exception (§5.3).
- Project rules apply unchanged: no magic numbers, no `#[allow]`, spec cited by private path, no spec text
  in public files, one function per job, deleted not deprecated.

## 4. Approaches considered

### 4.1 Where the seam lives

| | approach | trade-off |
|---|---|---|
| A | Traits and frame types inside the parser crate | One crate. Any correction to a trait is a breaking release of the crate every adopter uses. |
| **B** | **The seam is its own small crate** | **One more crate. A wrong trait breaks only that crate, which stays 0.x. A decoder from one project can feed an encoder from another, and `broadcast-loudness` can take PCM.** |
| C | No traits: a transcoder is any stage from compressed samples to compressed samples | Nothing to freeze. Cannot combine a decoder and an encoder from different crates; no raw audio for loudness. |

**Chosen: B** (owner, 2026-11-26).

### 4.2 How deep the parsers go in v1

| | approach | trade-off |
|---|---|---|
| **a** | **Facts only, behind an API that can grow** | **What exists today, moved. Fits before the release. The round-trip invariant is waived for this crate for now.** |
| b | Lossless parse and writers for the headers that get rewritten | The project invariant, applied. Large (SPS with VUI, scaling lists, HEVC extensions); would miss the release. |
| c | Everything lossless, slice data included | Decoder territory. Rejected. |

**Chosen: a now, b as the crate's stated roadmap** (owner, 2026-11-26).

## 5. `broadcast-bitstream`

### 5.1 What it holds

Moved out of `transmux/src/`, one feature per codec family, all on by default:

| feature | from `transmux/src/` | what |
|---|---|---|
| (always) | `bitreader.rs`, `rbsp` | exp-Golomb, emulation-prevention removal, on top of `broadcast_core::bits` |
| `h264`, `hevc`, `vvc` | `sps.rs`, `nal.rs`, `annexb.rs`, `au.rs`, `slice_header.rs`, `interlace.rs`, `hdr10plus.rs`, `hdr.rs` | parameter sets, NAL unit types, Annex B and length-prefixed iteration and conversion, access-unit boundaries, slice-header facts, interlace facts, HDR10+ and the HDR10 static metadata |
| `aac` | `aac_asc.rs` | AudioSpecificConfig; ADTS header (the header parser; `container-probe`'s ADTS *prober* stays) |
| `ac3` | `ac3_syncframe.rs` | AC-3 / E-AC-3 syncframe |
| `dts` | `dts_core.rs` | DTS core frame header |
| `mpeg2` | `mpeg_legacy.rs` (sequence half), `h262.rs` | MPEG-2 video sequence header, sequence/display extensions, the picture user-data walk (`mpeg2_display.rs` is a per-access-unit tracker over `transmux`'s IR and stays) |
| `mpeg-audio` | `mpeg_legacy.rs` (audio half) | MPEG-1/2/2.5 audio frame header |
| (always) | `user_data.rs` | AFD and bar data, shared by the H.262, AVC and HEVC paths, so no codec feature gates it |

The features as they exist: `h264`, `hevc`, `vvc`, `aac`, `ac3`, `dts`, `mpegh`, `mpeg2`, `mpeg-audio`,
`isobmff`, `serde` and `std`, with all but `serde`, `isobmff` and `std` in `default`. The per-codec
runtime dependencies (`memchr` for the start-code scan; `thiserror` for the error type) are the same in
every feature combination.

**Which types move.** A type moves if a codec specification defines it: a parameter set and what it
states (picture size and cropping rectangle, VUI colour description, timing information, profile and
level, HDR metadata as coded). A type stays in `transmux` if it is a presentation or container concept
built from those (`DisplayGeometry`, anything in `ir` that a muxer or manifest consumes). The staying type
gets a constructor that takes the bitstream type (`DisplayGeometry::from_avc_sps` keeps its name and takes
the moved `Sps`). Where `ir` today defines a type that is purely a coded fact, that type moves and `ir`
uses it from the new crate; its public path changes, which is a breaking change recorded under
`[Unreleased]`.

The bit reader: the exp-Golomb and RBSP layer is built on `broadcast_core::bits::BitReader`, with the
conversion from `BitError` inside the bitstream crate. `transmux`'s `bitreader.rs` is deleted. If the core
reader lacks something the layer needs, the addition goes into `broadcast-core` as a reviewed edit of its
API pin (additions are allowed there); a second reader is not.

The exact file list is the builder's to establish, starting with a written inventory (§9 step 0): a module that mixes bitstream parsing with `transmux`
IR types is split at that line, and the IR half stays in `transmux`. `nalu_ext.rs`, `codec_config_ext.rs`,
`audio_profile.rs` and `smooth_codec.rs` are the likely seams. `rtp-packet`'s `av3a` is an RTP payload
format and stays where it is.

### 5.2 API rules (what makes later growth non-breaking)

These are the reason approach (a) is safe. They are binding.

- Every parsed type has **private fields and accessor methods**. No public field. A type can then gain the
  rest of its syntax elements, and a writer, without changing shape.
- Every public struct and enum is `#[non_exhaustive]`.
- A parser never returns a transmux type. Where today's code returns IR types (`crate::ir`,
  `crate::pipeline`), the bitstream crate returns its own, and `transmux` converts.
- Its own error type, with `#[non_exhaustive]` variants carrying `what`/`need`/`have` like the rest of the
  workspace. No `transmux::Error` leaks.
- Public spec/field enums follow the label convention (`name()` + `impl_spec_display!`).
- No raw-byte public API: a function that returns "the bytes of the SPS" returns a typed wrapper.
- Limits are the caller's. A parser that must bound work (for example the number of NAL units scanned for a
  boundary) takes the bound from its caller and defaults to unlimited.

### 5.3 The recorded exception

`docs/CRATE-ACCEPTANCE.md` requires a symmetric serializer and byte-exact round trip for every parser. In
v1 that holds only for the types that already have it (`AudioSpecificConfig`, HDR10+). For the rest it is
waived, in writing, in the crate's README and in `CRATE-ACCEPTANCE.md`, with §8 as the plan that removes
the waiver type by type. Real-fixture parse tests, fuzz targets and the spec-drift tests are not waived.

### 5.4 Relationship to `isobmff`

`isobmff` keeps its parameter sets as opaque typed wrappers and gains no dependency. The glue that needs
both crates and nothing else moves to `broadcast-bitstream` behind an optional `isobmff` feature.

More of the glue sits behind that feature than the two configuration builders: `nalu_ext` and
`codec_config_ext` (the trait impls over `isobmff`'s parameter sets and decoder-configuration records),
`avc_config_from_sps_pps` (whose HEVC counterpart turned out not to exist and was written, not moved), the
`dac3`/`dec3`/`ddts` builders (`into_dac3`, `into_dec3`, `into_ddts`,
`ec3_specific_box_from_syncframes`), and `HdrStaticMetadata` with its `mdcv`/`clli` codec. Parsing each
of those codecs needs no `isobmff`; only the box builders do. The exception the crate records for the no-
raw-byte rule is the converters: `annexb_to_length_prefixed`, `length_prefixed_to_annexb`,
`normalise_nal_length_size`, `build_adts_header`, `AudioSpecificConfig::to_adts_header`,
`Hdr10PlusMetadata::to_hevc_prefix_sei_nal` and `hdr::to_hevc_declarative_sei_nal` return bytes because
bytes are the whole of what they produce.

What does not move:
- `avc_config_from_sprop` and `avc_config_from_fmtp` stay in `transmux` (`rtp_sdp.rs`). They are SDP
  interpretation and need base64; they call the moved bytes-in function.
- `rfc6381_codec_string(&CodecConfig)` stays in `transmux`: it takes an IR type. The per-codec helpers under
  it that work from a parameter set or a configuration record (`rfc6381_avc1`, `rfc6381_hvc1`,
  `rfc6381_vvc1`, `rfc6381_mp4a`) move, and it calls them.

`transmux` re-exports nothing that moved: callers import from the new home.

### 5.5 Who changes

- `transmux` depends on `broadcast-bitstream` and deletes the moved modules. Its public re-exports of the
  moved items are deleted, not kept.
- `media-plane`, `multimux`, `broadcast-client`, `smooth-runtime`, `media-doctor` import the items they use
  today (`annexb_to_length_prefixed`, `iter_annexb_nals`, `iter_length_prefixed_nals`,
  `avc_config_from_sps_pps`, `NalCodec`) from the new crate; `avc_config_from_sprop` stays a `transmux`
  import. `NalCodec` is the bitstream crate's own enum and needs no feature.
- `container-probe` depends on `broadcast-bitstream` (features `aac`, `mpeg-audio`) and calls the shared
  header parsers. It keeps `adts.rs` and `mp3.rs`: those files also hold the frame-chain walk, the scoring,
  the ID3v2 skip and a synthetic fixture builder, which are the probe's own evidence, not a header parse.
  Only `adts_frame_len` and `mp3_frame_len` change, to call the shared parser and re-apply the probe's own
  acceptance (six readable ADTS header bytes, MPEG-1 Layer III only).
  This changes a standing rule: `container-probe` was pinned to `broadcast-core` by dev-only drift tests.
  It gains one more runtime dependency, itself `no_std` on `broadcast-core` + `thiserror` + `memchr`, and
  the `transmux` drift test that pinned the two ADTS copies is deleted with the duplicate.
- `media-doctor`'s `codec_common`/`codec_signalling` use the crate where they duplicate it.

## 6. `broadcast-codec` (the seam)

Small on purpose. It holds only what two unrelated crates must agree on.

- `CodecId`: a `#[non_exhaustive]` enum naming a coded format.
- `CodecParameters`: `CodecId` plus the out-of-band configuration a decoder needs or an encoder produces,
  as a typed value, plus the stream facts both sides need (time base; sample rate and channel layout, or
  coded size).
- `Packet`: one coded access unit: bytes, presentation and decode time, key-frame flag.
- `AudioFrame`, `VideoFrame`: decoded data. Format enums (`SampleFormat`, `PixelFormat`) are
  `#[non_exhaustive]`. Frame storage is a `#[non_exhaustive]` enum with one variant today (planes in CPU
  memory, with strides), so a hardware surface can be added later.
- `Decoder` and `Encoder` traits, sans-IO and object-safe, in the send/receive shape the real codec
  libraries share: push one input, pull zero or more outputs, signal end of stream, drain. No threads, no
  clock, no allocation policy imposed. An encoder reports its `CodecParameters` once known.
- Errors: a `#[non_exhaustive]` enum that separates "need more input", "this input is refused" and "the
  codec failed" so a pipeline can react without knowing the codec.

It has no dependency on `broadcast-bitstream` or `transmux`. Time values use `broadcast_core::timescale`.
`transmux::payload::PayloadDecoder` already exists for timed-metadata payloads; the names must not collide
in documentation, and that trait is not replaced.

**The field lists are not fixed here.** They are settled by the spike (§7.3) and then written into the
crate. That is the point of building the spike in the same round.

## 7. Hooks in the pipeline

### 7.1 The property

Between a demux stage and a mux stage, a stage may: replace a track's `TrackSpec` (a different codec and
configuration), emit a different number of samples than it took, delay output by any number of inputs, and
flush at end of stream. Every consumer downstream (muxers, segmenters, manifests, `media-plane`'s trunk)
must accept that without special knowledge.

The existing events are believed sufficient (`TrackAdded`/`TrackUpdated`/`TrackRemoved`, samples carrying
their own timing). The builder's first task is to establish, with a test and not by reading, whether a
caller can insert such a stage today in each of: the `transmux` CLI pipeline, `Repackage`, and
`media-plane`'s ingest path. Where there is no insertion point, one is added, shaped like the neighbouring
stage plumbing. Where a consumer assumes "codec never changes" or "one sample in, one out", it is fixed.

### 7.2 The transcode stage

`transmux` gains an optional feature `codec` (off by default, not part of `cli`) that depends on
`broadcast-codec` and provides one stage: it takes a boxed `Decoder` and a boxed `Encoder` for a track,
maps samples to `Packet`s and back, and rewrites the track's `TrackSpec` from the encoder's
`CodecParameters`. `multimux` and `transmux` stay thin: the stage holds no codec knowledge.

A codec change is a property of the whole track, not an event inside it. The stage holds its input until
the encoder has reported its parameters, then announces the track once with the new `TrackSpec`, before
its first sample. Downstream stages never see a codec change inside a segment, so segment-boundary and
init-segment logic is not rewritten. A source `TrackUpdated` is passed to the decoder; whether it leads to
a new announcement downstream is the encoder's parameters changing, which is the existing `TrackUpdated`
path. "Nothing lost at flush" means: every sample the fake decoder was given is accounted for in the fake
encoder's output count at `finish`.

An insertion point added to the CLI, `Repackage` or `media-plane` is a public API addition, recorded under
`[Unreleased]`; no version changes in this work (§3).

It is tested with a **fake codec pair** defined in the test: a decoder and encoder that hold three inputs
before emitting, emit two outputs for some inputs, and report different parameters. The test drives real
fixtures through demux → transcode stage → mux for TS, CMAF and `LlHlsSegmenter`, and once through
`media-plane`'s `ProgramSegmenter` into a real `Trunk`, and asserts the
output's track describes the new parameters, timestamps are monotonic and nothing is lost at flush.

### 7.3 The spike adapter

One real adapter, to hit the traits with a real library before anyone depends on them: AAC decoded by
`symphonia` into `AudioFrame`s, fed to `broadcast-loudness`, measuring a real fixture's integrated loudness
and comparing it with the value an independent tool reports for the same file.

- The reference is ffmpeg's `ebur128` filter on the same file; the two integrated-loudness values agree
  within 0.1 LU. The fixture is a committed, permissively licensed AAC file already in `fixtures/`; the
  fixture licence rule applies to the spike as to any test.
- `AudioFrame`'s sample formats include whatever `broadcast-loudness` takes today, so the spike needs no
  conversion that is not also a real adapter's job.
- `symphonia` gets a `dep_policy` purpose entry naming it as the spike's audio decoder, scoped to the side
  workspace; `deny.toml` covers the main workspace only and is unchanged.
- It lives in its own side workspace (like `demo/`), with its own lockfile, `publish = false`, outside the
  main workspace and outside `deny.toml`'s graph.
- It is allowed to drive changes to `broadcast-codec`'s types. Each change it forces is listed in the PR.
- Audio only. Video frame types are reviewed against `rav1d`'s and `openh264`'s public frame types on
  paper (a table in the PR), not exercised.
- Whether it stays in the repository as an example or is removed after the round is the owner's call at
  review.

### 7.4 The libav adapter and its wall

A second spike wraps FFmpeg's libavcodec (`spikes/codec-libav/`, through the `ffmpeg-next` binding), so any
decoder or encoder libav has can sit behind the `broadcast-codec` traits. It is the real test of the video
frame types, and with the fake pair it drives the transcode stage of §7.2. Its tests skip loudly when the
system has no libav.

FFmpeg is LGPL, and GPL in builds that enable GPL parts. The adapter's own source is ours and carries the
workspace licence; the obligations fall on whoever distributes a binary linked against libav. The rest of
the workspace is kept clear of it by construction:

1. The dependency runs one way: the adapter depends on `broadcast-codec`. No crate of ours depends on the
   adapter, not as an optional feature either.
2. `dep_policy` and `deny.toml` ban every FFmpeg binding and the adapter's own name from the main workspace
   graph. The ban was added before the adapter existed.
3. No FFmpeg-derived text is committed: bindings are generated at build time by the binding crate; no
   vendored headers, no copied code.
4. Dynamic linking only. The adapter offers no static or build-from-source feature.
5. Binaries we ship (`transmux`, `multimux`, `media-doctor`, any prebuilt artefact) never link it. A
   libav-enabled tool is a separate binary its user builds.
6. The adapter's README states that it links FFmpeg, that the terms of the user's FFmpeg build apply to
   what they distribute, and that codec patent licences are the distributor's matter.
7. When it is released it moves to its own repository; the spike stays under `spikes/` until then.

This is engineering practice, not legal advice.

## 8. Roadmap for `broadcast-bitstream` (after this round)

Lossless parse plus a writer, byte-exact on real fixtures, one type at a time, each with its spec
transcription in the private repo first: H.264 SPS and PPS; HEVC VPS, SPS and PPS; AC-3 and E-AC-3
syncframe header; DTS core header; AV1 sequence header (new: there is no AV1 bitstream parser today);
MPEG-2 sequence headers. Slice headers and SEI stay read-only. Each lands as an additive release.

## 9. Order of work

0. **Inventory.** No code. For every candidate module: what it imports from `transmux`, which of its
   types move and which stay under the §5.1 rule, which tests construct IR types and so stay in
   `transmux`. The owner sees the size before step 1 starts. A reviewer found the coupling deeper than a
   move: `sps.rs` (4 376 lines) builds IR geometry in its tests, and every candidate uses `transmux`'s
   error type.
1. **Extract `broadcast-bitstream`**, one PR per codec family, in this order: the bit reader and NAL/Annex
   B layer; H.264 and HEVC; AAC; AC-3 and DTS; MPEG-2 and MPEG audio with the `container-probe` change.
   Each PR applies the §5.2 rules, moves its tests and fuzz targets, migrates its consumers, and keeps
   goldens byte-identical. Guard allowlist entries that are pinned to a moved line (`magic_numbers`,
   `panic_idioms`, `truncation`, `serialize_mask`) move with the code, re-pinned, each keeping its reason.
2. **`broadcast-codec` and the spike**, together. Independent of step 1.
3. **Pipeline property and the transcode stage** in `transmux`. After step 2.

Steps 1 and 2 can run side by side. All three land before the release.

## 10. Verification

- Every existing test of a moved parser moves with it and passes unchanged; pass counts before and after.
- Goldens and fixture round trips byte-identical; `Cargo.lock` gains only the two new packages.
- `cargo tree -p broadcast-bitstream --no-default-features` shows `broadcast-core` and `thiserror` only;
  each codec feature builds alone (`tools/check-bare-features.sh`).
- `cargo tree -p broadcast-codec` shows `broadcast-core` only.
- A `workspace-guards` entry bans the moved names from coming back in `transmux` (`deleted_names`), bans a
  second exp-Golomb reader or ADTS header parser outside the crate (a de-hand-roll needle), and checks that
  no crate in the main workspace depends on a codec library.
- Public-field check: a guard fails if `broadcast-bitstream` or `broadcast-codec` declares a `pub` field on
  a public struct.
- The fake-codec pipeline test (§7.2) and the spike's loudness comparison (§7.3).
- Fuzz: one target per parser family in `broadcast-bitstream`, moved or new.

## 11. Left to the builder

- The exact module split inside `transmux` and the internal module layout of both crates.
- Accessor naming, within `docs/API-CONVENTIONS.md`.
- The field lists of the `broadcast-codec` types, as the spike settles them.
- Whether the transcode stage takes boxed trait objects or is generic, if the spike shows a reason.
- Which consumer cluster migrates first.

## 12. Decisions recorded

Owner (2026-11-26):
- We depend on other people's codec work; we never write a decoder or encoder.
- Transcoding is possible later; the hooks are built now so adding it is not a breaking change.
- The seam is its own crate (approach B).
- Parsers are facts-only in v1 behind a growth-safe API; lossless parse and writers are the roadmap.
- Names: `broadcast-bitstream` (parsers and stream-format conversion) and `broadcast-codec` (the seam).
  Both free on crates.io on 2026-11-26.
- `container-probe` takes `broadcast-bitstream` as a second runtime dependency, loses its own ADTS and
  MPEG audio header parses (keeping the prober files themselves) and the drift test that pinned them.

Architect's calls, accepted by the owner with the names:
- One bitstream crate with a feature per codec, not a crate per codec.
- One bit reader; `transmux`'s private one merges into the bitstream crate.
- `container-probe` loses its duplicate header parses; `media-doctor` turned out to have none (it already
  called the shared parser, so only its imports changed).
- `isobmff` stays independent; the glue sits in `broadcast-bitstream` behind an optional feature.
- Adapters are separate crates outside the main workspace graph, never features of the core crates.
- Both crates land before the release; `broadcast-codec` ships at 0.x and is expected to change.

## 13. As built (2026-11-27)

The extraction landed as one PR per codec family; the crate is `publish = false` at `0.1.0` until the
release that follows this work.

| PR | what moved |
|---|---|
| #4164 | the bit reader, RBSP, NAL classification, Annex B conversion and the access-unit splitter |
| #4169 | the SPS decoders, the coded picture facts and the per-codec RFC 6381 helpers |
| #4174 | the slice-header layer, the interlace facts, the HDR10+ model, the HDR10 static metadata and the `isobmff` glue |
| #4177 | AAC/ADTS, AC-3/E-AC-3, DTS core and MHAS framing |
| the PR after #4177 | MPEG-2 video and MPEG audio, AFD and bar data, and the `container-probe` change |

Where the code differs from the design above:

- `memchr` is a dependency of `broadcast-bitstream` (the start-code scan), so the crate has four runtime
  dependencies, not three.
- More of the ISOBMFF glue sits behind the `isobmff` feature than §5.4 lists (see §5.4 above).
- `container-probe` keeps `adts.rs` and `mp3.rs` and loses only the two header parses (§5.5 above).
- The converters that return bytes are a recorded exception to the no-raw-byte API rule (§5.4 above).
- Nothing was added to `broadcast-core`: its bit extraction was enough, and `broadcast-common` is not a
  dependency of the crate at all.
- Two things the design did not name: `PictureUserData` is `#[non_exhaustive]` with private fields and a
  constructor, and the MPEG-2 sequence geometry gained `frame_rate_code()`/`frame_rate()` (Table 6-4) so a
  consumer can state a frame rate from the same header rather than inferring it from sample timing.

### Step 3, as built (2026-11-27)

The transcode stage and the pipeline property landed in one PR. Where the
properties above met the code:

- **The mux vocabulary had no track removal.** `DemuxEvent::TrackRemoved`
  existed; `MuxInput` did not name it, and the CLI dropped the event. It is now
  `MuxInput::TrackRemoved` (additive: the enum is `#[non_exhaustive]`). The MPEG-2
  TS muxer re-signals its PMT without the stream; every muxer whose output has a
  fixed track set (CMAF/LL-HLS/LL-DASH segmenters, progressive MP4, Matroska,
  Smooth, DASH, repackaging, RTP, FLV/RTMP) refuses it with a typed error rather
  than describing a track it will never see again.
- **A delayed announcement needed a seam of its own.** Every muxer builds its
  init segment, PMT or file header from the announced set at its first sample,
  and a segmenter refuses a `Track` after construction. A stage that announces a
  track only once its encoder reports parameters therefore cannot feed a muxer
  directly. `mux_stage::MuxChain` is the seam: it drives any
  `Stage<In = MuxInput, Out = MuxInput, Error = Error>`, holds its output until
  every track the upstream announced has been re-announced or removed, and
  releases the held *track announcements first*, then the samples in arrival
  order — a stage that delays one track's samples otherwise puts another
  track's samples ahead of that track's announcement, which no muxer accepts.
  The pre-settlement window is the caller's (`MuxChain::hold_limit`, unlimited
  by default).
- **Three assemblies, three answers.** `Repackage` had no insertion point:
  `Repackage::run_transformed(fmp4, transform)` adds one with the transform per
  call, so a configured `Repackage` stays `Clone`. The CLI's stages are private
  and its mux stage is built from the demuxer's specs before any sample, so it
  gained `cli::run_bytes_with(input, opts, transform)`, which builds the mux
  stage from the chain's settled set; a progressive output replays the input
  against a recorded plan and refuses a transform. `media-plane`'s ingest path
  needed nothing: an `IngestSession` is a `Stage`, so a caller wraps it, and
  the trunk and `ProgramSegmenter` already rebuild on a track-set change
  (`TracksChanged`); both are proved by a test with a delayed, non-1:1 stage.
- **The seam cannot rebuild a decoder mid-track.** The design's "a source
  `TrackUpdated` goes to the decoder" has no method to go through:
  `Decoder::parameters` is fixed at construction and `reset` only drops held
  state. The stage absorbs a re-announcement that restates what the decoder was
  built for and refuses one that changes it (`Error::TranscodeSourceChanged`),
  naming why, rather than decoding with the wrong configuration. A later
  addition to the seam (a parameter-update method, or a factory the stage can
  call) is additive.
- **The reverse conversion is total, not complete.** The seam carries no MPEG
  audio layer, no DTS sample-entry type, no VP8/Vorbis identity and no
  container provenance (`source_pid`, `program_number`, `es_info_descriptors`),
  so `track_spec_from_codec_parameters` returns
  `Error::TranscodeUnrepresentable` naming the missing fact for those, and a
  forward conversion drops provenance. `CodecProfile` is not carried either:
  the seam's profile key is the caller's to state.
- **The spikes.** `spikes/codec-oxideav` offers no encoder at all.
  `spikes/codec-libav` completes the whole loop for video (H.264 decoded,
  MPEG-2 encoded, muxed to TS, demuxed, decoded back to the fixture's own
  picture count). Its audio half stops at the binding: `ffmpeg-next` 8.1
  exposes no extradata getter, so the adapter's encoder reports no
  `AudioSpecificConfig` and a re-muxed AAC track cannot be decoded again. The
  test drives the whole pipeline, checks the packets and the announced track,
  and records this rather than pretending.
