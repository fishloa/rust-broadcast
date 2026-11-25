//! Elementary track identity + crypto — [`Track`], [`TrackSpec`], [`TrackProtection`].
//!
//! Moved out of `media.rs` ([`Track`]) and `pipeline.rs`
//! ([`TrackSpec`]) (media plane step 2a, no-op): same types, same fields,
//! same impls.

use alloc::vec::Vec;
use mpeg_ts::pid::Pid;

use super::{CodecConfig, DisplayGeometry, ProgramNumber, Sample, TrackId};
use crate::hdr::HdrStaticMetadata;
use isobmff::dovi::DoviDecoderConfigurationRecord;

/// A track's identity + codec config, used to build the init segment.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TrackSpec {
    /// Track ID (1-based, unique within the movie).
    pub track_id: TrackId,
    /// Media timescale (ticks per second, e.g. 90000 for video).
    pub timescale: broadcast_common::timescale::Timescale,
    /// Codec configuration + dimensions.
    pub config: CodecConfig,
    /// Source elementary-stream PID for TS-demuxed tracks; `None` for non-TS
    /// sources (fMP4/FLV/WebM/PS/RTP). (issue #582)
    pub source_pid: Option<Pid>,
    /// Raw PMT ES_info descriptor-loop bytes for this elementary stream
    /// (ISO/IEC 13818-1 §2.4.4.9), verbatim; empty for non-TS sources.
    /// transmux does not parse these — consumers use dvb-si. (issue #582)
    pub es_info_descriptors: Vec<u8>,
    /// MPEG-2 TS program_number from the declaring PMT (ISO/IEC 13818-1
    /// §2.4.4.4); `None` for non-TS sources (fMP4/FLV/WebM/PS/RTP).
    pub program_number: Option<ProgramNumber>,
    /// Display geometry (coded size + SPS VUI sample aspect ratio) of the
    /// track's *current* codec config, derived by [`TrackSpec::new`] from the
    /// first SPS in the `avcC`/`hvcC` record. `None` for non-AVC/HEVC codecs
    /// and for a config with no decodable SPS (e.g. a placeholder record) —
    /// never a fabricated value. A re-announced spec
    /// ([`DemuxEvent::TrackUpdated`](super::DemuxEvent::TrackUpdated))
    /// carries the geometry of the new config. See
    /// [`DisplayGeometry::sample_aspect_ratio`].
    pub display: Option<DisplayGeometry>,
    /// HDR10 static metadata (mastering display colour volume, content light
    /// level) of an HEVC track: derived by [`TrackSpec::new`] from prefix SEI
    /// NAL units carried in the `hvcC` record, or read from the `mdcv` /
    /// `clli` boxes of an fMP4 sample entry. `None` when the stream carries
    /// neither message.
    pub hdr: Option<HdrStaticMetadata>,
    /// Dolby Vision configuration of an HEVC track: read from the
    /// `DOVI_video_stream_descriptor` of the PMT ES_info loop
    /// ([`TrackSpec::with_source`]) or from the `dvcC` / `dvvC` box of an fMP4
    /// sample entry. `None` for a stream that does not announce Dolby Vision.
    pub dovi: Option<DoviDecoderConfigurationRecord>,
    /// Whether the first access unit of an HEVC track carries an HDR10+
    /// (SMPTE ST 2094-40) T.35 prefix SEI, so the track holds dynamic
    /// metadata on top of any [`hdr`](Self::hdr) static metadata.
    pub hdr10plus: bool,
    /// CENC/CBCS crypto metadata for this track, or `None` for cleartext
    /// (ISO/IEC 23001-7 §12.2). Per-sample IVs and subsample maps live on
    /// [`Sample::protection`](super::Sample::protection) instead.
    pub protection: Option<TrackProtection>,
}

impl core::fmt::Debug for TrackSpec {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut d = f.debug_struct("TrackSpec");
        d.field("track_id", &self.track_id)
            .field("timescale", &self.timescale)
            .field("config", &self.config)
            .field("source_pid", &self.source_pid)
            .field("es_info_descriptors", &self.es_info_descriptors)
            .field("program_number", &self.program_number)
            .field("display", &self.display);
        if let Some(hdr) = &self.hdr {
            d.field("hdr", hdr);
        }
        if let Some(dovi) = &self.dovi {
            d.field("dovi", dovi);
        }
        if self.hdr10plus {
            d.field("hdr10plus", &self.hdr10plus);
        }
        if let Some(protection) = &self.protection {
            d.field("protection", protection);
        }
        d.finish()
    }
}

impl TrackSpec {
    /// A track spec with no TS provenance (`source_pid = None`, no ES_info
    /// descriptors) — the common case for every non-TS demuxer/transform.
    pub fn new(
        track_id: TrackId,
        timescale: impl Into<broadcast_common::timescale::Timescale>,
        config: CodecConfig,
    ) -> Self {
        let display = DisplayGeometry::from_codec_config(&config);
        let hdr = match &config {
            CodecConfig::Hevc { config, .. } => HdrStaticMetadata::from_hevc_record(&config.config),
            _ => None,
        };
        Self {
            track_id,
            timescale: timescale.into(),
            config,
            source_pid: None,
            es_info_descriptors: Vec::new(),
            program_number: None,
            display,
            hdr,
            dovi: None,
            hdr10plus: false,
            protection: None,
        }
    }

    /// Set whether the track carries HDR10+ dynamic metadata.
    pub fn with_hdr10plus(mut self, hdr10plus: bool) -> Self {
        self.hdr10plus = hdr10plus;
        self
    }

    /// Replace the auto-derived [`display`](Self::display) (for a source that
    /// knows better than the SPS).
    pub fn with_display(mut self, display: Option<DisplayGeometry>) -> Self {
        self.display = display;
        self
    }

    /// Attach TS provenance (issue #582): the source elementary-stream PID
    /// and its verbatim PMT ES_info descriptor-loop bytes.
    ///
    /// A [`CodecConfig::Data`] track also carries the loop on its
    /// [`DataIdentity::Ts`](super::DataIdentity::Ts) identity, which is what
    /// the TS muxer reads; both are updated here so a descriptors-only PMT
    /// change cannot leave the identity stale. (Removing the duplicate
    /// `es_info_descriptors` copy for a data track changes a demux golden, so
    /// it is a separate, orchestrator-approved change.)
    pub fn with_source(mut self, source_pid: Pid, es_info_descriptors: Vec<u8>) -> Self {
        self.source_pid = Some(source_pid);
        if let CodecConfig::Data {
            identity: super::DataIdentity::Ts { descriptors, .. },
        } = &mut self.config
        {
            *descriptors = es_info_descriptors.clone();
        }
        if matches!(self.config, CodecConfig::Hevc { .. }) {
            self.dovi = crate::dovi_descriptor::find_video_stream_descriptor(&es_info_descriptors)
                .map(|d| d.record);
        }
        self.es_info_descriptors = es_info_descriptors;
        self
    }

    /// Attach the TS program_number from the declaring PMT.
    pub fn with_program(mut self, program_number: ProgramNumber) -> Self {
        self.program_number = Some(program_number);
        self
    }

    /// Attach the track's CENC/CBCS crypto metadata, returning `self`.
    pub fn with_protection(mut self, protection: TrackProtection) -> Self {
        self.protection = Some(protection);
        self
    }
}

/// One elementary track: its identity/codec config plus its coded samples.
///
/// A thin wrapper pairing the existing [`TrackSpec`] (track_id + timescale +
/// [`CodecConfig`]) with the track's decode-ordered [`Sample`]s.
///
/// `#[non_exhaustive]`: matches [`TrackSpec`], [`Sample`] and
/// [`Media`](super::Media), and
/// leaves room for per-track state the pipeline has already wanted twice
/// (`start_decode_time`); construct with [`Track::new`] or
/// [`Track::new_at`]; destructuring or matching from another crate needs a
/// trailing `..`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Track {
    /// Track identity + codec configuration (used to build the init segment).
    pub spec: TrackSpec,
    /// The track's coded access units, in decode order.
    pub samples: Vec<Sample>,
    /// Absolute decode time of the track's **first** sample, in this track's
    /// media timescale ([`TrackSpec::timescale`]) ticks.
    ///
    /// Since media plane step 2c, `Sample::timing` (`.dts`)/`Sample::timing` (`.pts`) are
    /// themselves absolute (in the same timescale), so this field is
    /// redundant with (and must be kept equal to) `samples.first().timing.map(|t| t.dts)` for a
    /// timed track — it survives as the anchor consumers reach for without
    /// touching the sample slice (e.g. building the fragment `tfdt`
    /// `baseMediaDecodeTime`, ISO/IEC 14496-12:2026 §8.8.12), and as the
    /// track-level anchor for tracks with no samples at all.
    ///
    /// Demuxers that recover an absolute timeline populate it
    /// ([`crate::mp4_demux::Mp4Demux`] from the first movie fragment's `tfdt`;
    /// [`StreamingTsDemux`](crate::ts_demux::StreamingTsDemux) from the first sample's
    /// unwrapped DTS; WebM, MPEG Program Stream, FLV, RTMP and RTP likewise
    /// anchor on their first sample's absolute timestamp or their own source
    /// clock, media plane step 2c). A source that carries no absolute anchor
    /// leaves it `0`. It is the input to the timeline transforms in
    /// [`crate::rebase`] (rebase-to-zero, offset), which shift every sample's
    /// `dts`/`pts` in lockstep.
    pub start_decode_time: u64,
}

impl Track {
    /// Create a track from its spec and samples, anchored at decode time `0`.
    pub fn new(spec: TrackSpec, samples: Vec<Sample>) -> Self {
        Self {
            spec,
            samples,
            start_decode_time: 0,
        }
    }

    /// Create a track from its spec and samples, anchored at an absolute
    /// `start_decode_time` (in the track's media timescale).
    pub fn new_at(spec: TrackSpec, samples: Vec<Sample>, start_decode_time: u64) -> Self {
        Self {
            spec,
            samples,
            start_decode_time,
        }
    }

    /// Set the absolute [`start_decode_time`](Self::start_decode_time) anchor,
    /// returning `self` (builder style).
    pub fn with_start_decode_time(mut self, start_decode_time: u64) -> Self {
        self.start_decode_time = start_decode_time;
        self
    }

    /// Track ID (1-based, unique within the movie).
    pub fn track_id(&self) -> TrackId {
        self.spec.track_id
    }

    /// Media timescale (ticks per second); non-zero by construction.
    pub fn timescale(&self) -> broadcast_common::timescale::Timescale {
        self.spec.timescale
    }

    /// Codec configuration.
    pub fn config(&self) -> &CodecConfig {
        &self.spec.config
    }
}

/// Per-track CENC/CBCS crypto defaults carried on [`TrackSpec::protection`]
/// (ISO/IEC 23001-7 §12.2).
///
/// The per-sample IVs and subsample maps are on
/// [`Sample::protection`](super::Sample::protection), so this is everything a
/// track states once: the protection scheme, the `tenc` box (KID, per-sample
/// IV size, and, for `cbcs`, the pattern's `crypt`:`skip` block counts and any
/// constant IV), and the constant-IV `senc` emission policy.
///
/// `#[non_exhaustive]`: construct with [`TrackProtection::new`] — the
/// ISO/IEC 23001-7 metadata set is still growing here (e.g. multi-`pssh`
/// carriage), and each addition should be additive.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TrackProtection {
    /// The protection scheme (`cenc` AES-CTR or `cbcs` AES-CBC pattern).
    pub scheme: isobmff::cenc::CencScheme,
    /// Track-level crypto defaults — KID, per-sample IV size, and (for
    /// `cbcs`) the pattern's `crypt`:`skip` block counts — ISO/IEC 23001-7
    /// §12.2.
    pub tenc: isobmff::cenc::TrackEncryptionBox,
    /// Whether a `cbcs` + [`crate::IvGen::Constant`](crate::cenc_encrypt::IvGen::Constant)
    /// track should emit a `senc` box with the constant IV replicated per
    /// sample. See [`ConstantIvSenc`].
    pub constant_iv_senc: ConstantIvSenc,
}

impl TrackProtection {
    /// Build a track's crypto defaults from its scheme, `tenc` box and
    /// constant-IV `senc` policy.
    pub fn new(
        scheme: isobmff::cenc::CencScheme,
        tenc: isobmff::cenc::TrackEncryptionBox,
        constant_iv_senc: ConstantIvSenc,
    ) -> Self {
        Self {
            scheme,
            tenc,
            constant_iv_senc,
        }
    }
}

/// Whether a `cbcs` track with [`IvGen::Constant`](crate::cenc_encrypt::IvGen::Constant)
/// emits a `senc` box with the constant IV replicated per sample (the
/// default, for interop with decryptors such as Bento4 `mp4decrypt` that
/// silently ignore protected tracks lacking a `senc`), or omits the
/// `senc`/`saiz`/`saio` triple entirely (the spec-minimal shape —
/// ISO/IEC 23001-7 §12.2/§12.3 permit it because
/// `tenc.default_constant_IV` carries the lone IV and there is no
/// per-sample subsample map to record). The self-consistency gate
/// ([`CencDecryptStage`](crate::cenc_decrypt::CencDecryptStage) round-trip) is tested for
/// both shapes; the new default shape is additionally verified against
/// Bento4's `mp4decrypt`. Has no effect when `iv` is not
/// [`IvGen::Constant`](crate::cenc_encrypt::IvGen::Constant) or `scheme`
/// is not [`CencScheme::Cbcs`](isobmff::cenc::CencScheme::Cbcs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ConstantIvSenc {
    /// Emit a `senc` box with each sample's entry carrying the constant IV
    /// replicated (`per_sample_iv_size = 16`). The default — chosen for
    /// maximum third-party decryptor interop (Bento4 `mp4decrypt`, Shaka
    /// Packager, and several other tools require an explicit `senc` in the
    /// `traf` to recognize the track as decryptable at all, even when a
    /// `tenc.default_constant_IV` is present). The few extra bytes per
    /// fragment are the cost of interop.
    #[default]
    Emit,
    /// Omit `senc`/`saiz`/`saio` entirely. The spec-minimal, `tenc`-only
    /// shape: every decryptor that follows ISO/IEC 23001-7 to the letter
    /// can decrypt this shape, and it costs zero overhead per fragment,
    /// but it is known to be silently ignored by common tools (notably
    /// Bento4 `mp4decrypt` 1.6.0.0 — it exits 0 but the `mdat` is
    /// unchanged). Opt in when you control the decryptor chain.
    Omit,
}

impl ConstantIvSenc {
    /// Human-readable label for this enum variant.
    pub fn name(&self) -> &'static str {
        match self {
            ConstantIvSenc::Emit => "emit",
            ConstantIvSenc::Omit => "omit",
        }
    }
}

broadcast_common::impl_spec_display!(ConstantIvSenc);

#[cfg(test)]
mod constant_iv_senc_tests {
    use super::ConstantIvSenc;

    #[test]
    fn constant_iv_senc_labels_are_pinned() {
        assert_eq!(ConstantIvSenc::Emit.name(), "emit");
        assert_eq!(ConstantIvSenc::Omit.name(), "omit");
        assert_eq!(alloc::format!("{}", ConstantIvSenc::Emit), "emit");
        assert_eq!(alloc::format!("{}", ConstantIvSenc::Omit), "omit");
    }
}
