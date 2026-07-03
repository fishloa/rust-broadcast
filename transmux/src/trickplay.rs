//! I-frame-only (trick-play / thumbnail) track derivation from a video track.
//!
//! This transform produces a sparse, all-keyframe rendition of a video track
//! suitable for timeline scrubbing and thumbnail extraction. The derived track
//! contains only the sync samples (random-access points) of the source, with
//! each kept sample's duration stretched to cover the gap to the next kept
//! sample, so the total timeline is conserved.
//!
//! # Spec basis
//!
//! The "sync sample" concept maps directly to the ISO/IEC 14496-12 `stss`
//! (Sync Sample Box, §8.6.2): a sample not listed in `stss` is a non-sync
//! sample (it depends on others for decoding). The trick-play track carries
//! only the samples that *would* appear in `stss` — every sample in the derived
//! track is a random-access point.
//!
//! ## Future signalling work (not implemented here)
//!
//! - **HLS**: The derived track would be signalled in the master playlist via
//!   `EXT-X-I-FRAME-STREAM-INF` (RFC 8216 §4.3.4.2). That tag references a
//!   separate I-frame media playlist whose segments each carry one keyframe.
//!
//! - **DASH**: The derived track would be placed in an `AdaptationSet` with a
//!   `SupplementalProperty` of `urn:mpeg:dash:trickmode:2016` (DASH trick-mode
//!   descriptor, ISO/IEC 23009-1 §5.8.5.8). Signalling is deferred to a
//!   follow-up issue; only the IR-level track derivation is in scope here.
//!
//! # Example
//!
//! ```rust
//! use transmux::{trickplay::derive_iframe_track, pipeline::{TrackSpec, Sample, CodecConfig}};
//! use transmux::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
//! use transmux::media::Track;
//!
//! // Build a minimal AVC TrackSpec for the example.
//! let spec = TrackSpec {
//!     track_id: 1,
//!     timescale: 90000,
//!     config: CodecConfig::Avc {
//!         config: AVCConfigurationBox {
//!             config: AVCDecoderConfigurationRecord {
//!                 configuration_version: 1,
//!                 profile_indication: 66,
//!                 profile_compatibility: 0,
//!                 level_indication: 30,
//!                 length_size_minus_one: 3,
//!                 sps: vec![],
//!                 pps: vec![],
//!                 chroma_format: None,
//!                 bit_depth_luma_minus8: None,
//!                 bit_depth_chroma_minus8: None,
//!                 sps_ext: vec![],
//!             },
//!         },
//!         width: 1920,
//!         height: 1080,
//!     },
//! };
//!
//! // 8 samples: sync at indices 0 and 4, all with duration 100.
//! let samples: Vec<Sample> = (0u8..8).map(|i| Sample {
//!     data: vec![i],
//!     duration: 100,
//!     is_sync: i == 0 || i == 4,
//!     composition_offset: 0,
//!     source_timing: None,
//! }).collect();
//!
//! let src = Track::new(spec, samples);
//! let trick = derive_iframe_track(&src).unwrap();
//!
//! // Two keyframes kept; durations folded to span the gaps.
//! assert_eq!(trick.samples.len(), 2);
//! assert_eq!(trick.samples[0].duration, 400);
//! assert_eq!(trick.samples[1].duration, 400);
//! ```

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::media::Track;
use crate::pipeline::Sample;

/// Derive an I-frame-only (trick-play) [`Track`] from a video track.
///
/// Only the source's sync samples (`is_sync = true`) are retained. Each kept
/// sample's `duration` is set to the sum of all source sample durations from
/// that sync sample up to (but not including) the next sync sample, so the
/// derived track covers the same total timeline as the source.
///
/// - The `data` bytes of each kept sample are copied byte-for-byte from the
///   source.
/// - `is_sync` is `true` for every sample in the derived track.
/// - `composition_offset` is preserved from the source sync sample unchanged.
/// - The [`TrackSpec`](crate::pipeline::TrackSpec) (codec config, timescale,
///   track_id) is cloned from the source.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the source track contains no sync samples,
/// as the derived track would be empty and useless.
///
/// # Spec basis
///
/// Sync samples correspond to ISO/IEC 14496-12 §8.6.2 (`stss`). Each sample
/// in the returned track is a random-access point (i.e. would appear in `stss`).
pub fn derive_iframe_track(src: &Track) -> Result<Track> {
    // Collect the indices of all sync samples in decode order.
    let sync_indices: Vec<usize> = src
        .samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_sync)
        .map(|(i, _)| i)
        .collect();

    if sync_indices.is_empty() {
        return Err(Error::InvalidInput(
            "source track has no sync samples; cannot derive an I-frame-only track",
        ));
    }

    let total = src.samples.len();
    let mut derived: Vec<Sample> = Vec::with_capacity(sync_indices.len());

    for (k, &idx) in sync_indices.iter().enumerate() {
        // The span for this keyframe runs from `idx` up to (but not including)
        // the next sync sample's index, or to the end of the track for the last.
        let span_end = if k + 1 < sync_indices.len() {
            sync_indices[k + 1]
        } else {
            total
        };

        // Sum the durations of all source samples in [idx, span_end).
        let folded_duration: u32 = src.samples[idx..span_end]
            .iter()
            .map(|s| s.duration)
            .fold(0u32, |acc, d| acc.saturating_add(d));

        let src_sample = &src.samples[idx];
        derived.push(Sample {
            data: src_sample.data.clone(),
            duration: folded_duration,
            is_sync: true,
            composition_offset: src_sample.composition_offset,
            // The derived sample opens where `src_sample` did — its source
            // timing anchor (if any) still applies.
            source_timing: src_sample.source_timing,
        });
    }

    Ok(Track::new(src.spec.clone(), derived))
}

/// Convenience: append a derived I-frame-only track to a [`crate::media::Media`].
///
/// Calls [`derive_iframe_track`] on the track at position `video_track_index`
/// (an index into `media.tracks`, **not** a track_id) and pushes the derived
/// track onto `media.tracks`. The new track inherits the source's `TrackSpec`
/// (including `track_id`); callers that intend to mux both the original and the
/// trick track into the same container must set a distinct `track_id` on the
/// returned `Track` before inserting it.
///
/// Returns the derived track so callers can inspect or modify it (e.g. to
/// assign a new `track_id`) before the borrow of `media` is released.
///
/// # Errors
///
/// Propagates [`derive_iframe_track`]'s error if the source track has no sync
/// samples, or returns [`Error::InvalidInput`] if `video_track_index` is out
/// of bounds.
pub fn append_iframe_track(
    media: &mut crate::media::Media,
    video_track_index: usize,
) -> Result<()> {
    if video_track_index >= media.tracks.len() {
        return Err(Error::InvalidInput(
            "video_track_index out of bounds for media.tracks",
        ));
    }
    let derived = derive_iframe_track(&media.tracks[video_track_index])?;
    media.tracks.push(derived);
    Ok(())
}
