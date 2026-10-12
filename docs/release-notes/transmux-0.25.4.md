# transmux 0.25.4

_Released 2026-10-13._

Live-player facts release (issue #1162, requested by an Apple TV IPTV player consuming `StreamingTsDemux`): typed audio track metadata and role, audio config facts including LFE-inclusive channel count, video interlace facts, a live-join helper, periodic PAT/PMT in `TsMux`, a documented and tested chunk-size invariance, and a CI cross-check for tvOS/iOS/macOS targets. Additive within 0.25 (no dependent cascade). `transmux` gains a `dvb-si` dependency (default features off) to reuse its descriptor parsers. Behaviour change: default `TsMux` output now contains extra PSI packets (media packets byte-identical; `with_psi_interval_ms(None)` restores the old layout).

### Added

- `TsMux::with_psi_interval_ms(Option<u32>)`, `TsMux::psi_interval_ms()` and `DEFAULT_PSI_INTERVAL_MS` (100): configure the PAT/PMT repetition interval (#1162 R7). `None`/`Some(0)` restores the old write-once-at-start layout.

### Changed

- `TsMux` now **repeats PAT and PMT periodically** (default every 100 ms of media time, between PES runs, continuity counters continuous on the PSI PIDs, PMT content/version unchanged), so a `TsDemux`->`TsMux` file can be joined at any packet-aligned offset (previously a cut such as 1128 bytes in yielded zero tracks) and meets ETSI TR 101 290 1.3/1.5. Elementary-stream packets are byte-identical to before; the only difference is the extra PSI packets. The classic-HLS segmenter (`ts_hls`) is unchanged (PSI at each segment start).

### Added

- `TrackSpec::audio_metadata()` -> `AudioMetadata` (#1162 R1): typed audio-track metadata decoded from the PMT `ES_info` descriptors a TS demux already stores. Carries every `ISO_639_language_descriptor` entry (`AudioLanguage`: language + `AudioType` clean-effects / hearing-impaired / visual-impaired-commentary), the DVB `supplementary_audio_descriptor` (`SupplementaryAudio`: `MixType`, `EditorialClassification`, language; EN 300 468 §6.4.11 Tables 154/155), and a derived `AudioRole` (`Main`/`AudioDescription`/`HearingImpaired`/`Unknown`). Documented heuristic: audio description = `audio_type` 3, editorial 1 or language `qad` (France 2/TF1); hearing impaired = `audio_type` 2 (ORF) or editorial 2; `audio_type` 1 (clean effects) does **not** imply non-main (ServusTV). Reuses `dvb-si`'s descriptor parsers (new `dvb-si` 11.0 dependency, `no_std`+`alloc`); the DASH `@lang` extraction now goes through it instead of its own walker. Real-fixture test on `fixtures/ts/france2.ts` (`fre`/`qad`+editorial 1/`qaa`).
- `CodecConfig::audio_sample_rate()`, `channels_with_lfe()` and `nominal_bitrate()` (#1162 R2). `channels_with_lfe()` adds the AC-3 `lfeon` / E-AC-3 `lfeon` + dependent-substream `chan_loc` channels (a 5.1 AC-3 track: `channel_count` 5, `channels_with_lfe()` 6; `channel_count` is unchanged). `nominal_bitrate()`: AC-3 `bit_rate_code` (Table F.4.1), E-AC-3 `dec3` `data_rate`, MPEG audio first-frame `bitrate_index`, DTS core `ddts` rate; AAC reports the `esds` `avgBitrate` only if the source signalled one (ADTS/ASC carry none). Also `Ac3SpecificBox`/`Ec3SpecificBox::channels_with_lfe`/`nominal_bitrate_kbps` and `MpegAudioFrameHeader::bitrate_bps`. Expected values verified against `ffprobe` on the committed fixtures.

### Changed

- `TsDemux`/`StreamingTsDemux`: an MPEG-1/2 audio track's `esds` now carries `maxBitrate`/`avgBitrate` taken from the first frame header (previously 0).

### Added

- **Video interlace facts** (#1162 R3), new `transmux::interlace` module. Per track: `CodecConfig::video_scan()` / `video_scan(&config)` -> `VideoScan` (`Progressive`, `Paff`, `Mbaff`, `FieldSequence`) from the SPS (`frame_mbs_only_flag`, `mb_adaptive_frame_field_flag`; HEVC `field_seq_flag`). Per sample: `video_picture_facts(&config, &sample.data)` -> `PictureFacts { field_pic, fields, bottom_field_first, pic_struct, source_scan_type }` — `field_pic_flag`/first-field parity from the slice header (a PAFF-paired sample reports `fields == 2` and the first field's parity), plus `pic_struct` (`PicStruct`, H.264 Table D-1 / H.265 Table D.2) and HEVC `source_scan_type` from the picture timing SEI when the stream carries one. The H.264 VUI walk now continues through `nal_hrd`/`vcl_hrd` `hrd_parameters()` to `pic_struct_present_flag`, and the HEVC SPS walk handles inter-RPS prediction (7-61/7-62) so `field_seq_flag` is reachable on every stream (it was discarded before). Facts that the stream does not carry are `None`, never inferred. A free function rather than a `Sample` field to keep the hot IR type untouched. Spec transcriptions: `docs/codec/h264-vui-pic-timing.md`, `docs/codec/h265-vui-pic-timing.md`.
- **`LiveJoin`** (#1162 R4), new `transmux::live_join` module: a sans-IO state machine fed a video track's samples in decode order that finds the first usable random-access point (`RapKind`: IDR, BLA, CRA, H.264 `recovery_point` SEI with its `recovery_frame_cnt`/`exact_match_flag`/`broken_link_flag`, or the SPS+I fallback) and drops what precedes it (`DropReason`): samples before the point, and *leading pictures* (HEVC RASL by NAL type, otherwise `pts` below the join point). `JoinPolicy::SelfContainedOnly` refuses non-self-contained recovery points (`NotSelfContained`), and `LiveJoin::rejoin()` asks for the next acceptable point after a decoder failure or `Discontinuity`.
- `StreamingTsDemux` rustdoc documents the **chunk-size invariance** invariant (#1162 R5) and the `TrackAdded`/`TrackUpdated`/`Discontinuity` semantics, pinned by `tests/streaming_chunk_invariance.rs` (whole buffer vs 1 B, 7, 187, 188, 189, 1000, 65536 B and 32 fixed-seed random chunkings, identical event sequence and sample hashes over audio+video, discontinuity and track-update captures).

---

Published from tag `transmux-v0.25.4`.
