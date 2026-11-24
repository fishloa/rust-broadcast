# isobmff extraction: Phase 1 reconnaissance

Source: `origin/main` (fetched 2026-11-24). Design: `docs/superpowers/specs/2026-11-24-streaming-first-containers-design.md` (4.1, 5 Phase 1, 10 Q6). Read-only; nothing edited, no cargo run. `transmux` is 0.33.0, `mp4-emsg` 0.9.0, broadcast-common 14.0.0.

Conventions used below: MOVE = pure box syntax; SPLIT = box items move, bitstream/IR items stay (items named); STAY. Line numbers are in the origin/main file. "Cut" = how to remove a dependency from the move set.

## 1. Module classification (110 files under transmux/src)

Totals by lines: MOVE 16 495, SPLIT 20 123 (of which roughly 10 to 11 k lines are box syntax, mostly init_segment, movie_fragment, cenc, ac3), STAY 87 487. Unit tests inside a moved file travel with it; several in-file test modules reach other box types (box_types `largesize_tests` drives every box writer).

| file | lines | class | reason |
|---|---|---|---|
| aac_asc.rs | 1873 | STAY | bitstream: AAC AudioSpecificConfig/ADTS |
| ac3.rs | 3003 | SPLIT | Ac3SpecificBox/Ec3SpecificBox/Ec3Substream (dac3/dec3, ~L929-1445 + consts) move; Ac3SyncframeInfo/Ec3SyncframeInfo BSI parsers, CRC, splitters and Ec3SpecificBox::from_syncframes/into_dac3/into_dec3 (take bitstream info) stay |
| ac4.rs | 62 | MOVE | dac4 opaque body; deps Error only (62 lines) |
| annexb.rs | 557 | STAY | bitstream: Annex B / length-prefixed NAL framing |
| au.rs | 1019 | STAY | bitstream: access-unit splitter |
| audio_profile.rs | 342 | STAY | IR: operator HE-AAC profile overrides |
| av1.rs | 284 | MOVE | av1C + av01 entry; deps box_types, sample_entries only |
| avc_config.rs | 895 | SPLIT | AVCDecoderConfigurationRecord/AVCConfigurationBox move; deps nalu_types::{AvcSps,AvcPps,AvcSpsExt,VALID_LENGTH_SIZES}; has_high_profile_ext is pub(crate) and used by rtp_sdp |
| avc_parameter_set.rs | 149 | MOVE | avcp sample entry + parameter-set sample; one use of annexb::iter_length_prefixed_nals_with, AvcNalUnit wrapper |
| avs2.rs | 146 | MOVE | av2c/la2c config boxes; pure box syntax built on avs3's shared items |
| avs3.rs | 1304 | MOVE | av3c/lavc/AvsVideoSampleEntry/temporal-layer + library sample-group entries (sgpd payloads); deps box_types, sample_entries |
| avs3_audio.rs | 1024 | MOVE | dca3 CA3SpecificBox: bit-packed config record of the Audio Vivid sample entry (box syntax, not an audio bitstream parser); deps bitreader::write_bits_checked |
| bitreader.rs | 473 | STAY | bitstream: bit reader/writer helpers over crate Error |
| box_types.rs | 1938 | MOVE | ISO 14496-12 box/fullbox header, BoxType, parse_box, box_iter, clamped_boxes, compact_form, write_*_header helpers; tests (largesize_tests) drive many other box types |
| caption_detect.rs | 420 | STAY | bitstream: CEA-608/708 caption detection |
| cenc.rs | 2357 | SPLIT | Box syntax tenc/senc/saiz/saio/pssh/sinf/schm/frma/schi moves (L62-1217); ConstantIvSenc enum (L1235-1264, an encryptor option consumed by movie_fragment) is the only non-box item. CencScheme is already a re-export of broadcast_common::cenc |
| cenc_crypto.rs | 993 | STAY | CENC cipher engine (AES) over the IR |
| cenc_decrypt.rs | 2036 | STAY | CENC cipher engine (AES) over the IR |
| cenc_encrypt.rs | 1978 | STAY | CENC cipher engine (AES) over the IR |
| cli.rs | 1488 | STAY | packager CLI |
| container.rs | 88 | STAY | Container enum (TS/FMP4/...) |
| dash.rs | 1476 | STAY | DASH packager/MPD over the IR |
| display_track.rs | 1008 | STAY | bitstream: display geometry/AFD/interlace/colour facts from SPS/SEI |
| dovi.rs | 919 | SPLIT | DoviDecoderConfigurationRecord + dvcC/dvvC entry kinds + profile enums (L81-400) move; DoviVideoStreamDescriptor and TS descriptor scanning (L400-571; uses ts_demux::DESCRIPTOR_HEADER_LEN) stay and embed the record |
| drm.rs | 399 | STAY | PlayReady/Widevine/FairPlay pssh payload builders (only pssh box syntax moves, via cenc.rs) |
| dts.rs | 1107 | SPLIT | DtsSpecificBox (ddts) + fourcc consts (L507-700) move; DtsCoreFrameInfo (incl. into_ddts), split_dts_core_frames, sync search stay |
| error.rs | 341 | STAY | transmux::Error (30 variants); gains conversion from isobmff::Error |
| flac.rs | 298 | MOVE | dfLa + FlacMetadataBlock; deps box_types::FULLBOX_EXTRA_SIZE, Error only |
| flv.rs | 2064 | STAY | FLV demux/mux |
| flv_stream.rs | 1742 | STAY | FLV demux/mux |
| frag_offsets.rs | 815 | MOVE | frag addressing rules 14496-12 8.8.7/8.8.8 over MovieFragmentBox (sample_ranges, mdat_ranges, sample_bytes_budget, tfdt/add_offset helpers); only deps TrackId, box_types, movie_fragment. Not named in spec 4.1; MOVE candidate, confirm |
| h262.rs | 457 | STAY | bitstream: MPEG-2 video / MPEG audio headers |
| h262/tests.rs | 350 | STAY | bitstream: MPEG-2 video / MPEG audio headers |
| hdr.rs | 553 | SPLIT | ChromaticityXy/MasteringDisplayColourVolume/ContentLightLevel (mdcv/clli bodies, L62-208) are box syntax shared with the SEI payload; HdrStaticMetadata and the SEI absorb/emit code (L209+, uses nal::sei_messages, hevc_config) stay. Not named in spec 4.1: confirm ownership |
| hdr10plus.rs | 962 | STAY | bitstream: display geometry/AFD/interlace/colour facts from SPS/SEI |
| hevc_config.rs | 783 | SPLIT | HEVCDecoderConfigurationRecord/HEVCConfigurationBox move; rfc6381() (L109-130) builds sps::HevcSpsInfo and calls sps::rfc6381_hvc1, which stays |
| init_segment.rs | 5547 | SPLIT | moov tree + stbl tables + dref + audio sample entries + SampleEntryVariant + flat-table/child-walk helpers are box syntax; protect_init_segment (takes ir TrackEncryption) and the protect_sample_entry wrapper policy are the cut; lib.rs ser_timescale is needed here |
| interlace.rs | 736 | STAY | bitstream: display geometry/AFD/interlace/colour facts from SPS/SEI |
| ir/audio_meta.rs | 405 | STAY | IR: Media/Track/codec config/display/events/ids/audio metadata; refs box types as data |
| ir/codec.rs | 668 | STAY | IR: Media/Track/codec config/display/events/ids/audio metadata; refs box types as data |
| ir/display.rs | 1436 | STAY | IR: Media/Track/codec config/display/events/ids/audio metadata; refs box types as data |
| ir/event.rs | 706 | STAY | IR: Media/Track/codec config/display/events/ids/audio metadata; refs box types as data |
| ir/ids.rs | 122 | STAY | IR TrackId/ProgramNumber newtypes; Debug/serde-transparent contract (golden hashes) that isobmff::TrackId must copy |
| ir/media.rs | 164 | STAY | IR: Media/Track/codec config/display/events/ids/audio metadata; refs box types as data |
| ir/mod.rs | 82 | STAY | IR: Media/Track/codec config/display/events/ids/audio metadata; refs box types as data |
| ir/sample.rs | 644 | STAY | IR: Media/Track/codec config/display/events/ids/audio metadata; refs box types as data |
| ir/track.rs | 281 | STAY | IR: Media/Track/codec config/display/events/ids/audio metadata; refs box types as data |
| klv.rs | 500 | STAY | KLV carriage |
| lib.rs | 501 | STAY | module list + root re-exports + ser_timescale; every `pub use` of a moved item is deleted (no shim) |
| live_join.rs | 665 | STAY | live-join / splice / stitch / rebase / timeline / trickplay: IR transforms |
| ll_dash.rs | 630 | STAY | DASH packager/MPD over the IR |
| ll_dash/mpd.rs | 288 | STAY | DASH packager/MPD over the IR |
| ll_hls.rs | 1804 | STAY | HLS/LL-HLS/TS-HLS/segmenter/repackage stages over the IR |
| main.rs | 34 | STAY | packager CLI |
| media.rs | 1966 | STAY | fMP4 demux/CmafMux/HlsPackager and box-to-IR mapping (codec_config_from_entry, track_spec_from_trak) |
| mkv_mux.rs | 1600 | STAY | MKV/WebM mux and demux |
| movie_fragment.rs | 2731 | SPLIT | moof/traf/tfhd/tfdt/trun boxes (L143-1220) move; media_fragment_run/MediaRunSample (L75-142, pub(crate) builder used by pipeline, ll_dash, smooth_fragment) and protect_media_segment/FragmentProtection (L1221-1607, needs ConstantIvSenc) are the questionable tail |
| mp4esds.rs | 2385 | MOVE | esds descriptor chain (ES/DecoderConfig/DecoderSpecificInfo/SLConfig) + EsdsBox; one pub(crate) method audio_specific_config() calls aac_asc::AudioSpecificConfig::parse (bitstream) and must leave; OTI_*/STREAM_TYPE_* pub(crate) consts are used elsewhere |
| mpeg2_display.rs | 236 | STAY | bitstream: display geometry/AFD/interlace/colour facts from SPS/SEI |
| mpeg2_display/tests.rs | 538 | STAY | bitstream: display geometry/AFD/interlace/colour facts from SPS/SEI |
| mpeg_legacy.rs | 481 | STAY | bitstream: MPEG-2 video / MPEG audio headers |
| mpegh.rs | 571 | SPLIT | MHADecoderConfigurationRecord/mhaC + fourcc consts (L1-240) move; MhasPacket walker, find_mpegh3da_config and private BitReader (L241-571) stay |
| nal.rs | 1655 | STAY | bitstream: NAL/SEI/slice/SPS/RBSP parsing |
| nalu_types.rs | 338 | SPLIT | AvcSps/AvcPps/AvcSpsExt/AvcNalUnit/HevcNalUnit/HevcNalArray newtypes + VALID_LENGTH_SIZES move (wrappers move with boxes, spec 4.1); decode()/rfc6381()/decode_sps() methods call sps::decode_*/rfc6381_* and must become transmux-side extension fns |
| opus.rs | 165 | MOVE | dOps + ChannelMappingTable; deps wire_cursor, Error only |
| pipeline.rs | 1067 | STAY | neutral IR (TrackSpec/Sample/CodecConfig/DemuxEvent), builds init/media segments from the IR via the box types |
| pipeline/stsd.rs | 556 | STAY | IR track kind -> stsd entry mapping; uses box consts and SampleEntryChild |
| progressive.rs | 976 | STAY | progressive MP4 mux/demux over the IR |
| progressive_demux.rs | 1040 | STAY | progressive MP4 mux/demux over the IR |
| ps_demux.rs | 1750 | STAY | PS/TS demux, TS mux |
| rbsp.rs | 83 | STAY | bitstream: NAL/SEI/slice/SPS/RBSP parsing |
| rebase.rs | 263 | STAY | live-join / splice / stitch / rebase / timeline / trickplay: IR transforms |
| repackage.rs | 643 | STAY | HLS/LL-HLS/TS-HLS/segmenter/repackage stages over the IR |
| rtmp.rs | 389 | STAY | FLV demux/mux |
| rtp.rs | 3080 | STAY | RTP/SDP/RTCP carriage |
| rtp_align.rs | 1005 | STAY | RTP/SDP/RTCP carriage |
| rtp_dts.rs | 651 | STAY | RTP/SDP/RTCP carriage |
| rtp_sdp.rs | 748 | STAY | RTP/SDP/RTCP carriage |
| rtp_sdp/track.rs | 402 | STAY | RTP/SDP/RTCP carriage |
| rtp_stream.rs | 2264 | STAY | RTP/SDP/RTCP carriage |
| sample_aes.rs | 1058 | STAY | CENC cipher engine (AES) over the IR |
| sample_entries.rs | 1319 | MOVE | AVC/HEVC/VVC/Mp4v VisualSampleEntry + SampleEntryChild + find_config_box helpers; deps all MOVE (avc_config, hevc_config, vvc_config, nalu_types wrappers, OpaqueBox) |
| sample_groups.rs | 2474 | MOVE | prft/sgpd/sbgp/subs/ssrt; deps: TrackId (prft reference_track_id), init_segment::bounded_entry_count |
| segment_cut.rs | 92 | STAY | live-join / splice / stitch / rebase / timeline / trickplay: IR transforms |
| segmenter.rs | 665 | STAY | HLS/LL-HLS/TS-HLS/segmenter/repackage stages over the IR |
| segments.rs | 523 | MOVE | ftyp/styp/mdat + starts_with_*/insert_event_message_boxes_after_styp (opaque emsg bytes) |
| slice_header.rs | 712 | STAY | bitstream: NAL/SEI/slice/SPS/RBSP parsing |
| smooth.rs | 1186 | STAY | Smooth Streaming packager/fragments/codec glue over the IR |
| smooth_codec.rs | 764 | STAY | Smooth Streaming packager/fragments/codec glue over the IR |
| smooth_fragment.rs | 262 | STAY | Smooth Streaming packager/fragments/codec glue over the IR |
| splice.rs | 715 | STAY | live-join / splice / stitch / rebase / timeline / trickplay: IR transforms |
| sps.rs | 4376 | STAY | bitstream: NAL/SEI/slice/SPS/RBSP parsing |
| stitch.rs | 352 | STAY | live-join / splice / stitch / rebase / timeline / trickplay: IR transforms |
| subtitle_entries.rs | 978 | MOVE | 14496-30 stpp/wvtt entries + vttc/vtte/WvttSampleBox; deps init_segment::OpaqueBox, sample_entries const |
| test_bits.rs | 98 | STAY | cfg(test) bit-writer helper for in-crate tests |
| timeline.rs | 592 | STAY | live-join / splice / stitch / rebase / timeline / trickplay: IR transforms |
| timing.rs | 1270 | MOVE | stts/ctts/cslg/elst/sidx; only deps are box_types, wire_cursor, init_segment flat-table helpers |
| track_reference.rs | 132 | MOVE | tref/TrackReferenceBox; uses ir::TrackId (-> isobmff TrackId) and init_segment::TrackBox |
| trickplay.rs | 208 | STAY | live-join / splice / stitch / rebase / timeline / trickplay: IR transforms |
| ts_demux.rs | 16977 | STAY | PS/TS demux, TS mux |
| ts_hls.rs | 2056 | STAY | HLS/LL-HLS/TS-HLS/segmenter/repackage stages over the IR |
| ts_mux.rs | 2562 | STAY | PS/TS demux, TS mux |
| units.rs | 9 | STAY | two crate-private consts (BITS_PER_KILOBIT, ROUNDING_DIVISOR); visual_ext needs ROUNDING_DIVISOR |
| user_data.rs | 184 | STAY | bitstream: NAL/SEI/slice/SPS/RBSP parsing |
| validate.rs | 1133 | STAY | fMP4/CMAF conformance validator; pure box-level but not named in spec 4.1 (lives with transmux); needs only box_types, frag_offsets, init_segment, movie_fragment, segments. Possible later move |
| visual_ext.rs | 744 | MOVE | pasp/clap/colr; only units::ROUNDING_DIVISOR (1 const) outside the move set |
| vp9.rs | 485 | MOVE | vpcC + vp09 entry; deps box_types, sample_entries, wire_cursor only |
| vvc_config.rs | 1319 | SPLIT | VvcDecoderConfigurationRecord/vvcC/PTL + private VvcBitReader/Writer move; dimensions() calls sps::decode_vvc_sps and rfc6381() calls sps::rfc6381_vvc1 (stay); uses bitreader::{bit_error,bytes_needed,write_bits_checked} |
| webm_demux.rs | 2174 | STAY | MKV/WebM mux and demux |
| wire_cursor.rs | 187 | STAY | shared byte-cursor helpers (be_u16, read_u32, take, u24_at...) used by 12 STAY files and all MOVE files; decision needed on its home (see section 2) |

Cross-cutting facts that change the classification:
- `init_segment.rs` is the hub: its `SampleEntryVariant` (L3154) names `AVCSampleEntry`, `HEVCSampleEntry`, `VVCSampleEntry`, `Mp4vSampleEntry`, `AVCParameterSampleEntry`, `Av1SampleEntry`, `Vp9SampleEntry`, `XmlSubtitleSampleEntry`, `WvttSampleEntry`, and its own `Mp4a/Ac3/Ec3/Opus/Flac/Ac4/Dts/Mha` entries; `StblChild` names timing boxes; `protect_sample_entry` names cenc boxes. So init_segment, sample_entries, av1, vp9, avc_parameter_set, subtitle_entries, avs3 (entry), timing, cenc and all codec config boxes form one strongly connected group and cannot be moved piecemeal without a seam.
- `init_segment.rs` L30-480 holds the shared box helper layer used by leaf files: `bounded_entry_count`, `check_entry_count`, `parse_flat_table`, `serialize_flat_table`, `flat_table_len`, `ChildBox`, `walk_children`, `ChildOrder`, `serialize_children`, `serialize_container`, `SerializeBox`, `box_body`, `OpaqueBox` (L486). `hevc_config`, `sample_groups`, `timing`, `subtitle_entries`, `track_reference` import these from `init_segment`; they must be extracted first (see slice 1).
- `box_types.rs` has 12 `pub(crate)` helpers (`box_len`, `encode_box_header`, `write_box_header`, `write_compact_box_header`, `write_full_box_header`, `compact_form`, `box_body`, `child_extent`, `child_extent_in`, `box_header_len`, `box_slices`, `find_top_box`, `child_boxes`, `find_child_box`, `descend_boxes`) and 7 `pub(crate)` consts. STAY files that call them today: media.rs (3 sites, `find_top_box`), pipeline.rs (2), pipeline/stsd.rs (6), cenc_decrypt.rs (4), cli.rs (`KEY_ID_LEN`), dash.rs (`KEY_ID_LEN`), drm.rs, validate.rs, smooth*.rs, progressive*.rs, frag_offsets.rs. They all become `pub` in isobmff.
- mp4-emsg is not under transmux/src, but `pipeline.rs` L55 re-exports `mp4_emsg::{EmsgBox, EmsgVersion, PresentationTime}` (root and `pipeline::` paths both used by consumers).

## 2. Dependency edges that block a clean move

Edges between two MOVE files are fine and omitted. Every edge below points from the move set to a STAY item.

### 2.1 Cross-cutting (every MOVE/SPLIT file)
| blocker | files | cut |
|---|---|---|
| `crate::error::{Error, Result}` | all (also the associated type `Parse::Error`/`Serialize::Error`) | `isobmff::Error` (section 3); transmux adds a conversion. `pipeline.rs::config_box<C: Serialize<Error = crate::error::Error>>` (L187) and `dovi.rs` (descriptor embeds record) are two places the bound leaks |
| `crate::wire_cursor::*` (`be_u16`, `be_u32`, `be_u64`, `read_u8/16/32/48`, `take`, `read_nalu_16`, `u16_at`..`u64_at`, `i16_at`, `i32_at`, `fourcc_at`, `U24_LEN`) | box_types, cenc, init_segment (11 sites), movie_fragment, timing, sample_*, subtitle, visual_ext, mp4esds, avc/hevc/vvc, vp9, opus, avs3, segments | also used by 12 STAY files (ac3, dovi, klv, mpeg_legacy, mpegh, nal, ps_demux, rtp, smooth, ts_demux, user_data, webm_demux). Options: (a) isobmff exports a public `wire` module and transmux imports it (no duplicate, but a public byte-helper API); (b) a shared home in broadcast-common (checked helpers exist only as `len::need_in/need_out/fit_*`; no be_u16 family there today); (c) duplicate (rejected by the one-purpose rule). Decision needed |
| `crate::TrackId` (`ir::ids`, serde-transparent, Debug/Display print the bare number) | init_segment (`mvhd.next_track_id`, `tkhd.track_id`, `trex.track_id`), movie_fragment (`tfhd.track_id`, `FragmentProtection.track_id`, `media_fragment_run`), sample_groups (`prft.reference_track_id`), frag_offsets, track_reference, validate | isobmff::TrackId must copy `new/value/next`, Debug, Display, `serde(transparent)`, Ord/Hash (goldens hash Debug output). STAY sites that touch a box's track id and must convert: cenc_decrypt.rs 6, flv_stream.rs 12, ts_demux.rs 5, media.rs 5, pipeline.rs 5, progressive_demux.rs 3, cli.rs 2, repackage.rs 1, smooth_fragment.rs 1; consumers media-doctor `container_codec.rs:108`, multimux tests `heaac_override.rs:262,278`, `split_audio.rs:399` |
| `crate::ser_timescale` (lib.rs, serde feature) | init_segment `mvhd`/`mdhd` (L644, L1065) | one 7-line fn: put it in isobmff (serde feature) and have transmux use that copy if IR types need it |
| `crate::units::ROUNDING_DIVISOR` | visual_ext | one const; inline as a named const in isobmff |
| `crate::bitreader::{bit_error, bytes_needed, write_bits_checked}` | vvc_config, avs3_audio | these wrap `broadcast_common::bits` with the transmux Error; give isobmff a local equivalent over its own Error (both files already import `broadcast_common::bits::{BitReader, BitWriter}`) |

### 2.2 Per-file edges to STAY items
| file | STAY items used | cut |
|---|---|---|
| avc_config.rs | `nalu_types::{AvcSps, AvcPps, AvcSpsExt, VALID_LENGTH_SIZES}`; `sps::is_high_profile` (doc only); `rtp_sdp` (doc only); `has_high_profile_ext` is `pub(crate)` and called by `rtp_sdp.rs` | NAL wrapper newtypes move with the boxes (spec 4.1); make `has_high_profile_ext` pub |
| hevc_config.rs | `nalu_types::{HevcNalArray, HevcNalUnit, VALID_LENGTH_SIZES}`; `sps::HevcSpsInfo` + `sps::rfc6381_hvc1` (in `rfc6381()` L109-130); `init_segment::bounded_entry_count` | wrappers move; `rfc6381()` becomes a transmux extension fn (or `rfc6381_hvc1` string formatting moves: owner call, spec says bitstream-derived strings stay but this one is built only from record fields) |
| vvc_config.rs | `nalu_types::VALID_LENGTH_SIZES`; `sps::decode_vvc_sps` (`dimensions()` L357); `sps::rfc6381_vvc1` (`rfc6381()`); bitreader helpers | `dimensions()` and `rfc6381()` to transmux extension fns; local bit helpers |
| nalu_types.rs (SPLIT) | `sps::{AvcSpsInfo, HevcSpsInfo, decode_avc_sps, decode_hevc_sps, rfc6381_avc1, rfc6381_hvc1}`; `nal::{HEVC_NAL_HEADER_LEN, HEVC_NAL_TYPE_MASK}`; `hdr::HEVC_PREFIX_SEI_NUT` (in `has_permitted_nal_unit_type`) | `AvcSps::decode/rfc6381`, `HevcNalUnit::decode_sps/rfc6381` leave as extension traits or free fns; the two NAL consts become isobmff-local named consts |
| sample_entries.rs | none beyond cross-cutting (`nalu_types::HevcNalArray`, `vvc_config` are MOVE) | `SAMPLE_ENTRY_RESERVED_LEN`, `find_config_box`, `entry_children*`, `serialize_visual_sample_entry` are `pub(crate)` and used by av1/vp9/avs3 (MOVE) and `pipeline/stsd.rs` (STAY): make pub |
| init_segment.rs | `media::TrackEncryption` (`protect_init_segment` L4728, doc at L4721); `media::codec_config_from_entry` (doc only); `cenc_decrypt` (doc only); `dts::DtsSpecificBox` (L2741 doc/impl) | `protect_init_segment` takes `&TrackEncryption` (IR carrier of `tenc` + scheme + per-sample entries): change signature to `(scheme, &TrackEncryptionBox)` or keep the function in transmux over `isobmff::protect_sample_entry`; `protect_sample_entry` itself needs only box types and moves |
| movie_fragment.rs | `ConstantIvSenc` (cenc.rs L1235, an encryptor option; used in `build_cenc_fragment_boxes` L1343); `media::TrackEncryption::samples` (doc only) | move `ConstantIvSenc` with `protect_media_segment`, or leave `protect_media_segment`/`FragmentProtection` in transmux. `media_fragment_run`/`MediaRunSample` are `pub(crate)` and called from pipeline, ll_dash, smooth_fragment: either pub in isobmff or stay in transmux (they take only duration/size/flags) |
| frag_offsets.rs | `cenc_decrypt::CencDecryptor`, `media::Fmp4Demux` (doc only); transmux `error.rs` and `movie_fragment.rs` quote `MAX_SAMPLE_BYTES_PER_INPUT_LENGTH` | none beyond TrackId/Error; the two budget errors (`SampleDataBudgetExceeded`, `ZeroWidthTrunBudgetExceeded`) become isobmff variants |
| cenc.rs | `media::TrackEncryption`, `cenc_decrypt::CencDecryptor`, `cenc_encrypt::CencEncryptor/IvGen` (docs only); `ConstantIvSenc` item | move `ConstantIvSenc` along (it is consumed by movie_fragment) or relocate it to `cenc_encrypt` and cut the movie_fragment use |
| mp4esds.rs | `aac_asc::AudioSpecificConfig::parse` in `audio_specific_config()` (L1130, `pub(crate)`) | delete from the box; callers do `AudioSpecificConfig::parse(esds.require_decoder_specific_info()?)` (make that accessor pub). `OTI_MPEG4_AUDIO/OTI_MPEG2_AUDIO/OTI_MPEG1_AUDIO/STREAM_TYPE_AUDIO` are `pub(crate)`; `flv.rs` (L66, L1030) uses `OTI_MPEG4_AUDIO` and `ts_demux`/`h262` tests build esds chains: make pub (the ordering guard also pins the OTI consts, section 8) |
| track_reference.rs | `ir::TrackId`; `init_segment::TrackBox` | own TrackId |
| avc_parameter_set.rs | `annexb::iter_length_prefixed_nals_with` (L14); `nalu_types::{AvcNalUnit, VALID_LENGTH_SIZES}` | the one NAL-length iterator is small: implement the length-prefixed walk locally or keep AVCParameterSample parse over a caller-supplied iterator |
| sample_groups.rs | `TrackId`; `init_segment::bounded_entry_count` | own TrackId; helper extracted in slice 1 |
| timing.rs, segments.rs, subtitle_entries.rs, visual_ext.rs | only `init_segment` helpers / `OpaqueBox`, `units`, `wire_cursor`, Error | covered by 2.1 |
| av1.rs, vp9.rs, avs3.rs, avs2.rs, opus.rs, flac.rs, ac4.rs | none beyond cross-cutting | `avs3::parse_sequence_header_record` is `pub(crate)` and used by avs2 (both MOVE) |
| ac3.rs (SPLIT) | box half uses nothing from the bitstream half except `Ec3SpecificBox::from_syncframes(&[Ec3SyncframeInfo])` (L1135) and `Ac3SyncframeInfo::into_dac3` / `Ec3SyncframeInfo::into_dec3` | those three constructors stay in transmux (free functions over public box fields); the rest of the box half needs `bitreader::read_bits_checked` (used by `parse_eac3_bsi_tail`, which is part of from_syncframes, stays), `units::BITS_PER_KILOBIT` (`nominal_bitrate_kbps`: inline const) |
| dts.rs (SPLIT) | `ac3::split_ac3_syncframes` (frame splitter), `bitreader::read_bits_checked`, `init_segment::DtsSampleEntry` (doc) | box half has none; `DtsCoreFrameInfo::into_ddts` stays and constructs the box via its pub fields |
| mpegh.rs (SPLIT) | own private BitReader/MhasPacket | box half has only `wire_cursor::be_u16` |
| dovi.rs (SPLIT) | `ts_demux::DESCRIPTOR_HEADER_LEN` (descriptor half only) | record and boxes move; descriptor half stays and imports the record |
| hdr.rs (SPLIT) | `hevc_config::HEVCDecoderConfigurationRecord`, `nal::{HEVC_NAL_HEADER_LEN, hevc_prefix_sei_nal, nal_unit_type, NalCodec, sei_messages}` (all in `HdrStaticMetadata`) | box-body types have none; the HDR carrier stays |

## 3. Error type

`transmux::Error` (`transmux/src/error.rs`, 341 lines): 30 variants, `#[non_exhaustive]`, `Debug + PartialEq + Clone` (no `Eq`: wraps `broadcast_hls::Error`), thiserror. Variants: BufferTooShort, LargesizeBufferTooShort, UuidBufferTooShort, BoxSizeUnderflow, OutputBufferTooSmall, InvalidValue, Wrmheader (std), ManifestWrite (std), Klv, InvalidField, UnexpectedBox, InvalidInput(&'static str), TimestampSpanOverflow, CompositionOffsetRange, ZeroWidthTrunBudgetExceeded, SampleDataBudgetExceeded, UnsupportedCodec, TooManyElementaryStreams, UnsupportedCencScheme, UnsupportedMkvCodec, Ps, EmsgSerialize, UnsupportedSampleEntry, UnmuxableDataTrack, UnmuxableSubtitleTrack, BufferCapExceeded, FieldOverflow, UnsupportedFeature, HlsAttrValue, TrickModeMainAdaptationSetNotFound.

Variants constructed in the MOVE/SPLIT box code (non-test, counted by `Error::X` literals; `?` conversions from broadcast-common kinds not counted):
| variant | where |
|---|---|
| BufferTooShort | box_types 2, cenc 3, init_segment 5, movie_fragment 3, sample_entries 6, sample_groups 3, subtitle_entries 3, mp4esds 9, vvc_config 3, av1, vp9 2, flac, ac3 2, segments 2, timing, track_reference, hdr, avs3 4, avs3_audio 2, frag_offsets |
| InvalidValue | box_types 3, cenc, init_segment, sample_groups 8, subtitle_entries 4, visual_ext, mp4esds 10, avc_config 3, hevc_config, vvc_config 4, av1, vp9 3, flac, ac3 5, dts 4, mpegh, dovi, timing 5, track_reference, hdr, avs3 |
| InvalidInput | movie_fragment 17, frag_offsets 12, init_segment 9, cenc 8, sample_groups 2, av1, vp9, ac3, dts, avs3, avs3_audio 3 (about 56 sites; no broadcast-common kind fits a free-text `&'static str`) |
| UnexpectedBox | init_segment 4, mp4esds, movie_fragment, segments 2, avs3 2 |
| LargesizeBufferTooShort 2, UuidBufferTooShort 1, BoxSizeUnderflow 2 | box_types only |
| FieldOverflow | mp4esds, timing (drm too, STAY) |
| OutputBufferTooSmall | avs3 (the `need_out` kind arrives via `?` everywhere) |
| ZeroWidthTrunBudgetExceeded | movie_fragment |
| SampleDataBudgetExceeded | frag_offsets (4) |
| UnsupportedFeature | cenc (1) |
Not constructed by MOVE code: Wrmheader/ManifestWrite (drm, STAY), Klv, UnsupportedCodec/UnsupportedCencScheme/UnsupportedMkvCodec/UnsupportedSampleEntry/Unmuxable*/TooMany*/BufferCapExceeded/Ps/HlsAttr*/TrickMode*/TimestampSpanOverflow/CompositionOffsetRange (IR, demux, packagers).

broadcast-common (`broadcast-common/src/error.rs`, "common failure kinds defined once"): ten plain Copy structs, `Debug+Clone+Copy+PartialEq+Eq`, no_std, each wrapped by a `#[error(transparent)] X(#[from] ...)` variant per crate: `BufferTooShort{need,have,what}`, `OutputBufferTooSmall{need,have}`, `FieldOverflow{field,value,max}`, `CrcMismatch`, `InvalidSync`, `ReservedBits`, `InvalidValue{field,value,reason}`, `InvalidField{field,reason}`, `UnsupportedVersion{what,found}`, `LengthMismatch`. These cover BufferTooShort, OutputBufferTooSmall, InvalidValue, FieldOverflow, and mp4-emsg's UnsupportedVersion. They do NOT cover InvalidInput(&'static str), UnexpectedBox, the three box-header variants, the two budget variants, UnsupportedFeature, or emsg's NotEmsg/InvalidSize/InvalidString (InvalidField could carry InvalidString).

Numbers that decide the conversion design: `Error::` variant literals in transmux (src+tests): InvalidInput 497, InvalidValue 274, BufferTooShort 248, UnexpectedBox 58, FieldOverflow 52, SampleDataBudgetExceeded 13, UnsupportedFeature 7, BoxSizeUnderflow 6, LargesizeBufferTooShort 5, ZeroWidthTrunBudgetExceeded 3, UuidBufferTooShort 3. Tests matching flat variants: transmux/tests/it/error_variants.rs 22, cenc_encrypt.rs 19, timing_groups.rs 16 and others. If transmux wraps `isobmff::Error` in a single `Isobmff(#[from])` variant, every such match and every `Err(Error::InvalidValue(..))` assert for a box failure changes shape; a hand-written `From<isobmff::Error> for Error` that maps variant-for-variant keeps them. External matches: dash-runtime `client/engine.rs:2435` (builds `transmux::Error::InvalidValue`), media-plane `ts_session.rs:1830` (`transmux::Error::LargesizeBufferTooShort`, a box variant), hls-runtime `server/engine.rs:2130,2139` (`UnsupportedCodec`, stays).

## 4. mp4-emsg

Files: `mp4-emsg/src/{lib.rs 26, emsg.rs 861, error.rs 50, version.rs 86}`, tests `tests/it/{fixture_scte35, fixture_v1_scte35_livesim, short_buffers, trait_round_trip}.rs`, examples `build_emsg.rs`, `parse_emsg.rs`, README, CHANGELOG. `no_std`+alloc, features `std` (default), `serde`.

Public API: `EmsgBox<'a>` (pub fields `flags: u32, scheme_id_uri: &str, value: &str, timescale: Timescale, presentation_time: PresentationTime, event_duration: u32, id: u32, message_data: &[u8]`; methods `version()`, `is_scte35()`, `serialized_len()`, `parse(&'a [u8])` inherent, `serialize_into(&mut [u8])`, `to_vec()`; `impl broadcast_common::Serialize`); `PresentationTime {Delta(u32), Absolute(u64)}` (`#[non_exhaustive]`, `version()`, `name()`, `impl_spec_display!`); `EmsgVersion {SegmentRelative, RepresentationRelative}` (`#[non_exhaustive]`, `from_u8/to_u8`); consts `EMSG_BOX_TYPE`, `EMSG_FLAGS`, `FULLBOX_HEADER_LEN`, `STRING_TERMINATOR`, `SCTE35_SCHEME_PREFIX`, `VERSION_0`, `VERSION_1`; `Error {BufferTooShort, OutputBufferTooSmall, NotEmsg{found}, InvalidSize{size,reason}, UnsupportedVersion, InvalidString{field,reason}, InvalidValue, FieldOverflow}` (`Eq`, non_exhaustive); `Result`.
Dependencies: broadcast-common 14 (no_std), thiserror 2, serde (optional); dev: test-fixtures. emsg.rs re-implements `read_u32/read_u64/read_u24_at` locally (L444-470): drops out when it uses the shared wire helpers.

Dependents (Cargo.toml):
| crate | kind | uses |
|---|---|---|
| transmux | regular | `pipeline.rs:55` re-export of `EmsgBox/EmsgVersion/PresentationTime`; `error.rs:206` `EmsgSerialize(#[from] mp4_emsg::Error)`; `tests/it/segment_emsg.rs` (`EMSG_BOX_TYPE, EmsgBox, EmsgVersion, PresentationTime`); `dash.rs:52` doc |
| timed-metadata | regular, features `std`,`serde` forward `mp4-emsg/std`,`mp4-emsg/serde` | `src/convert/emsg.rs`, `emsg_convert.rs` (6 `EMSG_FLAGS`), `src/error.rs:21` `Emsg(#[from] mp4_emsg::Error)` (public error variant: a signature change is a major-class bump of timed-metadata), tests `emsg_convert`, `emsg_interop`, `error_variants`, example `scte35_to_dash.rs` |
| ssai-runtime | dev | examples `condition_real_cue.rs`, `session_playlist.rs`, test `real_cue_conditioning.rs` |
| playout-runtime | dev | example `emit_splice_from_real_cue.rs`, test `real_fixture_transition.rs` |
| mpeg-ts | dev | test `tests/it/pusi_emsg.rs` (parses reassembled bytes with `EmsgBox::parse`); mpeg-ts is no_std and low in the graph: a dev-dep on isobmff there must not create a cycle (isobmff depends only on broadcast-common) |
| fuzz | `fuzz/Cargo.toml:75` | targets `emsg.rs`, `timed_metadata_convert.rs` |
Doc/other mentions to update: `multimux/src/origin/resource.rs:459` (comment), `media-plane/src/trunk.rs:384` (comment), `ssai-runtime/src/splice.rs:25`, `broadcast-common/src/{lib.rs:15, clock33.rs:16}` (docs), root `Cargo.toml:2` members, `.github/workflows/ci.yml` L409/427/661 (crate lists), `.github/workflows/release-mp4-emsg.yml` (tag `mp4-emsg-v*`), `release-playout-runtime.yml`, `release-ssai-runtime.yml`, `release-timed-metadata.yml` (mention the dep), `tools/prepare-release.py:264`, `tools/tests/test_*` fixtures (strings only), `fixtures/scte35/PROVENANCE.md:116-124` (`cargo run -p mp4-emsg --example`), workspace-guards tables (label, lint_suppressions, non_exhaustive, truncation each have an `mp4-emsg` row), `CLAUDE.md` crate list. History moves to a new `mp4-emsg` folder beside `docs/history/rtcp-packet/` (as `rtcp-packet` did).

## 5. Hand-rolled box walkers elsewhere

| file (lines) | what it hand-rolls | calls transmux today? | crate deps |
|---|---|---|---|
| container-probe/src/isobmff.rs (574) | header decode (u32 size, fourcc, largesize when size==1, to-EOF 0), size-chain walk to score `STRUCTURAL`; literals ftyp, styp, moov, moof, `!mov`, `!mof`, `!six`, `!ssx`, skip, free, mdat; brand at offset 8; consts `BOX_HEADER_MIN_SIZE`, `SIZE_TO_EOF`, `SIZE_INDICATES_LARGESIZE` re-declared; module doc cites `transmux/src/box_types.rs` | no | `broadcast-common` only at runtime; dev: mpeg-ts, mpeg-ps, st377-1, test-alloc, test-fixtures (drift guard) |
| container-probe/src/segment.rs (383) | `next_box()` (L123) re-implements header+largesize+uuid usertype walk; literals mdat, uuid, moof, styp, msdh, moov, sidx, prft, free, emsg | no | same |
| dash-runtime/src/ingest/session.rs (742) | already calls `transmux::parse_box` (L375, L404, L507) and `bx.header.box_type.is(b"mdat")`; scans for `styp` brands `lmsg`/`slat` by offset (`STYP_BRANDS_OFFSET`, `BRAND_LEN`) | yes (parse_box, Fmp4Demux, TrackSpec, TrackId, DashPackager in tests) | transmux (std) |
| dvr-runtime/src/recover.rs (314) | `Fmp4Scan` header-at-a-time scan with local `MOOV`, `EMSG`, `PRFT` four-CC consts, `BOX_HEADER_MAX`; already imports `transmux::box_types::{BOX_HEADER_MIN_SIZE, BoxHeader, LARGESIZE_SIZE, SIZE_TO_EOF}` and `transmux::segments::{FTYP, MDAT, MOOF, STYP}` | yes (partly; the 3 literals are hand-rolled) | transmux (std) |
| hls-runtime/src/client/engine.rs (4616) | `has_movie_box` (L148) loops `transmux::box_iter` and compares to `MOVIE_BOX` const | yes (`box_iter`) | transmux (`sample-aes`; std via feature) |
| hls-runtime/src/server/engine.rs (6410) | `first_moof` (L845) uses `transmux::box_types::clamped_boxes` + `MovieFragmentBox::parse_body`; L933 filters `b"mdat"`; also `frag_offsets::{mdat_ranges, sample_ranges_in}` | yes | transmux |
| media-doctor/src/container_codec.rs (553) | `clamped_boxes(..).find(.. is(b"moov"))` + `MovieBox::parse`; L168 literal allowlist ftyp/styp/moov/moof/`!mov`/`!mof`/free/skip as the compressed-box check | yes | transmux (default-features off; dev `cli`) |
| media-plane/src/segmenter/program.rs (5473) | non-test code has no box walking; only tests do (`STYP_BYTES`, `MOOF_BYTES` L2571-2572; a `tfdt` search and size decode L3588-3600) | tests only via live types | transmux optional (feature `std`) |
| test-harness/src/fmp4_split.rs (56) | full `top_level_boxes` (size32, largesize, to-EOF, 8/16-byte header consts) then `split()` into init + `moof`+`mdat` segments | no (defines own consts) | transmux (regular dep, no features) |
Extra hand-rolled walkers inside transmux tests: `transmux/tests/it/support/box_iter.rs`, `support/box_walk.rs` (test helpers), and many `u32::from_be_bytes` header decodes in `media-plane` test code.
Constraint check: container-probe must stay on broadcast-common only. Its box walk can therefore not move onto `isobmff` as a runtime dependency without breaking that statement; the practical move is to keep its two small walkers and strengthen the existing dev-only drift guard (add `isobmff` as a dev-dependency and pin its consts), as it does for mpeg-ts today. That is a design decision for the owner; the spec says "the seven hand-rolled walkers move onto it" but lists none by name and container-probe was apparently counted. test-harness has no isobmff reason to depend on transmux after the move (only fmp4_split is box-level; check its other uses).

## 6. Consumers of items that will move

Method: parsed every `use transmux::...` (with braces, nested, multi-line) and every inline `transmux::path` in all `*.rs` outside transmux on origin/main, mapped each leaf to its module via lib.rs re-exports. Counts are distinct `transmux::` paths and files; "moved" counts only items from MOVE/SPLIT files (box + emsg). Only Rust sources scanned; Cargo.toml-only mentions (broadcast-hls, broadcast-common, caption-convert, dvb-mabr, srt-runtime, ttml-subtitle, tools/differential-sweep) are dev/doc or stay-only.

| crate (dirs) | distinct paths / files with `transmux::` | files touching moved items | moved items |
|---|---|---|---|
| multimux (src, tests) | 184 / 93 | 18: src/origin/resource.rs, output/whep.rs, push/egress.rs, push/rtmp.rs, source/hls_pull.rs, source/segment_tests.rs, source/whip.rs; tests emsg_mp4box, glass_to_glass, hdr10_route, heaac_override, origin_iframe, push_rtsp_bounds, smooth_output, split_audio, subtitle_webvtt, whep_sdp_golden, whip_whep_sdp_documents | `AVCConfigurationBox`, `AVCDecoderConfigurationRecord`, `DecoderConfigDescriptor`, `DecoderSpecificInfo`, `ESDescriptor`, `EsdsBox`, `SLConfigDescriptor`, `EmsgBox`, `PresentationTime`, `MovieBox`, `MovieFragmentBox` (+`parse_body`), `TrackFragmentHeaderBox`, `XmlSubtitleSampleEntry`, `box_types::{BoxRef, BoxType, box_iter, clamped_boxes}`, `frag_offsets::{mdat_ranges, sample_ranges_in}`, `insert_event_message_boxes_after_styp`; split: `AvcPps`, `AvcSps`, `hdr::HEVC_PREFIX_SEI_NUT` |
| hls-runtime (src, tests) | 89 / 31 | 8: src/client/engine.rs, src/server/engine.rs; tests iframe_master, iframe_playlist, origin_codecs, resolution, subtitle_renditions, support/dummy_avc_config | `AVCConfigurationBox`, `AVCDecoderConfigurationRecord`, `box_iter`, `BoxHeader::parse`, `clamped_boxes`, `frag_offsets::{mdat_ranges, sample_ranges_in}`, `MovieFragmentBox` (+`parse_body`); split: `AvcPps`, `AvcSps`, `DoviBlSignalCompatibilityId`, `DoviDecoderConfigurationRecord`, `dovi::DOVI_RESERVED_ZERO` |
| dash-runtime (src, tests, examples) | 28 / 20 | 8: src/address/{base_index, plan, representation}.rs, src/ingest/session.rs; tests address_rules, dashif_ingest_session, vectors; examples/list_segments.rs | `SegmentIndexBox`, `timing::SidxReference`, `parse_box`, `segments::SegmentTypeBox` |
| media-plane (src) | 110 / 19 | 2: src/file_source.rs, src/segmenter/program.rs | `Av1ConfigurationBox`, `HEVCConfigurationBox`, `HEVCDecoderConfigurationRecord`, `VvcConfigurationBox`, `VvcDecoderConfigurationRecord`, `avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord}` |
| smooth-runtime (src) | 43 / 9 | 1: src/server/engine.rs | `AVCConfigurationBox`, `DecoderConfigDescriptor`, `DecoderSpecificInfo`, `ESDescriptor`, `EsdsBox`, `SLConfigDescriptor`, `subtitle_entries::XmlSubtitleSampleEntry::new` |
| dvr-runtime (src) | 18 / 11 | 1: src/recover.rs | `box_types::{BOX_HEADER_MIN_SIZE, BoxHeader, LARGESIZE_SIZE, SIZE_TO_EOF}`, `segments::{FTYP, MDAT, MOOF, STYP}` (its other 8 paths stay) |
| media-doctor (src) | 39 / 11 | 1: src/container_codec.rs | `AVCDecoderConfigurationRecord`, `HEVCDecoderConfigurationRecord`, `box_types::clamped_boxes`, `init_segment::{MovieBox, SampleEntryVariant, StblChild}` |
| broadcast-client (tests) | 69 / 62 | 1: tests/it/rtsp_playout.rs | `AVCConfigurationBox` |
| fuzz (fuzz_targets) | 30 / 11 | 2: `transmux_cenc_boxes.rs`, `transmux_sample_entries.rs` | `SampleEncryptionBox`, `SampleAuxInfoSizesBox`, `SampleAuxInfoOffsetsBox`, `ProtectionSchemeInfoBox`, `SchemeInformationBox`, `SampleDescriptionBox`, `TrackExtendsBox` (plus `emsg.rs` and `timed_metadata_convert.rs` use `mp4_emsg`) |
| demo (src) | 20 / 2 | 1: demo/src/mp4.rs | `box_iter`, `mp4esds::EsdsBox`; split: `DtsSpecificBox` |
| bindings (bindings/transmux-py/src/lib.rs; not a workspace member, consumes published transmux) | 15 / 1 | 1 | `EsdsBox`; split: `dts::DtsSpecificBox` (post-publish follow-up). `bindings/python` (dvb-si-py) has no transmux use in Rust source |
| container-probe, caption-convert, compliance-probe, rtmp-runtime, rtp-packet, st12-1, test-harness, tools | 2 / 1, 2 / 1, 2 / 1, 1 / 1, 2 / 2, 1 / 1, 16 / 3, 12 / 1 | 0 | none (container-probe/rtp-packet/st12-1/caption-convert/compliance-probe/rtmp-runtime mention transmux in docs or stay-only paths) |

Additional external edges: `media-plane/src/ts_session.rs:1830` builds `transmux::Error::LargesizeBufferTooShort` (box_types variant). Consumers that read `tkhd.track_id`/`tfhd.track_id` hold isobmff's TrackId after the move: media-doctor `container_codec.rs:108`, multimux tests `heaac_override.rs`, `split_audio.rs`. dash-runtime/hls-runtime/media-doctor/dvr-runtime would add `isobmff` next to `transmux` (none can drop transmux: each still uses the IR).

## 7. Tests, fuzz, examples, benches

`transmux/tests`: 230 files (top level: alloc_counts_sweep, alloc_measurement, cenc_senc_alloc_bound, golden_gate, hostile_count_sweep, hostile_input_bounds, review_trun_aggregate_alloc, trun_zero_width_alloc_bound; the rest under `tests/it/`). Class legend: **box** = exercises only MOVE items (move with the crate); **box-after-cut** = also uses only `Error` and/or `TrackId` outside the move set, so it moves once those are isobmff's; **split** = needs bitstream half; **ir** = needs the IR (stay). Classification came from an import scan, then spot-checked.

| class | files |
|---|---|
| box | tests/it/visual_ext.rs, timing.rs, esds_config.rs, codec_config_boxes.rs, hevc_array_rules.rs, captions.rs (vttc/wvtt boxes), cenc.rs, largesize_truncation.rs, sample_bytes_budget.rs (frag_offsets), avc_hvc_config.rs (uses `AvcSps::decode`: becomes split) |
| box-after-cut | tests/it/init_segment.rs, init_segment_sweep.rs, container_child_order.rs, uuid_child_roundtrip.rs (uses Fmp4Demux once: check), box_walk_untrusted_sizes.rs, cenc_saiz_stsz_v1_boxes.rs, serializer_length_truncation.rs (moves its box half; transmux keeps `transmux`-type box-range tests), timing_groups.rs, trun_budget.rs, avc_parameter_set_stream.rs, bit_shortfall_units.rs, tests/cenc_senc_alloc_bound.rs, tests/review_trun_aggregate_alloc.rs (Fmp4Demux: ir), tests/trun_zero_width_alloc_bound.rs (Fmp4Demux: ir), hostile_input_bounds.rs (ProgressiveDemux: ir) |
| split (box + bitstream) | ac3_syncframe_ranges.rs, eac3_bsi_walk.rs, eac3_real_*.rs (3), eac3_crc2_real_capture.rs, dolby.rs, dts.rs, dovi.rs, mpegh.rs, hdr10.rs, codec_config.rs (uses `AvcSpsInfo`/`sps`), codecs_new.rs |
| ir (stay) | everything that touches `Media`/`Track`/`TrackSpec`/`Sample`/`CodecConfig`/demuxers/muxers/packagers/CLI: golden*.rs, hub.rs, pipeline.rs, validate.rs (validator stays), cenc_decrypt*.rs, cenc_encrypt*.rs, cenc_mux.rs, fmp4_*.rs, ts_*.rs, dash*.rs, ll_*.rs, smooth*.rs, rtp*.rs, flv*.rs, webm*.rs, ps_*.rs, subtitle_*.rs, display_*.rs, hdr10plus.rs, etc. About 130 files import no moved item at all; 94 mix box items with IR |
| test support | tests/it/common/mod.rs, support/box_iter.rs, support/box_walk.rs (hand-rolled box walkers: replace with isobmff), hub_fixtures.rs (mixed) |

Fuzz (workspace `/fuzz`, not under transmux): moves with the box code: `fuzz_targets/emsg.rs` (`mp4_emsg::EmsgBox`), `transmux_cenc_boxes.rs` (senc/saiz/saio `parse_body`), `transmux_sample_entries.rs` (stsd entries, both header forms). Stay: `transmux_ts_demux*.rs`, `transmux_flv_stream.rs`, `transmux_webm_demux.rs`, `transmux_progressive_demux.rs`, `transmux_interlace_join.rs`, `transmux_slice_header.rs`; `timed_metadata_convert.rs` only re-points `EmsgBox`. `fuzz/Cargo.toml:75` has the `mp4-emsg` path dep. There is no `transmux/fuzz` directory.
Examples (`transmux/examples`): cenc_encrypt.rs, ssai_ad_stitch/{main,demo}.rs, transmux_any_to_any.rs, transmux_hub.rs: all IR, stay. `mp4-emsg/examples/{build_emsg,parse_emsg}.rs` move with emsg. New crate needs at least 2 examples (CRATE-ACCEPTANCE).
Benches: only `transmux/benches/annexb_scan.rs` (bitstream, stays).
Moves with the crate besides tests: golden files? `transmux/tests/it/golden*.rs` hash `Debug` of the IR (incl. `TrackEncryptionBox`, `SampleEncryptionEntry`): moving types between crates does not change derive(Debug) output, but isobmff::TrackId must print identically.

## 8. Guard tables and hard-coded paths

Hard-coded `transmux/src/...` paths outside transmux (excluding docs/): only
- `broadcast-common/tests/it/ordering_guard.rs:465` `"transmux/src/mp4esds.rs"` with `const OTI_MPEG4_AUDIO` .. `const OTI_MPEG2_AUDIO` (a MOVE file): update to `isobmff/src/<new path>`; the same table also pins `transmux/src/{ts_demux,ts_mux,au,webm_demux}.rs` (STAY, untouched). The guard skips silently if the sibling path is missing, so a stale path would pass without checking: update it deliberately.
- Doc/comment-only mentions to refresh: `container-probe/src/isobmff.rs:4` (cites `transmux/src/box_types.rs`), `fixtures/mp4/PROVENANCE.md:176` (`transmux/src/vp9.rs::...` test name), `fixtures/scte35/PROVENANCE.md:274` (avc_config.rs `has_high_profile_ext`), `:291` (init_segment.rs stsd arm), `RUST_AUDIT_REPORT.md:64` (init_segment.rs), `multimux/docs/mutants_dash_pull.py:96-98` (mutants edit `transmux/src/media.rs`, STAY), `bindings/transmux-py/src/lib.rs:15`.

`workspace-guards/tests/guards/*_data.rs` rows keyed by crate name or crate-relative `src/` paths (no entry names a MOVE file by path except as noted):
| table | transmux row | affects the move |
|---|---|---|
| `truncation_data.rs:188` | allowlist `src/annexb.rs` (1), `src/h262.rs` (7) | both STAY; new `isobmff` row needed (mirror mp4-emsg's row at L116 and transmux's `tests/it/serializer_length_truncation.rs` lexical half, which `truncation` covers) |
| `label_data.rs:1734` | `scope: Crate`, skip list includes moved enums `DataEntry` (init_segment), `SubsV2Source` (sample_groups), `WvttSampleBox` (subtitle_entries); `Error` | split skip list: those three move to a new `isobmff` row; `mp4-emsg` row at L1067 folds in; every moved public enum must keep `name()` + `impl_spec_display!` |
| `non_exhaustive_data.rs:179` | `runtime("transmux", [AVCDecoderConfigurationRecord, DoviDecoderConfigurationRecord, HEVCDecoderConfigurationRecord, MHADecoderConfigurationRecord, VvcDecoderConfigurationRecord, VvcPtlRecord], [])` (also comment naming `NclxColourInfo`, `DecoderSpecificInfo`) | all six are moved types: expected set moves to an `isobmff` row; `plain("mp4-emsg")` at L142 folds in |
| `no_dom_data.rs:88` | `allowed_types: XmlSubtitleSampleEntry` (an ISO BMFF `stpp` box, not XML parsing) | the allowance follows the type if isobmff is in the no_dom table; otherwise add isobmff to the table without XML (it does not touch XML) |
| `lint_suppressions_data.rs:46` | `tests/it/key_hygiene.rs` allowance (cenc_encrypt, STAY); `plain("mp4-emsg")` at L31 | add `plain("isobmff")`, drop `plain("mp4-emsg")` |
| `handroll_data.rs:35` | shape 1 `Dehandroll{ name: "transmux", allow: [] }` (`broadcast-common`, `broadcast-auth`, `scte35-splice`, `timed-metadata`, `transmux` use shape 1) | add `isobmff` with `allow: []` if it qualifies; also the shape-3 needle `thread::sleep`/base64 etc. do not apply |
| `clock33_data.rs:60` | `transmux` `BANNED_FULL` | MOVE code has no 2^33 use; isobmff not needed unless it handles timestamps (`sidx`/`tfdt` are 32/64-bit) |
| `secret_debug_data.rs:82` | allow `src/klv.rs` `KlvItem` | STAY, no change; check `tenc` default KID and `Debug` on `TrackEncryptionBox` under the secret_debug rule when it moves (isobmff needs a row if the rule matches `key`-named fields) |
| `dep_policy_data.rs` | `PURPOSES`/`BANNED` list every direct dep of every workspace member | `isobmff` must be listed with deps: broadcast-common, thiserror, serde (optional); transmux gains `isobmff` and drops `mp4-emsg` |
| `panic_idioms_data.rs`, `serialize_mask_data.rs`, `spawn_sites_data.rs`, `deadlines_data.rs`, `tls_provider_data.rs`, `test_module_reachability.rs` | no transmux or mp4-emsg row found by grep | `table_covers_every_workspace_member` makes a new crate fail unless added to or exempted from each table that has one: re-run it with isobmff to find out which |
| in-flight | branch `guard/magic-numbers` (current checkout, commits `a4cf72b6f`, `9038b2e35`, `76090b2b2`) adds a magic-number guard not on origin/main; if it merges first, its per-crate table gets transmux entries for moved files and an isobmff row | order the merges |

## 9. Proposed cut order

All slices build the whole workspace, no re-export shim. A slice that moves a file updates every importer (transmux internals plus the consumers in section 6) in the same PR.

**Slice 1: in-place seams, no new crate (transmux only).** Pure refactor inside transmux so each later move is a file move.
1. Extract the init_segment helper layer (L30-480 + `OpaqueBox` L486) to its own module; repoint hevc_config, sample_groups, timing, subtitle_entries, track_reference.
2. Split mixed modules into box and bitstream files in place: `ac3` (box half vs BSI/splitters + the 3 constructors as free fns), `dts`, `mpegh`, `dovi`, `hdr` (mdcv/clli types), `nalu_types` (wrappers vs `decode`/`rfc6381` extension fns), `mp4esds::audio_specific_config`, `hevc_config::rfc6381`, `vvc_config::dimensions/rfc6381`, `ConstantIvSenc` next to `movie_fragment` consumer, `init_segment::protect_init_segment` signature (no `TrackEncryption`).
3. Make the `pub(crate)` helpers named in section 1/2 `pub`. Gate: byte-identical goldens, existing tests unchanged.

**Slice 2: crate skeleton + leaf box layer.** Create `isobmff` (broadcast-common only): `Error` (section 3 design), `TrackId`, wire helpers (decision 2.1), `box_types`, helper layer from slice 1, `segments`, `timing`, `visual_ext`, `cenc` (all boxes), `sample_groups`, `frag_offsets`, `movie_fragment`. Importers: transmux internals, dvr-runtime (box_types/segments), dash-runtime (parse_box, SegmentTypeBox, SegmentIndexBox), hls-runtime, multimux, media-doctor, fuzz `transmux_cenc_boxes`. Tests: box + box-after-cut tests for these modules, with `workspace-guards` rows and the ordering_guard path. Risk: Error shape (flat mapping vs wrapper).

**Slice 3: emsg absorption.** Move `mp4-emsg` into isobmff (it only needs broadcast-common and the new wire helpers); fold `emsg::Error` into `isobmff::Error`; repoint transmux (`pipeline.rs` re-export deleted, `EmsgSerialize`), timed-metadata (public `Error::Emsg` changes: major-class), ssai-runtime/playout-runtime/mpeg-ts dev-deps, fuzz, CI lists, release workflow, a new `mp4-emsg` folder beside `docs/history/rtcp-packet/`. Independent of slices 4-5, so it can go before or after, but needs slice 2's crate. Needs the owner's release sequencing (mp4-emsg is live on crates.io; deleting it means a last mp4-emsg release or a yank-free deprecation: not decided in the spec).

**Slice 4: codec configuration boxes.** `avc_config`, `hevc_config`, `vvc_config`, NAL wrappers from `nalu_types`, `av1` (av1C only first; entry with slice 5), `vp9` (vpcC), `opus`, `flac`, `ac4`, `ac3` box half, `dts` box half, `mpegh` box half, `dovi` record, `hdr` mdcv/clli types, `mp4esds`, `avs2`, `avs3`, `avs3_audio`. Needs `sample_entries` helpers in slice 5 for av1/vp9/avs3 *entries*, so those three move whole in slice 5, config boxes only here. Importers: rtp_sdp (`has_high_profile_ext`), nal/sps/hdr (wrapper imports), media-plane, smooth-runtime, multimux, hls-runtime (Dovi), media-doctor, demo, bindings (post-publish).

**Slice 5: sample entries and the moov tree.** `sample_entries`, `subtitle_entries`, `avc_parameter_set`, `track_reference`, `init_segment` (rest), `av1`/`vp9`/`avs3` entries. Cluster must move together (SampleEntryVariant). Updates `pipeline.rs`, `pipeline/stsd.rs`, `media.rs`, `progressive*.rs`, `validate.rs`, `smooth*.rs`, media-doctor, multimux, fuzz `transmux_sample_entries`. Biggest slice (~11 k lines).

**Slice 6: walkers and cleanup.** Repoint the seven walkers (section 5) that can take the dependency (dash-runtime session, dvr-runtime recover, hls client/server, media-doctor, media-plane tests, test-harness `fmp4_split`; container-probe by dev-only drift guard), replace `tests/it/support/box_*.rs`, delete `wire_cursor` duplicates, CRATE-ACCEPTANCE items (>=2 examples, fuzz, docs, CHANGELOG, release lane), update CLAUDE.md crate map.

Unsure / needs an owner call:
1. Home of `wire_cursor` helpers (public isobmff module vs broadcast-common).
2. Whether `frag_offsets`, `validate`, `movie_fragment::protect_media_segment`/`media_fragment_run`, and mdcv/clli types are isobmff's (spec 4.1 does not name them).
3. container-probe: the spec's "seven walkers" cannot all be runtime users of `isobmff` while container-probe stays broadcast-common only.
4. Error mapping: flat variant-for-variant `From` (keeps ~1 000 test matches) vs one wrapper variant.
5. Where `rfc6381_hvc1/vvc1` formatting lives (hvcC/vvcC `rfc6381()` is derived from box fields but delegates to `sps`).
6. mp4-emsg end of life on crates.io and the major-class bump it forces on timed-metadata.
7. `serde` feature plumbing: moved types carry `cfg_attr(feature = "serde", ...)` (incl. `ser_timescale`); transmux's `serde` feature must forward to `isobmff/serde`.
8. Section 7 test classification is import-based (not compiled); a few "box-after-cut" files may hide an IR use.
