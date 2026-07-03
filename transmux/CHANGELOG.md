# Changelog

All notable changes to `transmux` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
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
### Changed
- Value-verified the `esds` / `mp4a` descriptor layout against the vendored
  ISO/IEC 14496-1 §7.2.6 (transcribed to `docs/codec/es-descriptor-14496-1.md`)
  and added a **byte-exact round-trip test on a real ffmpeg-authored `esds`**
  (AAC-LC, 4-byte-expanded descriptor sizes, real max/avg bitrates). No API change.

## [0.4.0] — 2026-07-01
### Added
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

## [0.3.0] — 2026-07-01
### Added
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

## [0.2.0] — 2026-07-01
### Added
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
