//! Elementary track identity + crypto — [`Track`], [`TrackSpec`], [`TrackEncryption`].
//!
//! Moved out of `media.rs` ([`Track`], [`TrackEncryption`]) and `pipeline.rs`
//! ([`TrackSpec`]) (media plane step 2a, no-op): same types, same fields,
//! same impls.

use alloc::vec::Vec;

use super::{CodecConfig, Sample};

/// A track's identity + codec config, used to build the init segment.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TrackSpec {
    /// Track ID (1-based, unique within the movie).
    pub track_id: u32,
    /// Media timescale (ticks per second, e.g. 90000 for video).
    pub timescale: u32,
    /// Codec configuration + dimensions.
    pub config: CodecConfig,
    /// Source elementary-stream PID for TS-demuxed tracks; `None` for non-TS
    /// sources (fMP4/FLV/WebM/PS/RTP). (issue #582)
    pub source_pid: Option<u16>,
    /// Raw PMT ES_info descriptor-loop bytes for this elementary stream
    /// (ISO/IEC 13818-1 §2.4.4.8), verbatim; empty for non-TS sources.
    /// transmux does not parse these — consumers use dvb-si. (issue #582)
    pub es_info_descriptors: Vec<u8>,
}

impl TrackSpec {
    /// A track spec with no TS provenance (`source_pid = None`, no ES_info
    /// descriptors) — the common case for every non-TS demuxer/transform.
    pub fn new(track_id: u32, timescale: u32, config: CodecConfig) -> Self {
        Self {
            track_id,
            timescale,
            config,
            source_pid: None,
            es_info_descriptors: Vec::new(),
        }
    }

    /// Attach TS provenance (issue #582): the source elementary-stream PID
    /// and its verbatim PMT ES_info descriptor-loop bytes.
    pub fn with_source(mut self, source_pid: u16, es_info_descriptors: Vec<u8>) -> Self {
        self.source_pid = Some(source_pid);
        self.es_info_descriptors = es_info_descriptors;
        self
    }
}

/// One elementary track: its identity/codec config plus its coded samples.
///
/// A thin wrapper pairing the existing [`TrackSpec`] (track_id + timescale +
/// [`CodecConfig`]) with the track's decode-ordered [`Sample`]s.
#[derive(Debug, Clone)]
pub struct Track {
    /// Track identity + codec configuration (used to build the init segment).
    pub spec: TrackSpec,
    /// The track's coded access units, in decode order.
    pub samples: Vec<Sample>,
    /// Absolute decode time of the track's **first** sample, in this track's
    /// media timescale ([`TrackSpec::timescale`]) ticks.
    ///
    /// [`Sample`] timing is stored *relatively* (per-sample [`Sample::duration`];
    /// a sample's DTS is the running sum of preceding durations, its PTS is
    /// `DTS + composition_offset`). This field is the missing absolute anchor:
    /// the DTS the first sample sits at on the presentation timeline. It maps
    /// directly onto the fragment `tfdt` `baseMediaDecodeTime`
    /// (ISO/IEC 14496-12:2015 §8.8.12) that [`crate::media::CmafMux`] writes
    /// for the first media segment.
    ///
    /// Demuxers that recover an absolute timeline populate it
    /// ([`crate::media::Fmp4Demux`] from the first movie fragment's `tfdt`;
    /// [`TsDemux`](crate::ts_demux::TsDemux) from the first sample's DTS).
    /// Demuxers whose source carries no absolute anchor (FLV, WebM, MPEG
    /// Program Stream, RTMP, RTP) leave it `0`. It is the input to the
    /// timeline transforms in [`crate::rebase`] (rebase-to-zero, offset,
    /// 33-bit MPEG wrap-unroll — ISO/IEC 13818-1 33-bit timestamps).
    pub start_decode_time: u64,
    /// CENC/CBCS crypto metadata for this track's samples, or `None` for
    /// cleartext. Populated by [`CencEncryptor`](crate::cenc_encrypt::CencEncryptor)'s
    /// [`Encrypt`](broadcast_common::Encrypt) impl (issue #564) or by a
    /// demuxer of an already-protected source (e.g.
    /// [`crate::cenc_decrypt::CencDecryptor::demux`]); read by the muxer's
    /// crypto-box emission (`sinf`/`senc`/`saio`/`saiz`).
    pub encryption: Option<TrackEncryption>,
}

impl Track {
    /// Create a track from its spec and samples, anchored at decode time `0`.
    pub fn new(spec: TrackSpec, samples: Vec<Sample>) -> Self {
        Self {
            spec,
            samples,
            start_decode_time: 0,
            encryption: None,
        }
    }

    /// Create a track from its spec and samples, anchored at an absolute
    /// `start_decode_time` (in the track's media timescale).
    pub fn new_at(spec: TrackSpec, samples: Vec<Sample>, start_decode_time: u64) -> Self {
        Self {
            spec,
            samples,
            start_decode_time,
            encryption: None,
        }
    }

    /// Set the absolute [`start_decode_time`](Self::start_decode_time) anchor,
    /// returning `self` (builder style).
    pub fn with_start_decode_time(mut self, start_decode_time: u64) -> Self {
        self.start_decode_time = start_decode_time;
        self
    }

    /// Track ID (1-based, unique within the movie).
    pub fn track_id(&self) -> u32 {
        self.spec.track_id
    }

    /// Media timescale (ticks per second).
    pub fn timescale(&self) -> u32 {
        self.spec.timescale
    }

    /// Codec configuration.
    pub fn config(&self) -> &CodecConfig {
        &self.spec.config
    }
}

/// Per-track CENC/CBCS crypto carrier attached to [`Track::encryption`].
///
/// The IR-side dual of the metadata [`crate::cenc_decrypt::CencDecryptor`]
/// harvests from an already-protected file's `sinf`/`tenc`/`senc` boxes: this
/// is exactly the shape [`CencEncryptor`](crate::cenc_encrypt::CencEncryptor)
/// produces (issue #564), and it is what the muxer's crypto-box emission
/// (`sinf`/`senc`/`saio`/`saiz`) reads back to (re)build the boxes without
/// needing to know how the samples were protected.
#[derive(Debug, Clone)]
pub struct TrackEncryption {
    /// The protection scheme (`cenc` AES-CTR or `cbcs` AES-CBC pattern).
    pub scheme: crate::cenc::CencScheme,
    /// Track-level crypto defaults — KID, per-sample IV size, and (for
    /// `cbcs`) the pattern's `crypt`:`skip` block counts — ISO/IEC 23001-7
    /// §12.2.
    pub tenc: crate::cenc::TrackEncryptionBox,
    /// Per-sample IV + subsample map, in decode order — ISO/IEC 23001-7 §12.3.
    /// `samples.len()` must equal the owning [`Track`]'s `samples.len()`.
    pub samples: Vec<crate::cenc::SampleEncryptionEntry>,
}
