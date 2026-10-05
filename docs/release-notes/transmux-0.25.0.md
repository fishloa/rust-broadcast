# transmux 0.25.0

_Released 2026-10-05._

### Added

- `CencDecryptor::from_fmp4_bytes` — `from_fmp4` over an already-shared `bytes::Bytes`, so the file is
  not copied and `demux` hands out samples as slices of it (#1081, r05-O2).
- `AudioSpecificConfig::effective_sampling_frequency` — the explicit rate when present, else the
  `samplingFrequencyIndex` rate (#1081, r04-O5).
- `impl Serialize for SampleEntryVariant` (#1081, r05-O3).
- `ll_dash::UTCTIMING_HTTP_HEAD_2014` / `_HTTP_XSDATE_2014` / `_HTTP_ISO_2014` /
  `_NTP_2014` / `_HTTP_NTP_2014` / `_DIRECT_2014` — the registered
  `urn:mpeg:dash:utc:*:2014` `UTCTiming` scheme identifiers (ISO/IEC 23009-1
  §5.8.4.11) accepted by `LlDashPackager::with_utc_timing`.
- `webm_demux::ns_to_ir_ticks` — nanoseconds to IR ticks, rounding to nearest
  (used for per-frame timing of laced WebM blocks).
- `ac3::Ec3SpecificBox::from_es_all` — parse every E-AC-3 syncframe of an
  elementary stream (independent plus dependent), stopping at the first bad
  sync word or truncated tail.
- `annexb::NAL_LENGTH_SIZE_MINUS_ONE` — the `lengthSizeMinusOne` (3) an
  `avcC`/`hvcC` must declare for the IR's 4-byte NAL length prefix
  (ISO/IEC 14496-15 §5.3.3).
- `frag_offsets::mdat_ranges` / `MdatRange` / `sample_ranges_in` — scan the
  file's `mdat` payload ranges once and resolve each `moof` against them.
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

- Dependency bumps, non-breaking (no public API change): `aes` 0.9, `ctr` 0.10, `cbc` 0.2 (RustCrypto 0.13 generation). CENC `cenc`/`cbcs` and HLS Sample-AES ciphertext is byte-identical (oracle fixtures pass).

Optimization sweep (#1079, #1080, #1081). Apart from the three behaviour changes listed under
`### Changed (breaking)` and `### Fixed`, every change below leaves the output byte-identical on the
committed fixtures (pinned by `tests/sweep_output_golden.rs`, whose hashes were captured on the
pre-sweep code) and is backed by a deterministic allocation or work counter
(`tests/alloc_counts_sweep.rs` and in-module counters), never a timing.

- `MovieFragmentBox` / `TrackFragmentBox` child walks use the crate's one container-child walker (`init_segment::walk_children`) instead of two private `parse_box` loops (#1141).
- The AAC `esds` -> `DecoderSpecificInfo` / `AudioSpecificConfig` lookup is one `EsdsBox` helper shared by DASH, TS mux and Smooth; a missing DSI in the TS muxer is now `Error::UnexpectedBox` (was `InvalidInput`) (#1141).
- AC-3 and DTS header parsing share one `bitreader::read_bits_checked` (the DTS copy lacked the `n > 64` check) (#1141).
- `box_types::box_slices` / `find_top_box` are now the one best-effort top-level box walker over `box_iter`; the copies in `cenc_decrypt`, `media` and `progressive` and `validate::children` are gone, behaviour unchanged (#1141).
- The four Annex B start-code scanners (`annexb::start_code_positions`, `au::first_nal_start`, the streaming splitter's resumable scan, `au::start_code_len`) and `mpeg_legacy::find_start_code` now share one `annexb::find_start_code_prefix` primitive; behaviour unchanged (#1141).
- Progressive `stsz`/`stsc`/`stco`/`co64` sample-layout expansion is one `progressive_demux::sample_layout` shared by `ProgressiveDemux` and the protected-progressive path of `CencDecryptor` (#1141).
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
- `jiff` replaces the hand-rolled `civil_from_days` (CLI `availabilityStartTime`); `base64`/`hex` crates replace the hand-rolled codecs.

Intentionally not done: `BitReader::from_rbsp` borrowing (a public lifetime change for one allocation
per SPS), the three merge-by-decode-time loops (their tie-break rules differ), and a typed-parser
rewrite of `cenc_decrypt`'s remaining box walker (#1081).

### Changed (breaking)

- **BREAKING: XML support now requires the `std` feature; hand-rolled XML replaced by `quick-xml`;
  `roxmltree`/the private tokenizer dropped.** `dash`, `dash_parse`, `smooth`, `smooth_parse`,
  `ll_dash::LlDashPackager` and `drm::{playready_wrmheader, playready_pro, playready_pssh}` are gated
  behind `std` (and so are their crate-root re-exports); a `--no-default-features` build keeps
  everything else (`LlSegmenter`/`Chunk`, the Widevine/FairPlay `pssh` builders, …). The private
  `xml_parse` tokenizer and `xml_writer` are deleted: parsing is a `quick_xml::Reader` pull loop and
  every attribute value and text node is written through quick-xml's escaping. Rendered MPD, Smooth
  manifest, LL-DASH MPD and WRMHEADER output is byte-identical to the previous release for every
  committed fixture; a value containing `\t`/`\n`/`\r` in an attribute is now written as a character
  reference (`&#9;`/`&#10;`/`&#13;`) so it survives a re-parse. The parsers are stricter on malformed
  input (an undefined entity or a mismatched end tag anywhere is an error; a raw `&` in an attribute
  is no longer tolerated) and `DashParseError`/`SmoothParseError` gain an `Xml { pos, message }`
  variant; `MismatchedEndTag::expected` is now a `String`. `LlDashPackager` now rewrites the base MPD
  with `quick-xml` events instead of line-based text surgery.

- **Behaviour differences from the hand-rolled XML code (all follow XML 1.0):**
  - a literal newline, tab or carriage return inside an MPD/Manifest attribute value is normalised
    to a space (attribute-value normalisation, XML 1.0 §3.3.3): `<Period id="a⏎b"/>` now parses
    `id` as `"a b"` (previously `"a\nb"`); write `&#10;` to keep a newline;
  - an attribute value containing `\t`, `\n` or `\r` is now *written* as `&#9;`, `&#10;`, `&#13;`
    so it survives a re-parse (`profiles = "a\nb"` renders `profiles="a&#10;b"`);
  - a duplicate attribute is an error: `<Period id="1" id="2"/>` is `MalformedAttribute` (previously
    the first value won silently);
  - `<BaseURL>a<x/>b</BaseURL>` (nested markup in a text-only element) now yields no base URL
    instead of a `MismatchedEndTag` error;
  - a character outside the XML 1.0 `Char` production, literal or via a reference
    (`<BaseURL>&#x1;</BaseURL>`, `id="&#x1;"`), is a structured error.
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
- `uri` module removed (`UriReference`, `resolve`, `merge`, `remove_dot_segments`, `resolve_segment`, `try_resolve*`, `first_forbidden_char` and the `RFC3986_*` tables, plus the crate-root `resolve_uri_reference`/`resolve_uri_segment`/`try_resolve_uri_reference`). Replaced by `base_url::{resolve, resolve_chain, first_forbidden_char}` over `url::Url` (`std` only; also `resolve_url_reference`/`resolve_base_url_chain` at the root). `Mpd::resolve_segment_url` now takes the MPD's own URL (`Option<&url::Url>`) as its first argument and returns `Option<String>`; `Mpd::try_resolve_segment_url` is removed. `BaseURL` entries are trimmed before the control-character guard (XML layout whitespace is harmless); an interior control character or space still yields `None`. Differences from the old resolver, each with an example: with no base a relative input still resolves to a contained relative result (`../../../etc/passwd` -> `etc/passwd`, as before; `%2e%2e/x` behaves like `../x`) and only an absolute-path reference the MPD wrote itself (`/x`) comes back absolute; an input naming the internal `transmux-relative:` scheme is `None`; the host is lower-cased and a default port dropped (`http://H:80/x` -> `http://h/x`); non-ASCII is percent-encoded (`é.m4s` -> `%C3%A9.m4s`); the `//g` network-path row serialises as `http://g/` and `http:g` against an `http` base resolves relatively (WHATWG, listed in `tests/base_url.rs`); a reference that does not parse is `None`.
- `build_sdp_with_connection(IpAddr, Vec<sdp_types::Media>)` replaces the `&str` media-block parameter; RTP SDP generation is `std`-only (`RtpOutput::sdp` exists only with the `std` feature). The SDP bytes are identical (golden-tested). `sdp-types` 0.2 and `url` types appear in the public API.
- DASH writer: `xs:duration` attributes are the shortest ISO 8601 form (`PT2.0S` -> `PT2S`, `PT0.0S` -> `PT0S`, `PT3.0S` -> `PT3S`, `PT90.0S` -> `PT1M30S`); equal durations, different bytes.
- `dash_parse::parse_iso8601_duration` now checks the XML Schema 1.1 `xs:duration` lexical space (then converts with `jiff`) instead of hand-splitting. Every valid form representable as an unsigned `Duration` is accepted (including seconds fractions longer than nine digits, truncated to nanoseconds as before: `PT3.6666666666666665S`). Accept/reject changes vs the previous parser, with examples:
  - now rejected, were accepted: a unit with no digits (`PTS`, `PTHS`, `PTMS` read as zero), `PT.5S` (XSD needs a digit before the point), a `+` sign (`PT+1S`, `P+1D`), and magnitudes beyond `jiff`'s span limits (`PT999999999H`, `P999999999999D`: the old parser saturated; they are now `InvalidDuration`, never a panic).
  - unchanged rejections (still `InvalidDuration`): lower-case designators (`PT1h30m`, `Pt1S`), weeks (`P1W`), fractions on hours/minutes (`PT1.5H`), the comma separator (`PT1,5S`), calendar units (`P1Y`, `P1M`, `P1Y2M3DT4H5M6.7S`: lexically valid but a `Duration` has no calendar), negative durations (`-PT1S`: valid XSD, `Duration` is unsigned), trailing garbage and mis-ordered units.
- `rtp::base64_decode` is stricter than the pre-crate decoder: input whose length is 1 mod 4 (`QUJDR`) is an error (the stray character used to be dropped silently), and `=` anywhere other than as the final one or two characters (`Zm9v=YmFy`, excess padding `Zg===`) is an error. Padding stays optional and non-zero trailing bits stay tolerated.
- CLI `--key` hex now rejects a `+`/`-` sign and non-ASCII input (it used to accept the sign, and panic on non-ASCII).

- `ESDescriptor::parse` now reads the bytes `Serialize` writes — `ES_DescrTag` + expandable-size varint + body — instead of the body alone, so the public `Parse` impl no longer misreads that framing as `ES_ID`/flags/`URLlength` and silently truncates the descriptor chain; an `esds` carrying a `URLstring` longer than ~100 bytes (up to the 255 its 8-bit `URLlength` allows, ISO/IEC 14496-1 §7.2.6.5) no longer fails parse with a spurious `BufferTooShort` (#1148).
  **Breaking (migration):** the *input contract* of the public `Parse` impl changed: it used to read
  the bare descriptor body and now reads the framed bytes `Serialize` writes (`ES_DescrTag` +
  expandable-size varint + body). A caller that previously fed `ESDescriptor::parse` the bare body
  (the bytes after a manually stripped tag/size prefix) must now pass the framed bytes instead, or
  keep parsing through `EsdsBox::parse_body`/`parse_box`, which strip the box and FullBox framing
  themselves and pass exactly what this impl expects. The only in-workspace caller was
  `EsdsBox::parse_body` (`esds` box path); no other crate, example, fuzz target or binding calls
  `ESDescriptor::parse`. Feeding bare-body bytes now returns `InvalidValue { field:
  "descriptor_tag" }` (or a silently misread result if the first body byte happens to be `0x03`).
- `ESDescriptor::parse` rejects a non-UTF-8 `URLstring` with `InvalidValue { field: "URLstring" }`
  instead of decoding it lossily: ISO/IEC 14496-1 §7.2.6.5 defines the field as UTF-8 (ISO/IEC
  10646-1), and a lossy decode (0xFF -> U+FFFD) silently broke byte-identical round trips (the
  re-serialized bytes differed, and 765 replacement bytes from a 255-byte input overflowed
  `URLlength`). A wire `esds` with an invalid URL byte failed to round-trip before and now fails to
  parse at all (#1148 E).

### Fixed

- `CencDecryptor` on a progressive protected MP4 now honours `co64` chunk offsets (the duplicated raw-byte stbl expander read only `stco`) (#1141).
- Progressive sample layout (`ProgressiveDemux` and the protected-progressive path of `CencDecryptor`) follows one rule for uniform and per-sample `stsz`: `stsz.sample_count` is authoritative, surplus `stsc`/`stco` capacity (a short last chunk, extra chunks) is tolerated and clamped, and only a shortfall (chunk tables cover fewer samples than `stsz` declares) is an error. `stsc.first_chunk` must now be >= 1 and strictly ascending (ISO/IEC 14496-12 §8.7.4), else `Error::InvalidValue`; the expansion is one forward pass (#1141).
- `frag_offsets`: a final `mdat` cut short by a truncated capture is now clamped
  to the file end as documented (it was dropped because `parse_box` reports a
  truncated box as `BufferTooShort`), so early samples in it resolve instead of
  failing with "outside every mdat". The `mdat` scan also runs once per file
  rather than once per `traf` (`Fmp4Demux`, `CencDecryptor` and the validator
  were O(fragments x boxes)). `cli`: the unsupported-container message no longer
  carries a run of spaces; `webm_demux` no longer has an `unreachable!` in
  library code.
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
- CLI `--key` no longer panics on a 32-byte non-ASCII argument and rejects a `+`/`-` sign in the hex.

---

Published from tag `transmux-v0.25.0`.
