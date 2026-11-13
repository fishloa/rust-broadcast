mod send_sync;
mod aes128;
mod audio_renditions;
mod audit_1485;
mod audit_1492;
mod audit_prerelease;
mod avg_bandwidth;
mod client_state;
mod closed_captions;
#[path = "support/dummy_avc_config.rs"]
mod dummy_avc_config;
mod frame_rate;
mod golden_client;
#[cfg(feature = "std")]
mod hls_skip_delta;
mod iframe_master;
mod iframe_playlist;
mod map_runs;
mod master_bandwidth;
mod master_playlist;
#[cfg(feature = "std")]
mod origin_hardening;
mod origin_loop;
mod pdt_arrival;
mod pdt_origin;
mod playlist_base;
mod rendition_validators;
mod resolution;
mod segment_failure;
mod subtitle_renditions;
mod target_duration_freeze;
mod tracks_display;
mod ts_hls_ingest;
mod url_control_chars;
mod video_range;
