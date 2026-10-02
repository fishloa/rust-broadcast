# Changelog

All notable changes to `transmux` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `CencDecryptor::from_fmp4_bytes` — `from_fmp4` over an already-shared `bytes::Bytes`, so the file is
  not copied and `demux` hands out samples as slices of it (#1081, r05-O2).
- `AudioSpecificConfig::effective_sampling_frequency` — the explicit rate when present, else the
  `samplingFrequencyIndex` rate (#1081, r04-O5).
- `impl Serialize for SampleEntryVariant` (#1081, r05-O3).
- `rfc6381_codec_string(&CodecConfig)` — the RFC 6381 codec string `DashPackager`
  writes into `Representation@codecs`, exposed so an HLS origin can fill
  `#EXT-X-STREAM-INF` `CODECS` (#1089).
- `smooth::SmoothPackager::package_track_fragment` — build **one** Smooth
  fragment for a single track at an explicit smooth-timeline start (a
  `SmoothFragment` whose `tfxd` `FragmentAbsoluteTime` is the caller's value
  and whose duration is the track's own summed `trun` durations), without
  segmenting the track. Needed by `multimux`'s live Smooth output, whose
  per-track `c@t` timeline must match the served fragment's `tfxd` and must be
  stable as the window slides (#1083).

- **`init_segment::sampling_rate_override`** (#1081, audit r05-W31) — reads the
  `srat` value out of an audio sample entry's config children, returning `None`
  when the box is absent, malformed, or declares `sampling_rate == 0` (a zero
  rate is not a rate; honouring it would replace a good 16.16 field with "0
  Hz"). `SamplingRateBox::parse` now also validates the four-CC.
- **`init_segment::SamplingRateBox` and `init_segment::AudioSampleEntryV1`**
  (#1081, audit r05-W31). `SamplingRateBox` is the `srat` FullBox
  (ISO/IEC 14496-12:2015 §12.2.3.1) with `new`/`FOURCC`/`SIZE` and the
  `Parse`/`Serialize` pair; `AudioSampleEntryV1` names the v1 sound-entry form
  (§12.2.3.2 as amended by Amd 1:2017) via `ENTRY_VERSION`,
  `STSD_VERSION`, `SAMPLERATE_PLACEHOLDER` and `rate_fits_v0`. Also new:
  `SampleEntryVariant::required_stsd_version()`, which reports the `stsd`
  version an entry demands (1 for an `AudioSampleEntryV1`, else 0).
- `sample_aes::eac3_encrypt_frame` / `eac3_decrypt_frame` — Sample-AES for a
  **multi-syncframe** E-AC-3 audio frame, and `ac3::split_eac3_syncframe_ranges`
  (#1080, audit r05-W2). The protected block is a single syncframe, so each gets
  its own 16-byte clear leader, whole 16-byte blocks are encrypted and a partial
  tail is clear (`ac3_encrypt_frame` applies one leader over whatever slice it is
  handed, which skips the later leaders in a multi-syncframe frame). The IV is
  **reset at every syncframe**, per the independent oracle
  (`transmux/tests/fixtures/sample_aes_eac3/`; ffmpeg decodes the reset form 9/9
  audio frames and a carried chain 1/9, and Bento4's encryptor is byte-identical
  to the reset reference). Both return `Result`: a payload that is not a clean
  run of syncframes (no sync word, or a truncated / trailing-junk syncframe) is
  `Error::InvalidInput` rather than being emitted unchanged or half-encrypted.
  Carrying the IV across an *independent + dependent* syncframe pair has no
  independent oracle (`ORACLES.md` B3) — that case resets too and is labelled
  unverified.
- **`uri` — RFC 3986 URI-reference parsing and resolution.** `UriReference::parse`
  (§3) and `to_uri_string` (§5.3), `resolve` (§5.2.2), `merge` (§5.2.3),
  `remove_dot_segments` (§5.2.4) and `resolve_segment` for a `BaseURL` chain.
  `try_resolve`/`try_resolve_segment` are the same with a base or reference
  containing a control character or whitespace **rejected** (`None`), since a
  raw CR/LF in a URL has no meaning in RFC 3986 and lets a manifest smuggle a
  second request line into anything that later writes an HTTP request from it;
  `first_forbidden_char` is the predicate. Also exports the standard's own
  §5.4.1 and §5.4.2 example tables (`RFC3986_NORMAL_EXAMPLES`,
  `RFC3986_ABNORMAL_EXAMPLES`) as data, which the crate's tests assert against.
  This is what `dash_parse::Mpd::resolve_segment_url` /
  `try_resolve_segment_url` resolve through (#1079, audit r04-W11). The crate
  root re-exports `resolve_uri_reference`, `resolve_uri_segment` and
  `try_resolve_uri_reference`.
- `ac3::split_ac3_syncframes_resyncing` and `ac3::split_ac3_syncframe_ranges` —
  frame splitting that resynchronises after an unparseable frame, returning byte
  ranges rather than slices (#1079, audit r04-W20).
- `ts_hls::StreamingTsHlsSegmenter::with_start_sequence` — seeds a streaming classic-TS
  segmenter's segment numbering from a caller-given value instead of `Self::new`'s implicit `0`,
  the classic-TS analogue of `ll_hls::LlHlsSegmenter::with_part_target_at`. Lets a consumer
  (`multimux::source::segment::ProgramSegmenter`) resume numbering across a rebuild rather than
  renumbering from `0` against a `Trunk` that already holds segments, which its monotonic
  `sequence_number` guard rejects forever after.

### Changed

Optimization sweep (#1079, #1080, #1081). Apart from the three behaviour changes listed under
`### Changed (breaking)` and `### Fixed`, every change below leaves the output byte-identical on the
committed fixtures (pinned by `tests/sweep_output_golden.rs`, whose hashes were captured on the
pre-sweep code) and is backed by a deterministic allocation or work counter
(`tests/alloc_counts_sweep.rs` and in-module counters), never a timing.

- `Segmenter` no longer rebuilds the whole `moov` on every cut to detect an init change that its API
  cannot produce; the dead detection is deleted (109 -> 18 allocations per cut) (#1081, r05-O5).
- `hvcC` parsing bounds its array/NAL pre-allocation by the body length (1 581 000 -> 32 bytes for a
  hostile header) (#1079, r04-O4).
- `AudioSpecificConfig::heaac_signaling`/`rfc6381` no longer allocate per call (#1079, r04-O2).
- Duplicates removed (#1079, r04-O1/O5, #1081, r05-O6): `ps_demux`'s copy of the Annex B start-code
  scanner (now `annexb::start_code_positions`) and its copy of the first-NAL-start finder (now
  `au::first_nal_start`); the three per-module bit readers (now one `read_bits_at`); the avcC/hvcC
  byte cursors; the sfi -> Hz table; the esds ASC accessor; the XML writer; the sample-flag constants;
  the `tfhd`/`trun` builder. The streaming splitter's resumable scan in `au` and
  `mpeg_legacy::find_start_code` answer different questions and stay separate.
- `PsDemux` keeps access-unit ranges instead of per-unit copies (MPEG-2 video 3.70x -> 3.05x of input
  bytes allocated); `TsDemux` hands a live track the PES payload by reference (10.08x -> 9.44x);
  `read_chunks` moves the per-csid context instead of cloning its partial message (468 750 464 -> 524 544
  bytes for a 200 kB message); the RTP depacketiser moves each packet into the AU buffer (7.47x ->
  6.47x); `WebmDemux` partitions blocks by track once (8000 -> 1000 block visits for 8 tracks x 1000
  blocks) (#1079, #1080, r04-O6/O7/O9/O10).
- `CencDecryptor` shares the file buffer and slices samples out of it (2.14x -> 1.21x of the file size;
  0.21x through the new `from_fmp4_bytes`), and its `stsc` expansion is one forward walk (80 599 -> 799
  entry examinations for 400 chunks) (#1081, r05-O1/O2).
- `ProgressiveMux` (4.12x -> 2.20x), `MkvMux` (11.45x -> 2.93x) and `TsMux` (7.70x -> 5.40x of the
  output size allocated) write into pre-sized output buffers (#1081, r05-O4).
- `Serialize` is implemented once on `SampleEntryVariant`, replacing four 17-arm matches (#1081, r05-O3).
- `cargo test -p transmux --no-default-features` builds again (the in-crate tests link `std`) (#1079).

Intentionally not done: `BitReader::from_rbsp` borrowing (a public lifetime change for one allocation
per SPS), the three merge-by-decode-time loops (their tie-break rules differ), and a typed-parser
rewrite of `cenc_decrypt`'s remaining box walker (#1081).

### Changed (breaking)

- The shared fragment `trun` builder (CMAF, LL-DASH and Smooth fragment writers) returns
  `Error::InvalidInput` for a sample larger than the 32-bit `sample_size` field (4 GiB); the old code
  wrapped the size with `as u32` and wrote a misframed `trun` (#1081, r05-O6).
- **The eight public audio sample-entry structs gained three fields** (#1081,
  audit r05-W31). `Mp4aSampleEntry`, `Ac3SampleEntry`, `Ec3SampleEntry`,
  `OpusSampleEntry`, `FlacSampleEntry`, `Ac4SampleEntry`, `MhaSampleEntry` and
  `DtsSampleEntry` each carry `entry_version: u16`, `reserved_1: [u8; 6]` and
  `compression_id_and_packet_size: [u8; 4]` — the fixed sound-description
  fields a parse now preserves (see the round-trip fix below). A struct-literal
  construction must name them; `entry_version`/`reserved_1` are the QuickTime
  version and `revision_level`/`vendor`, and
  `compression_id_and_packet_size` is ISO/IEC 14496-12's
  `pre_defined`/`reserved` pair or QuickTime's
  `compression_ID`/`packet_size` (all zeros for output this crate muxes).

- **Transport-stream packaging gained continuity state and closed its
  `stream_id` families; the CLI and the DASH/LL-DASH packagers changed shape**
  (#1080, audit r05-W22/W27/W29). New public API:
  - `transmux::TsContinuity` — per-PID `continuity_counter` state
    (ISO/IEC 13818-1 §2.4.3.3), threaded across every `.ts` segment by
    `TsHlsPackager` and `StreamingTsHlsSegmenter`, so concatenating a stream's
    segments no longer jumps the counter on every PID at each boundary.
  - `ll_dash::LlDashPackager::with_utc_timing(scheme, value)` and a
    `pub utc_timing` field, with the `ll_dash::UTCTIMING_*_2014` scheme
    constants: a `dynamic` MPD without `MPD/UTCTiming` (ISO/IEC 23009-1
    §5.8.4.11) leaves a client with no way to obtain the wall clock its
    `availabilityStartTime` is measured against.
  - `Error::TooManyElementaryStreams { family, max }`: a program exceeding the
    16-video / 32-audio `stream_id` families (Table 2-22) is refused instead of
    wrapping into `ECM_stream` or the video range.
  - `cli::CliError::UnsupportedContainer(&'static str)`: container detection now
    runs `container-probe` (a `cli`-only optional dependency) and distinguishes
    "recognised but not demuxable here" from "not recognised".
  - `TsMux` now returns `Error::BufferCapExceeded` for a non-video PES whose
    `PES_packet_length` would exceed 65535 (`0`, the unbounded form, is defined
    only for video), and it now prepends an access unit delimiter to every
    AVC/HEVC access unit that lacks one.
  - `validate::validate_media_segment` gained the `media.mdat.overlap`,
    `media.tfhd.default-base-is-moof`, `media.sample.non-sync-first` (WARNING)
    and `track.tfhd.unknown-track` checks; `track.tfdt.discontinuity` now pairs
    fragments by `tfhd.track_id`.
  - **The CLI's segmented outputs changed shape.** `--segment-duration` is now
    honoured by every segmented format, so:
    - `-f hls` writes a separate `init.mp4` (the `#EXT-X-MAP` target) and
      **media-only** `segN.m4s` files (`styp`+`moof`+`mdat`), where it used to
      write one self-initializing blob per `#EXTINF` and repeat the init bytes
      under every segment name. Each `#EXTINF` is the segment's real measured
      duration (and `#EXT-X-TARGETDURATION` its ceiling), not the presentation
      span divided by the segment count.
    - `-f dash` writes per-Representation media segments whose boundaries are
      **shared** across tracks (so the `segmentAlignment="true"` the MPD
      declares is true), each carrying no `moov` — ISO/IEC 23009-1 §6.3.4.2
      forbids one in a media segment.
    - `-f dash --ll` now emits a `static` MPD with `mediaPresentationDuration`
      (the CLI writes static files; a `dynamic` MPD over them computes a live
      edge a player cannot resolve) and **no** `MPD/UTCTiming` unless
      `--utc-timing-url <URL>` is given — the previous build baked a third-party
      time server into every LL MPD.
    - `AdaptationSet@id` values are now emitted (`t0`, `t1`, …), unique within
      the MPD.
- **Box containers record their children's wire order, and several public
  structs gained fields and constructors** (#1080, audit r05-W11/W12 and the
  round-3 follow-ups). A container's typed fields carry no order of their own,
  so serializing them in declaration order reordered any file that did not
  already match — a `moov`'s `pssh` moved after the `trak`s, a subtitle track's
  `sthd` moved after `stbl`, a `vp09`/`av01` entry's `pasp` moved after its
  config box, and a `moof`/`traf` lost every child the crate does not model.
  New public API:
  - `init_segment::ChildOrder` and a `pub order: ChildOrder` field on
    `MovieBox`, `TrackBox`, `MediaBox`, `MediaInformationBox`,
    `DataInformationBox`, `EditBox`, `MovieExtendsBox` and `SampleTableBox`;
    `ChildOrder::append_opaque(four_cc)` must accompany an `opaque` push on a
    parsed container so the two stay in step.
  - `movie_fragment::{TrafChild, MoofChild, OpaqueChild}` and a
    `pub order: Vec<TrafChild>` / `Vec<MoofChild>` field on
    `TrackFragmentBox` / `MovieFragmentBox`.
  - `init_segment::OpaqueBox` and `movie_fragment::OpaqueChild` gain
    `pub to_end: bool` and `pub largesize: bool`: the `size == 0` and `size == 1`
    wire forms are preserved rather than rewritten (ISO/IEC 14496-12:2015 §4.2).
    A `size == 0` child is only written that way when it is its container's last
    child, since §4.2 defines it as "extends to the end of the enclosing
    container" and a sibling appended after it would otherwise be swallowed. An
    opaque child's stored payload is the bytes after `size`/`type` — a `uuid`'s
    usertype included, the 8 `largesize` bytes excluded.
  - `sample_entries::SampleEntryChild` (`Config`, or `Other(bytes)`) and
    `Vp9SampleEntry::children` / `Av1SampleEntry::children`, replacing
    `extra_boxes`: the config box's position among the entry's children is now
    recorded.
  - `movie_fragment::TrackFragmentHeaderBox`/`TrackFragmentRunBox` gain
    `effective_flags()`; `TrackFragmentRunBox` gains `check_sample_fields`.
  - `movie_fragment::TrunSample::sample_composition_time_offset` and
    `frag_offsets::FragmentSampleRange::composition_offset` are `Option<i64>` /
    `i64`, not `Option<i32>` / `i32`: `trun` version 0's field is unsigned, so a
    legal v0 offset can exceed `i32::MAX`.
  - `sample_groups::SampleGroupDescriptionBox` gains
    `pub default_sample_description_index: Option<u32>` (the v2 syntax field).
  - `MovieBox`, `TrackBox`, `MediaBox`, `MediaInformationBox`,
    `DataInformationBox`, `EditBox`, `MovieExtendsBox`, `TrackFragmentBox` and
    `MovieFragmentBox` gain a `new(...)` constructor, and the `MkvMux` cluster
    builder returns `Result<Vec<u8>>`; a struct literal must switch to the
    constructor or set the new fields.
- **H.264 Sample-AES now chains the CBC state across the encrypted blocks of one
  NAL; the previous per-block IV reset is corrected** (#1080, audit r05-W1).
  `sample_aes::h264_encrypt_nal`/`h264_decrypt_nal` restarted the CBC context at
  every encrypted 16-byte block, so every block after the first in each slice
  NAL was wrong — the IV is reset only at the start of each NAL, then chains
  (each block uses the previous ciphertext block as its IV). This is what
  ffmpeg's `libavformat/hls_sample_aes.c` `decrypt_nal_unit` does: a chained
  stream decodes identically to clear (10/10 frames) while the old per-block
  reset decodes to nothing (0/10). Pinned by the independent fixtures in
  `transmux/tests/fixtures/sample_aes_h264/` (python reference, Bento4
  `mp4hls`, ffmpeg) — see `ORACLES.md`. **Output bytes change**, so any
  SAMPLE-AES H.264 output produced by this crate before this fix must be
  re-encrypted. The transcription (`docs/drm/hls-sample-aes.md` §3.4/§11) and
  the module doc are corrected to match.
- **`CencEncryptor::encrypt` rejects overlapping AES-CTR counter-block ranges
  under `cenc`** (#1080, audit r05-W3). `IvGen::Explicit`'s uniqueness check
  compared IVs for equality only, but under `cenc` the per-sample IV *is* the
  128-bit AES-CTR counter block and the counter advances once per 16-byte
  protected block (ISO/IEC 23001-7 §10.1). A caller that supplied the natural
  sequential list `base + i` passed that check while sample *i+1*'s first
  keystream block equalled sample *i*'s second — a two-time pad across every
  consecutive pair, the exact bug the uniqueness rule exists to prevent. Such a
  config is now `Error::InvalidInput`, checked on the planned IV/subsample
  sequences before a byte is ciphered (so a rejection still leaves `media`
  byte-identical), and `cbcs` is exempt because its IV seeds a CBC chain rather
  than a counter. An existing per-sample IV list must be spaced by at least the
  preceding sample's protected block count.
- **`IvGen::Counter` under `cbcs` now emits 16-byte per-sample IVs** (#1080,
  audit r05-W4). It emitted 8-byte IVs — the shape this module's own
  `IvGen::Constant` doc records that Bento4's `mp4decrypt` silently no-ops, so
  a default `EncryptConfig { scheme: Cbcs, iv: IvGen::default(), .. }` produced
  output a reference decryptor would not touch. The width is now
  scheme-dependent (8 under `cenc`, the common CMAF convention; 16 under
  `cbcs`), so existing `cbcs` output changes and any decryptor must be fed the
  matching `tenc.default_per_sample_iv_size`.
- **`smooth_parse::track_spec_from_quality_level` dispatches on the `QualityLevel`'s
  `FourCC` instead of assuming H.264/AAC** ([MS-SSTR] §2.2.2.5). Every audio level was
  parsed as an AudioSpecificConfig whatever its FourCC, so a Dolby `FourCC="EC-3"`/`"AC-3"`
  level (whose `CodecPrivateData` is Dolby, and which "parses" as an ASC for any ≥2-byte
  input) produced a garbage `CodecConfig::Aac` track with a nonsense rate/channel count;
  a non-H264 video level likewise built an `avcC` from bytes that are not Annex-B SPS/PPS.
  The match is now case-insensitive over `H264`/`AVC1` (video) and `AACL`/`AACH` (audio);
  any other token — a Dolby `EC-3`/`AC-3`, an `H265`, a private one — returns
  `Error::UnsupportedCodec` (reject-only: Dolby audio, HEVC video are not synthesised).
  An empty `CodecPrivateData` on an `AACL`/`AACH` level now synthesises the
  `AudioSpecificConfig` from `SamplingRate`/`Channels` (ISO/IEC 14496-3 §1.6.2.1) instead
  of failing `BufferTooShort` — the plain **core** AAC-LC config for both FourCCs, so an
  `AACH` level's SBR is carried implicitly (the hierarchical explicit-signalling form is not
  synthesised; the `esds` reports the core rate, not twice it). A `SamplingRate` that is not
  an ISO/IEC 14496-3 Table 1.10 entry, or a channel count with no Table 1.19 configuration
  (Table 1.19 has no 7-channel entry — `Ch7_1` is eight), is `Error::UnsupportedCodec`
  rather than a guessed value. Both the SPS-decoded geometry and the `MaxWidth`/`MaxHeight`
  fallback are converted to the IR's `u16` with `try_from`, so an over-range value is
  `Error::InvalidValue` rather than a wrapped dimension (#1080, audit r04-W41).
- **`vvc_config::VvcPtlRecord::general_constraint_info` is a `Vec<u8>`, not a `u64`**, and
  `VvcDecoderConfigurationRecord::dimensions` returns `Option<(u32, u32)>`, not
  `Option<(u16, u16)>` (#1080, audit r04-W42). `num_bytes_constraint_info` is a 6-bit field,
  so a `general_constraint_info` of more than 8 bytes (up to 502 bits) did not fit a `u64`:
  the reader rejected it with `n > 64`, failing the whole `vvcC` and with it the whole VVC
  track — and `decode_vvc_sps` likewise rejected any SPS with `gci_present_flag == 1`, which
  is ordinary in the field. The SPS reader now steps over the `general_constraint_info()`
  block exactly (ITU-T H.266 §7.3.3.2) instead of rejecting it, and the payload is carried as
  bytes. `dimensions()` widened because `sps_pic_width_max_in_luma_samples` is `ue(v)`: a
  `u16` cast reported a wrapped geometry (65 536 → 0). A caller converting to the sample
  entry's `u16` field uses `try_from`.
- **`sps::rfc6381_vvc1` takes the constraint bytes as `&[u8]`, not a `u64`**, and
  `VvcPtlRecord` serialization validates `general_constraint_info` (#1080, audit r04-W42).
  The `u64` argument could not represent a constraint block wider than 64 bits, so it went
  with the `Vec<u8>` field; a caller must pass `&ptl.general_constraint_info`. A record whose
  `general_constraint_info` is shorter than its `num_bytes_constraint_info` is now
  `Error::InvalidValue` on serialize instead of panicking on an out-of-bounds index. The SPS
  reader also skips the exact reserved-bit count of `general_constraints_info()` (ITU-T H.266
  §7.3.3.2: `gci_num_additional_bits - (n > 5 ? 6 : 0)` `gci_reserved_bit`s) instead of
  skipping none for a count of 1..=5.
- **Pass-through demuxers normalise NAL length prefixes to the crate's 4-byte form and say so
  in the config they emit** (`WebmDemux`, `FlvDemux`, `StreamingFlvDemux`) (#1080, audit
  r04-W43). An `avcC`/`hvcC` declares its NAL length-prefix size as `lengthSizeMinusOne + 1`
  (ISO/IEC 14496-15 §5.3.3: 1, 2 or 4 bytes), and these demuxers carried a block's payload
  verbatim — but the rest of the pipeline (`iter_length_prefixed_nals`, keyframe detection,
  the TS/RTP writers) is fixed at 4 bytes, so a 2-byte-length source parsed every sample as
  garbage lengths and produced a corrupt stream. New `annexb::normalise_nal_length_size` /
  `iter_length_prefixed_nals_with` rewrite a sample's prefixes, and the emitted
  `AVCDecoderConfigurationRecord`/`HEVCDecoderConfigurationRecord` now carries
  `length_size_minus_one = 3` so a fMP4/CMAF/TS mux describes the samples it actually has
  (previously it kept the source's value, promising 2-byte lengths over 4-byte samples). The
  framing is validated for every length size, including the canonical 4 (a declared length
  past the buffer is an error rather than a silent pass-through); a length size other than 1,
  2 or 4 is an error — 3 (`lengthSizeMinusOne = 2`) is reserved by §5.3.3, so it is rejected
  rather than guessed at.
- **`Fmp4Demux` and `ProgressiveDemux` normalise an AVC/HEVC track's NAL length prefixes the
  same way** (`media::normalise_track_nal_lengths`) (#1080, audit r04-W43). Both carried a
  sample's bytes verbatim, so an fMP4 whose `avcC`/`hvcC` declared `lengthSizeMinusOne` 0 or 1
  gave the IR 1-/2-byte-prefixed samples while every downstream consumer
  (`iter_length_prefixed_nals`, keyframe detection, the TS/RTP writers) is fixed at 4 bytes —
  a corrupt remux. The samples are now rewritten and the reported config's
  `lengthSizeMinusOne` set to match; a track whose samples then fail to walk (a truncated
  prefix, a declared length past the buffer) is skipped with a reason rather than passed on.
- **`vpcC` boxes whose FullBox version is not 1 are now rejected outright** (#1080, audit
  r04-W44). The VP Codec ISO Media File Format Binding v1.0 declares
  `FullBox('vpcC', version = 1, 0)` and states "Version 0 is deprecated and should not be
  used" — it publishes no v0 syntax for the record. The parser previously accepted any
  version and always read the v1 layout, so a **version-0 box (written by some early
  ffmpeg/libwebm muxers) parsed to wrong bit depth / chroma / colour values and was then
  re-serialized as v1**. Such boxes are now `Error::InvalidValue` on both parse and
  serialize, so media that used to demux (incorrectly) no longer parses at all — the intended
  behaviour, since a guessed layout is worse than a clear refusal. `Vp9ConfigurationBox`
  loses the two `color_space` / `transfer_function` fields that existed only for that
  undocumented layout, so a caller constructing the struct must drop them. `bit_depth` (4
  bits) and `chroma_subsampling` (3 bits) are written through the checked `fit_bits` helper,
  so an over-range value is `Error::FieldOverflow` instead of shifting bits out of the byte or
  corrupting `videoFullRangeFlag`. A `codecInitializationDataSize` that overruns the box is
  `Error::BufferTooShort` rather than a silent `min(len)` truncation.
- **Every serializer writes its reserved/`pre_defined` bytes as zeros instead of skipping
  them** (audit r04-W45). `Serialize::serialize_into` accepts any `&mut [u8]`, so a reused or
  dirty output buffer kept whatever was there: `VisualSampleEntry`'s 6 + 16 reserved bytes and
  the audio `SampleEntry`/`AudioSampleEntry` 6 + 8 + 4 reserved/`pre_defined` bytes
  (`sample_entries.rs`), the `stpp`/`wvtt` 6 reserved bytes (`subtitle_entries.rs`), `mvhd`'s
  reserved(10)/`pre_defined`(24), `tkhd`'s reserved(4)/reserved[2](8)/reserved(2), `mdhd`'s
  quality(2), `hdlr`'s `pre_defined`(4)+`reserved[3]`(12) and `smhd`'s reserved(2)
  (`init_segment.rs`), and the `vpcC`-style bit writer used by the VVC PTL record
  (`vvc_config.rs`, which OR-ed set bits and left alignment padding untouched) all emitted
  garbage, which a conformance validator rejects. Each site now zeroes the range it advances
  over, and a dirty-buffer test covers `mvhd`, `tkhd` (v0/v1), `mdhd`, `hdlr`, `smhd`, `vmhd`
  and every audio sample-entry FourCC.
- **The manifest XML tokenizer ends a start tag at the first `>` outside a quoted attribute
  value, and resolves numeric character references in attribute values** (audit r04-W46). XML
  1.0 §2.4 requires only `<` and `&` to be escaped in an attribute value, so a `>` there is
  legal raw — but the tag scan stopped at it, truncating a DASH `SegmentTemplate@media` or a
  `ContentProtection` value (and every later event was misparsed). Numeric references
  (`&#38;`, `&#x26;`) were also left literal, so a template written with them resolved to a
  URL containing `&#38;`; attribute values now use the same resolution rule as text content.
  A reference candidate also ends at the next `&` as well as the first `;`, so a bare `&`
  before a later valid reference (`"a & b &amp; c"`) no longer consumes it as text.
  A numeric reference outside XML 1.0 §2.2's `Char` production (a zero code point, a bare
  control character, a surrogate, anything above `#x10FFFF`, or a leading `+`/`-`) is left
  verbatim rather than decoded into an illegal character, and the reference scan advances past
  the terminating `;` (or the whole malformed `&`-run) every iteration, so it is linear in the
  input rather than rescanning each `&`.
- **`RtpPacketiser` now stamps each access unit's RTP timestamp with the sample's
  `pts`, not its `dts`** (audit r04-W29). RFC 6184 §5.1: "The RTP timestamp is set
  to the sampling timestamp of the content", and receivers "SHOULD use the RTP
  timestamp for synchronizing the display process" — a *presentation* time. On a
  B-frame stream the two differ substantially (`fixtures/ts/h264/high.ts`: 12 of
  its 15 samples), and stamping decode times made a receiver present frames in
  decode order at the wrong instants.
- **`RtpPacketiser` fragments an AAC access unit that exceeds the payload budget
  instead of refusing it** (audit r04-W31). RFC 3640 §3.2.3.1 requires it — one
  fragment per packet, all sharing the AU's timestamp, marker on the last — and
  §3.2.3.2 makes the AU-header's `AU-size` the size of the *entire* AU rather than
  of the fragment, which is exactly what lets a receiver reassemble. Such an AU is
  the ordinary case for high-rate audio (a 640 kb/s 5.1 frame is ~3.7 kB), and a
  track carrying one previously failed to packetise at all.

- **`RtpPacketiser` gives every stream its own SDP payload type.** A payload type is
  a session-wide binding (RFC 3551 §6 reserves 96-127 "for dynamic assignment"), but
  only "has a video track been seen" was tracked, so a third video (or audio) track
  reused `video_pt + 2` — colliding with the second track's type — and `96 + 2 = 98`
  collided with `DEFAULT_KLV_PT`. Allocation now walks the dynamic range from the
  configured bases (96 video, 97 audio by default, both still honoured for the
  common single-video/single-audio session) and never reuses a value, erroring with
  `Error::InvalidInput` if a session needs more than the 32 dynamic types. The
  generated SDP's `a=rtpmap` therefore binds each stream exactly once.
- **The generated SDP gains a session-level `c=` line** (RFC 4566 §5.7: "A session
  description MUST contain either at least one `c=` field in each media
  description or a single `c=` field at the session level"). Without it a strict
  parser had no connection data to resolve and rejected the description. The
  address is `LOCAL_CONNECTION_ADDRESS` (the loopback the `o=` line already names);
  `build_sdp_with_connection` is new for a caller that transmits elsewhere.

- **`rtp::RtpInputStream` is `#[non_exhaustive]` and can no longer be built by struct
  literal** — use `RtpInputStream::new(kind, packets)` plus `with_clock_rate`/
  `with_config`, which also validate kind/clock/config agreement (see below). It
  gains `clock_rate` and `config` fields on top of that, and the batch
  `RtpDepacketiser` now describes each depacketised track with its own payload
  format, RTP clock and codec config (audit r04-W30). An RTP timestamp is a count
  in the stream's own clock (RFC 3550 §5.1), but every track the depacketiser
  returned was declared AVC at the 90 kHz video clock — so an AAC stream came back
  as a 90 kHz video track whose durations were the audio sample counts (1024 per
  frame, not 1920), and any packager consuming it wrote a video sample entry for
  audio. `Media`'s movie timescale was `0` for the same reason and is now the video
  clock. `RtpInputStream::new`/`with_clock_rate`/`with_config` build the struct
  (`#[non_exhaustive]`, so its fields are no longer directly constructible).
  An AAC stream with no `config` is now `Error::InvalidInput` rather than silently
  described by an invented AVC config: RTP carries no codec config, so a caller
  must supply the SDP's `config=` parameter (RFC 3640 §4.1). A zero `clock_rate` is
  rejected for the same reason. **An AAC stream's clock rate is taken from its
  config's own sample rate** — RFC 3640 §3.1 defines an MPEG-4 audio stream's RTP
  clock as its sampling rate, so `with_config` sets it and the
  `DEFAULT_AAC_CLOCK_RATE` (48000 Hz) constructor placeholder is replaced; a rate
  asserted with `with_clock_rate` that disagrees with the config, a non-90 kHz
  clock on an AVC config, and a `kind` that disagrees with the config's variant are
  each `Error::InvalidValue`/`Error::InvalidInput` rather than silently timed at
  whichever value happened to win. A caller constructing the struct literally must
  add the two fields (plus the private explicit-clock flag).

- **`StreamingFlvDemux` now emits `DemuxEvent::TrackUpdated` when a publisher re-sends a track's
  sequence header, carrying the new codec config** (FLV/RTMP ingest). Previously the second and later
  sequence header was ignored, so an encoder that changed resolution, profile, sample rate or
  channel count mid-publish (routine for OBS/ffmpeg) kept reporting the stale config and every
  downstream init segment described a stream that was no longer being sent. `TrackUpdated` was
  documented as never changing `config`; it can now, and a consumer must treat it as "reload the
  init segment" (#1079, audit r04-W13). `DemuxEvent::TrackUpdated`'s doc is updated accordingly.
- `FlvDemux::unpackage` and `StreamingFlvDemux::feed` return
  `FlvError::Codec(Error::InvalidInput)` for an AVC sequence header whose SPS decodes to a width or
  height that does not fit the IR's `u16`. An SPS's `pic_width_in_mbs_minus1` is an unbounded
  `ue(v)`, so a 65 536-wide SPS previously came back as a track claiming width 0 (#1079, audit
  r04-W38).
- **`FlvMux::package` now returns `FlvError::Codec(Error::InvalidInput)` for a composition offset it
  cannot represent and for a zero track timescale**, rather than writing a truncated or flattened
  value. An offset outside `CompositionTime`'s signed 24-bit millisecond field (Annex E §E.4.3.2)
  used to be written with its low 24 bits, describing a moment the sample is not at; and a track
  whose `TrackSpec::timescale` is 0 made every tag's `Timestamp` 0, flattening the whole file onto
  one instant (#1079, audit r04-W12). A zero timescale is reachable: `RtpDepacketiser` has produced
  such tracks (audit r04-W30).
- `rtmp::RtmpError` gains the `Amf0TooDeep { depth }` variant. The enum is `#[non_exhaustive]`, so
  this is an additive change for matching callers, but an exhaustive `match` over it outside this
  crate (allowed only via a wildcard arm) now has a case it did not before (#1079, audit r04-W27).
- **Several paths that previously returned a value now return an error**, because accepting the
  input produced a box/header that misdescribed itself. Each is listed again under `### Fixed` with
  its full rationale:
  - `FlacSpecificBox::parse` rejects a `dfLa` metadata block whose declared 24-bit length runs past
    the end of the box, and a first block that is not STREAMINFO (r04-W17).
  - `BoxHeader::serialize_into` rejects a `uuid` header with no `usertype`, a header with
    `size == 1` but no largesize, and a compact header whose `size` exceeds 32 bits (r04-W8).
  - `ColourInformationBox::serialize_into` rejects a `nclx` `colr` with no `nclx` params (r04-W39).
  - `ESDescriptor::serialize_into` rejects a `streamDependenceFlag`/`URL_Flag`/`OCRstreamFlag` that
    disagrees with its field (r04-W24).
  - `Ec3SpecificBox::serialize_into` rejects `substreams.len() != num_ind_sub + 1` (r04-W6).
  - `webm_demux::WebmDemux` rejects a laced block whose declared lace sizes do not
    describe its payload (r04-W40).
- **`klv::crc16_ccitt` is removed, replaced by `checksum_bcc16`** (audit r04-W10). The
  name documented the wrong algorithm: MISB ST 0601 tag 1 is a running 16-bit sum, not a
  CRC (§5.5/§7.1). A caller that used `crc16_ccitt` directly should call `checksum_bcc16`
  instead; its result is deliberately different.
- **`klv::UasLocalSet::serialize_with_checksum` writes a different (correct) tag-1 value**,
  so byte-for-byte comparisons against a packet this crate produced before the change will
  differ — as will `verify_checksum`'s verdict on any real ST 0601 stream, which it now
  accepts (r04-W10).
- **`dash_parse::Mpd` gains a `base_url: Option<String>` field**, and
  `dash_parse::Period`, `AdaptationSet` and `Representation` each gain one
  (`BaseURL`, §5.3.9.2), and `Mpd::parse` now resolves
  `SegmentTemplate` inheritance attribute-by-attribute rather than whole-element, so the
  `segment_template` a caller reads is the merged effective one (r04-W11). A struct literal
  for any of the four must add the new field; code that relied on a child's template
  *replacing* its parent's now sees the inherited values, which is the spec behaviour.
  `Mpd::base_url_chain` and `Mpd::resolve_segment_url` are new.
- `ps_demux::PsDemux` emits one track per `private_stream_1` substream rather than one per
  `stream_id`, so a PS whose 0xBD stream multiplexes several substreams now yields several
  tracks (and skips substreams that are not AC-3) (r04-W19). Its signatures are unchanged,
  but a video stream whose reassembled Annex B data exceeds `au::AccessUnitSplitter`'s
  buffered-NAL cap is now an error rather than a silently absent track (r04-W21).
- `au::AccessUnitSplitter::push` now returns `Result<()>`: a stream whose open NAL or assembled
  access unit grows past a 64 MiB cap without completing is rejected instead of buffering without
  limit (`Error::InvalidValue`). The bound is far above any real coded picture, so a conformant
  stream never reaches it (#1079, audit r04-W3).
- `mp4esds` descriptor types now record the wire width of their size varint and every uncommon
  sub-descriptor, and `ESDescriptor` presence is derived from its fields. `DecoderSpecificInfo`,
  `SLConfigDescriptor` and `DecoderConfigDescriptor` gain a `size_width: usize` field;
  `DecoderConfigDescriptor` and `ESDescriptor` gain `unknown_descriptors: Vec<UnknownDescriptor>`
  (new public type); `ESDescriptor` gains `size_width`. Constructors `DecoderSpecificInfo::new`,
  `SLConfigDescriptor::new`/`predefined_two`, `DecoderConfigDescriptor::new` and
  `ESDescriptor::new` cover the common MP4-storage shapes. An `esds` authored with minimal
  (GPAC/Apple/Bento4) size varints previously always re-serialized with the 4-byte expanded form, so
  it grew on every round trip; unmodelled sub-descriptors (e.g. the
  `ProfileLevelIndicationIndexDescriptor` 0x14, an IPI pointer or language descriptor) were dropped
  entirely; and a `streamDependenceFlag`/`URL_Flag`/`OCRstreamFlag` set without its field made the
  declared descriptor size count bytes that were never written, misframing the descriptor while
  still returning `Ok` (now `Error::InvalidValue`) (#1079, audit r04-W24).

- `ac3::Ec3SpecificBox` gains a `reserved_tail: Vec<u8>` field (ETSI TS 102 366 §F.6.2.14) and its
  serializer now returns `Error::InvalidValue` when `substreams.len() != num_ind_sub + 1`, because
  §F.6.1 derives the independent-substream count from `num_ind_sub` — a mismatched struct previously
  serialized to a `dec3` that parsed back differently. The tail is captured verbatim on parse, so a
  Dolby Atmos `dec3` (whose extension signalling lives in those reserved bytes) now survives a
  parse/serialize round trip instead of being dropped (#1079, audit r04-W6).
- `ac3`'s `chanmap`->`chan_loc` derivation is corrected against ETSI TS 102 366 §E.1.3.1.8 / §F.6.2.13:
  Table E.1.4 numbers its bits **MSB-first** ("bit 0 … is stored in the most significant bit of the
  `chanmap` field"), so the previous `1 << n` test read every location mirrored. The `bsi()` walk
  also gates the programme-mix block on `strmtyp == 0x0` rather than "not a dependent stream", since
  `strmtyp == 0x2` is an AC-3-derived *independent* stream with no programme-mix block. Both tables
  and the `bsi()` structure are transcribed with clause citations in
  `docs/codec/eac3-bsi-chanmap.md`. (#1079, audit r04-W5)
- `ac3::Ec3SyncframeInfo` gains `chanmap: Option<u16>` and `bsmod: u8`;
  `Ec3SyncframeInfo::into_dec3` and the new `Ec3SpecificBox::from_syncframes` both return `Result`.
  They take the syncframes of a **single access unit** (`Ec3SyncframeInfo::from_es_first_au`) so
  dependent substreams are folded into `num_dep_sub`/`chan_loc` and `num_ind_sub` is §F.6.2.3's
  "substreamID of the last independent substream". Previously the count came from the writing
  frame's own `substreamid`, `num_dep_sub` was always 0, `chan_loc` always `None`, and `bsmod` was
  hardcoded 0 — so a 7.1 DD+ stream was signalled as 5.1 with an inflated substream count. Feeding
  a whole demuxer backlog (N repeated access units -> N substreams, `data_rate` summed) is what
  `ts_demux` did and produced a `dec3` its own serializer rejects; a dependent-only or empty slice
  now returns `Error::InvalidValue` rather than that unserializable box (#1079, audit r04-W5).

- **`sample_aes::ExtXKey::to_tag` now returns `Result<String>`** (was
  `String`), and the inherent `Display` impl for `ExtXKey` is removed
  (issue #1140 / audit r05-W10, T12): `uri`/`keyformat`/
  `keyformatversions` are caller-supplied (often assembled from a
  key-server request) and previously went straight into the tag with no
  validation, so a `"`, CR or LF in any of them terminated the attribute
  list and injected arbitrary playlist tags. `to_tag` now builds through
  `broadcast_hls::AttrValue`'s checked constructors and the shared
  `broadcast_hls::render_attribute_list` (the single workspace attribute
  renderer this issue introduced), returning the new
  `Error::HlsAttrValue` for an invalid value instead.
- Serializers now return an error, instead of silently truncating, when a length, count, or
  offset does not fit its wire field (#1129). `Error` gains a new `FieldOverflow` variant; several
  previously-infallible builders (`transmux::rtmp::write_chunks`, `MessageHeader::write_into`,
  `OutTag::write_into`, `HevcNalUnit`/`HevcNalArray::serialize_into`, `drm::playready_pro`/
  `playready_pssh`) now return `Result` for the same reason.
- `cenc::SampleAuxInfoSizesBox` and `init_segment::SampleSizeBox` gain a `sample_count: u32`
  field. Previously the uniform-size form (`default_sample_info_size`/`sample_size != 0`) derived
  the wire `sample_count` from the always-empty per-sample table, so it silently collapsed to 0 on
  every parse -> serialize round trip of a real whole-sample-protected or constant-bitrate track
  (#1013, #1018).
- `init_segment::Mp4aSampleEntry` gains a `codec_type: [u8; 4]` field recording the real sample
  entry four-CC (`mp4a` or `enca`); serialize previously hard-coded `mp4a`, silently re-labelling a
  CENC-protected (`enca`) audio track as clear (#1017).

- **`StreamingTsDemux` now emits `DemuxEvent::TrackUpdated` when a TS track's
  in-band codec configuration actually changes mid-stream, and drops — rather
  than delivers — access units damaged by a continuity-counter gap**
  (#1080, audit r04-W49/W51). Both are visible behaviour changes for a consumer,
  which is why they are listed here as well as under `### Fixed` below.

  **`TrackUpdated` from TS.** Codec-config recovery used to be single-shot for
  the life of a stream, so a mid-stream SPS/PPS or AAC-config change (an SD↔HD
  ad break, a re-encode, a multiplex reconfiguration) left the track labelled
  with its original `avcC`/`hvcC`/`esds`. Now an AVC/HEVC track whose
  accumulated parameter sets change, or an AAC track whose
  `audioObjectType`/`samplingFrequencyIndex`/`channelConfiguration` change,
  raises `DemuxEvent::TrackUpdated` with the same `track_id` and the new config
  — the two-source contract `DemuxEvent::TrackUpdated` already documented for
  FLV. A consumer that rebuilds its init segment only on `TrackAdded` will now
  miss such a change; `TrackUpdated` means "reload the init segment / rebuild
  the sample entry". An **unchanged** repeat emits nothing, so the event rate
  stays low (encoders resend their headers on every keyframe).

  **Damage-aware delivery.** An access unit is dropped rather than delivered
  truncated when either of two things is true, and only these two:

  1. a continuity-counter gap landed on one of **its own** packets (a gap on a
     continuation packet: those bytes provably belong to it); or
  2. it is a **bounded** PES (`PES_packet_length != 0`) that arrived shorter
     than its own header declared — no continuity-counter evidence is needed for
     this one, since the declared length settles it outright.

  A legal §2.4.3.3 duplicate packet is discarded before it reaches the
  reassembler (it used to be detected for the CC check and then fed in anyway,
  duplicating 184 payload bytes inside the access unit under construction). A
  stream with genuine CC gaps therefore yields **fewer samples than before** —
  the damaged ones are gone rather than corrupt.

  A `discontinuity_indicator` is **not**, by itself, a reason to drop anything.
  It marks where the source's time base changes, and a normal HLS or splice seam
  has an intact last unit of the old segment immediately before it; treating the
  indicator as damage lost that unit at every seam. When the indicator lands on
  a *continuation* packet it coincides with a gap and rule 1 applies as usual.

  **Documented tradeoff (unbounded PES at a CC-jump boundary).** A
  `payload_unit_start` ends the previous unit, so a counter restart there is the
  ordinary segment-concatenation shape and is not damage. For an **unbounded**
  PES (`PES_packet_length == 0`, what every video PES uses) that also means an
  access unit which really did lose its tail is indistinguishable from one that
  ended cleanly, and it is delivered. H.222.0 gives no length to tell them
  apart: the alternative — dropping every unit that precedes a counter jump —
  loses a whole access unit at every seam instead of occasionally carrying a
  short one. A bounded PES has no such tradeoff.

  **ADTS channel count.** `CodecConfig::Aac`'s `channel_count` for a TS AAC
  track now maps `channel_configuration` through ISO/IEC 14496-3 Table 1.19
  instead of using the raw field: configuration 7 reports **8** (7.1) where it
  used to report 7, and configuration 0 (an in-band `program_config_element`,
  which this crate does not decode) reports **0** — "not derivable", never a
  fabricated count, matching `flv_stream`'s convention. A caller that treated 0
  as "mono" must check for it.

  **Signalled-discontinuity rebase.** A TS track's decode timeline is now
  rebased at a signalled discontinuity so `dts` stays monotonic across a splice;
  previously every downstream muxer could write negative or zero deltas there.
  The rebase is **scoped to the signalling program**: per ITU-T H.222.0
  §2.4.3.5 a system time-base discontinuity is signalled on the program's
  `PCR_PID`, so only that program's elementary streams are lifted and a
  multi-program multiplex leaves the other services' timelines untouched. The
  lift is non-negative and one nominal frame period past the last stamp emitted
  (never a reduction, so a forward splice survives as the real gap it is), and
  the frame period it uses is the **median of the last five** plausible
  inter-access-unit steps, capped at one second — so a single late access unit
  or an 8-second splice cannot become "the frame period" for the frames that
  follow.

### Fixed

- `read_sei_varint` returns `None` when the running `payloadType`/`payloadSize` total would overflow
  `u32` (a ~16.8 MB run of `0xFF`), instead of panicking in debug builds and wrapping in release
  (#1079, r04-O6).
- `AVCSampleEntry::bare_parse` / `HEVCSampleEntry::bare_parse` report the minimum length they actually
  check (94 bytes) in `BufferTooShort::need`; the HEVC path said 96 (#1079, r04-O8).
- **The batch TS-HLS packager runs in O(samples), not O(segments × samples)**
  (#1081, audit r05-W33). `TsHlsPackager::package` recomputed each segment's
  per-track base DTS — and re-walked the anchor track's prefix — once per
  segment, so a two-hour 60 fps recording at 6 s segments cost roughly 10⁹
  `MediaClock::tick` calls. It now carries a running per-track clock and base
  across the segment loop, so each sample is ticked exactly once. Output is
  unchanged: a golden test pins the exact bytes (segment and playlist FNV-1a
  hashes recorded from `main`'s packager on the same fixture), and a second
  test asserts the literal tick count. A track whose samples a segment's range
  does not cover has its gap folded into the running base *before* that segment
  is muxed, rather than after, so a skipped sample can no longer land in the
  wrong segment.

- **`Repackage` no longer overflows when interleaving tracks whose timescales
  have a huge least common multiple** (#1081, audit r05-W32). The resegmenter
  normalised every track onto the LCM of their `mdhd` timescales to compare
  decode times; three large coprime timescales (all of which `mdhd` may legally
  carry, and which no validator can rule out) make that LCM exceed `u64`, so the
  `a / gcd * b` fold panicked in debug and wrapped in release — stranding a
  track's samples in one segment. The comparison is now an exact rational
  cross-multiply in `u128` (the same approach `LlSegmenter` already used), which
  cannot overflow for any pair of `u32` timescales.

- **Audio tracks at 88.2/96/176.4/192 kHz keep their real sampling rate instead
  of silently wrapping to a wrong one** (#1081, audit r05-W31). An
  `AudioSampleEntry`'s `samplerate` field is 16.16 fixed point, so its integer
  part tops out at 65535 Hz: muxing such a track wrote `192000 << 16` truncated
  to `0xEE000000`, which reads back as 60928 Hz (`96000` becomes
  `0x77000000`, read as 30464 Hz). `build_init_segment` now emits the
  `AudioSampleEntryV1` form the spec requires for these rates — `entry_version
  = 1`, the `samplerate` placeholder, a `SamplingRateBox` (`srat`) carrying the
  true rate, and an `stsd` version of 1 (ISO/IEC 14496-12:2015 §12.2.3.1 and
  §12.2.3.2 as amended by Amd 1:2017) — and the demux path reads `srat` in
  preference to the 16.16 field. Rates that fit the field are unchanged (still
  the version-0 form, no `srat`). Verified end-to-end: our own muxer's output
  is parsed back by MP4Box (which reads the sample entry's rate, 192000, and
  reports 60928 without the fix) and ffprobe, and its decoded PCM is
  md5-identical to the ffmpeg-made source (`tests/fixtures/audio_srat/`).

- **A parsed audio sample entry re-serializes byte-identically** (#1081, audit
  r05-W31). The writer re-derived `entry_version` as 0-or-1 and zeroed the two
  fixed regions it did not model, so a QuickTime sound description lost its
  `compression_ID` (QuickTime's VBR marker is `-2`, `ff fe`) and a QuickTime
  **v2** entry (`entry_version = 2`) was silently rewritten as v0. The version
  and both regions (`reserved_1`, `compression_id_and_packet_size`) are now
  parsed and written back verbatim.

- **`TsMux` emits a conformant access unit per video sample and never silently
  truncates a PES, and its PCR is anchored to a stream that can carry it**
  (#1080, audit r05-W17/W19/W20).
  - Every AVC/HEVC access unit now begins with an access unit delimiter
    (`00 00 00 01 09 F0` / `00 00 00 01 46 01 50`) when the sample does not
    already carry one — ITU-T H.222.0 §2.14.1 / §2.17.1 make one mandatory, and
    an IR sample from fMP4/MKV/FLV/RTMP never has one. A stream that already
    carries an AUD does not get a second.
  - A non-video PES longer than 65535 bytes is now an error rather than a
    clamped `PES_packet_length` over a longer payload (which every demuxer would
    truncate); §2.4.3.7 permits the unbounded `0` form for video only.
  - The PCR no longer falls back to a sparse PES-carried `Data` PID (DVB
    subtitles/teletext, whose PES packets arrive seconds apart, far past the
    §2.4.2.2 100 ms bound): audio is preferred, and where even the chosen PID
    leaves a gap, PCR-only adaptation packets hold the interval within 40 ms
    (TR 101 290 indicator 2.3). A PCR-only packet carries no payload, so it
    repeats the PID's continuity counter rather than advancing it (§2.4.3.3).
- **`validate` no longer reports phantom or missing CMAF problems** (#1080,
  audit r05-W30). The `mdat` bounds check now resolves *every* `trun` range
  (including one that omits `data_offset`, which continues from its predecessor)
  and reports `media.mdat.overlap` for ranges that claim the same bytes; a
  multi-fragment media segment is compared against its *last* fragment's decode
  end instead of its first, so it no longer raises a false
  `track.tfdt.discontinuity`; cross-segment continuity pairs fragments by
  `tfhd.track_id` rather than by position; a `tfhd` naming a track the init
  segment does not declare is reported; and `media.sample.zero-duration` is
  raised once per fragment with a count, not once per sample.
- **LL-HLS parts and LL-DASH chunks carry every track, and stay within the part
  target** (#1080, audit r05-W27). A regular part/chunk drained only the anchor
  (video) track and left all audio for the segment's final part, so a client
  playing at the live edge had video with no audio until the segment closed.
  Every track now contributes the samples whose decode span falls inside the
  part/chunk's span, and a part stops *before* the sample that would take it
  past `PART-TARGET` (RFC 8216bis §4.4.4.9 bounds a part duration above by the
  part target), instead of rounding every part up by one anchor sample.
- **DASH groups Representations by language and codec, and the CLI honours
  `--segment-duration`** (#1080, audit r05-W28/W29). One `AdaptationSet` per
  (kind, `@lang`, codec family) — ISO/IEC 23009-1 §5.3.3 makes the members of a
  set switchable encodings of the same content, and a single set for every
  language put an ABR client in a position to switch between `eng` and `fra`, or
  between AAC and AC-3, on a bandwidth change. The `hls`/`dash` outputs are now
  genuinely segmented at `--segment-duration` (a DASH media segment is
  `styp`+`moof`+`mdat` with no `moov`, which §6.3.4.2 forbids there), the CMAF
  HLS playlist lists one segment per slice with an `#EXT-X-MAP` and an `init.mp4`
  beside it, and `detect_container` handles M2TS (192-byte stride) and
  mid-packet captures.
- **`moof`/`traf` children the crate does not model are preserved, and a `traf`
  with no `trun` parses** (#1080, audit r05-W11). §6.2.3's `traf` table lists
  `sbgp`/`sgpd`/`subs`/`saiz`/`saio`/`meta` besides `tfhd`/`trun`/`tfdt`; only
  the last three were modelled, so a parse → serialize — and in particular
  `protect_media_segment`, which re-serializes any `moof` it touches — silently
  dropped roll-recovery and sample-encryption-group signalling (`sgpd`/`sbgp`
  `seig`), `subs`, `sdtp` and an existing `senc`/`saiz`/`saio` from a segment
  that merely had a track encrypted. `TrackFragmentBox`/`MovieFragmentBox` now
  record their children's wire order and carry every unmodelled child verbatim,
  so a parsed box round-trips byte-for-byte. Separately, a `traf` with zero
  `trun`s is **legal** — §8.8.3/§8.8.8 require none, and `tfhd`'s
  `duration-is-empty` flag (`0x010000`) exists to declare a track with no
  samples in this fragment — but was rejected with "traf missing trun", which
  failed the whole `moof` and with it `Fmp4Demux`, `cenc_decrypt` and
  `protect_media_segment`. A `traf` with a `tfhd` and **zero or more** `trun`s
  now parses. A `moof` with zero `traf`s is still rejected ("moof missing
  traf"), and so is a `traf` with no `tfhd`: those name no track, so there is
  nothing for a caller to resolve. See `### Changed (breaking)` above for the
  new fields and constructors.
- **`Fmp4Demux` tolerates a truncated final box and queues every pending
  `moof`** (#1080, audit r05-W26). `parse_box` on the cut-short tail of a live
  capture or an interrupted recording failed the *whole* demux, so a
  mostly-complete file could not be read at all; the demuxer now treats a
  short tail as the end of the data (its lenient-but-loud contract, the same
  rule `ProgressiveDemux` and the TS demuxers follow) and anything else as a
  real framing error. Separately, a single pending-`moof` slot meant that a
  `moof`, `moof`, `mdat` run — legal per §8.8.4, and what a multi-track CMF2
  segment writes — overwrote the first fragment and silently dropped its
  samples; the pending fragments are now a queue, resolved in order by the
  `mdat` that follows them.
- **`ProgressiveMux` interleaves the `mdat` and promotes a too-large duration
  to a version-1 header** (#1080, audit r05-W24/W25). Every track's samples used
  to be concatenated into a *single* chunk, so the `mdat` held all of track 1
  then all of track 2: starting playback needs the first video *and* the first
  audio sample, which sat roughly the whole video track apart, so `faststart`
  bought nothing for a client that can only read strictly forward. Each track
  is now cut into ~0.5 s chunks and the chunks are merged in start-time order
  (`stsc`/`stco` describe the resulting runs). The merge honours each track's
  own sample order: a `Media` whose decode time steps backwards partway (a TS
  discontinuity, a spliced stream) is legal input, and a flat sort keyed on
  `(start tick, track, sample)` reordered such a track's chunks and rejected it
  with `InvalidInput`; the tracks are now merged head-by-head, so the tick only
  decides the cross-track interleave. Separately, `mvhd`/`tkhd`/`mdhd`
  were always written at version 0, whose `duration` is 32 bits, while
  `set_track_durations` computed 64-bit values — so a duration past `u32::MAX`
  was truncated by the v0 serializer (a 10 MHz track, e.g. a
  [`smooth_parse`](crate::smooth_parse)-sourced timeline, wrapped after 7.2
  minutes; 90 kHz after 13.2 hours), and players reported a wrong length and
  sought wrongly. Each header is now promoted to version 1 when its own
  duration does not fit version 0 (ISO/IEC 14496-12:2015 §8.2.2.2, §8.3.2.2,
  §8.4.2.2). Both changes are verified against ffmpeg's demuxer as well as
  in-crate.
- **`MkvMux` starts a new `Cluster` when a block's relative timestamp would
  leave the signed 16-bit range, and rebases a negative `Cluster` base**
  (#1080, audit r05-W23). A `SimpleBlock` timestamp is a `signed int(16)`
  relative to the `Cluster` timestamp (RFC 9559 §12), but a `Cluster` was only
  split on a *forward* span: a backward step — a TS splice or signalled
  discontinuity, a timestamp wrap, or a large B-frame reorder across a keyframe
  — kept it inside the same `Cluster`, where `debug_assert!` panicked in debug
  builds and the `as i16` cast wrapped in release, putting blocks at arbitrary
  times. The split rule now also fires when `rel` would leave
  `[i16::MIN, i16::MAX]` in either direction. Separately, the base was written
  as `cluster_start.max(0)` while the blocks stayed relative to the *negative*
  `cluster_start` (a `Cluster` `Timestamp` is unsigned, RFC 9559 §13), so every
  block of a `Cluster` whose first PTS was negative — the ordinary result of
  B-frame composition — came out shifted by `-cluster_start` ms. The base and
  the blocks are now measured from the same value. Verified with ffmpeg's own
  Matroska demuxer (`ffprobe -v error` silent on this crate's output) as well
  as in-crate.
- **`tfhd`/`trun` presence bits are derived from their values, and a `tfhd` with
  both base rules is rejected** (#1080, audit r05-W16). Field presence in
  `tfhd`/`trun` is flag-driven (ISO/IEC 14496-12:2015 §8.8.7.1/§8.8.8.1) while
  the values live in `Option`s, and the serializers used the *stored* flags with
  `unwrap_or(0)` — so a builder that set a presence bit without the value wrote
  a `0` (a `base_data_offset` of 0 pointing at the start of the file rather than
  the fragment; a `sample_description_index` of 0 where §8.8.7.1 defines 1 as the
  first entry), and a builder that set the value without the bit silently
  dropped it (a `trun.data_offset` lost, so the samples resolved against the traf
  base instead of where the caller pointed them). The written flags are now
  computed from the values by `TrackFragmentHeaderBox::effective_flags` /
  `TrackFragmentRunBox::effective_flags`; every non-presence bit is kept from
  `flags`. A `trun` sample list that carries a per-sample field on some samples
  and not others is `Error::InvalidInput` — a record is fixed-width, so the field
  is either there for every sample or for none, and neither dropping the supplied
  values nor writing `0` for the gaps is acceptable. A `tfhd` that would carry
  both `default-base-is-moof` and an explicit `base_data_offset` is now
  **accepted and preserved**: §8.8.7.1 says of `default-base-is-moof` "if
  base-data-offset-present is 1, this flag is ignored", so such a box is
  well-formed (merely redundant) and a parser must be liberal — it parses and
  re-serialises byte-identically rather than being rejected. A `trun`'s
  `version` is also derived: any negative
  `sample_composition_time_offset` forces version 1 (§8.8.8.2 makes the field
  signed there), so a negative offset can never be written under version 0 where
  a reader would take it as a huge positive value. The offset is modelled as
  `i64` (`TrunSample::sample_composition_time_offset`,
  `frag_offsets::FragmentSampleRange::composition_offset`), because version 0's
  field is *unsigned*: a legal v0 offset of `0x8000_0001` used to wrap negative
  and be re-serialized as version 1, changing both the bytes and the meaning. A
  negative offset still selects version 1, and one that fits neither field is
  `Error::InvalidInput` rather than a wrap. A zero-sample `trun` keeps
  its own flag bits across a round trip (they describe the records it would
  carry).
- **`sgpd` version 2 round-trips, and a `roll` entry wider than two bytes stays
  opaque** (#1080, audit r05-W15). Version 2 of the Sample Group Description Box
  replaces `default_length` with `default_sample_description_index`
  (ISO/IEC 14496-12:2015 §8.9.3.2) — the same 4 bytes in the same position. The
  parser skipped the field and the serializer wrote it only for `version == 1`
  while keeping `version = 2`, so a parsed v2 box re-serialized 4 bytes short of
  its own syntax and every reader took `entry_count` from the wrong offset.
  `SampleGroupDescriptionBox` gains a `default_sample_description_index:
  Option<u32>` field, written for v2 (and required there — `None` is
  `Error::InvalidInput` rather than a guessed value). Separately, a `roll`
  description is exactly the 2-byte `roll_distance` (§10.6.1); a box whose
  `default_length`/`description_length` was larger was parsed down to those 2
  bytes and re-serialized 2 bytes wide, shrinking the whole entry list. Such an
  entry is kept opaque and byte-exact. (The `subs` v0 `subsample_size` and
  `subsample_count` truncations this warning also named were already fixed under
  #1129.)
- **Init-segment containers keep the child order the file used** (#1080, audit
  r05-W12). Each container serialized its typed children first and every opaque
  one after, so a file whose children did not already match that shape was
  reordered by a parse → serialize — breaking the "every other byte round-trips
  unchanged" claim of `protect_init_segment`. A subtitle track's `sthd` (and a
  data track's `nmhd`), which §6.2.3 places *first* in `minf`, moved after
  `stbl` where strict readers (older Android MediaExtractor, some STBs) reject
  it, and a `moov`'s `pssh` moved after the `trak`s. Each container now carries
  a `ChildOrder` recording the wire sequence, so a parsed container
  round-trips byte-exactly and a rewrite (`protect_init_segment`,
  `ProgressiveMux`) preserves the file's own layout. See
  `### Changed (breaking)` above for the fields and constructors.
- **An init-segment container's children are framed exactly: a `largesize`
  child keeps its siblings, a truncated child is an error, and a declared table
  count the body cannot hold is rejected** (#1080, audit r05-W13). Every
  container loop in `init_segment` read the four-byte `size` itself, clamped
  with `size.min(remaining)` and `break`ed on `size < 8`, so: a child written
  with 64-bit `largesize` (`size == 1`, §4.2) took itself *and every sibling
  after it* out of the parse (a conformant `moov` came back missing its
  `trak`s); a child whose declared size ran past its container was silently
  shortened, so a truncated file parsed "successfully"; and trailing bytes too
  short to hold a header were ignored. One shared `walk_children` now drives
  every container (and `stbl`), using the crate's `parse_box` so `largesize`
  and `usertype` are handled and a declared size past the buffer is
  `Error::BufferTooShort`; a container's own `Parse` locates its body through
  its real header length rather than assuming 8 bytes, so a container written
  in the `largesize` form parses correctly. Separately, `stsc`, `stco`/`co64`,
  `stss`, `stsz` (per-sample form) and `stsd` used to `break` out of their entry
  loops when the bytes ran out and still return `Ok`, so a truncated sample
  table parsed to *fewer* entries than its own `entry_count` declared and the
  tables then silently disagreed with each other; a count the body cannot hold
  is now `Error::BufferTooShort` (`check_entry_count`). A `size == 0` child
  ("extends to the end of the enclosing container", §4.2) is preserved in that
  exact form rather than rewritten with an explicit size. Trailing bytes
  shorter than a child header inside a `moov` are a hard error, matching every
  other container in `init_segment`; `tests/init_segment_sweep.rs` documents
  the policy against the TS/Matroska demuxers, which stop at a short tail
  instead because a live capture legitimately ends mid-packet while a container
  states its own length.
- **A `uuid` child box keeps its 16-byte `usertype` through a parse →
  serialize** (#1080, audit item 1). `parse_box`'s body begins after the *whole*
  header, so the `usertype` of a `uuid` box (ISO/IEC 14496-12:2015 §4.2) was not
  part of it — and every container stores an opaque child's body verbatim. A
  `uuid` child therefore came back 16 bytes shorter than the original and
  shifted, with its extended type replaced by the first eight payload bytes:
  a PlayReady/ISMV `moov`/`trak` uuid, or a Smooth `tfxd`/`tfrf` in a `traf`.
  An opaque child now carries the payload after `size`/`type` with the
  `usertype` included, and remembers whether the wire used the compact or the
  `size == 1` + 64-bit `largesize` header form (§4.2) so that form is
  re-emitted. A child written in the compact form round-trips byte-identically;
  one written with `largesize` also does, because the 8 `largesize` bytes are
  *excluded* from the stored payload (the serializer writes its own). Fixtures:
  `tests/fixtures/mp4/uuid_boxes/` (real ffmpeg output with real `uuid`
  children spliced in — see its README for the generator and the `mp4dump`
  verification), consumed by `tests/uuid_child_roundtrip.rs`.
- **`protect_media_segment` counts a `moof`'s opaque children, refuses a `traf`
  that already carries `senc`/`saiz`/`saio`, and checks its `data_offset`
  shift** (#1080, audit items 2/3). The rebuilt `moof`'s length was
  `header + mfhd + Σ traf`, so any `moof` with a moof-level opaque child
  (`meta`, `pssh`, a `uuid`) under-counted by exactly those bytes and failed the
  consistency check at the end — such a segment could not be protected at all.
  The length and the `saio` running offset are now computed in the same wire
  order the serializer emits. A `traf` that already carries
  `senc`/`saiz`/`saio` (preserved as opaque children) is `Error::InvalidInput`
  rather than being given a second set, since ISO/IEC 23001-7 §12.3 defines one
  `senc` per `traf` — but `senc` is CENC by definition while `saiz`/`saio` are
  generic auxiliary-information boxes other schemes use, so only a pair whose
  `aux_info_type` names a CENC scheme (or is absent, which §8.7.8.2 defines as
  `cenc`) is refused; a differently-typed pair is left in place. The
  `trun.data_offset += delta` shift is a checked `i32` conversion returning
  `Error::InvalidInput` instead of a wrapping add.
  Verified end-to-end against Bento4 `mp4decrypt` and this crate's own
  `CencDecryptor` (`tests/cenc_mux.rs`).
- **`Fmp4Demux` tolerates exactly one thing: a truncated tail** (#1080, audit
  item 4). The fragment walk treated *any* `parse_box` failure as end-of-data,
  so a mid-file `BoxSizeUnderflow` (a declared size of 2..7, or a bad
  `largesize`) silently truncated the demux at the first corrupt box — every
  later fragment vanished with no error. Only a "does not fit the bytes present"
  error on the tail (a declared size past the end of the data, which is how a
  live capture or an interrupted recording ends) ends the walk; anything else
  propagates. That now covers all three variants `parse_box` reports for it —
  a body past the end, a `uuid` usertype cut short, and a `largesize` field cut
  short — since a file cut inside a trailing `uuid` box's header is the same
  truncation as one cut inside a normal box body.
- **A `vp09`/`av01` sample entry keeps its other children (`pasp`, `btrt`,
  `colr`, …)** (#1080, audit item 9). `Vp9SampleEntry`/`Av1SampleEntry` modelled
  only their config box, so a parse → serialize dropped 46 bytes of every real
  ffmpeg AV1/VP9 file — caught by the new sweep over every committed init
  segment (`tests/init_segment_sweep.rs`, 63 files) rather than by the two
  fixtures W12 checked. Both now carry `children: Vec<SampleEntryChild>`
  recording the entry's child wire order: emitting the config box first
  unconditionally reordered an entry whose `pasp` preceded it, the walk
  `break`ed on a child with `size < 8` (dropping every sibling after it), and a
  truncated child was kept with a size field longer than its bytes — it now
  comes from the same `walk_children`/`parse_box` pair every container uses and
  reports `Error::BufferTooShort`.
- **`MkvMux` writes monotonic `CueTime`s and errors instead of clamping a block
  time** (#1080, audit item 7). A backward timestamp jump opens a new `Cluster`,
  so a later cluster's keyframe can sit at an *earlier* presentation time than
  an earlier one; emitting it left the `Cues` index unsorted and defeated a
  player's binary search. Every keyframe still gets a cue — one whose time would
  step backwards is emitted at the previous cue's time rather than dropped, so
  its cluster stays reachable by seeking (RFC 9559 §20 requires the sort, not
  uniqueness). The cluster builder also returned a silently `clamp`ed block
  timestamp in release builds (a `debug_assert` does nothing there), writing the
  block at an arbitrary instant; an out-of-range relative timestamp is now
  `Error::InvalidInput`, and the builder returns `Result`.
- **Fragment sample ranges are bounded to an `mdat` payload and to the file
  length** (#1080, audit r05-W7 follow-up). A `trun` could previously name any
  offset in the file, so a hostile fragment read `moov`/`moof` bytes as sample
  data and returned `Ok`; many runs aimed at the same offset could also amplify
  work without bound. Each range must now lie inside some `mdat` payload, the
  total bytes handed out is capped at the file length, and `tfdt`/dts/pts
  arithmetic is checked (an over-range `tfdt` or a dp overflowing addition is
  `InvalidInput` rather than a wrap).
- **`CencDecryptor`'s second `decrypt` pass no longer `expect`s its lookups, and
  `assert_ctr_ranges_disjoint` skips zero-block samples** (#1080, audit r05-W5/W3
  follow-ups). The decrypt pass re-resolved the track record and content key with
  `.expect("pass 1 proved ...")`; those cannot fire today, but a panic on a path
  driven by untrusted input is not acceptable, so they now return the same
  `InvalidInput` errors pass 1 would, plus a sample-count recheck. A `cenc`
  sample that protects no whole block consumes no keystream and is skipped
  rather than given a degenerate zero-width counter range.
- **H.264/E-AC-3 Sample-AES verified against independent oracle fixtures, and
  the wrong transcription/claim corrected** (#1080, audit r05-W1/W2). The
  repo's own `docs/drm/hls-sample-aes.md` §3.4 contradicted §11 on H.264 CBC
  chaining, and `sample_aes/README.md` claimed the E-AC-3 IV was carried across
  syncframes. `transmux/tests/fixtures/ORACLES.md` now pins both with three
  independent implementations (a pycryptodome reference, Bento4 `mp4hls`, and
  ffmpeg's decryptor): H.264 chains across a NAL's encrypted blocks with the IV
  reset per NAL; E-AC-3 resets the IV at every syncframe. See the two
  `### Changed (breaking)` entries above for the resulting code changes.
  `tests/sample_aes_h264_oracle.rs` consumes the H.264 fixtures and
  `sample_aes::tests::eac3_reset_per_syncframe_matches_oracle` the E-AC-3 ones.
- **`cenc::SampleEncryptionBox::parse_body` rejects `senc` flag `0x000001`**
  (#1080, audit r05-W9). That flag (PIFF 1.1 / ISO/IEC 23001-7:2012 v0) inserts
  a 20-byte `AlgorithmID(24) IV_size(8) KID(128)` triple before `sample_count`
  to override the `TrackEncryptionBox`. The parser read such a box as if the
  flag were clear, consuming the override bytes as the count and the first
  entries — garbage IVs, or a spurious length error. It is now
  `Error::UnsupportedFeature` (this crate decrypts with the track-wide `tenc`
  only). The `senc` subsample-count and `saio` v0 offset truncations this
  warning also named were already fixed under #1129 (`FieldOverflow`).
- **Fragment addressing (§8.8.7/§8.8.8) is one shared resolver, and both
  `CencDecryptor::demux` and `Fmp4Demux` use it** (#1080, audit r05-W7). Both
  read every run as `moof_off + trun.data_offset`, so `tfhd.base_data_offset`
  (explicit-base files, e.g. some PIFF/Smooth-derived CMAF) was ignored, a
  `trun` with no `data_offset` sliced bytes from the start of the `moof`, and a
  later run's `data_offset` was measured from the previous run's end instead of
  the traf base — any of which CTR then "decrypted" to garbage and returned
  `Ok`. `transmux::frag_offsets::sample_ranges` now resolves the base
  (`tfhd.base_data_offset` when present, else the `moof` start under
  `default-base-is-moof`, else the running `moof` start / end of the preceding
  fragment's data), makes a `trun`'s `data_offset` relative to that **traf
  base** (never the previous run's end), continues a run with no `data_offset`
  after the previous run, and bounds every sample range to an `mdat` payload
  with a total cap of the file length. Verified against all nine
  `tests/fixtures/cenc_frag_layouts/enc_*.mp4` (three base rules × explicit /
  implicit multi-trun) for **both** the video and the audio traf
  (`all_layouts_decrypt_to_clear_video_samples`, `all_layouts_decrypt_audio_to_clear`),
  and against the clear variants
  (`tests/fmp4_frag_layouts.rs::all_clear_layouts_demux_identically`). The
  omit-tfhd-offset rule has no independent decryptor (see the fixture README).
- **`CencDecryptor`'s `Decrypt::decrypt` is now atomic across the whole `Media`**
  (#1080, audit r05-W5). It decrypted each track as it walked, so a missing
  content key on track 2, a sample-count mismatch, or a malformed subsample map
  on sample `k` returned `Err` *after* track 1 (or samples `0..k`) had already
  been decrypted in place — the caller could not tell which samples were now
  plaintext, and under AES-CTR a retry XORs those samples back to ciphertext.
  Every track is now validated in full (record match, `tenc`, key, sample count,
  and each sample's content-dependent preconditions) in a first pass that
  mutates nothing; a rejected call leaves `media` byte-identical.
- **`CencDecryptor::demux` skips a protected track whose original format is not
  AVC instead of failing the whole file, and reports it** (#1080, audit r05-W6).
  A typical CENC asset is `encv`+`enca`; the old code `return Err`ed on the first
  non-`avc1` protected track, so such a file — and any protected HEVC file —
  could not be demuxed at all. The AVC track is still reconstructed, and a
  `Media` narrowed to a skipped track still fails clearly in `decrypt`. The skip
  is recorded in `Media::skipped` (the dropped track's original-format FourCC
  and the reason) — including an **unprotected** track sitting next to a
  protected one, which was previously dropped with no record at all.
- **`StreamingTsDemux` drops, rather than delivers, an access unit whose
  reassembly spanned a continuity-counter gap, and discards legal §2.4.3.3
  duplicate packets before reassembly** (#1080, audit r04-W49). The demux
  already *reported* a CC gap but then handed the truncated access unit on: the
  IR cannot tell a complete PES from one missing 184-byte payloads, so every
  downstream muxer wrote the corruption out (a `payload_unit_start` clears the
  mark, because it begins a new, intact unit). Legal duplicates — a repeat
  with the same `continuity_counter` and identical bytes bar the PCR field —
  were likewise detected only for the CC check and then fed to the reassembler
  anyway, duplicating that packet's payload into the access unit under
  construction; they are now skipped. The duplicate comparison covers the
  whole packet (not just `pkt.payload`, which excludes a legally re-encoded
  adaptation field) via the shared `broadcast_common::ts_dup` helper, and a
  PID's continuity baseline is reset when its track is removed, so a re-added
  PID is not judged against the pre-removal sequence. A unit is damaged by
  exactly two things — a CC gap on one of its *own* packets, or a bounded PES
  arriving short of its declared length — and never by a
  `discontinuity_indicator` alone, which would drop the intact last unit of the
  old segment at every seam. **Visible behaviour change:** a stream with genuine
  CC gaps now yields fewer samples than before — the damaged ones are gone
  rather than corrupt — and an unbounded (video) unit ended by a counter jump is
  still delivered, so a genuinely truncated tail can pass through; H.222.0
  offers no length to distinguish the two.
- **`StreamingTsDemux` interpolates a PES packet that carries no PTS/DTS
  instead of reusing the previous access unit's stamps** (#1080, audit r04-W48).
  `PTS_DTS_flags == '00'` is legal (ISO/IEC 13818-1 §2.4.3.7) and the stamps
  are only required periodically, so a video PID's access units routinely
  alternate stamped/unstamped. Both were given the same `(pts, dts)`, so the
  one-behind duration rule gave the earlier one `duration = 0` and the next
  stamped access unit absorbed the whole gap — zero-duration samples plus one
  long one, which breaks playback cadence and trick modes. Each unstamped
  access unit now continues the timeline by the measured frame period (the
  span between the last two stamped access units, divided by the access units
  it covered), exactly the T-STD derivation §2.4.2.6 permits. Until a period
  has been measured the previous stamps are still reused, since there is
  nothing to interpolate from.
- **`StreamingTsDemux` rebases the decode timeline at a signalled
  discontinuity** (#1080, audit r04-W50). `discontinuity_indicator`
  (ISO/IEC 13818-1 §2.4.3.5) marks a sample of a *new system time clock* for
  the program — a splice, an encoder switch, a remultiplex — and the new base
  routinely begins *below* the old one. The 33-bit unroll cannot tell that
  from a genuine backward jump within the range, so `dts` went non-monotonic
  and violated the IR's "samples in decode order with a non-decreasing
  absolute dts" invariant. Per §2.4.3.5 the indicator is carried on the
  program's `PCR_PID`, so exactly that program's elementary streams lift their
  timelines — a multi-program multiplex leaves the other services alone. The
  lift is non-negative (a forward splice is never pulled back, so a real gap
  survives), lands one nominal frame period past the last stamp emitted, and
  uses the median of the last five plausible inter-access-unit steps capped at
  one second, so jitter (3754/3753 at 23.976 fps) cannot consume the flag and an
  8-second splice cannot become the frame period. Decode order stays monotonic
  across the seam while the intervals inside each time base are untouched. The
  same trajectory also applies at the final `finish()` flush, where a bounded
  PES short of its declared length is now dropped rather than delivered.
- **`StreamingTsDemux` reports a changed in-band codec config as
  `DemuxEvent::TrackUpdated`, and maps ADTS `channel_configuration` through
  ISO/IEC 14496-3 Table 1.19** (#1080, audit r04-W51, extending W16). Codec
  config recovery was single-shot for the life of the stream, so a mid-stream
  SPS/PPS or AAC-config change (SD↔HD ad break, re-encode, multiplex
  reconfiguration) left the track labelled with the original `avcC`/`hvcC`/
  `esds` and the init segment describing a stream that had stopped being sent.
  An AVC/HEVC/AAC track whose parameter sets or ADTS header actually change now
  re-probes on that access unit and emits `TrackUpdated` with the same
  `track_id` — and emits nothing for an *unchanged* repeat, which encoders send
  routinely (often on every keyframe). Separately, `CodecConfig::Aac`'s
  `channel_count` came from the raw `channel_configuration` field: configuration
  7 is **eight** channels (7.1) and 0 means the mapping is in-band via a
  `program_config_element`, which this crate does not decode, so 7.1 was
  reported as 7 channels and a PCE-signalled stream as 0. The field now maps
  through Table 1.19, with `0` marking "not derivable" rather than a fabricated
  count, matching `flv_stream`'s existing convention.
- **`ts_demux`'s codec-config probes scan only the newest access unit**
  (#1080, audit r04-W47). Every probe but H.264/HEVC re-walked the whole
  accumulated backlog — with byte-by-byte sync scans inside — on *every*
  incoming access unit, so a PID whose config never resolves (garbage payload,
  or an ADTS `sampling_frequency_index` of 13/14) grew toward
  `MAX_PROBE_BACKLOG_BYTES` while re-scanning it each time: O(n²) byte probes
  per PID, ~4 × 10⁹ over a 2 000-access-unit PID, enough to pin a core on a
  hostile PMT. All probes now scan the newest access unit only, and the
  H.264/HEVC probes carry parameter sets seen earlier forward explicitly, since
  an encoder may put SPS and PPS in separate access units. Guarded by a
  complexity test that counts probes at the scan site and asserts they stay
  proportional to the input length.
- **`uri` resolution is total over `&str`: no input panics.** `remove_dot_segments`
  sliced at byte 1 to skip a leading `/`, which panicked on any multi-byte first
  character — reachable straight from `resolve` with a non-ASCII relative
  reference (`"g:é"`) or base. It walks character boundaries now, and every other
  slice in the module is derived from a `find`/`rfind` result rather than a raw
  byte offset. Covered by a path/query/fragment sweep of all 19 607 strings up to
  length 5 over `["a", "/", ".", "é", "?", "#", ":"]` plus empty, very long, and
  multi-byte-in-every-position cases (#1079).
- **`uri` rejects a control character or whitespace in a resolved reference**
  (`try_resolve`/`try_resolve_segment`, and `dash_parse`'s
  `Mpd::try_resolve_segment_url`). A raw CR/LF in a URL has no meaning in
  RFC 3986 and lets a manifest smuggle a second request line into anything that
  later writes an HTTP request from the resolved value; a percent-encoded
  `%0D%0A` is still allowed through, since that is data rather than structure
  (#1079).
- **`dash_parse::Mpd::parse` keeps `@duration` and `SegmentTimeline` exclusive in
  both directions.** A child `SegmentTemplate` that declares its own `@duration`
  used to inherit the parent's `SegmentTimeline` unconditionally, so the
  effective template carried both — contradicting §5.3.9.4.4 and silently
  cancelling the child's own `@duration` (#1079).
- **`ps_demux` keeps a decided substream's bytes when its header is unusable.**
  A mid-stream packet with `first_access_unit_pointer == 0` together with a
  non-zero `number_of_frames` (or a pointer past the payload) was skipped even
  after the substream had been identified as AC-3, dropping real audio — and a
  muxer writing pointer 0 on *every* packet never classified at all and produced
  no track. The consistency check now applies only while the substream is
  undecided, and classification falls back to the payload's own start when the
  pointer is 0 (#1079).
- **`ps_demux`'s `first_nal_offset` folds back every leading zero byte**, not just
  one, so a run longer than the 4-byte start code (`00 00 00 00 01`, legal
  padding) no longer shifts every access-unit offset. The splitter's
  slice-equality check that catches such a shift is a returned error rather than
  a `debug_assert!`, so release builds cannot emit mis-stamped samples silently
  (#1079).
- **`klv::UasLocalSet`'s tag-1 Checksum is now the MISB ST 0601 running 16-bit
  sum, not a CRC-16/CCITT** (audit r04-W10). ST 0601 §5.5 specifies "a running
  16-bit sum through the entire LDS packet", §7.1 repeats "Lower 16-bits of
  summation", and the standard's change history records removing the earlier
  "CRC-16" wording because tag 1 "represents a checksum and not a cyclic
  redundancy check" (`checksum_bcc16` replaces the `crc16_ccitt` helper, which
  is removed). Every packet `serialize_with_checksum` emitted carried a
  checksum a conformant receiver rejects, and `verify_checksum` disagreed with
  every real UAS stream — while the crate's own round-trip tests stayed green,
  because both sides shared the wrong algorithm. Now pinned to three published
  third-party packets (sensor-geometry, checksum-only and unknown-tag fixtures
  from the jmisb test suite, each carrying its own expected tag-1 bytes), which
  the old algorithm fails by a wide margin, plus a byte-for-byte
  serialize-against-real-packet check (#1079).
- **`webm_demux::WebmDemux` now unlaces laced blocks instead of rejecting the
  file**, and the block's frames are spaced by a rounded `DefaultDuration`
  rather than a truncated one (audit r04-W40). Lacing is how one Matroska block carries several
  frames of a track, and mkvmerge — the most common Matroska muxer — laces
  audio by default, so "lacing is not supported" made every ordinary
  mkvmerge-muxed MKV/WebM fail outright. All three encodings of RFC 9559 §12
  (Xiph, EBML and fixed-size) are now decoded, and each frame becomes its own
  sample timed from the block's timestamp at the track's frame cadence: frame
  `i` of a block starts `i` frame durations after the block's own time, where
  the duration is `DefaultDuration` when the TrackEntry declares one (converted
  from nanoseconds with rounding, not by truncating to whole milliseconds first)
  and is otherwise derived from the interval to the next block. Matroska times the
  *block*, not the frames inside it, so giving every frame of a laced block the
  block's single timestamp would leave all but the last with duration 0; a
  declared lace size that overruns the payload, an EBML size that goes negative,
  a zero-length frame, and a fixed-laced payload that is not a whole multiple of
  the frame count are each `Err` rather than a silent truncation. A laced block
  may carry at most the format's 256 frames (the count byte is `FrameCount - 1`,
  RFC 9559 §12), and a frame declared with zero length is rejected rather than
  emitted as an empty sample (#1079).
- **`ps_demux::PsDemux` demultiplexes `private_stream_1` substreams** (audit
  r04-W19). A 0xBD PES is a container: it multiplexes independent substreams
  (AC-3, DTS, LPCM, subpictures), told apart by the `substream_id` byte at the
  front of each payload's substream header. Elementary streams are now keyed by
  `(stream_id, substream_id)`. A substream is classified from the syncword at
  its header's `first_access_unit_pointer`, not at the payload's first byte, so
  a stream whose first packet resumes a frame an earlier packet opened is still
  carried (its leading partial-frame tail is dropped, since no syncword scan can
  start mid-frame); the DTS (`0x88..=0x8F`), LPCM (`0xA0..=0xA7`) and
  subpicture (`0x20..=0x3F`) substream ids are skipped explicitly. A header
  whose pointer names bytes the packet does not carry is rejected only while the
  substream is still undecided: once it is known, every byte it carries is frame
  data and is kept, so a muxer that writes `first_access_unit_pointer = 0`
  (meaning "no access unit starts here", which legal muxers do write) loses
  nothing and still yields its track — so a VOB
  with two audio languages plus subpictures no longer comes out as one track of
  interleaved, unrelated bytes. Pinned to real fixtures
  `fixtures/ps/ffmpeg-mpeg2video-2xac3.ps` (0x80 and 0x81, each 58 syncframes
  per ffprobe) and `ffmpeg-mpeg2video-2xac3-with-dts.ps` (a DTS substream
  alongside them, which ffprobe reads as a `dts` stream) (#1079).
- **`ps_demux::PsDemux` splits AC-3 frames by each syncframe's declared
  length** (audit r04-W20). The old scanner split at every raw `0x0B77` byte
  pair, but that 16-bit value occurs inside AC-3 payload by chance, so real
  frames were cut in half and each half stamped with a full 1536-sample
  duration — corrupt access units plus A/V drift against the PES stamps. The
  shared `ac3::split_ac3_syncframes` (length-driven from `frmsizecod`,
  ETSI TS 102 366 Table 4.13) is now used (#1079).
- **`ps_demux::PsDemux` splits H.264 access units without requiring an
  access-unit delimiter** (audit r04-W21). An AUD is optional in H.264 and is
  absent from most PS captures, so an AUD-only split left such a stream as a
  single "access unit" spanning the whole file — one sample, one IDR flag. The
  shared `au::AccessUnitSplitter` (which decides a boundary from
  `first_mb_in_slice`, H.264 §7.4.3, as well as from an AUD) is now used.
  Pinned to a new real AUD-free fixture, `fixtures/ps/ffmpeg-h264-noaud.ps`,
  whose 75 pictures all arrive in one PES packet. The access-unit offset walk
  now anchors at the stream's first start code rather than at byte 0, so bytes
  before it no longer make every unit mismatch and lose the whole track, and a
  splitter resource-limit rejection (`AccessUnitSplitter`'s 64 MiB cap) is
  returned as an error instead of being swallowed into "no video track" (#1079).
- **`splice::concat`/`splice_insert` match tracks by `track_id` independently of
  document order** (audit r04-W35). The #992 injectivity fix still took the
  first id match and fell back to the *positional* index when that match was
  already claimed, so with `a` ids `[2, 2, 3]` and `b` ids `[2, 3, 2]` — three
  same-codec, same-timescale audio tracks — `a[1]` and `a[2]` were cross-wired.
  Nothing in the compatibility check can see that (the codecs and timescales
  match), so language tracks were silently swapped. The search now takes the
  first *unclaimed* id match (#1079).
- **`splice::splice_insert` cuts every track at the same absolute instant**
  (audit r04-W36). The video cut is the snapped sample's absolute `dts`, but the
  non-video cut was found by comparing each sample's position past *its own*
  track start against the video-*relative* offset, and by accumulating
  `duration`s from 0. Both differ from the video's instant whenever the tracks
  do not share a start or a track is not gap-free (a late audio join, a
  per-fragment `tfdt` reseed, a dropped run of audio), so the audio cut landed
  at the wrong wall-clock position and A/V desynchronised at the ad boundary.
  `sample_index_at_absolute_dts` now compares the video's absolute cut instant
  against each sample's own absolute `dts` (in `i128`, so a rebased timeline's
  negative decode times sort before the cut rather than wrapping), rescaled into
  the track's timescale with `div_ceil` — flooring the rescale picked a tick
  *before* the video's instant and cut the audio one sample early. The module doc
  already promised every boundary read was absolute (#1079).
- **`ac3::split_ac3_syncframes_resyncing` resynchronises after an unparseable
  frame** (audit r04-W20 review). A splitter that stops at the first frame it
  cannot parse discards the whole remainder of the stream to recover from a
  single corrupt or truncated frame, which is exactly what a real capture
  contains. It also returns each frame's `(start, end)` byte range, because
  after a resync the frames are no longer contiguous from offset 0, so a caller
  reconstructing offsets by summing lengths slices mid-frame. `ps_demux` uses
  both; the plain `split_ac3_syncframes` keeps its stop-at-first-bad behaviour
  (#1079).
- **`dash_parse::Mpd::parse` merges `SegmentTemplate` attributes across the
  `Period` > `AdaptationSet` > `Representation` chain instead of replacing
  whole elements** (audit r04-W11). ISO/IEC 23009-1 §5.3.9.1 makes segment
  information hierarchical, with a lower level overriding only the attributes
  it declares; the standard's own Annex G examples depend on it (G.13 gives
  each `Representation` a `SegmentTemplate` carrying only `@initialization`
  and inherits `@media`/`@timescale`/`@duration`/`@startNumber` from the
  `AdaptationSet`). Previously such a Representation came back with
  `timescale = 1` (the spec default) and no `initialization` or timeline, so
  every segment time and URL was wrong or missing. A `Period`-level
  `SegmentTemplate` was skipped entirely as an opaque element. `@duration` and
  `SegmentTimeline` are mutually exclusive (§5.3.9.4.4), so the effective
  template never carries both: a child that introduces a timeline does not also
  inherit the parent's `@duration`, and a child that declares its own `@duration`
  does not also inherit the parent's `SegmentTimeline` (the direction that was
  missing — the timeline used to be taken from the parent unconditionally, which
  silently cancelled the child's own `@duration`). Covered by a derived
  `fixtures/dash/manifest-inheritance.mpd` (the real ffmpeg fixture
  restructured into the G.12/G.13 shapes; see `fixtures/dash/README.md`), whose
  resolved URLs are checked against the committed segment files (#1079).
- **`dash_parse` resolves `BaseURL` chains and segment URLs** (audit r04-W11).
  `Mpd::try_resolve_segment_url` is the same as `resolve_segment_url` but
  returns `None` for a reference carrying a control character or whitespace.
  `BaseURL` is inherited over the same `MPD` > `Period` > `AdaptationSet` >
  `Representation` chain as segment information (§5.6.5), and several
  `BaseURL` children of one element are *alternates* consulted in order, so the
  first non-empty one is kept — a later `BaseURL` no longer replaces an earlier
  one, and an empty one no longer clears the value. `Mpd`/`Period`/
  `AdaptationSet`/`Representation` each report their own level's value
  (`Mpd::base_url` is new) and `Mpd::base_url_chain`/`Mpd::resolve_segment_url`
  walk the chain, resolving each reference by RFC 3986 §5 (new `uri` module:
  `parse`/`resolve`/`merge`/`remove_dot_segments`, verified against the
  standard's own §5.4.1 and §5.4.2 example tables) (#1079).
- **`rtp::RtpDepacketiser` no longer fails the whole input on ordinary loss**
  (audit r04-W31). An FU-A continuation fragment with no preceding start fragment
  — a mid-stream capture, or one lost start packet — made every packet after it
  unreadable, because the run returned `Err` for the entire input; it is now
  skipped, so the rest of the stream is recovered, which is what the streaming
  `RtpStreamDepacketiser` already did (`DamagedAccessUnit`).
- **`rtp::RtpDepacketiser` now uses RTP sequence numbers to detect loss inside a
  reassembly run, and runs that check *before* closing the access unit.** A lost
  fragment in the middle of an FU-A NAL, or between two single-NAL/STAP-A packets
  of one access unit, previously produced an access unit concatenated across the
  hole — a NAL silently missing its middle, or an AU missing a NAL, emitted as if
  intact. RFC 3550 §5.1 makes each packet's sequence number one more than the last,
  so a discontinuity now drops the AU it damaged (including any open FU-A run, so
  its continuation cannot join a later one) and reassembly resumes at the next
  NAL/AU, matching what the streaming `RtpStreamDepacketiser` already does per
  RFC 3550 §A.1. Because the check precedes the timestamp flush, a packet lost at
  the *end* of an access unit costs exactly that one AU — the previous placement
  attributed the gap to the *next* AU and discarded its first packet too, so one
  lost packet cost two AUs.
- **`rtp::RtpDepacketiser` reassembles RFC 3640 §3.2.3.1 access-unit
  fragmentation.** An AAC AU larger than the payload budget arrives split over
  packets sharing one timestamp, each AU-header declaring the AU's full size; the
  fragments are now concatenated into the access unit, and a run is accepted only
  when it is **contiguous and exactly `AU-size` bytes** — a run with a hole, or one
  whose bytes overrun the declared size (duplicated fragments), is dropped rather
  than emitted truncated or oversized, since the header states the AU's full size
  (§3.2.3.2). Previously such a packet was
  rejected as `BufferTooShort` and the whole stream failed. Covered by a new real
  5.1 640 kb/s fixture, `fixtures/ts/aac-5_1-640k.ts`, whose frames are 785 bytes
  — far over the stereo fixtures' access units that fit one packet.
- **`RtpDepacketiser` reads that timestamp back as `pts`, and reconstructs `dts`
  from the whole stream.** RTP carries no decode time, so for a reordered stream
  the wire cannot supply one. If the stream's presentation order *is* its wire
  order, `dts == pts` exactly (keeping the earlier behaviour, and the only correct
  answer for a variable-frame-rate or lossy stream). Otherwise the decode timeline
  the decode instants are the *presentation* instants re-laid in wire (decode)
  order — every frame is presented exactly once, so only the assignment changes —
  delayed by the smallest constant that keeps `dts <= pts` at every sample. That
  is exact rather than an estimate: a real 23.976 fps stream at the 90 kHz clock
  presents frames 3753 or 3754 ticks apart, so any fixed "frame period" (their
  greatest common divisor is 1) would give every duration 1-2 ticks, 2000x too
  short. Both series are then translated so the track starts at decode time 0,
  which keeps `dts` non-decreasing and every composition offset (`pts - dts`)
  non-negative, as the IR and every container writer require. The delay is found
  in one sort rather than per-sample scan (the previous shape was O(n²), ~4e9
  comparisons on a 90 000-access-unit batch input). One origin **for the whole
  `Media`** is then applied to every track, so a reordered video track and a
  non-reordered audio track keep one relationship rather than one being zero-based
  while the other stays absolute. A non-final frame never gets a zero duration
  (two access units stamped with the same instant give the timeline a zero step,
  and every writer in this crate rejects a zero-duration sample): it falls back to
  the last non-zero step, and to 1 tick if there has not been one. A stream fed
  through this path previously produced a non-monotonic `dts` sequence.
- **`rtp::RtpDepacketiser` gains `poll_timing_warning`, `dropped_timing_warnings`
  and the public `MAX_TIMING_WARNINGS`, and `rtp::RtpTimingWarning` is a new public
  type** reporting a decode timeline the wire did not state itself
  (`ReorderedPresentationTimestamps`). Drain the queue after each `unpackage` call.
  It is **bounded** (RTP is untrusted input, and one warning per access unit over a
  long session is an unbounded-allocation vector): the oldest entries are dropped
  and the number dropped is reported by `dropped_timing_warnings`, so the loss is
  never silent.
- **`rtp_stream::RtpLossEvent` gains the `NonMonotonicTimestamp` variant**, raised
  when an access unit's wire (presentation) timestamp is earlier than a previous
  one's. The streaming depacketiser keeps its documented low-delay model
  (`dts == pts`) — it cannot know the frame period from a live stream — so a
  reordered source now reports the assumption's failure instead of emitting a
  silently non-monotonic `dts`. The variant is additive for matching callers
  (`#[non_exhaustive]`).

- **`rtp_sdp::avc_config_from_sps_pps` fills in the High profile `chroma_format`/
  `bit_depth_luma_minus8`/`bit_depth_chroma_minus8` trailer for a High profile SPS**
  (audit r04-W33). ISO/IEC 14496-15 §5.3.3.1.2 makes those record fields
  conditionally present on `AVCProfileIndication ∈ {100, 110, 122, 244}`, and the
  values are already in the SPS the function parses — but the record was always
  built without them, so every `avcC` recovered from an RTSP session (RTSP cameras
  are overwhelmingly High profile) was four bytes short of what ffmpeg writes for
  the same stream, and a strict reader mis-parses the record. Verified byte-exact
  against ffmpeg 8.1's own `avcC` for `fixtures/ts/h264/high.ts`, committed as
  `tests/fixtures/rtp/high-ffmpeg.*` with provenance. A non-High profile still
  omits the trailer, as the spec requires: the gate is the record's own
  `{100, 110, 122, 244}` set, not the wider H.264 Table A-1 list the SPS *parser*
  uses, so a profile in the wider list but outside that set is left alone. An SPS
  for one of the four that cannot be decoded is now an error rather than a record
  silently written without the trailer — the trailer's values exist nowhere else,
  and a record missing them is the very misparse this fixes.
- **The `samplingFrequencyIndex` → Hz table (ISO/IEC 14496-3 Table 1.10) now has
  one copy, `aac_asc::SamplingFrequencyIndex::table_hz`.** `rtp_sdp`, `flv`
  (`asc_rate_hz`), `dash` and `smooth` each carried their own transcription of the
  same 13 values, so a correction to one would silently leave the others wrong
  (audit r04-W33).
- **`rtp_sdp::aac_config_from_asc_bytes` derives the channel count from
  ISO/IEC 14496-3 Table 1.19 instead of copying the raw `channelConfiguration`
  field** (audit r04-W33). Configuration 7 was reported as 7 channels (it is 8 —
  7.1), and configuration 0 (the mapping is carried in-band by a
  `program_config_element`) was reported as 0 channels rather than as undetermined;
  both now go through the same `channel_count()` mapping the FLV demuxers use, with
  0 marking "not derived". This is the fourth copy of that table removed — the
  values come from `aac_asc`, not a local duplicate.

- **`RtpStreamDepacketiser` now applies RFC 3550 §A.1's `update_seq` verbatim as its
  sequence-number gate** (replacing the first attempt at this in the same release).
  A stray packet far behind the live run is discarded and merely arms `bad_seq`;
  only a *second* packet at the new numbering — §A.1's "two sequential packets ...
  just re-sync" — restarts the source. The previous "abandoned range" version was
  wrong: one stray packet moved the baseline, the range was never cleared once set
  (so the live run was itself discarded when it climbed past the bound, up to the
  original 32 768-packet blackout), and only an SSRC change could clear it. A
  confirmed resync is now §A.1's `init_seq` and nothing else carries over, so a run
  of any length — past the old numbering, past the 16-bit wrap, and repeated seeks —
  is delivered in full. See `transmux/docs/rtp/rtp-sequence-validation.md` for the
  arm-by-arm mapping and the one deliberate divergence (a bounded reorder buffer,
  which §A.1 has no equivalent of).
- **`RtpStreamDepacketiser` resyncs when the sender moves its RTP sequence number
  backward on the same SSRC, instead of discarding every following packet** (audit
  r04-W32). A large backward jump is a seek (a new `RTP-Info` `seq` after an RTSP
  `PLAY`) or a source restart that kept its SSRC, not a late packet: the old
  classification treated it as a duplicate and never moved the expected number, so
  the stream went dark until the 16-bit counter came back round — up to 32 768
  packets, tens of seconds of media, with no loss signal. RFC 3550 §A.1's
  `MAX_MISORDER` now bounds which side of the seam is "misordered" and which side
  is abandoned, and a resync drops the access unit straddling the seam and
  re-establishes the track's timestamp origin rather than measuring a delta across
  two unrelated random RTP clock origins (§5.1). No `RtpLossEvent` is raised: the
  sender renumbered, nothing was lost. See
  `transmux/docs/rtp/rtp-sequence-validation.md` for the updated account of where
  this follows RFC 3550 §A.1 and where it deliberately diverges (#1079).

- `rtmp::read_chunks` applies a fmt-3 chunk's inherited timestamp **delta**: a fmt-3 chunk that
  begins a new message advances the running timestamp by the delta the preceding fmt-1/fmt-2 chunk
  declared (§5.3.1.2.4), rather than reusing the previous timestamp. `librtmp`/`ffmpeg` send
  constant-rate audio exactly this way, so every such message previously carried an identical DTS
  and `FlvDemux` saw zero-duration samples (#1079, audit r04-W26).
- `rtmp::read_chunks` honours an **Abort Message** (§5.4.2), discarding the in-progress message on
  the chunk stream the message names. Ignoring it left the abandoned bytes buffered, so the sender's
  re-sent message was appended to them and emitted as one misframed message (#1079, audit r04-W26).
- `AmfValue::parse` caps AMF0 object/ECMA-array nesting at `MAX_AMF0_DEPTH` (32), returning
  `RtmpError::Amf0TooDeep` past it. Each nesting level costs about four wire bytes, so an
  unconstrained recursive descent over peer-supplied command bytes (a ~1 MiB `connect` reaches
  ~250 000 frames) overflowed the stack, which aborts the process and cannot be caught (#1079,
  audit r04-W27).
- `StreamingFlvDemux::feed` consumes a tag whose codec config is structurally corrupt instead of
  leaving it buffered. Before, a single bad ASC/avcC stayed in the pending buffer, so every later
  `feed` re-parsed the same bytes and returned the same error — the live ingest could never get
  past one bad tag (#1079, audit r04-W13).
- `StreamingFlvDemux` emits `DemuxEvent::TrackUpdated` only when a re-sent sequence header's bytes
  actually **differ** from the config the track currently carries. Encoders commonly repeat the
  header on every keyframe; reporting each repetition as a change would make a consumer rebuild its
  init segment hundreds of times a minute for a config that never changed (#1079, audit r04-W13).
- `StreamingFlvDemux::feed` now memmoves the pending buffer once per call rather than once per tag.
  Each completed tag used to `drain(0..total)` from the front, so a single `feed` of a large chunk
  moved the whole remainder once per tag — quadratic in the tag count (8 MiB of small audio tags,
  ~20 000 of them, moved ~80 GB). The bytes are now parsed at a moving cursor with one drain at the
  end (#1079, audit r04-W14).
- The FLV demuxers no longer truncate AVC dimensions into the IR's `u16`: `config.width as u16`
  folded a 65 536-wide SPS down to 0, producing a track that misdescribed its own coded size. A
  dimension that does not fit is now `FlvError::Codec(Error::InvalidInput)` (the `#997` class,
  previously fixed only in `ts_demux`) (#1079, audit r04-W38).
- `aac_asc::ChannelConfiguration` gains `channel_count() -> Option<u16>`, the ISO/IEC 14496-3
  Table 1.19 channel count for the configuration, and both FLV demuxers use it for
  `CodecConfig::Aac::channel_count` instead of the raw field. The field is an index, not a count:
  configuration 7 is **8** channels (7.1) but was reported as 7, and configuration 0 was reported
  as 0 — which reads as "no channels" for what is really "not determined". For 0 (the mapping is
  carried in-band by a `program_config_element`, which this crate does not decode — a PCE is a raw
  data element, not part of the ASC) and for the reserved values 8..=15, the count is now the
  documented `AAC_CHANNEL_COUNT_UNKNOWN` placeholder, `0`, meaning "not derived": the same
  convention `ts_demux` already uses for MPEG-H. The distinction is that the placeholder is now
  deliberate and named, not whatever the configuration index happened to be (#1079, audit
  r04-W16).
- `FlvDemux` gains `last_walk_was_truncated()`, reporting whether the most recent
  `unpackage` call ended on a truncated final tag. That case was already tolerated (the tags before
  it are returned) but silently — indistinguishable from a intact file. The returned `Media` is
  unchanged; the flag lets a caller warn, trim or re-fetch. A tag that is complete except for its
  redundant trailing `PreviousTagSize` (Annex E §E.4.1) is *not* reported: it is kept, and the walk
  ends cleanly (#1079, audit r04-W15).
- `FlvDemux::unpackage` stops at a truncated **final** tag and returns every complete tag before it,
  instead of failing the whole demux. A recorded or captured live FLV routinely ends mid-tag, and
  the previous behaviour discarded all of it (`FlvError::TagOverrun` for the entire file). A stream
  whose *first* tag is already incomplete still reports `TagOverrun`, since there is nothing to keep
  (#1079, audit r04-W15).
- `FlvMux::package` now rescales each track's `Sample::duration` running sum and its composition
  offset from the track's own timescale into FLV's millisecond `Timestamp`/`CompositionTime` fields
  (Annex E §E.4.1/§E.4.3.2), instead of writing the raw tick count. A 90 kHz input (any `TsDemux`
  source) previously came out with timestamps 90× too large, so the A/V timeline was wrong and every
  composition offset past ±8 388 607 ms was silently truncated into the SI24 field. An offset that
  cannot be represented, or a tag clock that would run backwards, is now an error rather than a
  truncation (#1079, audit r04-W12).
- `visual_ext::NclxColourInfo` gains a `reserved: u8` field recording the 7 `reserved` bits of the
  `nclx` flag byte, which are now preserved instead of being zeroed on a round trip (#1079,
  audit r04-W39).
- `mp4esds::UnknownDescriptor` gains a `position: usize` field and descriptors are re-emitted in
  place, so an `esds` that interleaves an unmodelled descriptor between the decoder config and the
  SL config keeps it there rather than collecting it at the end (#1079, audit r04-W24).
- `BoxHeader::serialize_into` rejects a compact header whose `size` exceeds the 32-bit field
  instead of silently wrapping it (`Error::InvalidValue`) (#1079, audit r04-W8).
- `visual_ext::ColourInformationBox` no longer mis-frames an `nclx` `colr` with no `nclx` params
  (`serialized_len` counted 7 bytes but only 4 were written and 4 returned — now
  `Error::InvalidValue`), and the trailing bytes some writers pad an `nclx` body with are kept
  verbatim so the box re-serializes byte-identically (#1079, audit r04-W39).
- `sample_entries` `hvc1`/`hev1`/`vvc1` sample entries now capture every sibling box, not just the
  codec config: `colr`, `pasp`, `btrt`, `clli`, `mdcv` and Dolby Vision's `dvcC`/`dvvC`. A sibling
  box placed before the config is captured too (the AVC path previously only walked forward from
  `avcC`). An HDR10 or Dolby Vision profile 8.1 source previously lost its colour signalling on an
  fMP4-to-fMP4 repackage and played as SDR, with Dolby Vision lost entirely (#1079, audit r04-W37).

- `nal::access_unit_is_rap` no longer treats a bare SPS as a random-access point: the SPS fallback
  (for open-GOP streams that omit the `recovery_point` SEI) now also requires the access unit to
  carry an I-slice (`slice_type` 2/7 modulo 5, ITU-T H.264 §7.4.3 Table 7-6). Hardware encoders and
  IP cameras commonly repeat SPS/PPS on every picture, so every access unit of such a stream was
  previously reported as a RAP and a segmenter cut segments starting on a P/B frame that cannot be
  decoded independently (#1079, audit r04-W23).

- `BoxHeader::serialize_into` now writes the header form the header was parsed or built with,
  instead of re-deriving it from `size`. A wire header that used `size == 1` + 64-bit `largesize`
  and whose `size` fits in 32 bits previously serialized as the compact 8-byte form while
  `serialized_len()` reported 16, leaving 8 stale bytes in any caller that laid out `header_size()`
  bytes. A `uuid` header with no `usertype` and a header with `size == 1` but no largesize now
  return `Error::InvalidValue` rather than writing an under-length or misframed header (#1079,
  audit r04-W8).
- `AudioSpecificConfig::serialize_into` no longer zeroes the whole caller buffer: it clears only the
  bytes it writes. A caller batching several configs into one larger shared buffer previously lost
  everything it had already written past this config (#1079, audit r04-W9).

- `AnnexBNalIter::next` no longer recurses once per empty NAL: a run of consecutive start codes
  (reached from any hostile H.264 PES through `annexb_to_length_prefixed`) used to grow the stack
  until the process aborted, which cannot be caught (#1079, audit r04-W4).
- `AccessUnitSplitter` now keeps trailing non-VCL NALs with the picture they follow, per codec
  (H.264 §7.4.1.2.3 filler/EOS/end-of-stream; H.265 §7.4.2.4.4 suffix SEI/FD/EOS/EOB and the
  reserved/unspecified suffix ranges; H.266 §7.4.2.4.3 suffix APS/SEI). A CBR AVC stream whose
  pictures end in filler previously alternated between a picture access unit and a filler-only
  one, and every HEVC suffix SEI was attributed to the following picture. Its start-code scan also
  resumes from a cursor instead of rewalking the whole buffer, so a 1 MiB IDR fed in 1316-byte
  chunks is no longer rescanned ~800 times — and the cursor is rebased when the buffer is trimmed,
  without which the next push resumed past the retained bytes, skipped their start codes and merged
  NALs, losing access-unit boundaries (#1079, audit r04-W2, r04-W3).
- `FlacSpecificBox::parse` rejects a metadata block whose declared 24-bit length runs past the end
  of the box, and enforces `isoflac.txt`'s "the first metadata block MUST be STREAMINFO" rule. A
  truncated block previously parsed as a shorter valid one, so re-serializing it wrote a different
  length and silently broke the byte-exact round trip (#1079, audit r04-W17).
- `OpusSpecificBox::parse` rejects a `dOps` whose channel-mapping table is shorter than its
  `OutputChannelCount` declares, instead of accepting a shorter map and re-serializing a different
  box (#1079, audit r04-W25).
- `AudioSpecificConfig::to_adts_header` no longer emits ADTS profile 0 (AAC Main) for HE-AAC /
  HE-AAC v2 explicit hierarchical signaling (`audioObjectType` 5/29): it now decodes the core AOT
  the same way `heaac_signaling` does, rejects a core AOT ADTS's 2-bit `profile` field can't
  represent (anything outside 1..=4), and rejects `sampling_frequency_index == Escape` instead of
  copying the forbidden `0xF` into ADTS (#1008).
- `PsDemux` no longer misidentifies MPEG-2 video as H.264 for every `stream_id` in the video range
  (0xE0-0xEF): it now probes the reassembled elementary stream for a MPEG-2 `sequence_header()`
  before falling back to the H.264 SPS/PPS path, and MPEG-2 video now comes out as
  `CodecConfig::Mpeg2Video` with real picture geometry instead of a garbage `avc1` track built
  from MPEG-2 slice start codes misread as an AUD/SPS/PPS (#1009).
- `SegmentIndexBox` (`sidx`) parse/serialize now read/write the 16-bit `reserved` field before
  `reference_count`, per ISO/IEC 14496-12 §8.16.3.2 — every conformant sidx previously parsed with
  `reference_count == 0` (its `reserved` bytes misread as the count), and every sidx this crate
  serialized was 2 bytes short of a conformant one (#1010).
- `WebmDemux` no longer loses every Cluster after the first when Clusters are written
  unknown-size (live/recorded WebM from a browser `MediaRecorder`, `ffmpeg -f webm -live 1`, or
  OBS): an unknown-size Segment child now terminates at the next Segment-level sibling element
  (RFC 8794 §6.2), not at the end of the buffer (#1011).
- `TsDemux` no longer leaves the 2 `crc_check` bytes of a CRC-protected ADTS frame
  (`protection_absent == 0`) inside the emitted AAC sample, and an ADTS frame with
  `number_of_raw_data_blocks_in_frame > 0` now gets a duration scaled by the number of raw data
  blocks it actually carries instead of always `1024` samples (#1012).
- `avcC`/`hvcC` NALU-array serializers no longer silently wrap `numOfSequenceParameterSets`
  (5-bit), `numOfArrays`/`numOfPictureParameterSets`/`numOfSequenceParameterSetExt` (8-bit), or a
  NALU/config length (16-bit) past their field width while still emitting every entry (#1129).
- `mhaC` (`mpegh3daConfigLength`), `vpcC` (`codecIntializationDataSize`), `dfLa` (24-bit
  `METADATA_BLOCK_LENGTH`, issue-run W17), and `esds` (`URLstring` length) config records now
  reject an oversized field instead of misframing (#1129).
- RTMP `ChunkWriter`/FLV muxer no longer silently truncate a 24-bit `message_length`/`DataSize`
  past 16 MiB, and AMF0 object/ECMA-array key/string/count fields are range-checked (#1129).
- CENC `senc` (`sample_count`, `subsample_count`), `saio` (`entry_count`, v0 32-bit offset),
  `saiz` (`sample_count`), `pssh` (`KID_count`, `DataSize`), `tenc` (`default_constant_IV_size`),
  and the `subs`/`sgpd`/`sbgp` sample-group boxes' `entry_count`/`subsample_count`/
  `subsample_size` fields are now range-checked (#1129).
- The PMT `section_length` is now bounded to 1021 bytes (§2.4.4.4) and returns
  `Error::BufferCapExceeded` instead of masking the 12-bit field and emitting an out-of-spec or
  (past 4095 bytes) wrapped section (#1129).
- `sidx`'s version-0 `earliest_presentation_time`/`first_offset` (32-bit) and its per-reference
  31-bit `referenced_size`/28-bit `sap_delta_time`, and `elst`'s version-0
  `segment_duration`/`media_time` (32-bit), are now range-checked instead of silently masked or
  narrowed (#1129).
- `trun`/`stsz`/`dref`/`stsc`/`stco`/`co64`/`stss`/`stsd` box `sample_count`/`entry_count` fields
  are now range-checked before narrowing to their 32-bit wire field (#1129).
- `mvhd`/`tkhd` version-1 (64-bit `creation_time`/`modification_time`/`duration`) boxes now use
  the correct size and field offsets: `mvhd` v1 is 120 bytes, not 124, with `next_track_id` at
  byte 116 (#1015); `tkhd` v1's `duration`/`layer`/`alternate_group`/`volume`/`matrix` were read 4
  bytes early (#1016). Previously a real v1 `mvhd`/`tkhd` (produced whenever a duration in movie-
  timescale ticks exceeds `u32::MAX`) failed to parse or was silently misframed on serialize.
- H.264 SAMPLE-AES encryption no longer encrypts a NAL's final block when exactly 16 bytes of the
  eligible region remain; the HLS SAMPLE-AES spec (§3.2, `docs/drm/hls-sample-aes.md`) leaves that
  trailing block clear (`bytes_remaining() > 16`, strictly greater) (#1014).
- The E-AC-3 `dec3` (`EC3SpecificBox`) serializer no longer under-sizes a substream that has
  dependent substreams (`num_dep_sub > 0`): such a substream is 4 bytes on the wire (the extra
  `chan_loc(9)` field), not always 3, so re-serializing a 7.1 E-AC-3 init segment no longer
  panics/misframes (#1055).
- `TsMux` and `ProgressiveMux` now take each sample's timing from the IR's own `dts`/`pts` on one
  origin shared by every track, instead of rebuilding every track from zero as a sum of
  durations; inter-track start offsets and gaps survive, and `ProgressiveMux` places each track
  on the movie timeline with an `elst` (#1020).
- `Media::trim` measures its window on one origin for all tracks and starts the non-anchor tracks
  at the anchor's snapped keyframe, and `Repackage` keeps each track's start offset in its first
  `tfdt`, so audio no longer plays early by the keyframe snap distance (#1021).
- `SmoothPackager` writes `trun` sample durations and composition offsets, and the `c@t`/`tfxd`
  times, in the 10 MHz manifest `TimeScale` instead of the track's media timescale (#1022).
- `HlsPackager` emits one `#EXTINF` covering the presentation span of all tracks instead of one
  sequential entry per track, and the CLI's `-f hls` writes the single segment it names (#1023).
- `ProgressiveMux` (and the Smooth fragment builder) derive the `mdat` header length from the
  payload size, so chunk offsets are no longer 8 bytes early once the `mdat` needs a 64-bit
  `largesize` header (#1019).


## [0.24.2] - 2026-09-25

### Security
Fixes for twelve advisories: GHSA-q6vr-4pp5-mcfv, GHSA-3554-x4jf-7frw, GHSA-2mg2-jj9h-c5vr,
GHSA-6vc8-3c25-4c9w, GHSA-643q-pgwj-8v2h, GHSA-c722-96gr-hgq2, GHSA-gmmr-7cqm-5p32,
GHSA-9m92-jr3c-4cq7, GHSA-j76h-pqjr-p54h, GHSA-mq46-69j5-7gqj, GHSA-59ph-4f24-q79x,
GHSA-v965-v82c-2f8x. Upgrade if transmux parses or encrypts untrusted input.

### Fixed
- Reject malformed KLV BER lengths, `esds` descriptor sizes, `sinf` child boxes and `pssh` v1
  bodies with an error instead of panicking.
- Bound sample-table, sample-group and SPS field counts by the input actually present, so a
  malformed file is rejected instead of allocating or looping without limit.
- RTMP chunk reassembly starts a fresh message on every fmt 0/1/2 chunk header instead of
  carrying over bytes from an incomplete previous message.
- `CencEncryptor::encrypt` validates every sample before encrypting any, so a rejected call leaves
  the media and the IV counter untouched, as documented.
- `KeyMap`, `CencEncryptor` and `CencDecryptor` no longer derive `Debug`: a manual impl on each
  now redacts content key bytes (KIDs, which are not secret, still print) and, for
  `CencDecryptor`, summarizes the protected file by length instead of dumping its bytes.
- `progressive_demux`'s `stsc` chunk-run expansion now clamps each entry's chunk range to the
  track's actual chunk count before iterating, instead of iterating the wire `first_chunk` value
  directly (an out-of-range `first_chunk`, up to `u32::MAX`, could cost billions of loop
  iterations per entry).
- `cli::Args` now implements `Debug` by hand: `--key` entries are redacted to their (non-secret)
  KID half instead of being printed as the raw `<KID>:<key>` argument.
- `cli::CliError::BadKey` now carries an already-redacted form of the offending `--key` argument
  (the KID half if it parsed, otherwise just the argument's length) instead of the raw string, so
  neither its `Display` nor its derived `Debug` can print key hex.

## [0.24.1] - 2026-08-30

### Fixed
- **OOM guards** for unbounded wire-driven allocations:
  - **#987**: `progressive_demux` `total_samples` (stsc × stco/co64 sum) now
    bounded against file size; protects 7 downstream capacity-sized allocations.
  - **#988**: `sample_groups` sgpd/sbgp/subs `entry_count` now guarded by
    `bounded_entry_count` against remaining buffer length.
  - **#983**: `movie_fragment` trun `sample_count` now guarded by
    `bounded_entry_count` against remaining buffer and per-sample field size.
- **Parse panic guards**:
  - **#989**: `cenc::SchemeInformationBox::parse` and
    `ProtectionSchemeInfoBox::parse` panicked on input shorter than `BOX_HDR`.
    Now returns `BufferTooShort` instead.
- **Silent truncation/overflow fixes**:
  - **#996**: `composition_offset()` bare `as i32` cast → clamped to `i32` range.
  - **#997**: AVC width/height bare `as u16` → `.min(u16::MAX as u32) as u16`,
    matching the HEVC path.
  - **#981**: ADTS frame_len bare `as u16` → 13-bit max validation (8191).
  - **#984**: AMF0 string length bare `as u16` → `u16::MAX` length check with
    error on overflow.
  - **#998**: WebM `CLUSTER_TIMESTAMP` uses `i64::try_from`; `cluster_ts + rel_ts`
    uses `checked_add` with error on overflow.
- **MKV u64 offsets** (#995): `running_offset`/`pos_info`/`pos_tracks`/
  `cluster_offsets` widened from `u32` to `u64`, preventing wrap on files > 4 GB.
- **Reserved field zeroing** (#986): mvhd/tkhd `serialize_into` now zero-fills
  all reserved byte regions instead of skipping them with `c += N`.
- **CENC seig detection** (#990): content using `seig` sample-group key rotation
  (ISO/IEC 23001-7 §12.2) is now detected and explicitly rejected with
  `Error::UnsupportedFeature` instead of silently decrypting with the wrong key.
- **mdat lower-bound validation** (#991): `data_offset` values pointing into the
  moof itself (below mdat payload start) are now rejected.
- **Splice match_tracks non-injective mapping** (#992): track matching now
  tracks claimed indices, preventing two source tracks from mapping to the
  same destination track in dual-audio content.
- **Repackage presentation_times** (#993): now uses each sample's absolute
  `dts` when available, falling back to the running-sum accumulator only when
  a sample has no explicit dts.
- **LL-DASH Timeline addressing** (#994): `inject_ll` now matches both
  self-closing `<SegmentTemplate …/>` and non-self-closing
  `<SegmentTemplate>…</SegmentTemplate>` (Timeline/`$Time$` addressing).

## [0.24.0] - 2026-08-11

### Fixed
- Six audit findings in the fMP4/CMAF/CENC parse paths, all reachable from
  ordinary or malformed third-party media (no fuzzing required):
  1. **`cenc::TrackEncryptionBox::parse_body` panicked** on a `tenc` body of
     exactly 19 bytes: the minimum-length guard undercounted the fixed
     1-byte fields ahead of the 16-byte `default_KID` by one
     (ISO/IEC 23001-7:2016 §12.2.2), so a 19-byte body passed the guard and
     then panicked slicing the KID at offset `4..20`. The guard now requires
     the correct 20 bytes.
  2. **`sample_entries::find_config_box` panicked** on a config-region child
     box declaring a wire `size` (ISO/IEC 14496-12:2015 §4.2) larger than
     the region actually holding it. The returned slice is now clamped to
     the region's remaining length, matching the bound
     `init_segment::parse_stbl_children` already applied to its own child
     walk.
  3. **`init_segment::parse_stbl_children` silently discarded malformed
     `stts`/`ctts`/`stsc`/`stsz`/`stco`/`co64`/`stss` boxes**, defaulting
     each to an empty typed box that falsely claimed the table had zero
     entries — the same defect issue #952 fixed for `stsd`. All seven now
     get the `stsd` arm's treatment: kept as raw bytes
     (`StblChild::Opaque`) so the real parse error survives; a new
     `progressive_demux::find_stbl_child` helper re-parses a same-four-CC
     `Opaque` box to recover that error at the point of use, so a corrupt
     sample table now fails the carrying track loudly (visible in
     `Media::skipped`) instead of silently behaving as absent or empty.
  4. **Seven wire-count-driven `Vec::with_capacity` sites in
     `init_segment.rs`** (`dref`/`stsc`/`stsz`/`stco`/`co64`/`stss`/`stsd`)
     allocated on an untrusted `u32` entry count *before* validating it — a
     16-byte `co64` declaring `count = 0xFFFFFFFF` asked for ~32 GB up
     front. A new `bounded_entry_count` helper caps the count against what
     the remaining buffer could actually hold, computed before any
     allocation (the same discipline `cenc::SampleEncryptionBox::parse_body`
     already applied to `senc`'s `sample_count`); the per-iteration bounds
     checks these parsers already had make this a pure allocation-size fix,
     not a behavior change for any input that parsed successfully before.
  5. **`ll_hls.rs` divided by the anchor track's timescale with no zero
     guard** in three places (`part_target_secs`, whole-segment duration,
     part duration) — `ts_hls.rs` already guards the identical computation
     with `.max(1)` at every call site; `ll_hls.rs` now does too. A
     malformed zero `mdhd.timescale` used to turn these into
     `f64::INFINITY`/`NaN`, rendered verbatim into `#EXT-X-PART-INF`/
     `#EXT-X-PART`/`#EXTINF` — a wrong value shipped to every client with no
     panic to flag it.
  6. **`repackage::anchor_index` only recognised `CodecConfig::Avc` as a
     video anchor**, so HEVC/AV1/VVC-only media (all supported elsewhere in
     this crate) fell through to `unwrap_or(0)` — track 0, which may be
     audio, cutting `Media::trim`/resegment boundaries on audio "keyframes"
     instead of real video IDRs, on ordinary well-formed input. It now
     delegates to the shared `segmenter::choose_anchor` (first video track
     of any codec, else the first anchor-capable track; the same fix issue
     #628 made for `Segmenter`/`ts_hls`), so `Repackage` and `Segmenter`
     can no longer disagree on which track anchors segmentation. A media
     with no anchor-capable track at all is now a named `Error::InvalidInput`
     instead of an unchecked index-0 fallback.
- `Fmp4Demux` dropped an entire H.264 video track when a High-profile `avcC`
  omitted its optional ISO/IEC 14496-15:2017 §5.3.3.1.2 trailer
  (`chroma_format`/`bit_depth_*`/`sps_ext`) — a real DASH-IF `livesim2`
  capture does exactly this, and ffmpeg reads it without complaint (issue
  #952). Two defects, both fixed:
  1. `avc_config::AVCDecoderConfigurationRecord::parse` read the trailer
     unconditionally whenever `profile_indication` was in the High-profile
     family (100/110/122/244), even with zero bytes remaining. It is now
     read only when at least one byte remains, leaving `chroma_format`/
     `bit_depth_luma_minus8`/`bit_depth_chroma_minus8` `None` (never an
     invented default) when the encoder omitted it; `Serialize` mirrors this
     so a trailer-less record round-trips without growing one back.
  2. `init_segment::parse_stbl_children`'s `stsd` arm swallowed a parse
     failure into a blank placeholder (`entries: Vec::new()`), which cost
     the *entire* track and surfaced only a generic "expected box: stsd
     entry" — hiding which field actually failed and why. A `stsd` that
     fails to parse is now kept as raw bytes (`StblChild::Opaque`);
     `media::track_spec_from_trak` re-parses those bytes so the real error
     reaches `Media::skipped`'s `SkippedTrack::reason` (shared by
     `Fmp4Demux` and `ProgressiveDemux`, both of which call
     `track_spec_from_trak`).
  Audited `hvcC`/`vvcC` for the same shape: `hvcC`'s chroma/bit-depth fields
  are unconditionally mandatory per ISO/IEC 14496-15:2017 §8.3.3 (no `if`
  gate at all); `vvcC`'s optional PTL block is already gated by an explicit
  on-the-wire `ptl_present_flag` that `VvcDecoderConfigurationRecord::parse`
  correctly branches on. Neither has this bug.

### Changed
- Internal-only duplication-audit consolidation, no behaviour or public API
  change:
  - `ts_demux`'s 33-bit PTS/DTS wrap-unroll (`unwrap_ts`) now delegates to
    `broadcast_common::clock33::unwrap_delta` — the shared owner of this
    math (also used by `timed-metadata`, `media-doctor`, `compliance-probe`,
    which previously each hand-rolled their own copy). `transmux`'s own
    algorithm was already the correct, bidirectional one; nothing about its
    behaviour changes here.
  - The six independent MSB-first bit-extraction loops in `bitreader`
    (RBSP/Exp-Golomb), `mpegh`, `vvc_config`, `ac3`, `dts`, and `aac_asc` now
    all delegate the actual bit extraction to
    `broadcast_common::bits::BitReader` (already reused by
    `dvb-t2mi`/`rdd29`/`st291`) instead of re-implementing the same loop six
    times; a bit-order/overrun fix there now reaches all of them. Each
    module keeps its own bounds-checking, error type, and higher-level
    semantics (Exp-Golomb, MHAS escaped-value coding, etc.) exactly as
    before — only the innermost extraction moved.
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.

## [0.23.1] - 2026-08-08

**Note:** this was cut as a *patch* release while also adding public API
(`flv_sequence_header_payloads`/`flv_frame_payloads`/`FlvPayload`/
`FlvPayloadKind`, below) — a minor by this workspace's own 0.x scheme. That
mismatch is what produced the understated-floor bug fixed under `multimux`
(its `push::rtmp` called these functions while multimux's manifest still
declared `transmux = "0.23"`, which resolves to 0.23.0 and does not have
them). Recorded here so the next release does not repeat the mistake of
shipping additive API as a patch.

### Added
- `MkvMux` — a Matroska (MKV) container muxer, the exact inverse of
  `WebmDemux`: EBML header + `Segment` (`SeekHead`/`Info`/`Tracks`/`Cluster`s
  of `SimpleBlock`s/`Cues`), sharing `WebmDemux`'s CodecID ↔ `CodecConfig`
  mapping (`V_MPEG4/ISO/AVC`, `V_MPEGH/ISO/HEVC`, `V_VP9`, `V_VP8`, `V_AV1`,
  `A_AAC`, `A_OPUS`, `A_VORBIS`, `A_AC3`, `A_EAC3`) plus the ISOBMFF-family
  codecs a DVR recording carries, so `{WebM/Matroska} → IR → {Matroska}`
  round-trips (issue #915). Previously shipped at `HEAD` with no version
  bump, an empty changelog entry, and no README coverage-table row — all
  three fixed here.
- `flv::flv_sequence_header_payloads` + `flv::flv_frame_payloads` + `FlvPayload`/`FlvPayloadKind`
  (issue #934): build the same `VideoTagHeader`+`AVCVIDEOPACKET` /
  `AudioTagHeader`+`AACAUDIODATA` tag *bodies* `FlvMux::package` writes into
  each FLV tag, without FLV tag/file framing — the bodies an RTMP
  `send_video`/`send_audio` message needs. `flv_frame_payloads` rescales each
  sample's absolute `dts`/composition-offset (in its track's own timescale)
  to FLV's millisecond clock directly, unlike `FlvMux::package`'s zero-based
  running-duration sum — safe to call once per drained batch from a live
  push driver, where "start of this batch" isn't "start of the stream".
  Added for `multimux`'s RTMP push output, which previously shipped raw
  MPEG-2 TS as an RTMP video payload (no RTMP server can decode that).

### Fixed
- Documentation-only corrections: several places where module/README docs
  overclaimed what the code does.
  - The crate root's and README's "every `{input} → {output}` combination
    composes" claim did not disclose that WebM's VP8/Vorbis tracks have no
    ISOBMFF sample entry in this crate and so cannot be muxed into
    fMP4/CMAF/progressive-MP4/DASH/LL-DASH/CMAF-HLS/LL-HLS/Smooth — only
    `WebM → WebM`/MKV round-trips them. Now stated explicitly in both docs;
    the README codec table marks VP8/Vorbis 🟡 (new legend entry) instead
    of a bare ✅.
  - Smooth Streaming output (`SmoothPackager`) rejects every codec but
    H.264 video + AAC-LC audio, but this restriction was undisclosed
    everywhere Smooth was presented as a supported output; now called out
    in the crate root and README.
  - The README's TS-demux "nothing dropped" guarantee (opaque `Data` track
    fallback for unrecognised `stream_type`s) was stated without
    qualification; it does not extend to the FLV demux, which silently
    skips non-AVC video and non-AAC audio tags (no track, no error, no
    other signal). Now called out in the README and in the `src/flv.rs`/
    `src/flv_stream.rs` module docs.
  - `WebmDemux`'s lacing limitation (a laced block is a hard error, not
    silently skipped) was documented only in that module's own doc
    comment; now also surfaced in the crate root and README.

## [0.23.0] - 2026-08-05

### Added
- `TrackSpec::program_number: Option<u16>` — the MPEG-2 TS program_number from
  the declaring PMT (ISO/IEC 13818-1 §2.4.4.3), populated by `StreamingTsDemux`
  at ES promotion time. `None` for non-TS sources. Enables downstream consumers
  (e.g. `multimux`'s `ProgramTracker`) to distinguish programmes in an MPTS
  (issue #906).
- `TrackSpec::with_program(u16)` builder method.
- `LlHlsSegmenter::next_sequence_numbers()` — returns the `(next_seq,
  current_segment)` pair the segmenter would give the next emitted part
  and segment (issue #781). Used by a caller that rebuilds the segmenter
  mid-stream to resume sequence numbering.
- `LlHlsSegmenter::with_part_target_at()` — builds a segmenter whose
  `mfhd.sequence_number` and 1-based segment number start from the given
  values, resuming where a previous segmenter left off.

## [0.22.0] - 2026-08-02

### Fixed
- `HlsPackager` (CMAF-HLS) now emits an `#EXT-X-MAP` tag on every Media
  Segment, with a `BYTERANGE` covering the leading `ftyp`+`moov` span of
  the self-initializing CMAF artifact each segment names (issue #870).
  Previously no `#EXT-X-MAP` was emitted at all — a real conformance gap
  caught by wiring Apple's `mediastreamvalidator` in as an independent
  oracle over the packager's actual output ("Each fMP4 Segment in a Media
  Playlist MUST have an EXT-X-MAP tag applied to it"), not by any of this
  crate's own round-trip tests, which shared the same blind spot.

### Changed (BREAKING)
- **HLS (M3U8) playlist syntax moved to the new `broadcast-hls` crate**
  (issue #878). `transmux::hls` and every `transmux::{MediaPlaylist,
  MasterPlaylist, MediaSegment, Variant, IFrameVariant, LowLatencyConfig,
  OpenSegment, PartSpec, MapTag, ByteRange, PreloadHintType,
  RenditionReport, SkipInfo, CENC_KEYFORMAT, CENC_KEYFORMATVERSIONS,
  cenc_ext_x_key, mark_init_discontinuities}` path no longer exists — no
  compatibility re-export. Depend on `broadcast-hls` directly for playlist
  syntax; `transmux` now depends on it for its own HLS/LL-HLS
  **segmenters** (`ts_hls`, `ll_hls`, which still live here — they produce
  container bytes, not playlist syntax). `Error::HlsParse` is removed from
  `transmux::Error` (playlist parsing, and its error type, moved with the
  syntax); `broadcast_hls::Error::HlsParse` replaces it.
- **`CencScheme` moved to `broadcast-common` 9.2** (`broadcast_common::cenc`).
  `transmux::CencScheme` / `transmux::cenc::CencScheme` still resolve — they
  are now re-exports of that single shared definition, so `transmux` and
  `broadcast-hls` name the *same* type and `broadcast_hls::cenc_ext_x_key`
  takes it with no conversion. Two consequences for callers:
  - `CencScheme` is `#[non_exhaustive]` **across a crate boundary** now, so a
    downstream `match` on it needs a wildcard arm.
  - New `Error::UnsupportedCencScheme { scheme }` — the encrypt/decrypt cipher
    dispatch sites now reject a scheme this crate has no cipher for rather
    than relying on compile-time exhaustiveness. Unreachable today (only
    `cenc` and `cbcs` exist); it is there so adding `cens`/`cbc1` to
    `broadcast-common` later can never turn those sites into a panic.
- **`rtp::hex_encode` moved to `broadcast-common` 9.2** (`broadcast_common::hex`).
  `transmux::rtp::hex_encode` still resolves, as a re-export.
  `rtp::hex_decode` is unchanged and stays here (it reports through this
  crate's own `Error`).
- Requires `broadcast-common` **9.2** (was 9.0) — the floor for the two items
  above. Same caret epoch, so no consumer migration.

## [0.21.1] - 2026-07-30

### Fixed
- Floor `mpeg-ts` to `0.3.1`. The `^0.3` bucket also contains 0.3.0, which is
  built against `broadcast-common` 8, so a consumer could resolve two
  `broadcast-common` majors into one graph and hit trait-resolution errors
  pointing at this crate's internals (#858).

## [0.21.0] - 2026-07-30

### Added
- `RtpPacket` public type (exported from `transmux::rtp`).
- `InputDegradation` enum + `DemuxEvent::InputDegraded` (issue #778):
  `StreamingTsDemux` now emits `TransportError` when the MPEG-2 TS
  `transport_error_indicator` (`tei`) is set, and `ContinuityGap { expected,
  got }` when a genuine continuity-counter gap is detected (excluding
  legal duplicates — same CC + identical payload — and signalled
  discontinuities per ISO/IEC 13818-1 §2.4.3.3). A consumer repackaging a
  lossy UDP multicast can now distinguish a clean stream from one losing
  packets, rather than seeing silently-corrupt samples.
- `ConstantIvSenc` enum (`Emit`/`Omit`) and `constant_iv_senc` field on
  `EncryptConfig` (issue #783). Controls whether a `cbcs` +
  `IvGen::Constant` track emits a `senc` box with the constant IV
  replicated per sample (the new default, for interop with Bento4
  `mp4decrypt` and other tools that require an explicit `senc`), or omits
  the `senc`/`saiz`/`saio` triple entirely (the spec-minimal, `tenc`-only
  shape). The old shape remains reachable via `ConstantIvSenc::Omit` and
  is still tested for self-consistency round-trips.
- `tests/non_exhaustive_coverage.rs` drift guard (issue #806).

### Changed (Breaking)
- `RtpStream::packets` is now `Vec<RtpPacket>` instead of `Vec<Vec<u8>>`
  (issue #777). `RtpPacket` carries a small, owned `header: Bytes` and a
  `payload: Bytes`; on the single-NAL and FU-A paths the payload is a
  zero-copy `Bytes::slice` of the original sample data, so fanning a sample
  out to multiple RTP streams doesn't copy the coded payload bytes.
  `RtpPacket::as_contiguous()` returns a single `Bytes` for consumers that
  need a contiguous packet (e.g. the depacketise path, which reassembles by
  concatenation and may reasonably copy).
- `packetise_klv()` now takes `&Bytes` instead of `&[u8]`; each fragment's
  payload is a zero-copy `Bytes::slice`.
- `EncryptConfig` gains `constant_iv_senc: ConstantIvSenc` — every
  struct-literal construction site must add this field.
- `TrackEncryption::new` gains a fourth argument (`constant_iv_senc:
  ConstantIvSenc`).
- The following public enums now carry `#[non_exhaustive]` (issue #806's
  non_exhaustive drift-guard audit -- every other public enum in the
  workspace already did): `Addressing`, `MediaKind` (`dash`); `SgpdEntry`
  (`sample_groups`); `PreloadHintType` (`hls`); `SampleEntryVariant`,
  `StblChild` (`init_segment`); `MpdType` (`dash_parse`); `ColourType`
  (`visual_ext`); `MpegAudioLayer` (`mpeg_legacy`); `StreamType`
  (`smooth_parse`); `VvcNalUnitType` (`vvc_config`); `SmoothStreamType`
  (`smooth`); `FormatArg`, `CliError`, `Output` (`cli`, `cli` feature). A
  downstream `match` on any of these now needs a wildcard arm.
- Fixed a latent panic this same audit surfaced: `smooth_parse::StreamType`
  gaining `#[non_exhaustive]` exposed that `ll_hls_runtime`/`multimux`'s
  Smooth-pull ingest matched `Video`/`Audio`/`Text` exhaustively and would
  have panicked on any future stream type; see `multimux`'s own changelog for
  the corresponding fix.

### Changed
- STAP-A aggregation and AAC-hbr audio packets interleave headers with
  payload, so they build the full RTP packet in a `BytesMut` (which copies).
  The parameter sets and audio AUs are small, so this is negligible.

## [0.20.0] - 2026-07-28

### Fixed

- **RTP depacketisation now detects loss/reorder instead of silently
  corrupting reassembly** (issue #779): `RtpHeader.sequence` was parsed but
  never read at any non-test call site, so `RtpStreamDepacketiser` decided
  access-unit boundaries purely from the RTP timestamp and marker bit — a
  dropped FU-A fragment was silently concatenated with its neighbours into a
  malformed access unit. Each track now tracks its expected 16-bit sequence
  number (RFC 3550 §5.1/§A.1, compared with wrapping arithmetic, never `>`):
  an in-order or legally-duplicate packet is unaffected; a reordered packet
  is held in a bounded buffer (`RtpStreamTrack::with_reorder_depth`, default
  `DEFAULT_REORDER_DEPTH`) and replayed once the gap fills, reassembling
  byte-identically to the in-order capture; a genuine gap drops the access
  unit under construction rather than reassembling it from a run missing a
  fragment, and records the new `RtpLossEvent::SequenceGap`
  (drained via `RtpStreamDepacketiser::poll_loss_event`) — a clean capture
  raises zero loss events end to end. The reorder buffer is a hard bound
  (never a fifth unbounded-allocation vector). See
  `transmux/docs/rtp/rtp-sequence-validation.md` for the RFC 3550 §A.1
  transcription this is adapted from, and `rtp_stream`'s module docs for why
  the signal surfaces locally rather than in `ir::DemuxEvent`.


**Publish order:** `broadcast-common` 8.7.0 → `transmux` 0.20.0 → `media-doctor` → (steps 4/5: `ll-hls-runtime`, `multimux`, `multimux-cli`).

Media plane step 2: consolidates the IR to carry absolute, unwrapped
timestamps (`Sample::dts`/`pts` as `Option<i64>`, `duration` as `Option<u32>`)
so downstream callers (splice, DVR, pacing, live origin) can work with real
time, and adds track-lifecycle events (`TrackUpdated`/`TrackRemoved`/
`TrackAbandoned`) for mid-stream PMT changes. An aggregate review of the whole
step-2 range then found 11 blocking defects; the five worst are fixed here
(folded into this still-open release rather than shipped separately),
alongside two CENC confidentiality vulnerabilities disclosed under Security
below.

- **`splice::concat`/`splice_insert` now rebase the spliced-in content's
  `dts`/`pts` onto the join (issue #782).** Both used to place a second
  `Media`'s samples after a join point but never shift their absolute
  timestamps onto it — harmless while decode time was reconstructed by
  summing `duration` from `start_decode_time`, but since this release's
  switch to reading absolute `dts` (above) the spliced-in asset simply kept
  whatever timeline it was demuxed with. Two independently-demuxed assets
  have unrelated absolute timelines (each anchored on its own
  `tfdt`/PCR/FLV clock), so a join could jump backwards or forwards by an
  arbitrary offset while the segment structure claimed continuity —
  producing a wrong fragment `tfdt` and wrong PTS/DTS on TS re-mux, and a
  player that stalls or skips at an SSAI ad break (`splice_insert`'s
  advertised use case). Every incoming sample's `dts`/`pts` (when `Some`;
  a section-carried sample with `dts: None` has nothing to rebase and
  fabricates nothing) now shifts by a single offset derived from one
  reference track (video, else the first timed track) and converted into
  each matched track's own timescale — never derived independently per
  track, which would silently re-align tracks relative to each other and
  destroy A/V sync.
- `TsDemux` stored **audio** sample timing in 90 kHz PES-clock ticks while the
  track's timescale is its sample rate, so `dts` deltas (e.g. 2089) disagreed
  with `duration` (1024 AAC samples). Audio `dts`/`pts` — and the audio track's
  `start_decode_time` anchor — are now rescaled into the track's own timescale,
  the same unit as `duration`. Latent before this release only because the old
  `SourceTiming` was never read back.
- `PsDemux` left every AC-3 sample's `duration` at `0`, making the recovered
  audio timeline uninterpretable; it now carries the intrinsic 1536-sample
  syncframe duration (ETSI TS 102 366 §4.1) and absolute time rescaled from the
  PES stamps.
- `RtpDepacketiser` (batch) discarded the RTP timestamp and the per-AU sync
  flag entirely, emitting `duration: 0` / `is_sync: true` for every sample. It
  now carries the unwrapped absolute RTP media clock and the real IDR-derived
  sync flag.
- **CRITICAL — a PMT that reclassifies a PID's codec no longer panics the
  process.** PMT version diffing (above) made PMT application *destructive*,
  which turned several latent weaknesses into live faults; this is the worst
  of them. `apply_pmt_diff` wrote `stream.codec` in place, leaving the
  `ConfigProbe`, the `Carrier`, and any buffered access units built for the
  **old** codec. A version change that reclassified a still-probing PID — DVB's
  routine `stream_type` `0x06` gaining an `AC-3_descriptor`, so
  `Codec::Data(0x06)` becomes `Codec::Ac3` (issue #641) — then reached
  `finalize_probe`'s `unreachable!("ConfigProbe::Data is only created for
  Codec::Data")` and **aborted on ordinary broadcast input**. A codec change is
  a different elementary stream, so the PID is now torn down and re-registered,
  which rebuilds every derived piece of state in one move. That also fixes the
  stale-`Carrier` half of the same bug: ISO/IEC 13818-1 Table 2-34 splits
  `stream_type` into PES- and section-carried families, so a `0x86` → `0x1B`
  reclassification used to feed H.264 PES bytes to a `SectionReassembler` and
  produce silence while the track still claimed to exist. The `unreachable!`
  itself is gone: a probe/codec mismatch now degrades to "unresolved" (and is
  concluded by the existing `TrackAbandoned` paths) rather than aborting a
  `#![forbid(unsafe_code)]` library on remote input. Every remaining
  panic-class site in `ts_demux` reachable from parsed input was converted the
  same way.
- **CRITICAL — PSI `CRC_32` is validated before any PAT/PMT is acted on**
  (ISO/IEC 13818-1 §2.4.4.1, via `broadcast_common::crc32_mpeg2`). Nothing
  checked it before: `CRC32_LEN` was used only to skip the trailer in length
  arithmetic. Now that PMT application tears tracks down, one bit error in a
  version byte or an ES loop destroyed a live track and reassigned its
  `track_id`; and because `process_packet` consults `pmt_reasm` *before*
  `streams`, a corrupt PAT permanently hijacked an elementary PID into PMT
  reassembly, shadowing its stream for the rest of the run. A section failing
  CRC — or clearing `section_syntax_indicator`, which a PAT/PMT never legally
  does — is now dropped silently and disturbs nothing, not even
  `last_applied_version` (bumping that off a corrupt section would have
  swallowed the genuine version that follows).
- **A PAT may remap a PMT PID, and a "next" PAT is not applied.** The
  PAT-derived `program_number` was write-once (`entry().or_insert_with()`) and
  the PAT was applied ignoring `current_next_indicator`, so a legitimate remap
  — or a `cni == 0` "next" PAT — froze the binding and made the defensive
  `program_number` cross-check reject every PMT on that PID **forever**: a
  silent zero-track demux. The `cni == 1` rule PMT application already used now
  gates the PAT too, and a current PAT updates the binding (clearing
  `last_applied_version`, since the version counter belongs to the program, not
  the PID).
- **A removed PID's payload is not replayed into the re-added track.**
  `remove_track` left the dropped PID's traffic flowing into `unattributed` —
  the *pre-registration* replay buffer — so post-removal orphan payloads
  accumulated to the 4 MiB cap and were then delivered as the **re-added**
  track's first samples, anchoring its `start_decode_time` in the past. The
  backlog is now purged on removal and the PID is blacklisted from that buffer
  until a PMT declares it again.
- **An ES PID declared by two programs survives one of them dropping it.**
  `streams`/`es_seen` are global while a PMT's `applied_es` is per-PMT, so a
  shared audio/subtitle component was torn down as soon as *either* program's
  PMT stopped listing it. Removal is now refcounted by declaring PMT PID; only
  the last declarer's drop removes the track.
- **The audio re-anchor threshold is derived from real muxer behaviour, not
  from one sample period.** The B5 anchor's discontinuity threshold was
  `ceil(90000 / sample_rate)` — 3 ticks at 44.1 kHz — while a 1024-sample AAC
  frame is `2089.795…` ticks, so a muxer stamping the rounded constant `2090`
  drifts `+0.204…` ticks per frame *on a perfectly continuous stream* and
  crossed the threshold about every 15 frames; non-frame-aligned MP2 PES (issue
  #638) crossed it on essentially every access unit. `TimelineReanchored` was
  therefore pure noise and the anchor effectively inert. The bound is now
  20 ms of 90 kHz (1800 ticks) — below the lip-sync detectability floor the
  broadcast recommendations work to (ITU-R BT.1359-1; ATSC A/85's ±15 ms), so
  re-anchoring inside it would trade a real `Discontinuity` event for an
  inaudible correction — floored at two intrinsic sample periods for
  degenerate sample rates. `DiscontinuityKind` had no assertion coverage
  anywhere in the crate; it now has both halves of the contract (a
  constant-increment 44.1 kHz stream emits **zero** re-anchors; one genuine gap
  emits exactly one).
- **A gapped/discontinuous fMP4 keeps its gap.** `Fmp4Demux` seeded `next_dts`
  from only the **first** fragment's `tfdt` and then pure-summed `trun`
  durations, so every sample after a gap came out short by exactly the gap —
  re-muxing to a wrong `tfdt`/PES stamp and permanently desyncing A/V. `tfdt`
  is per-fragment and authoritative (ISO/IEC 14496-12:2015 §8.8.12), so every
  fragment that carries one now re-seeds the cursor; `Track::start_decode_time`
  still records the first fragment's anchor. A gapless stream is byte-for-byte
  unaffected.
- **`rescale_to_track` preserves a negative audio anchor.** Its `.max(0) as
  u128` fabricated `dts = 0` for audio where a legitimately negative unwrapped
  anchor (reordering across the 2^33 boundary) is carried through verbatim for
  every other track kind — desyncing the audio track alone.
- **CRITICAL — a `duration` of `Some(0)`/`None` on the anchor track no longer
  stalls segmentation forever** (all four segmenters: `Segmenter`,
  `LlSegmenter`, `LlHlsSegmenter`, `StreamingTsHlsSegmenter`). The anchor
  accumulator advanced only from `Sample::duration`, so a stream carrying
  `Some(0)` never reached the segment target: no part and no segment was ever
  emitted, `pending` grew without bound, and `Stage::demand()` still reported
  "not saturated", inviting a well-behaved driver to feed to exhaustion. This
  was reachable on a shipped path — `StreamingFlvDemux` derives `duration` as
  the forward delta between FLV tag timestamps, so an RTMP publish's first
  sample (and any two tags sharing a timestamp) is `Some(0)`. Since
  `Sample::dts` is absolute, the new shared `segmenter::MediaClock` advances
  on each sample's own `duration` when that is a real, non-zero span and on
  the **`dts` delta** otherwise; a stream with real durations segments exactly
  as before. Per-track `tfdt`/`base_media_decode_time` accounting was the same
  duration sum and is fixed alongside it, so segment decode times no longer
  all collapse to 0. `StreamingTsHlsSegmenter::push` also no longer rejects an
  anchor sample with `duration: None` outright.
- **CRITICAL — a legal single-IDR / infinite-GOP stream is now bounded instead
  of growing without limit** (all four segmenters). With one keyframe at the
  start and none after, no cut is possible — and cutting mid-GOP would break
  CMAF's (and classic HLS's) random-access guarantee — so the pending buffer
  is bounded on **data**, not time (these types are sans-IO and `no_std`): at
  the new `segmenter::MAX_PENDING_SAMPLES_PER_TRACK` un-cut samples,
  `Stage::demand()` reports `saturated` so a cooperative driver stops feeding,
  and `push`/`Stage::feed` then return a named `Error::InvalidInput`. `flush`/
  `finish` closes the trailing partial segment and input flows again.
- **`LlSegmenter`/`LlHlsSegmenter` anchor on any video codec, not just AVC.**
  Both selected the anchor with `matches!(config, CodecConfig::Avc { .. })`,
  so an HEVC-plus-AAC media with audio first anchored on the **audio** track:
  segments did not begin on an IRAP, and since every AAC sample is a sync
  sample every `PartInfo.independent` was `true`, advertising
  `INDEPENDENT=YES` (RFC 8216bis §4.4.4.9) on parts that actually start
  mid-GOP. Both now use the shared `segmenter::choose_anchor`, matching
  `Segmenter`/`ts_hls` as their doc comments already claimed. A track set with
  no anchor-capable track (all section-carried) is a construction error in all
  four rather than a silent stall.
- **`TsHlsPackager` places a timestamped section sample in the right segment.**
  `partition_tracks` advanced its per-track placement clock only from
  `duration`, which a section-carried track never has, so **every** section
  sample landed in segment 0 — an SCTE-35 cue for t=40 s was muxed at t=0 —
  while the streaming path placed it in the segment that was open on arrival.
  Both paths now share `placement_secs`, which falls back to the sample's
  absolute `dts`, restoring the batch/streaming equivalence the module docs
  claim (and now enforced by a test).
- **`ProgressiveDemux` cannot return silently-wrong sample payloads after a
  buffer-cap rejection.** A `Stage::feed` rejected for exceeding `max_bytes`
  discarded that chunk but was neither terminal nor recorded, and `demand()`
  still advertised headroom — so a following smaller chunk was accepted and
  `finish()` parsed a buffer **with a hole** using file-absolute `stbl` chunk
  offsets (ISO/IEC 14496-12:2015 §8.7.5), yielding either a misleading
  `UnexpectedBox` or a `Media` whose samples carried the wrong bytes. The
  rejection now poisons the demuxer permanently: every later `feed` and
  `finish` re-report the original `BufferCapExceeded`, `demand()` stays
  `saturated`, and no parse is attempted. `feed` after `finish` is likewise
  rejected instead of silently appending, and `finish` releases the
  accumulated buffer rather than retaining the whole file alongside a copy of
  every sample.
- **`StreamingFlvDemux::demand()` reports the real want.** It returned a
  constant 11-byte `want_bytes` even mid-tag-body, so a driver sizing its
  reads by `want_bytes` made ~1.5 million `feed` calls to deliver one 16 MiB
  tag. It now returns exactly the bytes still missing for the unit `feed` is
  blocked on.
- **`StreamingTsHlsSegmenter` keeps one ready queue, not two.** Its `Stage`
  adapter had a separate `stage_ready` beside the inherent inline return, so
  delivery routed per call and the two could drift; both now drain a single
  `ready`. The `Stage` impls of `StreamingTsHlsSegmenter` and
  `StreamingFlvDemux` also call their inherent methods by fully-qualified
  path, since `self.finish()` resolved to the inherent one only by
  inherent-over-trait precedence at identical arity — renaming it would have
  turned the delegation into silent infinite recursion.
- **`Segmenter::push` no longer loses the sample that triggered a failing
  cut.** A `build_media_segment` failure (reachable since `new` stopped
  filtering BMFF-unmuxable tracks) propagated *before* the triggering keyframe
  was buffered, punching a hole in the timeline on every subsequent anchor
  keyframe; the sample is now buffered first and the error surfaced after.
- **`trickplay::derive_iframe_track` doc corrections**: dropped a bullet
  naming the removed `composition_offset` field, and the false claim that the
  derived track "covers the same total timeline as the source" (it starts at
  the first sync sample, so it is shorter when `samples[0]` is not one).
- **`TsMux` no longer drops a recognised codec's PMT `ES_info` descriptors**
  (issue #775, closes #775; nullified a shipped rust-skyfire track-picker
  feature). `plan_elementary_streams` only carried a track's inherited
  `TrackSpec::es_info_descriptors` into the re-muxed PMT for an opaque
  `CodecConfig::Data` track — a stale guard from issue #576, written when the
  IR genuinely carried no descriptors for a decoded codec. Since issue #582
  `TsDemux` populates `es_info_descriptors` for **every** track, so a
  recognised codec's audio-language (`ISO_639_language_descriptor`) and DVB
  `subtitling_descriptor` were silently lost on a TS re-mux — a track lost
  information precisely *because* its codec was understood.
  - The new policy (documented with its spec citations in the `ts_mux` module
    doc) is a **deny-list, not an allow-list** — an allow-list would silently
    drop an unknown-but-valid broadcaster-private or newly-registered
    descriptor, which is the same class of bug.
  - **`CA_descriptor` (tag `0x09`, ISO/IEC 13818-1 §2.6.16) is denied.** It
    signals that the elementary stream is scrambled and names the `CA_PID`
    carrying its ECMs; this muxer emits cleartext, so copying it forward would
    falsely advertise the re-mux as encrypted and point at a `CA_PID` absent
    from the new PMT.
  - Inherited descriptors are **de-duplicated against the descriptors the
    muxer synthesises itself** (e.g. the `MPEG-H_3dAudio_descriptor` built
    from the typed `mpegh3daProfileLevelIndication`, issue #579), keeping the
    synthesised copy — emitting both yields a malformed `ES_info` loop with
    contradictory signalling under one tag.
  - Surviving descriptors keep their **source order**.
  - A merged loop over the 12-bit `ES_info_length` field's 4095-byte maximum
    (§2.4.4.8) returns `Error::BufferCapExceeded { what: "PMT ES_info
    descriptor loop", cap }` rather than being silently truncated into a
    malformed PMT.
- **BREAKING — one strictness policy everywhere: DEMUX = lenient but loud,
  MUX = strict but filterable** (B1-B4).
  - `Fmp4Demux` no longer fails the whole file on one track it cannot
    reconstruct (a QuickTime hint/chapter track, `c608`/`c708`, GoPro
    `gpmd`, ...) — it skips that track and records it, named, in the new
    `Media::skipped: Vec<SkippedTrack>`, matching `ProgressiveDemux`'s
    existing per-track leniency (the two used to diverge on identical
    input).
  - `CodecConfig::is_muxable_in_bmff()` (new, `pub`) now covers both the
    opaque `CodecConfig::Data` carriage and `CodecConfig::Subtitle` — B1:
    `CmafMux` (and every other fMP4/CMAF mux entry point) previously had no
    predicate covering `Subtitle`, so a subtitle-bearing CMAF asset that used
    to repackage fine now failed. A caller must pre-filter with
    `media.select_tracks_by(|t| t.spec.config.is_muxable_in_bmff())` before
    muxing a `Media` that mixes carriable and non-carriable tracks.
  - `Error::UnmuxableSubtitleTrack { track_id, format }` (new): the named
    rejection for a `Subtitle` track, mirroring `UnmuxableDataTrack`.
  - The strict-but-filterable check is now centralized in
    `build_init_segment` itself, so `CmafMux`, `ProgressiveMux`,
    `Segmenter`, `LlSegmenter`, and `LlHlsSegmenter` all reject a
    non-muxable track the same way (previously only `CmafMux` did; the
    other four silently dropped it).
  - The `transmux` CLI (`cli` feature) now filters non-muxable tracks (with
    a stderr warning naming them) before every fMP4/CMAF-based output
    format, so it no longer fails on an ordinary real-world DVB multiplex
    (DVB subtitle/teletext/ANC/SCTE-35 tracks are routine).
- **Audio DTS is frame-exact again (B5)**: `TsDemux` no longer re-derives an
  audio track's dts/pts from the lossy 90 kHz PES clock on every access
  unit (which injected up to ±1 track tick of jitter at every PES boundary,
  since 90000 does not evenly divide a typical sample rate) — it anchors
  once from the first access unit, then advances by the intrinsic per-frame
  duration, and only re-anchors (emitting `DemuxEvent::Discontinuity`) on a
  genuine gap. The same re-anchor-on-every-stamp bug, found via the new
  invariant test below, is fixed identically in `PsDemux`'s AC-3 track
  recovery.
- Added a per-timed-track invariant, checked on real fixtures across every
  demuxer in the crate (`tests/demux_timing_invariant.rs`): a track's
  `start_decode_time` must equal its first sample's `dts`, and
  `sum(sample.duration)` must equal the span from the first to the last
  sample's `dts` plus the last sample's `duration` — the standing guard
  against the whole re-derive-from-a-lossy-clock class of bug.
- **A codec-changed re-registration bypassed the shared-PID refcount.** The
  "removed" branch of `apply_pmt_diff` already refused to tear down a PID
  another PMT still declares (above); the codec-changed branch never got the
  same check, so a PID declared by two programs (an ordinary shared audio/
  subtitle component) had its track torn down and rebuilt — a fresh
  `track_id`, a spurious `TrackRemoved`/`TrackAdded` — the moment *either*
  declaring PMT reclassified its codec, even though the other program's
  declaration was unchanged. The codec-changed branch now consults
  `es_declarers` too: reclassification is refused while any other declarer
  still lists the PID (the existing classification wins; two programs
  declaring one PID under different codecs is a malformed multiplex, and
  last-writer-wins was rejected as letting either program's routine version
  bump flip the shared track back and forth), proceeding only once this PMT
  is the *last* declarer. The same re-registration also now restores the
  PID's original PMT-declaration-order slot in `codec_order`/`data_order`
  instead of losing it to the back of the list, which reordered `TrackAdded`
  emission and could block a later-ranked PID's promotion behind it.
- **`splice::concat`/`splice_insert` read a sample's true absolute `dts` for
  the join/snap position, not a duration-sum reconstruction.**
  `track_end_decode_time`, `snap_to_preceding_sync`, and `splice_insert`'s own
  boundary-decode-time calculation still derived decode time as
  `start_decode_time + Σ duration` — harmless while the two representations
  agreed, but the per-fragment `tfdt` reseed (above) can legitimately leave a
  gap between `start_decode_time` and a sample's true `dts`, which silently
  mis-placed the splice join/snap one step downstream of that fix. All three
  now read the sample's own absolute `dts` directly, falling back to the
  duration sum only for a genuinely timestamp-less (section-carried) sample.
- **The four flagship segmenter doctests (`Segmenter`, `LlSegmenter`,
  `LlHlsSegmenter`, `StreamingTsHlsSegmenter`) actually run now** (#780).
  Each was wrapped in `# if false { … }` with a `spec()` stub returning
  `unimplemented!()`, so rustdoc only type-checked the body and never
  executed it — a user copying the example got code that merely compiled.
  `spec()` now builds a real minimal AVC `TrackSpec`, the `# if false`
  wrapper is gone, and each example carries an assertion tied to the
  segmenter's actual cut/flush behaviour (verified to fail under a mutated
  segmenter, then reverted).
- **Cleared the non-blocking latest-stable clippy canary** (issue #770),
  failing on `main` unnoticed for many merges because it doesn't gate CI.
  Every change is a behaviour-preserving rewrite; no `#[allow]` was added.
  - `StreamingTsDemux::try_promote_ready`'s `loop { let Some(..) = .. else {
    break }; .. }` is now a `while let` (`clippy::while_let_loop`).
  - `FlvStreamDemux::process_audio_tag`'s nested `if
    audio.track_id.is_some()` inside the `aac_packet_type::RAW` match arm is
    now a match guard (`clippy::collapsible_match`); a guard-fail falls
    through to the pre-existing `_ => {}` no-op, exactly as before.
  - `tests/ir_timing.rs` drops two no-op `i64 as i64` casts
    (`clippy::unnecessary_cast`).

  The canary was red workspace-wide, not only here: `cargo` stops at the
  first crate that fails to compile, so clearing transmux's errors unmasked
  four more latest-stable-only lints in `multimux` and `ll-hls-runtime`,
  fixed in the same change (see those crates' changelogs). The full
  `cargo +stable clippy --workspace --all-features --all-targets -- -D warnings`
  now exits 0.

### Added

- **`CodecConfig::Subtitle { format: SubtitleFormat }`** (media plane step 2d):
  `Fmp4Demux` now demuxes `stpp` (TTML/IMSC, ISO/IEC 14496-30 §7.2) and `wvtt`
  (WebVTT, §9.2) ISOBMFF sample entries into this variant instead of silently
  dropping the track — samples stay opaque (never cue-parsed).
  `SubtitleFormat` (`#[non_exhaustive]`, `name()`/`Display` per the #204
  convention) also carries `DvbBitmap`/`Teletext` tokens for the PES-carried
  broadcast subtitle formats (still `CodecConfig::Data` on the TS demux path
  today). There is no re-mux path yet for a `Subtitle` track — `build_trak`
  rejects it with `Error::UnsupportedCodec` (`TODO(#753)`).
- The `ac-4` ISOBMFF sample entry now demuxes to the existing
  `CodecConfig::Ac4` (the mux direction already worked; only the demux arm was
  missing) — a full mux ↔ demux round trip.
- **Shared segmentation primitives in `transmux::segmenter`**, now public
  because all four segmenters use them and their behaviour is observable:
  `MediaClock` (per-track elapsed-media accounting: `duration` when it is a
  real, non-zero span, else the absolute `dts` delta), `choose_anchor` /
  `is_anchor_capable` (first video track of any codec, else the first track
  whose clock can advance; a section-only track set is an error), and
  `MAX_PENDING_SAMPLES_PER_TRACK` (the un-cut buffer bound). Previously each
  module carried its own copy, so the four could — and did — drift apart.
- **`DemuxEvent::TrackUpdated`/`TrackRemoved`/`TrackAbandoned`, PMT version
  diffing** (issue #774, unblocks rust-skyfire#96): `StreamingTsDemux` now
  diffs a PMT's `version_number`/`current_next_indicator` (ISO/IEC 13818-1
  §2.4.4.8) instead of only ever inserting newly-seen PIDs.
  - A version change that no longer lists a previously-declared PID emits
    `TrackRemoved { track_id, provenance }` (only for a PID that had already
    reached `Live` — a real `track_id` a consumer has seen).
  - A version change that alters an existing PID's `es_info_descriptors` or
    reclassifies its `stream_type` emits `TrackUpdated(TrackSpec)` (codec
    config recovery itself stays single-shot and permanent).
  - A track whose codec config never becomes recoverable by end of input, or
    whose probe/parked backlog exceeds its byte budget, emits
    `TrackAbandoned { track_id: Option<u32>, reason: AbandonReason,
    provenance }` (`AbandonReason::ConfigUnrecoverable` /
    `AbandonReason::BudgetExceeded`).
  - A carousel-repeated identical-version PMT section (several times a second
    on a real broadcast) is parsed but never re-diffed — no spurious events on
    an unchanged track set.

### Changed

- **BREAKING — two silent drops are now typed errors** (media plane step 2d,
  landed only after the coverage above, so `stpp`/`wvtt`/`ac-4` — which used
  to hit these paths — no longer do):
  - `Fmp4Demux` no longer silently skips a track whose sample entry it cannot
    reconstruct into a `CodecConfig`; it now returns
    `Error::UnsupportedSampleEntry { fourcc }`, naming the offending sample
    entry.
  - `CmafMux` no longer silently filters `CodecConfig::Data` tracks out of the
    init/media segments; it now returns
    `Error::UnmuxableDataTrack { track_id, stream_type }`. A caller that wants
    the old best-effort behaviour must pre-filter explicitly, e.g.
    `media.select_tracks_by(|t| !matches!(t.spec.config, CodecConfig::Data { .. }))`.
- **BREAKING — `Sample` timing is now absolute and optional** (media plane step
  2c, `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §4).
  `Sample` is now `{ data, dts: Option<i64>, pts: Option<i64>, duration:
  Option<u32>, flags: SampleFlags, provenance: Option<Provenance> }`:
  - `dts`/`pts` are **absolute** tick values in the track's own media timescale
    (`TrackSpec::timescale`), replacing the previous model in which a sample's
    time was the running sum of preceding `duration`s anchored on
    `Track::start_decode_time` — an anchor that FLV, WebM, MPEG Program Stream,
    RTMP and RTP all left at `0`. The IR can now address a splice point, rebase
    across sources, and express a send deadline.
  - 33-bit (MPEG-2 Systems, ISO/IEC 13818-1 §2.4.3.7) and 32-bit (RTP,
    RFC 3550 §5.1) rollover is unwrapped **once, at the demux edge**, and never
    re-derived downstream.
  - `Option`, not mandatory: section-carried tracks (SCTE-35 `stream_type`
    `0x86`, DSM-CC, private sections) genuinely have no timestamp, so they keep
    `dts`/`pts`/`duration` of `None` rather than a fabricated value.
  - `composition_offset` is no longer a stored field — it is implied by the
    pair, via the new `Sample::composition_offset()` (`pts - dts`), and still
    round-trips fMP4 `ctts` byte-identically.
  - `is_sync` moved into the `#[non_exhaustive]` `SampleFlags` struct
    (`sample.flags.is_sync`; `SampleFlags::SYNC` / `NON_SYNC` / `new`).
  - `Sample::new`/`from_annexb` take `(data, dts, pts, duration, is_sync)` and
    `from_raw` takes `(data, dts, pts, duration)`.
- **BREAKING — `SourceTiming` deleted.** It was write-only (the crate's own docs
  admitted "all mux paths in this crate ignore this field"). The source
  container's raw, pre-unwrap wire stamps survive as the debug-only
  `Provenance { wire_dts, wire_pts }` side-field (`Sample::with_provenance`), so
  no information is lost — the crate just stops presenting a debug field as a
  timing model.
- **BREAKING — `rebase::unroll_33bit_wraps` and `rebase::MPEG_TS_WRAP` removed.**
  Wrap-unrolling now happens once at the demux edge, so re-folding the IR
  timeline back into 33 bits in order to unwrap it again was exactly the
  anti-pattern this step removes. `rebase_to_zero` / `apply_offset` /
  `insert_discontinuity_gap` now shift every sample's absolute `dts`/`pts` in
  lockstep with `Track::start_decode_time` (and never fabricate a timestamp for
  a `None`-timed sample).
- **BREAKING — `DemuxEvent` reshaped a second time within this release**
  (found by the aggregate review below; applied here rather than deferred to a
  later major bump):
  - `TrackAdded(Track)` → `TrackAdded(TrackSpec)` — drops the always-empty
    `samples` and never-set `encryption` fields that came along with the full
    `Track`; every existing consumer already only read `track.spec`.
  - `Discontinuity { track, provenance }` → `Discontinuity { track, kind:
    DiscontinuityKind, provenance }`, and the variant is now `#[non_exhaustive]`
    with a `DemuxEvent::discontinuity(...)` constructor. `DiscontinuityKind` is
    `Signalled` (an MPEG-2 TS adaptation-field `discontinuity_indicator`),
    `TimelineReanchored` (a live audio track's frame-exact anchor drifted from
    the wire PES clock), or `BudgetExceeded { bytes }` (a per-PID buffer cap
    dropped in-flight data). `abandon_backlog`'s probe/parked-backlog budget
    cap now emits `TrackAbandoned` instead of a `Discontinuity` — it was
    mis-typed before (a budget overflow abandons the track; nothing survives
    to "continue" from).
  - `TracksResolved` → `TracksResolved { generation: u32 }` — fixes a live bug
    where the de-dup key was the known-PID *count*: a removal immediately
    followed by an addition could return the count to a previously-seen
    value, silently suppressing the re-fire a consumer needs. `generation` is
    a monotonic counter bumped once per applied PMT diff (add/update/remove).
  - `ClockReference` is now `#[non_exhaustive]` too, with a matching
    `DemuxEvent::clock_reference(...)` constructor — both non-exhaustive
    variants can grow a field later (e.g. a wall-clock/UTC anchor) without a
    further breaking change.
  - Documented `DemuxEvent`'s event-order guarantee (observation order per
    emission class, not wire order across classes) and its removal semantics
    (no `Sample` for a removed `track_id` ever follows its `TrackRemoved`;
    removal tracks only the PMT-declared set, never a silence timeout).
- **BREAKING — the rest of the IR surface is `#[non_exhaustive]` too**, applied
  in this same release rather than costing a major bump later (`Media`/`Track`
  were already done): the `DemuxEvent::Sample`,
  `TrackRemoved`, `TrackAbandoned` and `TracksResolved` **variants**, and the
  `Provenance`, `PcrSample`, `SkippedTrack`, `TrackEncryption` and
  `FragmentTrackData` structs. `TracksResolved` took a field this release,
  which is exactly the case this prevents recurring. Every affected type gained
  a constructor so none became unconstructible from outside the crate:
  `DemuxEvent::{sample, track_removed, track_abandoned, tracks_resolved}`,
  `Provenance::new`, `PcrSample::new`, `SkippedTrack::new`,
  `TrackEncryption::new`, `FragmentTrackData::new`. Pattern matches on the four
  variants need a trailing `..`. `EventProvenance` stays `Copy + Eq + Default`.
- **BREAKING — `HlsPackager` omits a timestamp-less track instead of emitting
  `#EXTINF:0.000`.** A section-carried track (SCTE-35 `stream_type` `0x86`,
  DSM-CC, private sections) has `duration: None` on every sample, deliberately
  and never fabricated; summing `unwrap_or(0)` over it rendered a zero
  `EXTINF`, which RFC 8216 §4.3.2.1 defines as a real playback duration a
  player would honour. An HLS media playlist is a timeline of playable
  segments and such a track is not one, so it is left out (its content reaches
  an output through the paths built for it — an inband `emsg`, an
  `EXT-X-DATERANGE`). A `Media` whose tracks are *all* timestamp-less is now a
  named `Error::InvalidInput` rather than an empty playlist.

### Security

Four CENC (ISO/IEC 23001-7) defects, two of them a **total loss of
confidentiality** for content encrypted by this crate. Anything encrypted by
`CencEncryptor` with the affected configurations must be re-encrypted with a
fresh content key — the exposure is in the ciphertext already published, not
only in the code.

**Affected releases: 0.16.0 through 0.19.0** (`CencEncryptor` first shipped in
0.16.0); fixed in 0.20.0. **Affected configurations:** the keystream-reuse
defect requires `CencScheme::Cenc` and a `Media` carrying **two or more
tracks** encrypted in one `encrypt` call — the overwhelmingly common
video+audio case. Single-track output, and all `CencScheme::Cbcs` output, are
unaffected by it. Re-keying alone is not sufficient if the same content is
re-encrypted with colliding IVs; take the 0.20.0 IV semantics below.

- **CRITICAL — BREAKING: AES-CTR keystream reuse across tracks (a two-time
  pad).** `CencEncryptor::encrypt` restarted its per-sample IV counter at
  `base` for *every* track while applying one shared content key, so under
  `cenc` (AES-CTR) video sample *i* and audio sample *i* were encrypted with
  the same key **and** the same counter block. XOR-ing the two ciphertexts
  cancels the keystream and yields the XOR of the two plaintexts — both
  disclosed without the key. ISO/IEC 23001-7 §9.2 requires the IV to be unique
  per *key*; "unique per track" is not sufficient when one key covers every
  track. Measured on the two-track `fixtures/ts/h264_aac.ts`: 75 of 206
  per-sample IVs collided. Every pre-existing test narrowed its fixture to a
  single track, which is why a green suite shipped it. An adversarial review
  of the first fix (below) found it incomplete in two further ways, both
  closed in this same 0.20.0 (never separately released): the duplicate-IV
  backstop ran *after* every track had already been keystreamed in place
  (fixed by validating the full planned IV sequence before ciphering a single
  byte), and IV uniqueness was only ever guaranteed *within* one `encrypt`
  call, not across separate calls sharing a key (fixed by moving the running
  counter onto the encryptor instance). See the two follow-up entries below.
  - `IvGen::Counter { base }`'s sample index now runs continuously across the
    whole `Media` in (track, sample) order, and never resets per track.
  - **BREAKING** — `IvGen::Explicit(ivs)` now requires exactly
    `media.tracks.iter().map(|t| t.samples.len()).sum()` IVs, consumed in
    (track, sample) order. Previously it was validated against *each track's*
    count, so one list was replayed verbatim for every track — the same defect
    by a different route. A caller passing a per-track-sized list now gets
    `Error::InvalidInput` instead of silent keystream reuse.
  - `IvGen::Explicit` also rejects a list containing any **duplicate** IV.
  - All IV validation — including the whole-`Media` duplicate check — now
    happens before the first sample is ciphered, so a rejected configuration
    leaves the `Media` untouched instead of half-encrypted (see the follow-up
    entry immediately below for why this is now actually true, not just
    documented as true).
- **CRITICAL — BREAKING (adversarial-review follow-up, F1): the duplicate-IV
  backstop ran after ciphering, not before.** The check above that rejects a
  duplicate per-sample IV used to run against every track's *already-recorded*
  `Track::encryption` — i.e. **after** `encrypt`'s per-track cipher loop had
  already overwritten every sample via a real AES-CTR/CBC pass. Its whole
  purpose is catching a *reintroduced* per-track-reset bug (the defect above),
  so on the one path it exists to guard, `encrypt` returned `Err` while `media`
  was left irreversibly two-time-padded with the plaintext already gone and no
  rollback — exactly contradicting the CHANGELOG claim above ("a rejected
  configuration leaves the Media untouched"), which was therefore false for
  this one path. `CencEncryptor::encrypt` now resolves the **entire** planned
  (track, sample) → IV mapping up front (`plan_sample_ivs`) and validates the
  whole plan for duplicates (`assert_ivs_unique`) *before* any cipher work
  runs; the cipher loop then consumes that exact, already-validated plan
  (never re-resolving), so there is no window in which what was checked can
  drift from what gets recorded. Verified by reintroducing the historical
  per-track reset into `plan_sample_ivs` (Edit-then-revert): `encrypt` returns
  `Err` and every sample's bytes remain byte-identical to the input.
- **CRITICAL — BREAKING (adversarial-review follow-up, F2): IV uniqueness was
  only guaranteed within one `encrypt` call, not per key for all time.**
  AES-CTR requires IV uniqueness per **key**, for the life of that key — not
  merely within a single `encrypt` invocation. `IvGen::Counter { base: 0 }`
  (the `Default`) restarted its counter at `0` on every call, so two
  invocations sharing a key — a video-only + audio-only split of one asset, or
  successive live segments encrypted under one key period — reproduced the
  exact two-time pad the first fix closed *within* one call. `CencEncryptor`
  is now a **stateful** value constructed with its content key
  (`CencEncryptor::new(key)` / `CencEncryptor::resume(key, next_counter)`),
  owning the running `IvGen::Counter` index itself and advancing it after
  every successful call rather than resetting it — reusing *one* instance
  across every call that shares a key continues the counter instead of
  restarting it. Consequences:
  - **BREAKING** — `Encrypt::encrypt` (the `broadcast-common` hub trait) now
    takes `&mut self`, not `&self` — the only production implementor
    (`CencEncryptor`) needs mutable state to own the counter; the only other
    implementor workspace-wide is a trait-usability test fixture inside
    `broadcast-common`'s own test module, updated identically.
  - **BREAKING** — `EncryptConfig` no longer carries a `key` field (the key
    now lives on the `CencEncryptor` instance, so a per-call key could never
    silently pair one running counter with a different key). `IvGen::Counter`
    is now a unit variant (no `base` field); construct a resumed encryptor
    with `CencEncryptor::resume(key, base)` for the equivalent effect.
  - Verified: two successive `encrypt` calls on one `CencEncryptor` instance
    that would previously collide now produce fully disjoint IV sets.
    Constructing a second, *separate* `CencEncryptor::new(key)` with the same
    key still collides (each starts its counter at `0`) — this is
    structurally undetectable from inside the type (it has no way to know
    another instance ever used `key` before) and is documented on
    `CencEncryptor` as the caller's remaining obligation: reuse one instance
    per key, never construct a fresh one for a key already in use.
- **Minor — cipher-core IV lengths tightened to exactly 8 or 16 bytes.**
  `cenc_crypto`'s shared `apply_ctr`/`resolve_cbcs_iv` (used by both the
  encrypt and decrypt paths, so this also hardens decryption of untrusted
  files) accepted any length `1..=16`, silently zero-padding it to a 16-byte
  counter/CBC-seed block — so two differently-invalid-length IVs whose
  non-zero bytes happened to coincide could zero-pad to the *same* block.
  Only 8 and 16 bytes are valid on the wire (ISO/IEC 23001-7 §9.2/§12.2); any
  other length is now `Error::InvalidValue`.
- **CRITICAL — BREAKING: `IvGen::Constant` + `CencScheme::Cenc` produced
  unreadable output encrypted under an all-zero counter.** The `cenc` cipher is
  never handed the track's `tenc`, so `IvGen::Constant`'s 16-byte seed (which
  lives only in `tenc.default_constant_IV`) never reached it: every sample was
  encrypted with an all-zero counter block — one keystream for the entire
  track, and output no conformant decryptor can read. The combination is now
  rejected with `Error::InvalidInput`; a constant IV is fundamentally
  incompatible with CTR mode. `IvGen::Constant` is documented as `cbcs`-only
  (where it is the standard convention) and is unchanged under `cbcs`.
- **`cenc` decrypt no longer "succeeds" over garbage.** A conformant file whose
  `tenc` declares `default_per_sample_iv_size == 0` with a
  `default_constant_IV` yields empty `senc` IVs; the `cenc` path decrypted
  those against an all-zero counter and returned `Ok(())` over rubbish. An
  empty per-sample IV under `cenc` is now a typed error in both directions.
- **`senc` no longer sizes an allocation from an unbounded wire field (remote
  DoS).** `SampleEncryptionBox::parse_body` sized
  `Vec::with_capacity(sample_count)` from the untrusted 32-bit `sample_count`
  before any bounds check: a 20-byte box carrying `FF FF FF FF` requested a
  single **206 GB** allocation (measured). `sample_count` is now bounded
  against the box body's own length via each entry's minimum on-wire size, and
  a `senc` whose entries would carry no bytes at all (no per-sample IV *and* no
  subsample map — a shape whose count the wire cannot corroborate at any
  length) is rejected. `saio`'s `entry_count * offset_size` is likewise
  `checked_mul`'d so it cannot wrap past its length check on a 32-bit target.

### Fixed (CENC, lower severity)

- **A failed cipher pass no longer commits a half-encrypted payload.**
  `cenc_crypto`'s in-place rewrite committed the buffer even when the cipher
  returned `Err`, so a malformed `senc` that overran on its *second* subsample
  left the first subsample keystreamed and stored that back into
  `Sample::data`. Both cipher entry points now validate the entire subsample
  map up front, after which their block loops cannot fail — so an error leaves
  the sample byte-identical, and never leaves it empty.
- **Subsample maps must now cover the whole sample on decrypt too** (ISO/IEC
  23001-7 §9.3, matching the encrypt side): a map declaring 100 bytes of a
  1000-byte sample used to return `Ok(())`, leaving 900 bytes of ciphertext
  presented as plaintext.
- **BREAKING — `CencDecryptor`'s `Decrypt::decrypt` pairs tracks by `track_id`,
  not by position.** It zipped `Media` tracks to crypto records positionally
  while storing the `track_id` for exactly that purpose, so a
  `select_tracks_by`-narrowed `Media` decrypted (say) audio with the *video*
  track's IVs — and when the two tracks' sample counts coincided, the existing
  count check did not catch it either. A media track with no matching record is
  now `Error::InvalidInput`; unmatched *records* remain fine (the narrowed
  case).
- **`iter_length_prefixed_nals` checks its NAL-length addition.** A 4-byte NAL
  length of `0xFFFFFFFF` wrapped `start + len` on a 32-bit `usize`, defeating
  the overrun guard and panicking on the subsequent slice. Reachable from
  `CencEncryptor`'s subsample-map construction on caller-supplied sample data.
- Added a `transmux_cenc_boxes` fuzz target covering the `senc`/`saiz`/`saio`
  parsers (untrusted third-party media; 7.1 M executions clean).
- **(F4) `CencDecryptor` can now decrypt a legitimately `senc`-less fragment.**
  A constant-IV, whole-sample-protected `cbcs` fragment carries none of
  `senc`/`saiz`/`saio` at all (nothing for them to carry — see
  `movie_fragment.rs`'s `build_cenc_fragment_boxes`, issue R3); the decrypt
  side's per-fragment harvesting only ever populated a track's per-sample
  entries *from* a `senc` box, so this shape's `crypto.samples` silently
  stayed empty and `Decrypt::decrypt` always rejected it with a generic
  "sample count mismatch", despite `tenc.default_constant_IV` alone being
  everything a conformant decryptor needs. `harvest_fragment_senc` now
  synthesizes the placeholder per-sample entries this shape needs (one empty
  IV/subsample-map pair per `trun` sample) when `senc` is legitimately absent
  (`default_per_sample_iv_size == 0`); a genuinely missing `senc` on a
  per-sample-IV track is unaffected (still tolerated, still caught downstream
  by the same count check). Found and fixed while adding end-to-end coverage
  for this branch, which had previously been exercised only by this crate's
  own parser, never a real decryptor.
  - That same new coverage found a real, verified interop limitation in
    Bento4's `mp4decrypt` (1.6.0.0): given this exact, ISO/IEC-23001-7-legitimate
    `senc`-less shape, `mp4decrypt` does not decrypt it at all — the output
    `mdat` comes back byte-for-byte unchanged (still ciphertext), no error
    reported. This is not a defect in this crate's cipher math (independently
    re-verified outside this crate: the real ciphertext, AES-128-CBC-decrypted
    with the real key and `tenc.default_constant_IV`, reproduces the true
    plaintext exactly) or in the `saio` anchor (there is no `senc`/`saio` in
    this shape to be wrong about) — it appears `mp4decrypt` requires *some*
    per-sample aux-info structure present in the `traf` before it will attempt
    to decrypt a track, regardless of `tenc`. Documented on the new
    `tests/cenc_encrypt_e2e.rs` test rather than silently masked; the crate's
    own (now-fixed) self round-trip remains that test's hard gate. Tracked as
    a known third-party limitation, not a defect introduced or fixable by this
    story.

## [0.19.0] - 2026-07-26

### Added

- `StreamingFlvDemux` (issue #738): incremental (event-driven) FLV → samples
  demux for live RTMP ingest, the FLV analogue of `StreamingTsDemux`. Feed
  bytes of any size/alignment via `feed` (down to one byte at a time; feed's
  fallible over a hard structural error — bad signature, an implausible
  header `DataOffset`, or a corrupt codec config), drain `DemuxEvent`s
  (`TrackAdded`/`Sample`) one at a time via `poll_event`, and call `finish` to
  flush each track's trailing pending sample — the exact `feed`/`poll_event`/
  `finish` pull idiom `StreamingTsDemux` uses, so a caller can drive both
  demuxers with one uniform drain loop. Reuses `FlvDemux`'s tag-header and
  codec-config (AVC/AAC) parsing verbatim rather than duplicating it.
  Memory-bounded regardless of stream length: the internal buffer never holds
  more than one in-progress tag, plus one pending sample per track; a header
  `DataOffset` above a sane bound is rejected before buffering rather than
  grown toward, closing a remote OOM/DoS a malicious `DataOffset` could
  otherwise trigger. `AVCDecoderConfigurationRecord::parse` (`avc_config.rs`)
  now also rejects an avcC declaring 0 SPS, closing a remote-crash (index
  panic) hole on this same untrusted-ingest path.
- `dash_parse` (issue #758 T1): a hand-rolled MPD parser (`Mpd::parse`), the
  structural inverse of `dash`'s `DashPackager` writer — `no_std`+`alloc`, no
  external XML dependency. Parses `MPD`/`Period`/`AdaptationSet`/
  `Representation`/`SegmentTemplate`/`SegmentTimeline` (ISO/IEC 23009-1
  §5.3/§5.3.9), resolves `SegmentTemplate` inheritance from `AdaptationSet`
  down to `Representation`, and tolerates unmodeled elements
  (`SegmentList`/`SegmentBase`/`ContentProtection`/…) without choking.
  `SegmentTemplate::resolve` substitutes `$RepresentationID$`/`$Number$`/
  `$Time$`/`$Bandwidth$` (with `%0Nd` width and `$$` escaping) into a
  template; `SegmentTimeline::enumerate` expands `<S t= d= r=>` runs into the
  `(number, time)` sequence for `$Time$` addressing, and
  `SegmentTemplate::number_sequence` does the equivalent for constant-
  `@duration` `$Number$` addressing. Also exposes `parse_iso8601_duration`
  for `xs:duration` attributes. Malformed/truncated XML never panics —
  every failure path returns `DashParseError`.
- `smooth_parse` (issue #759 T1): a hand-rolled MS-SSTR **client manifest**
  parser (`SmoothManifest::parse`), the structural inverse of `smooth`'s
  `SmoothPackager` writer — reuses the shared `xml_parse` tokenizer (no
  second XML parser), `no_std`+`alloc`. Parses `SmoothStreamingMedia`/
  `StreamIndex`/`QualityLevel`/`c` (§2.2.2.x), including the live-only
  `IsLive`/`LookAheadFragmentCount`/`DVRWindowLength` attributes (§2.2.2.1).
  `StreamIndex::enumerate_chunks` expands a `c@r` repeat run into the
  `(t, d)` sequence, capped at `MAX_CHUNK_RUN` (100k, mirroring
  `dash_parse::MAX_TIMELINE_SEGMENTS`) against a hostile Manifest's unbounded
  `<c r="...">`; `hex_decode`-ing a `QualityLevel@CodecPrivateData` value is
  likewise capped at `MAX_CODEC_PRIVATE_DATA_HEX_LEN` before any decode
  allocation. `StreamIndex::resolve_fragment_url` substitutes the
  `{bitrate}`/`{start time}` tokens (§2.2.4.1) into the fragment-URL
  template. Because Smooth has no bootstrapping init segment,
  `track_spec_from_quality_level` synthesizes a `pipeline::TrackSpec` from a
  `QualityLevel`'s `CodecPrivateData` (Annex-B SPS/PPS → `avcC` for
  `FourCC="H264"`, raw `AudioSpecificConfig` → `esds` for `FourCC="AACL"`) —
  feed the result to `pipeline::build_init_segment` so `media::Fmp4Demux` can
  absorb the Smooth fragment stream. Also splits `rtp_sdp::avc_config_from_sprop`/
  `aac_config_from_asc_hex` into byte-level building blocks
  (`avc_config_from_sps_pps`/`aac_config_from_asc_bytes`) that a non-SDP
  caller (this module) can reuse directly, rather than duplicating the SPS
  classification / `avcC`/`esds` construction logic.
- **Shared `xml_parse` tokenizer** (issue #758 T1 + #759 T1): both `dash_parse`
  and `smooth_parse` modules extract and reuse a hand-rolled, no_std-capable
  XML token stream parser (`xml_parse::XmlTokenizer`) that neither depends on
  an external XML library nor allocates during parsing — the tokenizer walks
  wire bytes and yields tokens on demand.

### Fixed

- `dash_parse` (issue #758 T1): two critical remote alloc-DoS vectors in the
  MPD parser capped + end-tag name matching to prevent silent truncation:
  1. `SegmentTimeline::enumerate` now returns `Result` and caps total segment
     enumeration at `MAX_TIMELINE_SEGMENTS` (100k), rejecting hostile MPDs with
     unbounded `<S r="...">` repeats instead of allocating/looping unboundedly.
  2. `SegmentTemplate::resolve` clamps `%0Nd` format width to `MAX_FORMAT_WIDTH`
     (20 digits, the max for u64), defending against a hostile `$Number%9999999999d$`
     template allocating/looping unboundedly.
  3. All element parsing functions now validate closing-tag names — a stray
     `</Period>` no longer silently truncates the Period list. `Mpd::parse` +
     callers now return `DashParseError::MismatchedEndTag` on nesting errors.

## [0.18.1] - 2026-07-21

### Added

- `RtpStreamDepacketiser::push_sender_report`/`push_rtcp` + `sync_start_decode_times`
  (issue #722): RTCP Sender Report (RFC 3550 §6.4.1) NTP-wallclock ↔
  RTP-timestamp correlation for cross-track A/V sync, replacing the module's
  `TODO(P5.3)`. Feeding an SR for at least two tracks lets
  `sync_start_decode_times` compute each anchored track's `start_decode_time`
  on one common wallclock, preserving the real inter-track offset that
  independent per-track rebase-to-0 discards. Strictly additive/opt-in: with
  no Sender Reports fed, `sync_start_decode_times` returns an empty `Vec` and
  existing callers keep the unchanged v1 behaviour.

## [0.18.0] - 2026-07-21

### Changed (BREAKING)
- Renamed `RtpPacketiser`, `RtpDepacketiser`, `RtpStreamDepacketiser`, and the
  free functions `packetise_klv`/`depacketise_klv` to British spelling (issue
  #663; all were previously spelled with a "z"). Pure rename —
  behaviour-preserving, no functional change. RFC 6184's SDP `fmtp`
  `mode`-selection attribute key is an external wire-protocol string and was
  deliberately left spelled exactly as the RFC defines it.

### Added

- `hls::MediaPlaylist::parse` + `hls::MasterPlaylist::parse` (issue #717
  slice 1): the symmetric inverse of the existing `to_m3u8()` renderers,
  parsing an m3u8 string back into the same structs — so an LL-HLS client
  can reuse the origin's wire model rather than growing a second one.
  Recognizes `#EXTM3U`/`#EXT-X-VERSION`/`#EXT-X-TARGETDURATION`/
  `#EXT-X-MEDIA-SEQUENCE`/`#EXT-X-DISCONTINUITY-SEQUENCE`/`#EXTINF`/
  `#EXT-X-ENDLIST`/`#EXT-X-DISCONTINUITY`/`#EXT-X-BYTERANGE`/`#EXT-X-MAP`
  plus the LL-HLS client-relevant tags `#EXT-X-PART-INF`,
  `#EXT-X-SERVER-CONTROL` (`CAN-BLOCK-RELOAD`/`PART-HOLD-BACK`/
  `CAN-SKIP-UNTIL`), `#EXT-X-PART` (`DURATION`/`URI`/`INDEPENDENT`/`GAP`/
  `BYTERANGE`), `#EXT-X-PRELOAD-HINT` (`TYPE`/`URI`/byte-range),
  `#EXT-X-RENDITION-REPORT`, `#EXT-X-SKIP` (delta updates), and
  `#EXT-X-STREAM-INF`/`#EXT-X-I-FRAME-STREAM-INF` on the Multivariant side.
  Unrecognized tags are preserved verbatim into `extra_tags` (never an
  error); a malformed known tag (missing required attribute, unparsable
  value) returns the new `Error::HlsParse { line_no, line, reason }`.
  New public types: `ByteRange`, `MapTag`, `PreloadHintType`,
  `RenditionReport`, `SkipInfo`. `MediaSegment` gained `byte_range: Option<ByteRange>`
  and `map: Option<MapTag>`; `PartSpec` gained `byte_range`/`gap`;
  `LowLatencyConfig` gained `can_skip_until`/`preload_hint_type`/
  `preload_hint_byte_range_start`/`preload_hint_byte_range_length`;
  `MediaPlaylist` gained `rendition_reports`/`skip` — all breaking for any
  external struct-literal construction (all five structs now derive
  `Default`, so existing call sites can add `..Default::default()`).
  `to_m3u8()` gained matching render support for every new field
  (`#EXT-X-MAP` dedup/carry-forward, `#EXT-X-BYTERANGE`,
  `#EXT-X-SKIP`, `#EXT-X-RENDITION-REPORT`, `CAN-SKIP-UNTIL`, part
  `BYTERANGE`/`GAP`, preload-hint `TYPE`/byte-range) so the round trip is
  lossless. `#EXT-X-MEDIA` (Multivariant alternate renditions) remains
  unmodeled on both the render and parse side — a documented gap, not a
  silent drop (nothing to preserve it into on `MasterPlaylist`, which has
  no `extra_tags` field).
  - `hls::OpenSegment` (the in-progress LL-HLS segment builder) gained a
    `map: Option<MapTag>` field and an `OpenSegment::with_map` builder, so a
    muxer assembling an open segment can carry forward the Media
    Initialization Section in effect for it, mirroring `MediaSegment::map` on
    the parse side. `OpenSegment::new` still defaults to no map.
  - **Fix** (found while building the `ll-hls-client` tokio adapter, issue
    #717 slice 5): `LowLatencyConfig` gained `can_block_reload: bool` — the
    `CAN-BLOCK-RELOAD` attribute's actual `YES`/`NO` value was previously
    discarded by `parse` (only the tag's *presence* set `low_latency =
    Some(...)`), so a client had no way to distinguish a genuine
    `CAN-BLOCK-RELOAD=NO` origin from one that supports blocking reload —
    both parsed identically. `parse` now reads the real value, defaulting to
    `false` (RFC 8216bis: absent means not supported) when the attribute (or
    the whole `#EXT-X-SERVER-CONTROL` tag) is missing; `to_m3u8()` renders
    the actual value instead of always emitting `YES`.
    `LowLatencyConfig::default()` still defaults to `true` (matching every
    existing in-repo caller that builds one from scratch via
    `..Default::default()`, all of which do support blocking reload) — only
    the *parser's* default differs, deliberately, since it reflects the wire
    rather than a convenience default.

### Security

- **Bounded RTP/TS reassembly buffers** (issue #663 P5.2, audit-ingest
  #4 — memory-exhaustion hardening): two streaming-reassembly buffers
  previously grew without bound against malformed or hostile input.
  `rtp_stream::RtpStreamDepacketiser`'s per-track in-progress access-unit
  buffer (a dropped/corrupted final FU-A fragment, or a marker bit that
  never arrives, previously accumulated RTP packets for the life of the
  session) is now capped at `MAX_AU_BUFFER_BYTES` (4 MiB); on overflow the
  partial AU is dropped and `push` returns the new recoverable
  `Error::BufferCapExceeded`, with internal state already reset so the next
  packet starts a fresh AU. `ts_demux::StreamingTsDemux`'s per-PID PES
  buffer (a `payload_unit_start_indicator` that never recurs — the
  unbounded-video `PES_packet_length = 0` case on a wedged/lossy stream —
  previously accumulated forever) is now capped at `MAX_PES_BUFFER_BYTES`
  (4 MiB); on overflow the partial PES is dropped and a
  `DemuxEvent::Discontinuity` is raised for the PID. Neither cap changes
  behaviour for any well-formed stream (a real access unit/PES is orders of
  magnitude smaller); PSI/private-section buffering needed no equivalent
  change — `mpeg_ts::ts::SectionReassembler` is already inherently bounded
  by `section_length`'s 12-bit field.

## [0.17.0] - 2026-07-15

### Added

- Streaming, timing- and config-aware RTP depayloader `RtpStreamDepacketizer`
  (RFC 6184 H.264 / RFC 3640 AAC): incremental `push`/`flush`, real per-sample
  duration from RTP-timestamp deltas, `is_sync` from IDR detection. v1 assumes
  low-delay H.264 (no B-frame reorder; `composition_offset = 0`), one AAC access
  unit per packet, and in-arrival-order packet feed. `RtpStreamTrack` is
  `#[non_exhaustive]` — construct via `RtpStreamTrack::new`.
- SDP fmtp/rtpmap → `CodecConfig` helpers in `rtp_sdp`, all re-exported at the
  crate root (issue #702): `fmtp_param` — a proper, anchored (no
  substring-match false positives), charset-safe parser for the `a=fmtp:<pt>
  <params>` `;`-separated `key=value` list (RFC 4566 §5.14); full-fmtp-line
  entry points `avc_config_from_fmtp` (RFC 6184 §8.1
  `sprop-parameter-sets` → avcC) and `aac_config_from_fmtp` (RFC 3640 §4.1
  `config` → esds) built on it; and `rtpmap_clock_rate`, parsing the
  companion `a=rtpmap:<pt> <encoding>/<clock>[/<channels>]` clock rate (RFC
  4566 §6). The prior value-level `aac_config_from_fmtp(config_hex)` is
  renamed `aac_config_from_asc_hex` (breaking) to make room for the new
  full-fmtp-line function of the same codec-pair naming; `avc_config_from_sprop`
  is unchanged. This replaces the fragile hand-rolled substring fmtp parsing
  multimux previously needed to do itself.
- `hls::OpenSegment` + `MediaPlaylist::open_segment` (issue #702): renders
  an in-progress, `#EXTINF`-less LL-HLS segment as trailing `#EXT-X-PART`
  lines at the live edge (RFC 8216bis §4.4.4.9), so a live LL-HLS origin can
  publish its still-open trailing segment through the library instead of
  hand-rolling `#EXT-X-PART` lines. Opt-in and gated the same way as the
  existing closed-segment parts: rendered only when `low_latency` is `Some`.
  `MediaPlaylist` gained the `open_segment: Option<OpenSegment>` field
  (breaking for any external struct-literal construction).

## [0.16.0] - 2026-07-14

### Added

- **`CencEncryptor` — CENC/CBCS encrypt path** (issue #564): a new
  `Encrypt` impl that protects a cleartext `Media` in place, the write-side
  counterpart to the existing `CencDecryptor`. Takes an
  `EncryptConfig { scheme, kid, key, iv, pattern, subsample }`: `scheme`
  selects `cenc` (AES-128-CTR, full-block) or `cbcs` (AES-128-CBC,
  `crypt`:`skip` pattern); `iv` is an `IvGen` (`Counter { base }` — an 8-byte
  per-sample counter; `Explicit(Vec<Vec<u8>>)` — one caller-supplied IV per
  sample; `Constant([u8; 16])` — the standard real-world `cbcs` convention,
  `tenc.default_constant_IV` with no per-sample `senc` IV); `subsample`
  selects the per-codec clear/protected byte-range policy (`SubsamplePolicy`).
  Encrypting a track populates `Track::encryption` (`TrackEncryption`
  carrying the resolved `tenc` + a `SampleEncryptionEntry` per sample) for the
  muxer's box emission and the DASH/HLS signalling below to read.
- **`protect_init_segment` / `protect_media_segment` — CENC/CBCS box
  emission** (issue #564): post-processing passes over an already-muxed CMAF
  init/media segment that splice in the CENC/CBCS boxes from a
  `TrackEncryption`, without CENC plumbing through the lower-level
  `TrackSpec`/pipeline: `protect_init_segment` (`init_segment.rs`) rewrites
  the target track's sample entry to `encv`/`enca` + `sinf`
  (`frma`/`schm`/`schi`/`tenc`), recomputing every ancestor box
  (`stsd`/`stbl`/`minf`/`mdia`/`trak`/`moov`) from its typed children;
  `protect_media_segment` (`movie_fragment.rs`, given a `FragmentProtection`
  per protected `traf`) adds the fragment's `senc`/`saiz`/`saio` (`saio`
  anchored `moof`-relative, verified against Bento4's `mp4decrypt` — see the
  e2e entry below). Every other byte, box, and track round-trips unchanged.
- **DASH/HLS CENC/CBCS DRM signalling** (issue #564): both driven straight off
  `Track::encryption` — no DRM logic in either. **DASH**
  (`DashPackager::package`) auto-derives the generic-CENC identification
  element per `AdaptationSet` — `<ContentProtection
  schemeIdUri="urn:mpeg:dash:mp4protection:2011" value="cenc"|"cbcs"
  cenc:default_KID="...">` (ISO/IEC 23001-7) — from the set's encrypted
  tracks; `ContentProtectionSystem` gained an optional `pssh: Option<Vec<u8>>`
  field so caller-supplied DRM-system entries
  (`schemeIdUri="urn:uuid:..."`, e.g. built with the `drm` module's pssh
  builders) render a base64 `<cenc:pssh>` child. **HLS** — the new
  `cenc_ext_x_key(scheme, kid, key_uri)` renders
  `#EXT-X-KEY:METHOD=SAMPLE-AES,URI="...",KEYFORMAT="urn:mpeg:dash:mp4protection:2011",KEYFORMATVERSIONS="1",KEYID=0x<kid>`
  for `cbcs`, returning `None` for `cenc` (AES-CTR has no valid HLS `METHOD` —
  `cenc`-protected CMAF is signalled on the DASH side only). A new runnable
  example, `examples/cenc_encrypt.rs`, drives the whole path (demux -> encrypt
  -> mux -> protect -> DASH/HLS signalling) against the real
  `fixtures/ts/h264/main.ts` fixture.
- **CENC/CBCS encrypt-path end-to-end proof** (issue #564): new
  `tests/cenc_encrypt_e2e.rs` exercises the full `CencEncryptor::encrypt` ->
  `CmafMux::package` -> `protect_init_segment`/`protect_media_segment`
  pipeline against `fixtures/ts/h264/main.ts`, then verifies the resulting
  fMP4 two independent ways: a self round-trip through `CencDecryptor`, and a
  golden-interop cross-check against Bento4's real `mp4decrypt` CLI. Both
  `cenc` and `cbcs` pass; the `cenc` case confirms the `saio` moof-relative
  anchor decision against real third-party tooling.

### Fixed

- **`cenc_decrypt`: fragmented CMAF support (`moof`/`traf`/`senc`/`saiz`/`saio`) +
  `cbcs` (AES-CBC pattern) decrypt** (issue #564): `CencDecryptor` previously
  only supported progressive fMP4 (single `moov`/`mdat`, sample layout from
  `stsz`/`stsc`/`stco`) — the real-world fragmented CMAF case (`moov` + one or
  more `moof`/`mdat` pairs, each `traf` carrying its own `senc`/`saiz`/`saio`)
  was entirely unsupported. `CencDecryptor::from_fmp4`/`demux` now walk every
  `moof`, matching each `traf` to its `moov`-declared track by `tfhd.track_id`
  and concatenating every fragment's samples in file order, reusing the
  already-typed fragment parsers in `movie_fragment` (`MovieFragmentBox`,
  `TrackFragmentHeaderBox`, `TrackFragmentRunBox`) rather than a second
  `moof`/`traf`/`trun` walker.
  Also implements the `cbcs` (AES-128-CBC pattern cipher) scheme, previously
  unimplemented (`Decrypt::decrypt` unconditionally rejected any non-`cenc`
  scheme): the `default_crypt_byte_block`:`default_skip_byte_block` pattern is
  applied across a sample's protected bytes, chaining across pattern-skip runs
  within one subsample's protected range and resetting to the sample's seed IV
  at the start of every subsample's protected range (see the next entry — the
  cross-subsample reset rule was corrected after this fix originally shipped),
  with the IV resolved from either the per-sample `senc` entry or the track's
  `tenc.default_constant_IV`.
  New fixtures `fixtures/transmux/h264_cenc.mp4` / `h264_cbcs.mp4` (real,
  fragmented, Bento4 `mp4encrypt`-produced) back
  `tests/cenc_fragmented_fixture.rs`, including a golden-interop cross-check
  against Bento4's own `mp4decrypt`.
- **`cbcs`'s CBC pattern chain now resets per subsample, and `cenc_encrypt`
  gained constant-IV/16-byte-IV support** (issue #564): `cenc_crypto.rs`'s
  `cbcs` pattern cipher (shared by `CencEncryptor` and `CencDecryptor`)
  previously carried its running CBC chain over from one subsample's last
  encrypted block into the *next* subsample's first encrypted block; it now
  resets the chain to the sample's resolved seed IV at the **start of every
  subsample's protected range**, while still chaining correctly *within* one
  subsample's own pattern-skip runs (unchanged, and unchanged for `cenc`'s CTR
  counter, which stays continuous across subsamples as before). Triangulated
  against Bento4's `mp4decrypt` and Shaka Packager (ISO/IEC 23001-7 itself is
  unowned/paid, so the reference implementations are the source of truth): the
  old cross-subsample-continuous chain reproduced only the first protected
  subsample of a multi-subsample sample correctly and silently diverged from
  Bento4 on every later subsample's first crypt block, while still
  round-tripping through this crate's own encrypt/decrypt pair
  (self-consistent, not spec/interop-correct) — undetectable without an
  external oracle. `cenc_encrypt.rs`'s `IvGen` gained a `Constant([u8; 16])`
  variant (the standard real-world `cbcs` convention: `tenc.default_constant_IV`
  + `default_per_sample_iv_size = 0`, no per-sample `senc` IV), and
  `default_per_sample_iv_size` is now derived from the chosen `IvGen` instead
  of a hard-coded `8` (which Bento4's `mp4decrypt` silently no-ops on for
  `cbcs`). `tests/cenc_encrypt_e2e.rs`'s `cbcs` case now uses
  `SubsamplePolicy::Video` (a real per-NAL multi-subsample map — the case that
  exposed the bug) and `IvGen::Constant`, proving both the fix and the
  constant-IV wire convention end to end against the real `mp4decrypt` oracle;
  `tests/cenc_fragmented_fixture.rs`'s existing single-subsample-per-sample
  `h264_cbcs.mp4` regression (which the bug never affected, since it never
  crosses a subsample boundary) remains byte-exact green.

### Security

- **CENC encrypt-path input validation hardening** (issue #564 review): the
  encrypt API previously accepted nonsense configuration and silently shipped
  unprotected or corrupt output instead of erroring. `CencEncryptor::encrypt`
  now rejects: a per-sample `IvGen::Explicit` IV whose length isn't exactly 8
  or 16 bytes (an empty IV previously built an all-zero AES-CTR counter — a
  two-time-pad reusing the same keystream across every sample); a `cbcs`
  pattern with `crypt_byte_block == 0` and a nonzero `skip_byte_block` (the
  shared `cenc_crypto::cbcs_sample`, used by both encrypt and decrypt, now
  guards this — previously it left the "protected" range in cleartext while
  `tenc.default_is_protected` still claimed protection); and a `cbcs` pattern
  component (`crypt_byte_block`/`skip_byte_block`) above 15, which would
  otherwise silently truncate to its low 4 bits when packed into `tenc`
  (ISO/IEC 23001-7 §12.2). Also promotes a `debug_assert` in
  `movie_fragment::protect_media_segment`'s `moof`/`saio`-offset consistency
  check to a real `Error` return (release builds no longer risk silently
  shipping a corrupt `moof`), and tightens that function's doc comment to
  state it protects a single-fragment media segment (one `moof`/`mdat`), which
  is its actual (and the normal CMAF) behaviour.

## [0.15.3] - 2026-07-12

### Added

- **`ssai_ad_stitch` example** (issue #664): a runnable, real-fixture
  end-to-end SSAI (server-side ad insertion) walkthrough wiring
  `scte35-splice` (cue parsing), `timed-metadata` (SCTE-35 -> HLS
  `EXT-X-DATERANGE` / DASH `emsg` conversion), and transmux's own
  `splice_insert` + HLS/DASH packaging together: extracts a hand-built,
  spec-correct `splice_insert` cue from a real MPEG-2 TS PID, converts its
  90 kHz PTS to the base track's own timescale, splices in a stand-in ad
  clip, and renders both an HLS media playlist (`#EXT-X-DISCONTINUITY` +
  `#EXT-X-DATERANGE`) and a DASH MPD (`InbandEventStream` + an inband `emsg`
  box) describing the spliced timeline. `cargo run -p transmux --example
  ssai_ad_stitch`; covered by `transmux/tests/ssai_ad_stitch.rs`, which
  `#[path]`-includes the example and asserts on the exact rendered manifest
  text and a full `emsg` -> SCTE-35 round-trip. Dev-dependency only
  (`scte35-splice`, `timed-metadata`) — no change to transmux's own public
  API or its dependency graph.

### Fixed

- **`splice::splice_insert` mis-scaled the ad/resume cut point on every
  non-anchor track whose media timescale differs from the anchor (video)
  track's** — which is virtually always true for real content (e.g. 90 kHz
  video vs. 44.1/48 kHz audio). The video-timescale tick offset used to be
  passed straight to `sample_index_at_offset` for the audio track without
  any unit conversion, so the audio split landed at the wrong wall-clock
  position (and could run past the end of the audio samples, panicking on
  index-out-of-bounds) on any base media with video and audio at different
  timescales. `splice_insert` now rescales the offset into each track's own
  timescale before searching for its split sample. No existing test exercised
  a multi-track (video + audio) `Media` before this fix — none of
  `transmux/tests/splice.rs`'s cases used more than one track; the new
  `ssai_ad_stitch` example/test (issue #664) exercises a real video+audio
  splice and would have caught this immediately.
- **DASH output emitted every Representation's segments as the full
  multi-track CMAF artefact instead of a genuinely single-track one** (#614).
  `OutputFormat::Dash` muxed one multi-track `CmafMux` blob from all tracks
  and cloned it under every `init-stream{id}.m4s`/`chunk-stream{id}-1.m4s`
  name, so each DASH Representation's segments carried every other track's
  samples too, violating ISO/IEC 23009-1 §5.3.9.1 — invisible to the golden
  gate test's own ffprobe-based checks because they only run against a
  dash-demuxer-capable ffprobe build (rare locally, present on CI, where the
  test was failing). Each track's segments are now muxed from a filtered
  single-track `Media` (`Media::select_tracks_by`), and the golden gate
  `ts_to_dash_mpd_validated` test gained its own single-track segment
  assertion (`assert_single_track_matches_source`) to keep catching this
  class of regression instead of loosening the existing multi-track check.

### Changed

- **Internal only, no public API change**: the RTCP control-packet codec
  (`SenderReport`/`ReceiverReport`/`SourceDescription`/`Bye`/`App`/
  `RtcpPacket`/`CompoundPacket` and friends) moved to the new standalone
  [`rtcp-packet`](https://crates.io/crates/rtcp-packet) crate (issue #654,
  part of epic #653 — the same extraction `rtp-packet` went through in
  #646/#650). `transmux::rtcp` is now a thin `pub use rtcp_packet::*;`
  re-export, so every `transmux::rtcp::*` and crate-root `transmux::*` call
  site keeps working unchanged; existing RTCP tests
  (`transmux/tests/rtcp.rs`) pass byte-for-byte unchanged. Not released as
  its own transmux version bump (same precedent as the rtp-packet
  migration).

## [0.15.2] - 2026-07-09

### Fixed

- **DVB `stream_type 0x06`/`0x15` Dolby/DTS audio never classified past
  opaque data** (#641). AC-3/E-AC-3/DTS carried the standard DVB way --
  `stream_type 0x06` (PES private data) or `0x15` (metadata in PES) plus an
  AC-3 (`0x6A`), enhanced AC-3 (`0x7A`), or DTS (`0x7B`) ES_info descriptor,
  per ETSI EN 300 468 -- fell through to opaque `CodecConfig::Data` and was
  silently dropped from HLS/fMP4 output, exactly like the native
  `0x81`/`0x87`/`0x8*` stream_types would have been recognised. The PMT
  parser now consults the ES_info descriptor loop for those two
  `stream_type`s and reclassifies to the matching audio codec, reaching the
  existing `ConfigProbe::Ac3`/`Eac3`/`Dts` syncframe recovery unchanged.

## [0.15.1] - 2026-07-07

### Fixed

- **MPEG audio / ADTS frame splitting never resynced past a bad sync**
  (#638). `split_mpeg_audio_frames` and `split_adts_frames` assumed a PES
  payload always starts exactly on a frame boundary; a real DVB-S broadcast
  multiplexer routinely splits PES payloads without regard to audio frame
  length, so a misaligned payload silently yielded zero frames (a track
  stuck in `Probing` forever if no buffered PES happened to align, or
  silently dropped samples on an already-live track). Both splitters, and
  the MP2/AAC config-probe backlog scans, now resync forward to the next
  valid frame header instead of bailing on the first byte that isn't one.

## [0.15.0] - 2026-07-06

### Added
- **Late-resolving live tracks: `DemuxEvent::TracksResolved` +
  `StreamingTsHlsSegmenter::add_track`** (#624). A live MPEG-TS feed resolves
  `DemuxEvent::TrackAdded` incrementally per PID (`ts_demux.rs`'s
  `Probing`→`Parked`/`Live` lifecycle), and an audio PID's first frame
  commonly parses after the first video keyframe — a consumer that built
  `StreamingTsHlsSegmenter` at the first video keyframe therefore got a
  permanently video-only segmenter with silently no audio, and no way to fix
  it. `StreamingTsDemux` now emits `DemuxEvent::TracksResolved` once every
  currently-known PMT-declared PID has left `Probing` (no PID stuck
  probing), re-arming (and firing again) if a later PMT version bump adds a
  new PID that itself then resolves — de-duplicated against a
  known-PID-count high-water mark so it never fires once per repeated PMT
  section or per packet on an already-stable track set.
  `StreamingTsHlsSegmenter::add_track(spec)` registers a track after
  construction: errors on a `track_id` collision; otherwise, if nothing has
  been cut or buffered yet (`total_segments == 0`, every track's `pending`
  empty) and the newly-added track is AVC while the current anchor isn't, the
  anchor (and its target-duration tick count) is recomputed to the new video
  track — recovering the construction-time "first video, else first track"
  rule for the case this issue targets — otherwise the existing anchor and
  already-cut segments are left untouched and the track simply joins future
  muxing. Segments cut before `add_track` have no PES data for the new track
  (spec-legal — ISO/IEC 13818-1 §2.4.4.8 permits a PMT to declare a track
  with zero samples in a given segment); every segment cut after `add_track`
  carries it correctly in both the PMT and the PES.
- **Streaming Annex B → access-unit splitter** (`au::AccessUnitSplitter`,
  `au::split_access_units`) (#601). An IP-camera SoC encoder emits a continuous
  Annex B byte stream (ITU-T H.264 Annex B) with no TS/PES framing; the on-camera
  LL-HLS origin path needs to cut it into access units incrementally as bytes
  arrive. The new splitter buffers pushed bytes and emits each complete access
  unit at the next AU boundary — an access-unit delimiter (AVC 9 / HEVC `AUD_NUT`
  35 / VVC `AUD_NUT` 20), a VCL NAL that is the first slice of a new picture
  (AVC `first_mb_in_slice`==0 / HEVC `first_slice_segment_in_pic_flag`==1), or a
  non-VCL NAL following a VCL (H.264 §7.4.1.2.4). Codec-aware (AVC/HEVC; VVC on
  AUD/non-VCL boundaries only), `no_std`, and byte-exact: concatenating the
  emitted units reproduces the input from its first start code. Complements the
  existing per-NAL `annexb::iter_annexb_nals` and the private, one-shot AUD-only
  splitter in `ps_demux`.
- **MPEG-H 3D Audio MPEG-2 TS carriage** (#579): `ts_demux.rs` recognises PMT
  `stream_type 0x2D` (ISO/IEC 13818-1 Table 2-34 / ETSI TS 101 154 §6.8) as
  MPEG-H Audio, parses the MHAS elementary-stream packet framing
  (`transmux::mpegh`'s new packet walker — the three-tier "escaped value"
  header coding, empirically verified against a real Fraunhofer
  MPEG-H-in-TS fixture; see `transmux/docs/codec/mpegh-ts-101154.md`) to
  locate the `PACTYP_MPEGH3DACFG` packet's opaque `mpegh3daConfig()` payload,
  and builds a `CodecConfig::MpegH` track from it — one opaque `Sample` per
  MHAS access unit, `is_sync` set from whether the access unit is a
  random-access point (ETSI TS 101 154 §6.8.4.1). `ts_mux.rs` gains
  `EsKind::MpegH` (stream_type `0x2D`, MHAS passthrough) and synthesizes the
  PMT `MPEG-H_3dAudio_descriptor` (`0x3F` extension descriptor) from the
  track's `mpegh3daProfileLevelIndication`. `referenceChannelLayout` /
  `channel_count` / `sample_rate` are not derivable from TS-layer signalling
  alone (would require decoding the opaque `mpegh3daConfig()` bitstream,
  ISO/IEC 23008-3, paid) and are left as documented `0`/"unspecified"
  placeholders; sample timing is unaffected (durations anchor on the 90 kHz
  TS clock, not an audio sample count). No MPEG-H audio bitstream decode —
  config/sample passthrough only, mirroring the existing AC-3/DTS TS
  carriage and the crate's existing ISOBMFF `mha1`/`mhaC` support. Gated on
  the private `private/fixtures/ts/mpegh-cicp01-baseline.ts` fixture
  (`transmux/tests/mpegh_ts.rs`, skips cleanly when the private submodule
  isn't checked out).
- **`nal::caption_cc_data`** — extract ATSC A/53 caption SEI (in-band
  CEA-608/708 carriage) from an H.264/HEVC access unit (#599, follow-up to
  #595's SEI machinery). Walks every NAL, finds each
  `user_data_registered_itu_t_t35` SEI message (H.264 type 6 / HEVC
  prefix/suffix types 39/40, `payloadType` 4) matching the ATSC A/53 §6.2.3
  signature (country `0xB5`, provider `0x0031`, `user_identifier` `"GA94"`,
  `user_data_type_code` `0x03`), and returns the concatenated
  `MPEG_cc_data()` bytes (`cc_data()` + trailing `marker_bits`) in AU order —
  the same wire form the PES-carried `cc_data()` path already produces, ready
  for `cc_data::CcData::parse`. Reuses `recovery_point_sei`'s SEI RBSP /
  EBSP-unescape / `payloadType`-varint walk (issue #595). Works on IR sample
  bytes (length-prefixed or Annex B); no `ts_demux.rs` change. Validated
  against a real ATSC A/53 caption SEI captured byte-for-byte from
  `samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2885/transformers_EIA608_H264.ts`
  (fetched on demand into `.test-streams/`, see
  `tools/fetch-test-streams.sh transformers-eia608-h264`) plus a full
  `TsDemux` round trip on the same capture.
- **Player-validated golden gate** (#569): `tests/golden_gate.rs` packages the
  real `fixtures/ts/h264_aac.ts` fixture through `transmux::cli::run_bytes`
  (the same code path the `transmux` binary uses) into CMAF/fMP4, progressive
  MP4, classic TS-HLS (segment + `.m3u8` playlist), and DASH MPD, then hands
  each artefact to an **independent** decoder — `ffprobe` — asserting it
  parses cleanly and reports the same track count / codec / dimensions /
  sample-rate as the source fixture's own `ffprobe` identification. Every
  other transmux test proves round-trip symmetry against the crate's own
  parsers only; this closes that self-referential gap with an external
  oracle. DASH falls back to a structural MPD check + per-segment `ffprobe`
  when the local `ffprobe` build lacks the `dash` demuxer (common — it needs
  libxml2); TS-HLS additionally resolves the whole playlist through
  `ffprobe`'s `hls` demuxer when present. `ffprobe` availability (and
  specific demuxers) is probed at runtime and each case skips cleanly with a
  printed reason when the tool is absent, so `cargo test` stays green without
  ffmpeg installed. A `mutated_cmaf_output_fails_the_gate` self-test proves
  the oracle bites (a truncated artefact must fail `ffprobe`, not pass). New
  non-blocking `golden-gate` CI job (`.github/workflows/ci.yml`) installs
  ffmpeg and runs the harness.

### Fixed
- **Reverted the #629 `#EXT-X-DISCONTINUITY-SEQUENCE` change — the original
  eviction-based bookkeeping was correct; the "fix" was not.** #629 diagnosed
  the header as reflecting the window's leading segment's discontinuity-count
  one cut too late, and changed it to stamp each segment's absolute count at
  cut time. A pre-tag audit re-derived RFC 8216 §6.2.1's literal definition —
  *"a segment's Discontinuity Sequence Number is the value of the
  EXT-X-DISCONTINUITY-SEQUENCE tag (or zero if none) plus the number of
  EXT-X-DISCONTINUITY tags in the Playlist preceding the URI line of the
  segment"* — and found the "fix" double-counts: the inline `#EXT-X-
  DISCONTINUITY` tag rendered on a discontinuous segment (while it's still in
  the window) already accounts for that segment's boundary, so advancing the
  header *at the same time* makes every segment still sharing the window with
  it compute a Discontinuity Sequence Number one too high — e.g. window
  `[s2, s3]` with `s2` discontinuous: the "fix" reported both segments' true
  client-computed DSN as `2` instead of `1`. The original eviction-based
  logic (advance the header only once a discontinuous segment rolls *off* the
  window, so its own inline tag stops rendering) is exactly correct per the
  spec's formula and was verified stable at every window state. Restored it.
  The regression test (`transmux/tests/streaming_tshls.rs`) now computes each
  segment's *client-observable* DSN (header + preceding inline tags), not the
  raw header integer in isolation — the isolation-only check is what let the
  wrong "fix" pass its own test. Batch `TsHlsPackager::package` was, and
  remains, unaffected (no rolling window; always emits
  `discontinuity_sequence: 0`).
- **TS mux silently dropped HEVC/MPEG-2-video/MPEG-1/2-audio tracks** (#627).
  `ts_mux::EsKind::from_config` only mapped `CodecConfig::Avc`/`Aac`/`Ac3`/
  `Eac3`/`Dts`/`MpegH`/`Data` to a carriable elementary stream — every other
  codec fell to `None` and `plan_elementary_streams` skipped it ("uncarriable
  codec: skip, never fatal"), so a HEVC/MPEG-2-video/MPEG-1/2-audio track was
  silently absent from TS and TS-HLS output instead of degraded. Added
  `EsKind::Hevc` (stream_type `0x24`), `EsKind::Mpeg2Video` (stream_type
  `0x02`, raw ES passthrough), and `EsKind::MpegAudio` (stream_type `0x03`/
  `0x04` selected from the recovered `esds` `objectTypeIndication`, raw frame
  passthrough) — ISO/IEC 13818-1 Table 2-34. HEVC access units get the same
  independently-decodable guarantee AVC already had: a new
  `build_hevc_annexb_au` (mirroring `build_annexb_au`, HEVC's 2-byte NAL
  header and VPS(32)/SPS(33)/PPS(34) types) prepends the track's VPS/SPS/PPS
  (new `EsPlan::hevc_vps_sps_pps`, recovered from `hvcC` in AU order) to any
  keyframe access unit that does not already carry its own SPS. VVC/AV1/VP9
  are not modeled as `CodecConfig` variants in a way this container layer
  carries into TS today and remain out of scope for this fix (`EsKind` has no
  mapping for them, same as before).
- **Anchor-track selection only recognised AVC as video** (#628).
  `ts_hls::choose_anchor` and `Segmenter::new` both picked the segmentation
  anchor by matching `CodecConfig::Avc` only, so any other video codec (HEVC,
  MPEG-2 video, VVC, AV1, VP9, VP8) was never chosen as the anchor — falling
  back to "first track", which is wrong whenever video isn't track 0. Added
  `CodecConfig::is_video` (mirrors the existing `is_audio`, covering every
  video variant the enum defines) and switched both call sites to it.
  `StreamingTsHlsSegmenter::add_track` (#624) had a *third*, undisclosed
  AVC-only anchor-promotion check that #628 missed — a late-resolving HEVC
  (or any non-AVC video) track added via `add_track` never got promoted to
  anchor, silently reintroducing the audio-anchors-and-video-never-cuts bug
  for exactly the codec #627 exists to carry. Found by a pre-tag audit;
  switched to `is_video()` there too.

## [0.14.0] - 2026-07-04

### Fixed
- **Open-GOP AVC access units now anchor segmentation, not just IDR** (#595).
  Broadcast H.264 is frequently open-GOP — no IDR (NAL type 5) at all; each
  GOP opens instead with SPS(7)/PPS(8) + a non-IDR I-slice, usually announced
  by a `recovery_point` SEI (ITU-T H.264 Annex D.1.7/D.2.7). `is_keyframe_nal`
  only matched IDR, so `TsDemux`/`StreamingTsDemux` set `Sample.is_sync` to
  `false` on every access unit of such a stream, and `Segmenter` never found
  an anchor — it buffered the entire input into one giant segment.
  `transmux::nal` gains `recovery_point_sei` (parses a type-6 SEI NAL's
  `sei_message()` `payloadType`s) and `access_unit_is_rap`, which recognises
  an AVC access unit as a random-access point on IDR **or** a
  `recovery_point` SEI **or** (pragmatic open-GOP fallback) an SPS in the
  access unit; HEVC/VVC keyframe detection is unchanged (their IRAP range
  already covers CRA/BLA). `video_sample_bytes` in `ts_demux.rs` now sets
  `is_sync` via this access-unit-level check for AVC, so both `TsDemux` and
  `StreamingTsDemux` benefit. Segments opened this way are non-IDR — correct
  for open-GOP decode and DASH-IF/CMAF-acceptable, but not a strict
  ISO/IEC 14496-12 "clean" sync sample. `is_keyframe_nal`/
  `access_unit_is_keyframe` keep their strict IDR-only meaning for existing
  callers.

## [0.13.0] - 2026-07-04

### Added
- **DASH MPD generation — `$Time$`/`SegmentTimeline` addressing + live/content
  extensions** (#566), extending the existing `DashPackager`:
  - `Addressing` (`Number` default / `Timeline`) + `DashPackager::segments`
    (`Vec<TrackSegments>`, caller-supplied per-track segment durations, e.g.
    from `Segmenter`): `Addressing::Timeline` emits `$Time$` addressing with
    an explicit `<SegmentTimeline>` of run-length-encoded `<S t= d= r=>`
    entries (ISO/IEC 23009-1 §5.3.9.6); `Addressing::Number` now also accepts
    `segments` to use a real per-segment nominal `@duration` instead of the
    whole-track total, while staying unchanged when `segments` is empty.
  - Live-profile MPD attributes: `DashPackager::publish_time`,
    `time_shift_buffer_depth`, and `suggested_presentation_delay` (alongside
    the existing `availability_start_time`/`minimum_update_period`),
    ISO/IEC 23009-1 §5.3.1.2 Table 3.
  - Every `AdaptationSet` now carries a `Role` (`urn:mpeg:dash:role:2011`,
    `main`, §5.8.5.5) and, when every `Representation` agrees, an inherited
    `@lang` resolved from a TS-sourced audio track's
    `ISO_639_language_descriptor` (ETSI EN 300 468 §6.2.19) in
    `TrackSpec::es_info_descriptors`.
  - `DashPackager::content_protection` (`Vec<ContentProtectionSystem>`) — a
    `<ContentProtection>` hook (§5.8.4.1), optionally carrying
    `cenc:default_KID` (ISO/IEC 23001-7); full CENC `pssh` carriage remains a
    separate epic.
  - `DashPackager::inband_event_streams` (`Vec<InbandEventStream>`) —
    `<InbandEventStream>` (§5.3.3 / §5.10.3.3) on the video `AdaptationSet`
    for an inband `emsg` scheme/value.

## [0.12.0] - 2026-07-04

### Breaking
- `TrackSpec` gains two fields, `source_pid: Option<u16>` and
  `es_info_descriptors: Vec<u8>` (#582) — external struct literals must be
  migrated to the new `TrackSpec::new(track_id, timescale, config)`
  constructor (+ `.with_source(pid, descriptors)` to attach TS provenance).
- `CodecConfig`, `Sample`, and `TrackSpec` are now `#[non_exhaustive]` (#580,
  the crate-wide convention already applied to most other public config/error
  enums). External code can no longer build these with a struct/variant
  literal or exhaustively `match`/destructure without a wildcard arm:
  - `Sample` — construct via `Sample::new(data, duration, is_sync,
    composition_offset)` (the new general-purpose constructor),
    `Sample::from_annexb`, or `Sample::from_raw`, then `.with_source_timing(t)`
    if needed.
  - `TrackSpec` — construct via `TrackSpec::new(...)` /
    `.with_source(...)` (see above).
  - `CodecConfig` — exhaustive external `match`/`if let` sites need a
    trailing `_ =>` (or `other =>`) arm.

### Fixed
- **`avcC` high-profile extension gate + population** (#563, #582): the
  `AVCDecoderConfigurationRecord` extension gate
  (chroma_format/bit_depth_luma_minus8/bit_depth_chroma_minus8) checked
  `profile_idc ∈ {100,110,122,144}`; `144` was a pre-Amendment-3 placeholder,
  finalized by ITU-T H.264 as **profile_idc 244** ("High 4:4:4 Predictive").
  Fixed to the shared `sps::is_high_profile` set. Additionally, `TsDemux` had
  hardcoded those three fields to `None` when recovering `avcC` from a TS AVC
  stream, so High 10/4:2:2/4:4:4 lost chroma/bit-depth end-to-end — now
  populated from the SPS. (Verified against `fixtures/ts/h264/high*.ts`.)

### Added
- **Streaming/incremental classic-HLS segmentation for live input** (#571):
  `TsHlsPackager::package` needs the whole `Media` up front, which does not
  fit an unbounded live feed. `StreamingTsHlsSegmenter` is the incremental
  analogue, mirroring `Segmenter`'s CMAF push/flush model: `push(track_id,
  sample)` buffers one coded sample at a time and returns a finished `.ts`
  `TsSegment` whenever the anchor track crosses a keyframe past the target
  duration (byte-identical to the corresponding `TsHlsPackager::package`
  segment for the same input), `finish()` flushes the trailing partial
  segment, and `playlist()` renders a rolling media playlist over a
  configurable sliding window — advancing `#EXT-X-MEDIA-SEQUENCE` and
  `#EXT-X-DISCONTINUITY-SEQUENCE` as older segments roll off, and omitting
  `#EXT-X-ENDLIST` until `finish` has been called. `mark_discontinuity()`
  marks the next cut as an `#EXT-X-DISCONTINUITY` (e.g. on an upstream
  PID/PCR reset). The batch packager and the streaming segmenter now share
  one anchor-selection/cut-decision/duration implementation so the two paths
  cannot silently drift apart.
- **Origin PID + PMT ES_info descriptors on every TS-demuxed track** (#582):
  a DVB player track-picker can now select/label tracks by PID and by
  ES_info descriptor (ISO-639 language `0x0A`, DVB subtitling `0x59`, E-AC-3
  `0x7A`, …) without running its own parallel PAT/PMT parser.
  - `TrackSpec::source_pid` — the source elementary-stream PID, populated for
    every `StreamingTsDemux`/`TsDemux`-produced track (codec **and** opaque
    `CodecConfig::Data`, not just `Data` as before); `None` for non-TS
    sources (fMP4/FLV/WebM/PS/RTP).
  - `TrackSpec::es_info_descriptors` — the verbatim PMT ES_info
    descriptor-loop bytes (ISO/IEC 13818-1 §2.4.4.8) for that elementary
    stream, for every TS-demuxed track; empty for non-TS sources. transmux
    does not parse these — consumers use `dvb-si`.
  - `TrackSpec::new(track_id, timescale, config)` — the new constructor
    every non-TS demuxer/transform now builds a spec with; `.with_source(pid,
    descriptors)` attaches TS provenance (builder style).
- **Progressive (non-fragmented) MP4 demux** (#561): `ProgressiveDemux`
  (`Unpackage<Input = &[u8]>`) parses a single-file, non-fragmented `.mp4` —
  `moov` sample tables, no `moof` — into the crate's `Media` IR, the
  file-on-disk counterpart to the fragmented `Fmp4Demux` and the demux side
  of the existing `ProgressiveMux`. Reuses `Fmp4Demux`'s `stsd` →
  `CodecConfig` reconstruction verbatim; per-sample decode duration and
  composition offset come from `stts`/`ctts` (ISO/IEC 14496-12:2015
  §8.6.1.2/§8.6.1.3, v0 unsigned / v1 signed), sync flags from `stss`
  (§8.6.2, absent ⇒ all sync), and each sample's coded bytes are sliced
  directly out of the input via `stsc` + `stco`/`co64` chunk offsets
  (§8.7.4/§8.7.5, already file-absolute) and `stsz` sizes (§8.7.3). Verified
  against `fixtures/transmux/h264_aac_prog.mp4` (real interleaved,
  multi-chunk H.264 High + AAC-LC capture) with sample counts/PTS/DTS/sync
  flags cross-checked against `ffprobe -show_packets -ignore_editlist 1`,
  and a demux → `CmafMux` → `Fmp4Demux` round-trip proving byte-identical
  sample data and preserved codec config.

### Internal
- `tests/label_coverage.rs` drift-guard (#580): fails CI if any new public
  spec/field enum in `transmux/src/` lacks a `name()` + `Display` impl (the
  issue #204 convention), mirroring the guard already run in `dvb-si` and
  other crates. `ColourType` (`src/visual_ext.rs`) gained `name()` +
  `Display` as part of closing the existing gap the guard found.
- **H.264/HEVC profile-matrix hardening test** (#563):
  `tests/sps_profile_matrix.rs` — table-driven, ffprobe-oracle-backed
  coverage of `decode_avc_sps`/`decode_hevc_sps` + the `TsDemux` → `CmafMux`
  TS→IR→fMP4 path across Baseline/Main/High/High10/High422/High444/
  High-1080-cropped/interlaced + HEVC Main/Main10 (profile/level/dims/
  chroma/bit-depth/interlace/fps vs real fixtures).

## [0.11.0] - 2026-07-04

### Breaking
- **New public API surface is additive but two changes are source-breaking**
  (0.x minor bump per Cargo SemVer): `CodecConfig::Data` gained a `carriage:
  DataCarriage` field (external construct/match sites must add it), and `Sample`
  gained a `source_timing: Option<SourceTiming>` field (external struct literals
  must set it — use the `Sample::from_*` builders + `with_source_timing`). The
  new `DataCarriage` enum is `#[non_exhaustive]`.

### Added
- **Lossless carriage of ANY MPEG-2 TS elementary stream** (#576): TS→IR→TS
  is no longer limited to a hardcoded stream_type allowlist.
  - `CodecConfig::Data` gains a `carriage: DataCarriage` field
    (`Pes`/`Sections`, with `name()`/`Display`) recording whether the
    elementary stream carries PES packets or PSI/private sections
    (ISO/IEC 13818-1 §2.4.4.8 / Table 2-34).
  - **TS → IR demux**: `Codec::from_stream_type` now returns an opaque
    `CodecConfig::Data` for **every** `stream_type` it does not decode to a
    typed codec (never `None`/dropped). A fixed section-carried set (`0x05`
    private_sections, `0x0A`-`0x0D` DSM-CC, `0x14` DSM-CC synchronized
    download, `0x86` SCTE-35/ANSI-scoped) is reassembled via a
    `mpeg_ts::ts::SectionReassembler` instead of a PES assembler — each
    complete section becomes one `Sample` with no PTS/DTS
    (`source_timing: None`); every other stream_type (including the
    pre-existing `0x06`/`0x15` carriage) is PES-reassembled as before.
  - **IR → TS mux**: `EsKind::Data { stream_type, carriage }` re-emits the
    preserved `stream_type` verbatim; a PES-carried Data track is wrapped in
    a `private_stream_1` (`0xBD`) PES packet, payload pass-through; a
    section-carried Data track's samples are re-emitted directly via
    `mpeg_ts::mux::SectionPacketizer`, never PES-wrapped. `build_pmt_section`
    now writes each ES's preserved `ES_info` descriptor bytes into the PMT
    (previously always empty) so a receiver can identify a carried stream
    (e.g. its DVB subtitling/teletext descriptor); `program_info` stays
    empty. The classic TS-HLS packager (`TsHlsPackager`, built on the same
    `ts_mux` machinery) carries every such stream for free, re-emitting a
    complete PMT (every ES + its descriptors) at the start of every segment.
  - Fixed a latent packet-interleaving bug in `TsMux`/`TsHlsPackager`
    (`ts_mux::mux_tracks_at`) exposed by this change: the global
    packet-interleave sort previously keyed on the on-wire, 33-bit-wrapped
    DTS, which could reorder a single track's own packets against each other
    once its cumulative decode time crossed the wrap point (only reachable
    once an opaque `CodecConfig::Data` track — whose recovered per-sample
    durations are untrusted input — could reach the TS mux path at all). The
    interleave key is now a separate, never-wrapped monotonic value.
  - **fMP4/CMAF mux**: `CodecConfig::Data` tracks have no ISOBMFF sample
    entry, so `build_init_segment` (and therefore `CmafMux`, `ProgressiveMux`,
    `Segmenter`, `LlSegmenter`, `LlHlsSegmenter`) now **skips** them
    gracefully instead of failing the whole mux with `UnsupportedCodec` — a
    TS multiplex mixing carriable and opaque streams now produces a valid
    fMP4/CMAF output for its carriable (video/audio) tracks.
- **DTS Transport-Stream spoke** (#560): DTS is no longer dropped on the TS
  side.
  - **TS → IR demux**: PMT `stream_type` `0x82`/`0x85`/`0x8A` now resolves a
    `CodecConfig::Dts` track instead of being silently skipped. The new
    `dts::DtsCoreFrameInfo::from_es` parses a DTS **core substream** frame
    header (ETSI TS 102 114 §5.3/§5.4, sync `0x7FFE8001`) — sample rate
    (Table 5-5), channel count (Table 5-4 + LFE), samples/frame
    (`32×(NBLKS+1)`) — and `into_ddts()` builds a core-only `ddts`
    `DtsSpecificBox` from it (§E.2.2.3.2, Tables E-2/E-3), mirroring the
    existing AC-3/E-AC-3 recovery path. Each PES access unit is split into
    individual core frames (`dts::split_dts_core_frames`, using each frame's
    own `FSIZE`) and emitted as one `Sample` with interpolated per-frame
    `SourceTiming`, the same pattern as E-AC-3 syncframe splitting (#556).
  - **IR → TS mux**: new `EsKind::Dts` (`stream_type` `0x82`, ETSI TS 101 154
    §G) — a `CodecConfig::Dts` track is now emitted to TS (PES payload
    passthrough) instead of being dropped.
- **Event-driven streaming TS demuxer** (#555): `StreamingTsDemux` is a new
  incremental MPEG-2 TS demux core — `feed()` accepts bytes of any size or
  alignment (down to one byte at a time; resynchronises via
  `mpeg_ts::resync::TsResync`), `poll_event()` drains `DemuxEvent`s
  (`TrackAdded`/`TrackUpdated`/`Sample`/`Pcr`/`Discontinuity`) as they become
  known, and `finish()` flushes trailing partial access units. `TsDemux` is
  now a thin batch wrapper over it (feed the whole buffer, `finish()`, fold
  the event stream into a `Media`) — there is no separate whole-buffer demux
  implementation; every existing `TsDemux` behaviour (per-sample
  `SourceTiming`, AC-3/E-AC-3 syncframe splitting, opaque `Data` tracks, PCR
  collection, 33-bit wrap-unroll) is produced by the streaming core.
  Codec-config recovery is single-shot and incremental (mirrors the old
  whole-file `find_map` scans, applied access-unit-by-access-unit); track IDs
  and `TrackAdded` order still follow PMT declaration order (codec tracks
  first, then data tracks, each group in PMT order), independent of which
  PID's config happens to resolve first. Memory is bounded independent of
  stream length (per-PID PES/PSI reassembly state, one pending sample per
  live video/data track, small per-PID config-recovery backlogs, and a
  FIFO-capped pre-PMT `unattributed`-payload buffer for PIDs whose own packets
  arrive before their PMT registration completes — so a full-multiplex live
  feed with unrelated-service PIDs that never appear in the followed PMT stays
  bounded regardless of stream length; see the `StreamingTsDemux` doc comment
  for details.
- **Per-sample source-container timing** (#556): `Sample` gains
  `source_timing: Option<SourceTiming>` (`SourceTiming { pts, dts }`, the
  33-bit-unwrapped 90 kHz PES clock for TS sources) and a
  `with_source_timing` builder; every in-crate `Sample` constructor/literal
  updated. All mux paths (`build_media_segment`/`CmafMux`) ignore the field —
  fMP4 output timing stays duration-based.
  - `TsDemux` now sets `source_timing` on every video (H.264/HEVC/MPEG-2) and
    audio (AC-3/E-AC-3/AAC/MPEG audio) sample it emits: the first frame taken
    from a PES access unit carries that PES's unwrapped PTS/DTS exactly;
    subsequent frames split out of the same PES payload get interpolated
    timing (`pes_pts + i * samples_per_frame * 90000 / sample_rate`, floored,
    `u128` math).
  - **AC-3 syncframe splitting**: `ts_demux` now splits each PES payload into
    individual AC-3 syncframes (`ac3::split_ac3_syncframes`, using the
    Table 4.13 frame-size table — ETSI TS 102 366 §4.4.1.4 — via the new
    `Ac3SyncframeInfo::frame_len_bytes()`) instead of emitting one
    zero-duration `Sample` per PES access unit. Every syncframe gets
    `duration = AC3_SAMPLES_PER_SYNCFRAME` (1536 = 6 blocks × 256
    samples/block, §4.3.0 `syncframe()`).
  - **E-AC-3 syncframe splitting**: `ac3::split_eac3_syncframes` splits each
    PES payload into access units; a dependent-substream syncframe
    (`strmtyp == 0x1`, Annex E §E.1.2.2 `bsi()`) is concatenated into the
    preceding independent syncframe's access unit. `duration = numblks * 256`
    from the independent frame.
  - A previously TS-sourced AC-3/E-AC-3 track muxed through `CmafMux` no
    longer produces all-zero `trun` sample durations.
- **Opaque PES data tracks** (#557): PMT `stream_type` 0x06 (PES private
  data — DVB subtitles/teletext/SMPTE 2038/etc.) and 0x15 (metadata in PES)
  are now carried into the IR as a new `CodecConfig::Data { stream_type,
  descriptors }` variant (`descriptors` is the raw PMT ES_info
  descriptor-loop bytes) instead of being silently dropped. Mirrors the
  existing `CodecConfig::Vp8`/`Vorbis` WebM-only precedent: carried in the IR
  for inspection / `{TS} → IR → {TS}`, but has no ISOBMFF sample entry, so
  `build_trak`/`CmafMux` reject it with the same `Error::UnsupportedCodec`
  every other site that dispatches exhaustively on `CodecConfig` (`dash.rs`
  RFC 6381 codec string, `flv.rs` codec name, `splice.rs` codec kind) gained
  a matching `Data` arm.
  - `TsDemux` builds one Data track per opaque PES elementary stream: one
    `Sample` per PES access unit (verbatim payload bytes), `timescale =
    90_000`, `is_sync = true`, `composition_offset = 0`, `source_timing` from
    the unwrapped PES PTS/DTS, `duration` = the delta to the next access
    unit's unwrapped PTS (last sample reuses the previous duration). Data
    tracks are ordered after every codec track, in PMT order.
- **PCR timeline** (#557): `Media` gains `pub pcr: Vec<PcrSample>`
  (`PcrSample { pcr_27mhz, pid, packet_index, discontinuity }`, ISO/IEC
  13818-1 §2.4.3.4/§2.4.3.5) and a `with_pcr` builder; empty for every
  demuxer except `TsDemux`, which now collects every PCR observation from
  every TS packet's adaptation field (via `mpeg_ts::ts::TsPacket::
  adaptation_field()`), in packet order.
### Fixed
- `TsDemux`'s 33-bit PES-clock wrap-unroll (`unwrap_ts`/`decode_order`/the
  new `unwrap_all`) could misread a stream's first *genuine* PTS/DTS as a
  spurious backward wrap when an earlier access unit on the same elementary
  stream carried no PES header timing at all (its PTS/DTS default to a
  placeholder `0`) — most commonly hit by opaque PES data streams (#557),
  whose access units are sometimes sparse "heartbeat" PES packets with no
  timing. `push_access_unit` now falls back to the *previous* access unit's
  resolved timestamps (rather than a hardcoded `0`) when a PES carries
  neither PTS nor DTS, and the wrap-unroll itself defers wrap-jump detection
  until each of the PTS/DTS channels has seen its first genuine (non-`0`)
  value.

## [0.10.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.
### Added
- **HEVC SPS VUI timing fields** (#546): `HevcSpsInfo` gains three new fields —
  `num_units_in_tick: Option<u32>`, `time_scale: Option<u32>`, and
  `fps: Option<f32>` — mirroring the AVC equivalents added in #523.
  `decode_hevc_sps` now walks the full HEVC SPS syntax (ITU-T H.265 §7.3.2.2.1)
  past the mandatory fields to parse `vui_parameters()` (§E.2.1) and extract
  `vui_num_units_in_tick`/`vui_time_scale`.  Frame rate is derived as
  `vui_time_scale / vui_num_units_in_tick` (no factor-of-2 — HEVC, unlike H.264,
  expresses the tick rate directly).  All three fields are `None` when
  `vui_parameters_present_flag` or `vui_timing_info_present_flag` is 0, or when
  the SPS is truncated before the VUI.  The `Eq` derive is dropped from
  `HevcSpsInfo` (now `PartialEq` only) to accommodate the `f32` field, matching
  the pattern established for `AvcSpsInfo`.
- **HLS Sample-AES + full-segment AES-128 encryption** (#479): new `sample_aes`
  module (feature `sample-aes`) implementing Apple's HLS-native content
  protection — distinct from CENC — per Apple's "MPEG-2 Stream Encryption Format
  for HTTP Live Streaming" (`transmux/docs/drm/hls-sample-aes.md`) and RFC 8216
  §4.3.2.4. All crypto stays feature-gated; the default `no_std` core build pulls
  none.
  - **AES-128 full segment** (`METHOD=AES-128`): `aes128_encrypt_segment` /
    `aes128_decrypt_segment` — AES-128-CBC over the whole segment with PKCS#7
    padding.
  - **H.264 SAMPLE-AES**: `h264_encrypt_nal` / `h264_decrypt_nal` — encrypts only
    NAL types 1 and 5 longer than 48 bytes, with a 32-byte clear leader and the
    16-byte-block / ≤144-byte-skip (~10%) pattern; emulation-prevention bytes are
    stripped before encryption and re-inserted after; IV reset per NAL.
  - **AAC / AC-3 / E-AC-3 SAMPLE-AES**: `aac_encrypt_frame`/`aac_decrypt_frame`
    (ADTS header + 16-byte leader clear) and `ac3_encrypt_frame`/
    `ac3_decrypt_frame` (16-byte leader clear), 16-byte CBC blocks, `<16` trailer
    clear.
  - **`EXT-X-KEY` rendering**: `ExtXKey` (`METHOD`/`URI`/`IV`/`KEYFORMAT`/
    `KEYFORMATVERSIONS`) with `HlsEncryptionMethod` (`AES-128` / `SAMPLE-AES`),
    `ExtXKey::fairplay_sample_aes` (`skd://` + `com.apple.streamingkeydelivery`),
    `ExtXKey::aes128`, and `iv_from_sequence_number` (implicit IV = media
    sequence number as a 128-bit big-endian integer).
  - AES-128-CBC pinned by a NIST SP 800-38A F.2 known-answer test; the block
    cipher is the RustCrypto `aes` crate driven by the new `cbc` mode crate (the
    only added dependency).
- **Multi-DRM `pssh` init-data generation** (#480): new `drm` module building
  the system-specific `Data` payloads and convenience `pssh`-box builders on top
  of the existing `ProtectionSystemSpecificHeaderBox` (ISO/IEC 23001-7 §12.1).
  - DRM system-ID UUID consts: `WIDEVINE_SYSTEM_ID`, `PLAYREADY_SYSTEM_ID`,
    `FAIRPLAY_SYSTEM_ID`, `COMMON_SYSTEM_ID`.
  - **PlayReady**: `playready_wrmheader` (WRMHEADER v4.2.0.0 XML, UTF-16LE at
    emit), `playready_pro` (the PlayReady Object: `u32` LE length, `u16` LE
    record count, type-`0x0001` header record), and `playready_pssh`. The
    critical CENC-UUID ↔ PlayReady LE-GUID byte-swap is exposed as
    `cenc_kid_to_playready` / `playready_kid_to_cenc`.
  - **Widevine**: `widevine_pssh_data` (hand-encoded `WidevineCencHeader`
    protobuf — repeated `key_id`, `provider`, `protection_scheme`; minimal
    varint + length-delimited encoding, no protobuf crate) and `widevine_pssh`.
  - **FairPlay**: `fairplay_pssh_data` / `fairplay_pssh` (the `skd://` URI as
    UTF-8 `Data` — packager convention, not a formal spec).
  - `ProtectionSystemSpecificHeaderBox` gains `parse_box` (full-box parse) and
    `to_vec` (whole-box serialize, lengths rebuilt from fields).
  - No new dependency: base64/UTF-16LE/protobuf helpers are hand-rolled.
- **KLV timed metadata + KLV-over-RTP** (#478): a new `klv` module implements
  SMPTE ST 336 KLV framing (via MISB ST 0601 + RFC 6597) and the MISB ST 0601
  UAS Datalink Local Set.
  - **BER length** codec (`encode_ber_length` / `ber_length`): short form
    (`< 128`) and long form (`0x80 | N` + `N` big-endian bytes), round-trippable;
    indefinite form rejected.
  - **BER-OID tag** codec (`encode_ber_oid` / `ber_oid`) for Local Set item keys.
  - **`KlvItem`** — a 16-byte `UniversalLabel` key + value, with `Parse`/
    `Serialize` (the wire length is *computed*, never echoed).
  - **`UasLocalSet`** / **`LocalSetItem`** — the MISB ST 0601 packet (`UAS_LS_KEY`
    Universal Label wrapping BER-OID-tagged items): `precision_timestamp()`
    (tag 2, u64 BE µs since the POSIX epoch), `serialize_with_checksum()` +
    `verify_checksum()` (tag 1 = CRC-16/CCITT, poly `0x1021`, init `0xFFFF`, over
    the whole packet incl. the UL key), and `crc16_ccitt`.
  - **KLV-over-RTP** (`rtp::packetize_klv` / `rtp::depacketize_klv`, RFC 6597,
    `smpte336m`): a KLV unit placed directly after the fixed header (no payload
    header), fragmented across sequential packets sharing one timestamp with the
    marker bit on the final fragment; new `DEFAULT_KLV_PT` / `KLV_ENCODING_NAME`.

## [0.9.0] - 2026-07-03
### Added
- Release-audit fixes: `rtcp` length uses `saturating_sub` (latent underflow);
  `avc1` bare-parse error `need` matches its guard; `cli::Container`/`OutputFormat`
  gain `#[non_exhaustive]`; new `RtmpError::UnknownControlMsgType` (was a misleading
  `BadControlLength{need:0}`); added an `RtmpMux` full-chain IR round-trip test.
- **Trick-play manifest signalling — HLS + DASH** (#477): playlist and MPD
  APIs for signalling an I-frame-only (trick-play / scrubbing) rendition
  derived by `derive_iframe_track`.
  - **HLS `#EXT-X-I-FRAME-STREAM-INF`** (RFC 8216 §4.3.4.2): new
    `IFrameVariant` struct + `MasterPlaylist::iframe_variants: Vec<IFrameVariant>`;
    `to_m3u8` renders each as a single `#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=…,URI="…"`
    tag line (URI is an attribute, not a following line — unlike `EXT-X-STREAM-INF`).
    Zero iframe variants → no tag emitted (strict opt-in).
  - **HLS `#EXT-X-I-FRAMES-ONLY`** (RFC 8216 §4.3.3.6): new
    `MediaPlaylist::iframes_only: bool`; when `true`, emits the tag in the
    header block and bumps the rendered version to ≥ 4 as required by the spec.
    Defaults `false` — existing playlists are byte-for-byte unchanged.
  - **DASH trick-mode `AdaptationSet`** (ISO/IEC 23009-1 §5.8.5.8): new
    `TrickModeAdaptationSet` + `TrickModeRepr` structs, `DashPackager::trick_mode:
    Option<TrickModeAdaptationSet>`. When set, `package()` emits an additional
    `AdaptationSet` with `<SupplementalProperty
    schemeIdUri="urn:mpeg:dash:trickmode:2016" value="<main-id>"/>` and
    `maxPlayoutRate`/`codingDependency="false"`. The scheme URI is the named
    constant `TRICKMODE_SCHEME`. Defaults `None` — existing MPDs unchanged.
  - All changes are additive; no existing public API is modified.
- **HEVC (H.265) elementary streams TS → IR** (#467): `TsDemux` now carries
  `stream_type 0x24` HEVC video into the neutral `Media` IR. The in-band
  VPS/SPS/PPS NAL units are gathered from the Annex-B access units, the SPS is
  decoded (`decode_hevc_sps`) for coded geometry + profile/tier/level/chroma/
  bit-depth, and an `hvcC` `HEVCDecoderConfigurationRecord` is assembled into a
  `hvc1` `CodecConfig::Hevc` track — identical to the config `Fmp4Demux`
  recovers from an fMP4 `hvcC`, so `{HEVC-in-TS} → IR → {any}` composes.
  Per-sample `is_sync` marks IRAP access units (HEVC NAL types 16..=23). Both
  8-bit (Main) and 10-bit (Main 10) streams are supported. DTS-from-TS remains
  unimplemented (no `CodecConfig` DTS-from-ES variant). Additive change.
- **`AvcSpsInfo` VUI timing fields** (#523): `decode_avc_sps` now parses the H.264
  VUI `timing_info` block (ITU-T H.264 §E.1.1) and exposes three new optional
  fields on `AvcSpsInfo` — `num_units_in_tick: Option<u32>`, `time_scale:
  Option<u32>`, and `fps: Option<f32>` (= `time_scale / (2 × num_units_in_tick)`).
  All three are `None` when `vui_parameters_present_flag` or
  `timing_info_present_flag` is 0.  The VUI is walked in syntax order
  (aspect_ratio_info → overscan_info → video_signal_type → chroma_loc_info →
  timing_info) with no new dependencies.  Additive change; existing callers are
  unaffected.
- **`transmux` command-line packager + `cli` feature** (#482): a new opt-in
  `cli` feature (`clap` + `std`) builds a `transmux` binary that wires the
  existing demux and mux spokes into an any-to-any packager — `transmux <in>
  -o <out> -f <format>`. The input container is autodetected from its leading
  bytes (MPEG-TS `0x47`+188, MP4/CMAF `ftyp`/`styp`/`moov`/`moof`, MPEG-PS
  `00 00 01 BA`, WebM/EBML `1A 45 DF A3`, FLV `"FLV"`), demuxed to the neutral
  [`Media`] IR, then packaged into `cmaf` / `hls` / `ts-hls` / `dash` / `ts` /
  `progressive` (selected by `-f/--format` or inferred from the output
  extension). Flags: positional `<IN>` or `-i/--input`, `-o/--output`,
  `-f/--format`, `--segment-duration`, `--ll` (LL-DASH), `--tracks`, and (under
  the `cenc` feature) `--decrypt`/`--key`. Follows `docs/CLI-STANDARD.md` (clap
  derive, named flags, auto `--help`/`--version`). The library itself stays
  `no_std` and gains no dependencies; only the `cli` feature/binary pulls
  `clap`+`std`. New public module [`cli`] with a testable
  [`cli::run_bytes`] core and [`cli::detect_container`].
- **Low-Latency HLS — partial segments + preload hints** (#454, RFC 8216bis):
  a new [`ll_hls`] module with [`LlHlsSegmenter`], a segmenter that emits each
  segment's **partial segments** ("parts", RFC 8216bis §4.4.4.9) — independent
  CMAF `moof`+`mdat` fragments covering a configurable `part_target` sub-duration
  — before the parent segment closes. [`LlHlsSegmenter::with_part_target`]
  configures the part target (ms) alongside the segment target;
  [`LlHlsSegmenter::take_ready_parts`] drains ready [`PartInfo`]s (bytes,
  duration, `independent`, `segment_seq`, `part_index`) distinct from the full
  segments drained by [`LlHlsSegmenter::take_ready_segments`]. A part is flagged
  independent when it begins on a sync sample; a segment's parts concatenate to
  exactly the whole-segment [`build_media_segment`] media. The playlist model
  gains an opt-in [`hls::MediaPlaylist::low_latency`] config
  ([`hls::LowLatencyConfig`]) that renders the LL-HLS directives —
  `#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=<sec>`
  (§4.4.3.8, PART-HOLD-BACK held to ≥ 3× part-target),
  `#EXT-X-PART-INF:PART-TARGET=<sec>` (§4.4.3.7),
  `#EXT-X-PART:DURATION=<sec>,URI="…"[,INDEPENDENT=YES]` (§4.4.4.9,
  per [`hls::PartSpec`]), and `#EXT-X-PRELOAD-HINT:TYPE=PART,URI="…"` (§4.4.5.3).
  A plain playlist (no `low_latency`) renders none of these — LL-HLS is strictly
  opt-in.
- **IR timeline conditioning — PTS/DTS rebase & anchor** (#476): new `rebase`
  module of transforms over the `Media` IR, plus the absolute decode-time anchor
  they operate on. `rebase_to_zero` re-origins each track to decode time 0 (per
  track); `apply_offset(delta_ticks)` shifts every track's anchor by a signed
  delta (saturating at 0 on underflow); `unroll_33bit_wraps` undoes MPEG-2
  Systems 33-bit timestamp wraps (ISO/IEC 13818-1 §2.4.3.6; `MPEG_TS_WRAP` =
  `2^33`) so a timeline crossing the boundary is monotonic; and
  `insert_discontinuity_gap(track, at, gap_ticks)` extends the timeline by a gap
  for splice/gap conditioning. `Fmp4Demux` now populates the anchor from the
  first movie fragment's `tfdt` `baseMediaDecodeTime` (ISO/IEC 14496-12:2015
  §8.8.12) and `TsDemux` from the first sample's DTS (rescaled into each track's
  media timescale); FLV/WebM/MPEG-PS/RTMP/RTP carry no absolute anchor and leave
  it 0. Pairs with #475 (splice/concat) as the next consumer. All four transforms
  and the muxer wiring are re-exported from the crate root.
- **IR timeline splice / concatenation → SSAI** (#475): new `splice` module
  joining two `Media` timelines into one monotonic decode timeline for
  server-side ad insertion. `concat(a, b)` appends `b` after `a` on a shared
  timeline — matching tracks pairwise (by `track_id`, else by index; errors on
  incompatible track sets / codecs / timescales), rebasing each `b` track so its
  first sample's decode time meets `a`'s end decode time
  (`start_decode_time + Σ durations`), contiguous with no gap or overlap, sample
  bytes preserved verbatim. `splice_insert(base, ad, at_ticks)` plays `base` up
  to the splice, inserts `ad`, then resumes the base shifted forward by `ad`'s
  duration. A splice boundary must fall on a random-access point: the inserted
  content's first sample must be a sync sample, and `splice_insert` snaps
  `at_ticks` to the nearest **preceding** sync sample of the base video track via
  the testable `snap_to_preceding_sync` helper. Both return a `SpliceResult`
  (`media` + `discontinuity_points: Vec<SplicePoint>` — track id, sample index,
  and presentation time of each join) so a downstream HLS packager / `Segmenter`
  can emit `#EXT-X-DISCONTINUITY` (RFC 8216 §4.3.4.3) on exactly the join
  segments. Timeline model cites the ISO/IEC 14496-12 §8.8.12 `tfdt`
  `baseMediaDecodeTime`. SCTE-35-driven point *selection* (deciding where to
  splice from cue messages) is a follow-up. `concat`, `splice_insert`,
  `snap_to_preceding_sync`, `SplicePoint`, and `SpliceResult` are re-exported
  from the crate root.
- **`emsg` emission in media segments** (#455): [`build_media_segment_with_events`]
  emits one or more MPEG-DASH Event Message Boxes (`emsg`, ISO/IEC 14496-12 §8.8 /
  ISO/IEC 23009-1 §5.10.3.3) at the start of each media segment, after `styp` and
  before `moof` (DASH-IF IOP Part 10 §6.1 placement). Both version 0
  (`PresentationTime::Delta`, segment-relative) and version 1
  (`PresentationTime::Absolute`, representation-relative) are supported. The
  primary consumer is SCTE 35 in-band splice signalling (`urn:scte:scte35:2013:bin`,
  ANSI/SCTE 214-3). [`EmsgBox`], [`PresentationTime`], and [`EmsgVersion`] from the
  workspace `mp4-emsg` crate are re-exported from the `transmux` crate root so
  callers need no additional dependency. [`build_media_segment`] delegates to the
  new function with an empty event slice (byte-identical output).
- **fMP4/CMAF conformance validator** (#481): new `validate` module — the fMP4
  analogue of a TR 101 290 monitor. `validate_init_segment`,
  `validate_media_segment`, and `validate_cmaf_track` (cross-segment) walk the
  ISOBMFF box tree and return `Vec<ConformanceIssue>` (`Severity::Error` /
  `Warning`, each with a stable dotted `code` + spec-cited message) against
  ISO/IEC 14496-12 (box presence/order, `ftyp`/`moov`/`mvhd`/`trak` tree,
  `mvex`/`trex` fragmentation marker, `styp`/`moof`/`mfhd`/`traf`/`tfhd`/`tfdt`/
  `trun`, moof↔mdat pairing, `trun` sample-size/`data_offset` mdat-bounds,
  zero-duration samples) and ISO/IEC 23000-19 (CMAF — segment brands,
  single-track fragments, required `tfdt`, contiguous decode timeline, strictly
  increasing `mfhd.sequence_number`). Malformed input yields issues, never a
  panic.
- **HEVC SPS decode verified against real fixture** (#516): `decode_hevc_sps`
  proven correct on the committed `hevc_frag.mp4` hvcC record — asserts exact
  ffprobe oracle values (320×240, Main profile idc=1, 4:2:0, 8-bit, level 60).
  Truncated-input negative tests added. `decode_hevc_sps` doc now cites
  ITU-T H.265 §7.3.2.2.1 (syntax) + §7.4.3.2.1 (conformance-window semantics).
- **HLS discontinuity support** (#453): `MediaSegment::discontinuous` flag and
  `MediaPlaylist::discontinuity_sequence` field; `MediaPlaylist::to_m3u8()` emits
  `#EXT-X-DISCONTINUITY` immediately before every flagged segment (RFC 8216 §4.3.4.3)
  and `#EXT-X-DISCONTINUITY-SEQUENCE:<n>` in the playlist header when `n > 0`
  (RFC 8216 §4.3.3.3). `Segmenter::mark_discontinuity()` marks the next segment cut
  as discontinuous (explicit API); `Segmenter::take_ready_with_meta()` returns
  `(Vec<u8>, SegmentMeta)` pairs carrying the discontinuity flag. Auto-detection of
  init-segment changes is available via the new
  `mark_init_discontinuities(entries: &mut [(&[u8], &mut MediaSegment)])` helper,
  which compares consecutive init bytes and sets the flag where they differ.
- **RTMP transport spoke** (#515): `RtmpDemux` (`Unpackage`) / `RtmpMux` (`Package`) ⇄ IR,
  Adobe RTMP 1.0. De/frames the chunk stream (basic + message headers, all four `fmt`
  types incl. 2-/3-byte csid and extended timestamp — §5.3.1), reassembles multi-chunk
  messages honouring Set Chunk Size, and routes Audio (type 8) / Video (type 9) message
  bodies — which ARE FLV tag bodies — through the FLV spoke (`FlvDemux`/`FlvMux`) to the
  IR. Also typed `Handshake0/1/2` (§5.2), `ProtocolControl` (SetChunkSize/Abort/Ack/
  WindowAckSize/SetPeerBandwidth — §5.4), and AMF0 `AmfValue`/`Command` for
  `connect`/`publish`/`play`/`createStream`/`onMetaData` (§7). AMF0 only (AMF3 noted as
  out of scope). `no_std` + `alloc`.
- **I-frame-only trick-play track derivation** (#477): `trickplay::derive_iframe_track(&Track) -> Result<Track>` — retains only sync samples from a video track and folds each kept sample's duration to span the gap to the next keyframe, conserving the total timeline. `append_iframe_track(&mut Media, usize)` is a convenience that appends the derived track to an existing `Media`. Returns `Error::InvalidInput` when the source has no sync samples. Codec/container-agnostic; works with any `CodecConfig`. Downstream signalling (`EXT-X-I-FRAME-STREAM-INF` / DASH trick-mode) is deferred to a follow-up issue.
- **RTCP control packets** (#514): typed `Parse`/`Serialize` for the RFC 3550 §6 set —
  `SenderReport` (PT 200), `ReceiverReport` (201), `SourceDescription` (202, with
  `SdesChunk`/`SdesItem`/`SdesItemType`), `Bye` (203), `App` (204), the shared
  `ReportBlock` (24-bit sign-extended cumulative-lost), `CommonHeader`, and a
  `CompoundPacket` that enforces the §6.1 "first packet must be SR/RR" rule on
  construction, parse, and serialize. Dispatch via `RtcpPacket` / `RtcpPacketType`
  (`name()` + `impl_spec_display!`). RTP companion to `rtp.rs`; not a hub spoke.
- **Public NAL keyframe helper** (#517): `nal_unit_type` / `is_keyframe_nal` /
  `access_unit_is_keyframe` + `NalCodec` (Avc/Hevc/Vvc) — Annex-B and 4-byte
  length-prefixed, spec-cited to H.264/H.265/H.266 §7.3.1. `ts_demux` IDR detection
  now delegates to it (behaviour byte-identical).
- **FLV container spoke** (#513): `FlvDemux` (`Unpackage`) / `FlvMux` (`Package`) ⇄ IR,
  Adobe FLV v10.1 Annex E. H.264 (AVCVIDEOPACKET, avcC seq-header, CompositionTime →
  composition offset) + AAC (AACAUDIODATA, ASC seq-header); reuses `CodecConfig::Avc`/`Aac`.
  ms timescale, lossless timing round-trip. Skips spurious empty sequence-header tags;
  trusts the ASC over contradictory `onMetaData`.

### Fixed
- **HEVC sample-entry visual dimensions** (#467): `HEVCSampleEntry::bare_parse`
  read `width`/`height` (and the following visual fields) from the wrong byte
  offsets, so `Fmp4Demux` recovered `0×0` dimensions for `hvc1`/`hev1` tracks.
  Corrected to the ISO/IEC 14496-12 §12.1.3 `VisualSampleEntry` layout (width at
  `body[24]`), matching the AVC entry and `VisualSampleEntryFields::parse_body`.

### Changed
- **`Track` gains a `start_decode_time: u64` field** (#476): the absolute decode
  time of the track's first sample, in the track's media timescale — the
  fragment `tfdt` `baseMediaDecodeTime` (ISO/IEC 14496-12:2015 §8.8.12) anchor
  that `Sample` relative timing lacked. `Track::new` still defaults it to 0 (all
  existing callers compile); `Track::new_at(spec, samples, start)` and
  `Track::with_start_decode_time(start)` set it. This is an additive struct-field
  change → a **minor** version bump.
- **`CmafMux` now writes `Track::start_decode_time` as the first segment's
  `base_media_decode_time`** (#476), replacing the previously hardcoded `0`. A
  rebase/offset transform is therefore observable in the muxed `tfdt`.

## [0.8.0] — 2026-07-02
### Added
- **Any-to-any hub** (#466): the container-agnostic IR (`Media` / `Track`, thin wrappers
  over `TrackSpec`/`Sample`) + implementations of the new **broadcast-common 8.2.0**
  traits — `CmafMux` / `HlsPackager` (`Package`) and `Fmp4Demux` (`Unpackage`, fMP4 →
  `Media`). Every demux/mux spoke now targets one hub API; `Unpackage`⇄`Package` and
  `Encrypt`⇄`Decrypt` are inverse pairs mirroring `Parse`/`Serialize`. Additive —
  `build_init_segment`/`build_media_segment`/`Segmenter`/`TrackSpec`/`Sample` unchanged.
- Requires broadcast-common ≥ 8.2.0 (the trait definitions).
- **MPEG-H 3D Audio** input (promotion): `Fmp4Demux` now reconstructs
  `CodecConfig::MpegH` from `mha1`/`mha2`/`mhm1`/`mhm2` sample entries (re-parsing
  the `mhaC` record, ISO/IEC 23008-3 §20) — MPEG-H was previously output-only.
  Verified byte-exact against a real Fraunhofer/DASH-IF MPEG-H bitstream. This
  makes the codec set demux+mux complete across the hub.
- **VVC / H.266** (#474): `CodecConfig::Vvc` + `vvc1`/`vvcC` (VvcDecoderConfiguration-
  Record as a FullBox, byte-exact Parse/Serialize) mirroring HEVC. `decode_vvc_sps`
  (H.266 §7.3.2.4/§7.3.3.1) recovers dims/profile/tier/level; `Fmp4Demux` reconstructs
  `Vvc` from `vvc1`/`vvi1`; `CmafMux` emits `vvc1`. Byte-verified against a real
  vvenc+ffmpeg fixture. vvcC layout doc grounded in the FFmpeg reference (§11).
- **VP8 + Vorbis** (WebM): `CodecConfig::Vp8` (dims from the RFC 6386 key-frame
  header) + `CodecConfig::Vorbis` (channels/sample_rate + verbatim `CodecPrivate`
  from the Vorbis I identification header). `WebmDemux` now covers all four WebM
  codecs (VP9/VP8 video, Opus/Vorbis audio). WebM-native (no mp4 mux path).
- **MPEG-2 video (H.262) + MPEG-1/2 audio (MP1/2/3)** codecs: `CodecConfig::Mpeg2Video`
  + `MpegAudio`. `Fmp4Demux` reconstructs `mp4v`/esds (OTI 0x60–0x65) → `Mpeg2Video`
  (dims from the in-band sequence header, ISO 13818-2 §6.2.2.1) and `mp4a` OTI
  0x69/0x6B → `MpegAudio` (layer/rate/channels from the frame header, ISO 11172-3
  §2.4.1.3). `TsDemux` handles `stream_type` 0x02/0x03/0x04 — the classic broadcast
  pair now round-trips through both fMP4 and TS. New `Mp4vSampleEntry` +
  `MpegAudioLayer` enum.
- **`CodecConfig::Hevc`** + **complete `Fmp4Demux` codec-config reconstruction**
  (#467 codec tail): `Fmp4Demux` now reconstructs the IR codec config for every
  codec the crate can output — `hvc1`/`hev1`→`Hevc`, `av01`→`Av1`, `vp09`→`Vp9`,
  `Opus`→`Opus`, `fLaC`→`Flac`, `dac3`→`Ac3`, `dec3`→`Eac3`, `ddts`→`Dts` (plus
  the existing `avc1`/`mp4a`) — was previously deferred to AVC/AAC only. New
  `Hevc` variant muxes to an `hvc1`+`hvcC` sample entry. Every codec round-trips
  byte-identically (config box + coded samples) via fragmented-mp4 fixtures.
  Unknown sample entries skip the track rather than erroring.
- **RTP spoke** (#469): `RtpPacketizer` (`Package`) and `RtpDepacketizer`
  (`Unpackage`) — de/packetize the `Media` IR ⇄ RTP. H.264 single-NAL / STAP-A
  (SPS+PPS) / **FU-A** fragmentation at MTU (RFC 6184), AAC `AAC-hbr` AU-headers
  (RFC 3640), RTP fixed header with marker/seq/90 kHz timestamps (RFC 3550), and
  SDP generation (`rtpmap`/`fmtp` with `sprop-parameter-sets` + AAC `config`,
  RFC 4566). Round-trips byte-identically through the real demuxed NALs; no new
  dependency (hand-rolled base64/hex). New `RtpMediaKind` enum.
- **Microsoft Smooth Streaming output** (#473): `SmoothPackager` implements the hub
  `broadcast_common::Package` trait — `Media` IR → a Smooth client Manifest
  (`SmoothStreamingMedia`>`StreamIndex`>`QualityLevel`+`c`) + Smooth fragment-MP4
  fragments (`moof` with the `tfxd` `uuid` box + `mdat`). FourCC `H264`/`AACL`,
  `CodecPrivateData` = start-code SPS+PPS / raw ASC; TimeScale 10 MHz. New
  `TfxdBox` uuid type + `SmoothStreamType` enum. Fragments round-trip losslessly
  via `Fmp4Demux`. Cites [MS-SSTR] + ISO/IEC 14496-12.
- **Low-latency DASH** (#461): `LlSegmenter` (chunked CMAF — each segment
  subdivided into `moof`+`mdat` chunks, first chunk `styp`-prefixed, contiguous
  `tfdt`/sequence numbers) + `LlDashPackager` (LL-DASH MPD: `type="dynamic"`,
  `SegmentTemplate@availabilityTimeComplete="false"` + `@availabilityTimeOffset`,
  `<ServiceDescription><Latency>`). Both `impl broadcast_common::Package`. Chunks
  concatenate losslessly to a whole segment (verified via `Fmp4Demux`). ISO/IEC
  14496-12 chunk structure + ISO/IEC 23009-1 / DASH-IF LL IOP signalling.
- **WebM / Matroska demuxer** (#471): `WebmDemux` implements the hub
  `broadcast_common::Unpackage` trait — WebM (EBML) → `Media` IR, the fourth hub
  input (TS / fMP4 / MPEG-PS / WebM). Hand-written EBML/VINT tree walker
  (RFC 8794 framing, RFC 9559 element IDs); maps `V_VP9`→`CodecConfig::Vp9`
  (synthesised `vpcC`) and `A_OPUS`→`CodecConfig::Opus` (`dOps` from the CodecPrivate
  `OpusHead`); (Simple)Block timestamps in a millisecond IR timescale, Opus
  pre-skip codec delay applied. Gated against a per-frame **size-column** ffprobe
  oracle + a CMAF output round-trip (vp09/`vpcC` + Opus/`dOps`).
- **CENC decrypt** (#465): `CencDecryptor` implements the hub
  `broadcast_common::Decrypt` trait — unprotect a CENC (`cenc` / AES-128-CTR) fMP4
  given the content key. Reuses the existing `cenc.rs` box parsers
  (`tenc`/`senc`/`saiz`/`saio`/`sinf`/`frma`); subsample-aware (clear ranges
  skipped, CTR streams across protected ranges), IV left-justified to 16
  (ISO/IEC 23001-7 §10.1). AES via RustCrypto `aes`/`ctr` behind an optional
  `cenc` feature (`--no-default-features` drops it). `cbcs` documented
  unsupported. Verified by decrypting a real ffmpeg-encrypted fixture to
  byte-identical cleartext (+ wrong-key negative). New `CencScheme` enum.
- **MPEG-2 Program Stream demuxer** (#470): `PsDemux` implements the hub
  `broadcast_common::Unpackage` trait — MPEG-2 PS (`.ps`/VOB-style) → `Media` IR,
  the third hub input alongside TS and fMP4. Parses packs/system-header via
  `mpeg-ps`, maps elementary streams by `stream_id` (H.264 0xE0–0xEF; AC-3 in
  `private_stream_1` 0xBD), reassembles PES across packs, recovers H.264 `avcC`
  (in-band SPS/PPS) + AC-3 `dac3` (syncframe BSI). Gated against ffprobe timing +
  byte-identical `avcC`/video-NAL oracles (ISO/IEC 13818-1 §2.5).
- **Classic HLS (MPEG-2 TS segments)** (#472): `TsHlsPackager` implements the hub
  `broadcast_common::Package` trait (`Output = TsHlsOutput { segments, playlist }`),
  segmenting the `TsMux` output at keyframe boundaries into independently-decodable
  `.ts` segments (each re-emits PAT+PMT + a keyframe-aligned PES) plus an RFC 8216
  HLS media playlist (`#EXTINF` + `.ts` URIs, no `#EXT-X-MAP`). Per-segment base DTS
  keeps one monotonic timeline across boundaries — the concatenated segments
  round-trip losslessly through `TsDemux`.
- **DASH `.mpd` output** (#464): `DashPackager` implements the hub
  `broadcast_common::Package` trait (`Output = String`), emitting a DASH MPD
  (ISO/IEC 23009-1) alongside the HLS playlists from one CMAF —
  MPD→Period→AdaptationSet(`video/mp4`,`audio/mp4`)→Representation with
  `SegmentTemplate` (`$Number$`/`$RepresentationID$`). `codecs=` from the crate's
  own rfc6381 builders; `@width`/`@height` from the SPS, `@audioSamplingRate` from
  the ASC, integer `@bandwidth`; VOD (`static`) + `dynamic` (live). Dependency-free
  XML writer; integer-only arithmetic (`no_std`-clean).
- **fMP4/CMAF repackage** (#462): `Repackage` + `Media` IR transforms —
  `select_tracks` (track subset), `trim` (half-open presentation window, snapped
  back to the preceding sync sample per CMAF ISO/IEC 23000-19 §7.3.2.3), and
  resegment (via the existing `Segmenter`) to a new target duration. Composes
  demux → transform → mux with no new box parsers; lossless (byte-identical coded
  samples across identity repackage, verified against the `TsDemux` oracle).
- **TS muxer** (#460): `TsMux` implements the hub `broadcast_common::Package`
  trait — `Media` IR → a whole-188-byte-packet MPEG-2 TS, the byte-level inverse
  of `TsDemux`. Emits PAT→PMT (CRC-32/MPEG-2 sections), `stream_type` per codec,
  PCR on the first video PID; per-sample PES (PTS always, DTS when differing),
  video length-prefixed→Annex B with SPS/PPS re-injection on keyframes, AAC
  re-wrapped in ADTS from the `esds` ASC (ISO/IEC 13818-1 §2.4.3/§2.4.4). With
  `TsDemux` this closes the loop: `{fMP4/CMAF} → IR → {TS}` and byte-fidelity
  `TS → IR → TS` round-trips.
- **Progressive MP4 output** (#463): `ProgressiveMux` implements the hub
  `broadcast_common::Package` trait, muxing the `Media` IR into a single-file,
  non-fragmented `.mp4` (ftyp + one moov with full `stbl` sample tables + one
  mdat) — the VOD/download counterpart to `CmafMux`. Builds `stts`/`ctts`/
  `stsc`/`stsz`/`stco`|`co64`/`stss` from the sample stream (ISO/IEC 14496-12
  §8.5–§8.7); `faststart: bool` writes moov-before-mdat via a two-pass
  chunk-offset fixup. Adds typed `co64`/`stss` boxes. Gated against the ffmpeg
  faststart ref mp4 (byte-identical video samples + `avcC`).
- **TS demuxer** (#467, partial — H.264 + AAC): `TsDemux` implements the hub
  `broadcast_common::Unpackage` trait, turning MPEG-2 TS bytes into the `Media`
  IR — the input side of the any-to-any hub, so `{TS} → IR → {any}` works
  in-crate. Follows PAT→PMT, maps `stream_type`→codec, per-PID PES reassembly
  (PTS/DTS 33-bit unwrap), and recovers codec config from in-band parameters
  (H.264 SPS/PPS → `avcC`; AAC ADTS → ASC/`esds`; AC-3/E-AC-3 syncframe →
  `dac3`/`dec3`). Verified against ffprobe timestamp and ffmpeg `-c copy` byte
  oracles. HEVC/DTS are recognised in the PMT but not yet emitted (no IR
  HEVC-video variant / DTS-ES parser) — tracked on #467.
- `mpeg-ts` / `mpeg-pes` are now regular dependencies (were dev-deps).

### Fixed
- **ADTS `channel_configuration` decode** (`aac_asc`): the 3-bit field was split
  wrong (`byte2[0]<<3 | byte3[7:5]`); the correct ISO/IEC 13818-7 §6.2 layout is
  `byte2[0]<<2 | byte3[7:6]`. Build+parse were self-consistent so round-trip
  tests passed, but a real mono ADTS stream was misread as stereo. Both
  directions corrected.

### Fixed
- `TsDemux` now decodes AVC `width`/`height` from the in-band SPS (was left at 0).

## [0.6.0] — 2026-07-02
### Added
- **DTS** fMP4 carriage (#437, ETSI TS 102 114 §E.2): `dtsc`/`dtsh`/`dtsl`/`dtse`
  sample entries + `ddts` (DTSSpecificBox — DTSSamplingFrequency, max/avg bitrate,
  pcmSampleDepth, FrameDuration, StreamConstruction, channel layout, …) + a
  `CodecConfig::Dts` variant + `rfc6381()`. Typed Parse/Serialize with a spec-vector
  byte-exact round-trip + `build_init_segment` moov round-trip (ffmpeg has no `ddts`
  encoder, so the real-fixture gate is deferred).
### Changed
- `hvcC` value-verified against the ISO 14496-15:2017 §8.3.3.1 text (recovered via
  marker OCR of the scanned edition), matching FFmpeg movenc + the byte-exact oracle
  (#394). Docs only.

## [0.5.0] — 2026-07-01
### Added — fMP4 gap tier (real codecs + container completeness)
- **Codec sample entries + config boxes** (container-level; header parse only, samples
  pass through opaque): AV1 (`av01`/`av1C`, #436), VP9 (`vp09`/`vpcC`), Opus (`Opus`/
  `dOps`), FLAC (`fLaC`/`dfLa`) (#437), AC-4 (`ac-4`/`dac4`, #431), MPEG-H 3D Audio
  (`mha1`/`mhm1`/`mhaC`, #433), and HE-AAC SBR/PS AudioSpecificConfig signaling →
  `mp4a.40.5`/`mp4a.40.29` (#432). Each with a `CodecConfig` variant + `rfc6381()`.
- **CENC per-sample encryption** (#429, ISO/IEC 23001-7): `tenc`/`senc`/`saiz`/`saio`/
  `pssh` + `sinf`/`frma`/`schm`/`schi` + `enca`/`encv` sample entries.
- **Subtitle carriage** (#430, ISO/IEC 14496-30): `stpp` (TTML/IMSC) + `wvtt` (WebVTT +
  `vttC`/`vtte`/`vttc`/`payl`/`sttg`/`iden`).
- **Sample-entry extensions** (#434): `colr` (nclx — HDR/wide-gamut), `pasp`, `clap`.
- **Timing / grouping** (#435): `prft` (ProducerReferenceTimeBox), `sgpd`/`sbgp`
  (sample groups incl. `roll`), `subs` (sub-sample info).
- **avcC/hvcC value-verification** (#441/#394): byte-exact round-trip against real
  ffmpeg-muxer boxes; avcC now grounded on the text-layer 14496-15:2012.

All new boxes are typed with symmetric `Parse`/`Serialize` and byte-exact round-trip
tests against real ffmpeg-authored fixtures (config-box oracles = ffmpeg's own muxer
output); MPEG-H uses a spec vector (no redistributable fixture/encoder).

## [0.4.1] — 2026-07-01

Bundles three additive releases that were never separately published under
this name (crates.io jumps 0.1.0 → 0.4.1 directly; 0.2.0/0.3.0/0.4.0 below are
folded in here for a complete record, newest first).

### Changed
- Value-verified the `esds` / `mp4a` descriptor layout against the vendored
  ISO/IEC 14496-1 §7.2.6 (transcribed to `docs/codec/es-descriptor-14496-1.md`)
  and added a **byte-exact round-trip test on a real ffmpeg-authored `esds`**
  (AAC-LC, 4-byte-expanded descriptor sizes, real max/avg bitrates). No API change.

### Added (originally drafted as 0.4.0)
- AC-3 / E-AC-3 audio in the fMP4 path (ETSI TS 102 366 Annex F):
  - `Ac3SpecificBox` (`dac3`) + `Ec3SpecificBox` (`dec3`) — typed config boxes with
    Parse + symmetric Serialize + round-trip.
  - AC-3 / E-AC-3 syncframe BSI parsers (`0x0B77` syncword → `syncinfo()`+`bsi()` /
    E-AC-3 syncframe): build a `dac3`/`dec3` from an elementary stream.
  - `CodecConfig::Ac3` / `Eac3` + `Ac3SampleEntry` / `Ec3SampleEntry`
    (`SampleEntryVariant::Ac3`/`Ec3`), wired through `build_init_segment` to emit
    `ac-3` / `ec-3` sample entries.
  - `rfc6381()` → `"ac-3"` / `"ec-3"`.
- Gate `tests/dolby.rs`: parses real ffmpeg-encoded AC-3/E-AC-3 fixtures and asserts
  the built `dac3`/`dec3` bytes match ffmpeg's own MP4-muxer output byte-for-byte.

### Added (originally drafted as 0.3.0)
- SPS/VPS decode + RFC 6381 codec strings, so transmux no longer needs an external
  parser (e.g. `h264_reader`) to learn what it must put in an `avcC`/`hvcC`:
  - `AvcSps::decode() -> AvcSpsInfo` (ITU-T H.264 §7.3.2.1.1): profile_idc,
    constraint byte, level_idc, chroma_format_idc, bit-depths, `frame_mbs_only`,
    and coded width·height after frame cropping (chroma-dependent CropUnit +
    interlaced height factor). Handles the high-profile branch and skips
    SPS-embedded scaling lists.
  - `HevcNalUnit::decode_sps() -> Option<HevcSpsInfo>` (ITU-T H.265 §7.3.2.2 +
    profile_tier_level §7.3.3): general profile/tier/level, compatibility +
    constraint flags, chroma/bit-depth, conformance-window-cropped dimensions.
  - RFC 6381 strings: `AvcSps::rfc6381()` → `avc1.PPCCLL`;
    `HevcNalUnit::rfc6381()` → `hvc1.…`; `AudioSpecificConfig::rfc6381()` →
    `mp4a.40.<AOT>`. Plus a public `bitreader` (Exp-Golomb + emulation-prevention
    unescape) and `sps` module.
- Gate `tests/codec_config.rs`: decodes real ffmpeg-encoded fixtures across the
  full H.264 profile matrix (baseline/main/high/high10/high422/high444 + interlaced
  + 1080-cropped) and HEVC main/main10, asserting every field against a
  `trace_headers` oracle, plus a scaling-list spec vector and an avcC round-trip.

### Added (originally drafted as 0.2.0)
- `Segmenter`: a stateful streaming CMAF segmenter wrapping `build_init_segment` /
  `build_media_segment`. Feed coded samples in decode order (`push`), pull finished
  media segments (`take_ready`), and `flush` the trailing partial segment at
  end-of-stream. Segments are cut on the anchor track (first video track, else the
  first track) at a keyframe once the target duration is reached, so every video
  segment begins on a random-access point and the concatenation of all segments
  carries the full stream with contiguous per-track `tfdt`. This is the streaming
  state machine `build_media_segment` (a batch box builder) deliberately omits;
  it lets a live remuxer produce a CMAF track without hand-rolling segment cutting.
- `Error::InvalidInput(&'static str)` for caller-precondition violations (empty
  track list, non-positive segment duration, duplicate `track_id`, unknown
  `track_id` on `push`).

## [0.1.0] — 2026-07-01
### Added
- End-to-end `tests/ts_to_cmaf.rs`: demux a real H.264+AAC MPEG-TS
  (`fixtures/ts/h264_aac.ts`), synthesize `avcC` from the stream's SPS/PPS and
  `esds` from the AAC ADTS header, build init + media segments, then re-parse —
  asserting **byte-identical** avcC SPS/PPS + esds AudioSpecificConfig fidelity,
  75 video access units, the computed ADTS frame count, and first-sample
  round-trip. Closes the literal "TS in → CMAF out" acceptance (#408).
- HLS playlist generation (RFC 8216): `MediaPlaylist` / `MasterPlaylist` +
  `Variant` / `MediaSegment` with `to_m3u8()` emitters for the CMAF segments
  produced by the remux pipeline (`#EXTM3U` / `EXT-X-VERSION` / `TARGETDURATION` /
  `MEDIA-SEQUENCE` / `EXTINF` / `ENDLIST`; master `EXT-X-STREAM-INF` with
  bandwidth/codecs/resolution; `extra_tags` for `EXT-X-DATERANGE`). Generated
  playlists validate clean through `media-doctor::check_playlist`.
- Segment-level boxes: `FileTypeBox` (`ftyp`), `SegmentTypeBox` (`styp`),
  `MediaDataBox` (`mdat`, 32-bit + 64-bit largesize).
- Typed `MovieExtendsBox` (`mvex`) + `TrackExtendsBox` (`trex`) on `MovieBox`
  (fragmented-init movies); byte-identical round-trip of a real fragmented moov.
- Annex B ↔ length-prefixed NAL conversion (`annexb_to_length_prefixed`,
  `length_prefixed_to_annexb`, iterators).
- Samples-in TS→CMAF remux pipeline: `build_init_segment` (ftyp + fragmented-init
  moov with empty sample tables + mvex/trex) and `build_media_segment`
  (styp + moof{tfhd,tfdt,trun} + mdat) with `data_offset` computed from the
  finished `moof` and signed composition offsets. `CodecConfig` / `TrackSpec` /
  `Sample` / `FragmentTrackData` API.
- Typed init-segment `moov` box tree (`MovieBox`/`mvhd`/`trak`/`tkhd`/`mdia`/
  `mdhd`/`hdlr`/`minf`/`stbl`/`stsd` + sample descriptions) with byte-identical
  round-trip.
- `avcC`/`hvcC` config boxes, `esds`/ES_Descriptor, AAC AudioSpecificConfig +
  ADTS, movie-fragment boxes (`moof`/`mfhd`/`traf`/`tfhd`/`tfdt`/`trun`),
  timing boxes (`stts`/`ctts`/`stsc`/`stsz`/`stco`/`elst`/`sidx`), and generic
  box framing.

_Unreleased — `transmux` has not yet been published to crates.io._
