# Bitstream parsers and the codec seam

Status: design, awaiting owner approval. Follows the streaming-first programme
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

- `broadcast-bitstream`: `no_std` + `alloc`, depends on `broadcast-core` only (plus `thiserror`), optional
  `serde`, optional `isobmff` (§5.4). MSRV as the workspace.
- `broadcast-codec`: `no_std` + `alloc`, depends on `broadcast-core` only.
- Neither depends on the other, on `transmux`, or on `broadcast-common`.
- Every existing golden and fixture round trip stays byte-identical. This is a move plus an API tidy; no
  parser's behaviour changes.
- No release and no version change in this work. Both crates start at `0.1.0`, `publish = false` until the
  owner calls the release.
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
| `h264`, `hevc` | `sps.rs`, `nal.rs`, `annexb.rs`, `au.rs`, `slice_header.rs`, `interlace.rs`, `hdr10plus.rs` | parameter sets, NAL unit types, Annex B and length-prefixed iteration and conversion, access-unit boundaries, slice-header facts, HDR10+ |
| `aac` | `aac_asc.rs` | AudioSpecificConfig; ADTS header (from `container-probe`) |
| `ac3` | `ac3_syncframe.rs` | AC-3 / E-AC-3 syncframe |
| `dts` | `dts_core.rs` | DTS core frame header |
| `mpeg2` | `mpeg2_display.rs`, `user_data.rs`, `h262` | MPEG-2 video sequence/display, picture user data |
| `mpeg-audio` | (from `container-probe`'s `mp3.rs`) | MPEG audio frame header |

The exact file list is the builder's to establish: a module that mixes bitstream parsing with `transmux`
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
both (build an `avcC`/`hvcC` from parameter sets, `avc_config_from_sps_pps`, `avc_config_from_sprop`, an
RFC 6381 string from a configuration record) moves to `broadcast-bitstream` behind an optional `isobmff`
feature. `transmux` keeps re-exporting nothing: callers import from the new home.

### 5.5 Who changes

- `transmux` depends on `broadcast-bitstream` and deletes the moved modules. Its public re-exports of the
  moved items are deleted, not kept.
- `media-plane`, `multimux`, `broadcast-client`, `smooth-runtime`, `media-doctor` import the items they use
  today (`annexb_to_length_prefixed`, `iter_annexb_nals`, `iter_length_prefixed_nals`,
  `avc_config_from_sps_pps`, `avc_config_from_sprop`, `NalCodec`) from the new crate.
- `container-probe` depends on `broadcast-bitstream` (features `aac`, `mpeg-audio`) and deletes `adts.rs`
  and `mp3.rs`. Its scoring logic and frame-chain walk stay; only the header parse moves.
  This changes a standing rule: `container-probe` was kept to one runtime dependency and pinned to its
  siblings by dev-only drift tests. It gains one more runtime dependency, itself `no_std` and on
  `broadcast-core` only, and the ADTS/MP3 drift tests it no longer needs are deleted.
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

It is tested with a **fake codec pair** defined in the test: a decoder and encoder that hold three inputs
before emitting, emit two outputs for some inputs, and report different parameters. The test drives real
fixtures through demux → transcode stage → mux for at least TS, CMAF and one HLS segmenter, and asserts the
output's track describes the new parameters, timestamps are monotonic and nothing is lost at flush.

### 7.3 The spike adapter

One real adapter, to hit the traits with a real library before anyone depends on them: AAC decoded by
`symphonia` into `AudioFrame`s, fed to `broadcast-loudness`, measuring a real fixture's integrated loudness
and comparing it with the value an independent tool reports for the same file.

- It lives in its own side workspace (like `demo/`), with its own lockfile, `publish = false`, outside the
  main workspace and outside `deny.toml`'s graph.
- It is allowed to drive changes to `broadcast-codec`'s types. Each change it forces is listed in the PR.
- Audio only. Video frame types are reviewed against `rav1d`'s and `openh264`'s public frame types on
  paper (a table in the PR), not exercised.
- Whether it stays in the repository as an example or is removed after the round is the owner's call at
  review.

## 8. Roadmap for `broadcast-bitstream` (after this round)

Lossless parse plus a writer, byte-exact on real fixtures, one type at a time, each with its spec
transcription in the private repo first: H.264 SPS and PPS; HEVC VPS, SPS and PPS; AC-3 and E-AC-3
syncframe header; DTS core header; AV1 sequence header (new: there is no AV1 bitstream parser today);
MPEG-2 sequence headers. Slice headers and SEI stay read-only. Each lands as an additive release.

## 9. Order of work

1. **Extract `broadcast-bitstream`.** Untangle from `transmux` IR, merge the bit reader, apply the §5.2
   rules, move tests and fuzz targets, migrate consumers, remove `container-probe`'s and `media-doctor`'s
   duplicates. Crate-cluster by crate-cluster, goldens byte-identical throughout.
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

Architect's calls, accepted by the owner with the names:
- One bitstream crate with a feature per codec, not a crate per codec.
- One bit reader; `transmux`'s private one merges into the bitstream crate.
- `container-probe` and `media-doctor` lose their duplicate header parsers.
- `isobmff` stays independent; the glue sits in `broadcast-bitstream` behind an optional feature.
- Adapters are separate crates outside the main workspace graph, never features of the core crates.
- Both crates land before the release; `broadcast-codec` ships at 0.x and is expected to change.
