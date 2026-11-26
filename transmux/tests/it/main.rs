#[cfg(all(feature = "flv", feature = "matroska"))]
mod absolute_timing;
mod ac3_syncframe_ranges;
mod any_stream;
mod audio_config_facts;
mod audio_metadata;
mod audio_pes_straddle;
mod audio_srat;
mod avc_hvc_config;
mod base64_lenient;
#[path = "support/bit_writer.rs"]
mod bit_writer;
#[path = "support/box_walk.rs"]
mod box_walk;
#[cfg(feature = "captions")]
mod caption_detect;
mod cea608_in_band_carriage;
mod cenc_decrypt;
mod cenc_decrypt_frag_synthetic;
mod cenc_encrypt;
mod cenc_encrypt_e2e;
mod cenc_frag_layouts;
mod cenc_fragmented_fixture;
mod cenc_mux;
mod cenc_saiz_stsz_v1_boxes;
#[path = "support/cenc.rs"]
mod cenc_support;
mod clap_precedence;
mod classify_nals;
mod cli;
mod cli_binary;
mod cli_discontinuity;
#[path = "support/cmaf.rs"]
mod cmaf;
mod cmaf_stage;
mod codec_config;
mod codec_tail;
mod codecs_new;
#[cfg(feature = "cli")]
mod common;
#[cfg(feature = "rtp")]
mod damaged_unit_resync;
mod damaged_unit_resync_flv;
#[cfg(feature = "matroska")]
mod damaged_unit_resync_webm;
mod dash;
mod dash_audit22;
#[cfg(feature = "std")]
mod dash_mpd;
mod dash_xsd;
#[cfg(feature = "matroska")]
mod data_identity_map;
mod degraded_at_track;
#[path = "support/demux_baseline.rs"]
mod demux_baseline;
#[cfg(all(feature = "flv", feature = "rtp", feature = "ps", feature = "matroska"))]
mod demux_timing_invariant;
mod display_afd;
mod display_change;
mod display_colour;
mod display_crop;
mod display_dash;
mod display_demux;
mod display_fmp4_inband;
mod display_geometry;
mod display_mpeg2;
mod display_mux;
mod display_ps;
mod display_tkhd;
mod dolby;
mod dovi;
mod drm_pssh;
mod dts;
mod dts_ts;
mod eac3_bsi_walk;
mod eac3_crc2_real_capture;
mod eac3_real_capture;
mod eac3_real_dependent_substreams;
#[cfg(all(feature = "flv", feature = "matroska"))]
mod error_variants;
mod es_info_passthrough;
mod ffmpeg_oracle;
#[cfg(feature = "flv")]
mod flv;
#[cfg(feature = "flv")]
mod flv_boundaries;
#[cfg(feature = "flv")]
mod flv_demux_baseline;
#[cfg(feature = "flv")]
mod flv_stream;
mod fmp4_frag_layouts;
mod fmp4_shared_mdat_budget;
mod fmp4_trex_defaults;
#[cfg(feature = "hls")]
mod follower_cut;
mod frame_rate;
mod g1_damaged_param_sets;
mod garbled_input;
mod golden;
#[cfg(feature = "hls")]
mod golden_wire;
mod hdr10;
mod hdr10plus;
#[cfg(feature = "std")]
mod heaac_signalling;
mod hevc_damaged_parameter_sets;
mod hevc_sps;
mod hevc_vps_timing;
mod hls_discontinuity;
#[cfg(feature = "hls")]
mod hls_packager_playlist;
mod hub;
#[path = "support/hub_fixtures.rs"]
mod hub_fixtures;
#[cfg(feature = "hls")]
mod idr_fallback;
mod interlace_facts;
mod ir_labels;
mod ir_timing;
mod isobmff_bridge;
mod issue_952_avcc_no_trailer;
mod key_hygiene;
#[cfg(feature = "rtp")]
mod klv;
mod live_join_boundaries;
mod live_join_fixtures;
mod ll_dash;
#[cfg(feature = "hls")]
mod ll_hls;
#[cfg(feature = "std")]
#[path = "support/ll_packager.rs"]
mod ll_packager;
mod manifest_protection;
#[cfg(feature = "matroska")]
mod matroska_demux_golden;
#[cfg(feature = "std")]
#[path = "support/mkv_bytes.rs"]
mod mkv_bytes;
#[cfg(feature = "matroska")]
mod mkv_mux;
mod mp4_demux;
mod mp4_demux_data_tracks;
mod mp4_demux_differential;
mod mp4_demux_segments;
mod mpeg_legacy;
mod mpegh;
mod mpegh_ts;
#[cfg(feature = "std")]
#[path = "support/mux_bytes.rs"]
mod mux_bytes;
mod nal_keyframe;
mod opengop_anchor;
#[cfg(feature = "std")]
#[path = "support/packagers.rs"]
mod packagers;
mod paff_pairing;
#[cfg(feature = "motion-imagery-metadata")]
mod payload_plugins;
mod pcr_off_clock_extension;
mod pipeline;
mod pmt_version_diff;
mod pps_real_capture;
mod presentation_size;
#[cfg(feature = "hls")]
mod presentation_span_overflow;
mod progressive;
mod progressive_demux;
mod progressive_plan;
#[path = "support/progressive.rs"]
mod progressive_support;
#[cfg(feature = "ps")]
mod ps_demux;
#[cfg(feature = "ps")]
mod ps_demux_baseline;
#[cfg(feature = "ps")]
mod ps_demux_mpeg2;
#[cfg(feature = "ps")]
mod ps_demux_substreams;
mod reader_differential;
mod reader_fragmented_differential;
mod rebase;
#[path = "support/ref_mp4.rs"]
mod ref_mp4;
mod repackage;
mod review_findings;
#[cfg(feature = "rtp")]
mod rtcp;
#[cfg(feature = "rtp")]
mod rtcp_av_sync;
#[cfg(feature = "flv")]
mod rtmp;
#[cfg(feature = "flv")]
mod rtmp_client_play;
#[cfg(feature = "flv")]
mod rtmp_demux_baseline;
mod rtmp_ingest_fixture;
mod rtmp_play_peer;
#[cfg(feature = "flv")]
mod rtmp_spoke_equivalence;
#[cfg(feature = "rtp")]
mod rtp;
#[cfg(feature = "std")]
#[path = "support/rtp_run.rs"]
mod rtp_run;
mod rtp_sdp;
#[cfg(feature = "rtp")]
mod rtp_stream;
#[cfg(feature = "rtp")]
mod rtp_stream_paff;
#[cfg(feature = "rtp")]
mod rtp_stream_reorder;
#[cfg(feature = "hls")]
mod sample_aes;
#[cfg(feature = "hls")]
mod sample_aes_h264_oracle;
mod sample_table_digest;
#[cfg(feature = "std")]
#[path = "support/sdp_session.rs"]
mod sdp_session;
#[cfg(feature = "hls")]
mod segment_cut_policy;
mod segment_emsg;
#[cfg(feature = "std")]
mod segment_log;
mod segmenter;
#[cfg(all(feature = "flv", feature = "hls"))]
mod segmenter_progress_bounds;
#[path = "support/segmenter_stage.rs"]
mod segmenter_stage;
#[cfg(feature = "captions")]
mod sei_caption_real_capture;
mod serializer_length_truncation;
mod skipped_track_id;
mod slice_header_pps;
mod smooth;
#[cfg(feature = "std")]
mod smooth_fragment_inspect;
#[cfg(feature = "std")]
mod smooth_fragment_probe;
#[cfg(feature = "std")]
mod smooth_text_roundtrip;
mod splice;
#[path = "support/sps_builder.rs"]
mod sps_builder;
mod sps_display;
mod sps_profile_matrix;
mod ssai_ad_stitch;
#[cfg(all(feature = "flv", feature = "rtp", feature = "hls"))]
mod stage;
mod streaming_chunk_invariance;
mod streaming_demux;
mod streaming_demux_tracks_resolved;
mod streaming_ts_clock_reference_program;
mod streaming_ts_degraded_cap;
mod streaming_ts_demux_eac3_bsmod;
mod streaming_ts_demux_eac3_promotion;
mod streaming_ts_demux_liveness;
#[cfg(feature = "hls")]
mod streaming_tshls;
#[cfg(feature = "hls")]
mod streaming_tshls_config_change;
#[cfg(feature = "hls")]
mod streaming_tshls_late_track;
#[cfg(feature = "hls")]
mod streaming_tshls_pmt_version;
mod stsd_dimensions;
mod subtitle_ac4_strictness;
#[cfg(feature = "std")]
mod subtitle_manifests;
#[cfg(feature = "std")]
mod subtitle_real_producers;
mod subtitle_remux;
#[cfg(all(feature = "rtp", feature = "matroska"))]
mod sweep_output_golden;
#[cfg(feature = "std")]
#[cfg(feature = "std")]
#[cfg(feature = "captions")]
mod timed_metadata_sei_caption_fixture;
#[cfg(feature = "std")]
mod timed_metadata_teletext_tsduck_oracle;
mod timeline_oracle;
#[cfg(feature = "matroska")]
mod track_id_baseline;
mod track_provenance;
mod track_updated_causes;
mod trick_signalling;
mod trickplay;
mod truncation;
mod ts_continuity_semantics;
mod ts_demux;
mod ts_demux_baseline;
mod ts_hevc;
#[cfg(feature = "hls")]
mod ts_hevc_carriage;
#[cfg(feature = "hls")]
mod ts_hls;
#[cfg(feature = "hls")]
mod ts_hls_audio_catch_up;
#[path = "support/ts_hls_stage.rs"]
mod ts_hls_stage;
mod ts_mux;
mod ts_mux_pmt_update;
mod ts_mux_psi_pcr;
mod ts_mux_psi_repeat;
#[path = "support/ts_mux_stage.rs"]
mod ts_mux_stage;
mod ts_mux_streaming;
#[path = "support/ts_psi.rs"]
mod ts_psi;
mod ts_sync_loss_degraded;
mod ts_to_cmaf;
mod user_data_t35;
mod uuid_child_roundtrip;
mod validate;
#[cfg(feature = "matroska")]
mod vp8_vorbis;
mod vvc;
#[path = "support/webm_build.rs"]
mod webm_build;
#[cfg(feature = "matroska")]
mod webm_demux;
#[cfg(feature = "matroska")]
mod webm_demux_unknown_cluster;
#[cfg(feature = "matroska")]
mod webm_timestamp_scale;
#[cfg(feature = "std")]
#[path = "support/xml.rs"]
mod xml;
mod xml_escaping;

mod cli_stream;
