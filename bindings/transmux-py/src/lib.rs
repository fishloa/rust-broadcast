//! Python bindings for `transmux` — the any-to-any media container muxing hub
//! — exposing its neutral demux IR ([`transmux::media::Media`] /
//! [`transmux::media::Track`] / [`transmux::pipeline::Sample`]) to Python for
//! ML/analysis front-ends (docs/IDEAS.md item #7, issue #668): a Python caller
//! gets clean, PTS-tagged, opaque coded sample bytes it can hand straight to
//! PyAV / a WebCodecs-equivalent decoder / ONNX preprocessing, without needing
//! to understand transmux's own Rust IR types.
//!
//! `demux_ts(bytes) -> dict` runs [`transmux::TsDemux::demux`] over raw
//! MPEG-2 TS bytes and hand-converts the resulting `Media`/`Track`/`Sample` IR
//! into Python dicts. Unlike `dvb-si-py` (`bindings/python/`), which converts
//! Rust → `serde_json::Value` → Python because `dvb-si`'s table types are
//! fully `serde::Serialize`, transmux's pipeline IR (`Media`, `Track`,
//! `TrackSpec`, `CodecConfig`, `Sample`) carries **no** `serde` derive at all
//! (verified against `transmux/src/media.rs` + `transmux/src/pipeline.rs`: the
//! crate's `serde` feature only reaches the lower-level ISOBMFF box types, not
//! this IR) — so this binding cannot reuse that json round-trip and instead
//! builds `PyDict`s field-by-field from the real Rust structs.
//!
//! Only the TS demux entry point is exposed for v1 (issue #668's scope): it is
//! the one one-shot, batch-callable `Unpackage` impl transmux ships today.
//! `Fmp4Demux` exists too (`transmux::media::Fmp4Demux`) and could be wrapped
//! the same way in a follow-up if fMP4-sourced ML pipelines need it.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};

use broadcast_common::Parse;
use transmux::aac_asc::AudioSpecificConfig;
use transmux::dts::DtsSpecificBox;
use transmux::media::{Media, Track};
use transmux::pipeline::{CodecConfig, Sample};
use transmux::sps::rfc6381_avc1;
use transmux::{EsdsBox, TsDemux};

/// Demux raw MPEG-2 TS bytes into a Python `dict` describing the transmux
/// `Media` IR.
///
/// Returned shape (Python):
///
/// ```text
/// {
///   "movie_timescale": int,
///   "tracks": [
///     {
///       "track_id": int,
///       "timescale": int,               # ticks/sec for this track's duration/composition_offset fields
///       "start_decode_time": int,       # absolute DTS anchor, in `timescale` ticks
///       "source_pid": int | None,       # TS elementary-stream PID, when known
///       "codec": str,                   # short family label, e.g. "avc", "aac", "data"
///       "codec_string": str | None,     # RFC 6381 codec string, when cheaply derivable
///       "width": int | None,
///       "height": int | None,
///       "channel_count": int | None,
///       "sample_rate": int | None,      # Hz
///       "samples": [
///         {
///           "data": bytes,              # opaque coded access unit (length-prefixed NAL for AVC/HEVC, raw frame for audio)
///           "duration": int,            # in `timescale` ticks
///           "is_sync": bool,            # keyframe / random-access point
///           "composition_offset": int,  # pts - dts, in `timescale` ticks
///         },
///         ...
///       ],
///     },
///     ...
///   ],
/// }
/// ```
#[pyfunction]
fn demux_ts(py: Python<'_>, data: &[u8]) -> PyResult<PyObject> {
    let mut demux = TsDemux::new();
    let media: Media = demux
        .demux(data)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    media_to_py(py, &media)
}

fn media_to_py(py: Python<'_>, media: &Media) -> PyResult<PyObject> {
    let dict = PyDict::new(py);
    dict.set_item("movie_timescale", media.movie_timescale)?;
    let tracks = PyList::empty(py);
    for track in &media.tracks {
        tracks.append(track_to_py(py, track)?)?;
    }
    dict.set_item("tracks", tracks)?;
    Ok(dict.into())
}

fn track_to_py(py: Python<'_>, track: &Track) -> PyResult<PyObject> {
    let dict = PyDict::new(py);
    let spec = &track.spec;
    dict.set_item("track_id", spec.track_id)?;
    dict.set_item("timescale", spec.timescale)?;
    dict.set_item("start_decode_time", track.start_decode_time)?;
    dict.set_item("source_pid", spec.source_pid)?;

    let identity = codec_identity(&spec.config);
    dict.set_item("codec", identity.codec)?;
    dict.set_item("codec_string", identity.codec_string)?;
    dict.set_item("width", identity.width)?;
    dict.set_item("height", identity.height)?;
    dict.set_item("channel_count", identity.channel_count)?;
    dict.set_item("sample_rate", identity.sample_rate)?;

    let samples = PyList::empty(py);
    for s in &track.samples {
        samples.append(sample_to_py(py, s)?)?;
    }
    dict.set_item("samples", samples)?;
    Ok(dict.into())
}

fn sample_to_py(py: Python<'_>, sample: &Sample) -> PyResult<PyObject> {
    let dict = PyDict::new(py);
    dict.set_item("data", PyBytes::new(py, &sample.data))?;
    dict.set_item("duration", sample.duration)?;
    dict.set_item("is_sync", sample.is_sync)?;
    dict.set_item("composition_offset", sample.composition_offset)?;
    Ok(dict.into())
}

/// Codec family label + everything a Python-side consumer needs to feed
/// `Sample::data` to an external decoder/model without understanding
/// transmux's own `CodecConfig` enum.
struct CodecIdentity {
    codec: &'static str,
    codec_string: Option<String>,
    width: Option<u16>,
    height: Option<u16>,
    channel_count: Option<u16>,
    sample_rate: Option<u32>,
}

/// Resolve family label / RFC 6381 codec string / geometry / audio identity
/// from a [`CodecConfig`].
///
/// The RFC 6381 string is filled in wherever this crate already exposes a
/// cheap helper for it (every video codec's config-box `rfc6381()`, plus
/// AC-3/E-AC-3/Opus/FLAC/AC-4/DTS/MPEG-H's `rfc6381()`; AVC is built directly
/// from its `avcC` fields via [`rfc6381_avc1`], since `AVCDecoderConfigurationRecord`
/// has no `rfc6381()` method of its own; AAC is decoded from the `esds`
/// `AudioSpecificConfig` bytes). For MPEG-1/2 legacy audio and MPEG-2 video —
/// codecs this crate does not give an RFC 6381 helper for — the string is
/// built from the `esds` `objectTypeIndication` per the RFC 6381 `mp4a`/`mp4v`
/// OTI-hex convention (ISO/IEC 14496-1 §7.2.6.6 Table 5), which needs no
/// further per-codec decode. WebM-native VP8/Vorbis and opaque TS `Data`
/// tracks have no RFC 6381 convention implemented in this crate, so
/// `codec_string` is `None` for them — callers still get the `codec` family
/// label.
fn codec_identity(config: &CodecConfig) -> CodecIdentity {
    match config {
        CodecConfig::Avc {
            config,
            width,
            height,
        } => CodecIdentity {
            codec: "avc",
            codec_string: Some(rfc6381_avc1(
                config.config.profile_indication,
                config.config.profile_compatibility,
                config.config.level_indication,
            )),
            width: Some(*width),
            height: Some(*height),
            channel_count: None,
            sample_rate: None,
        },
        CodecConfig::Hevc {
            config,
            width,
            height,
        } => CodecIdentity {
            codec: "hevc",
            codec_string: Some(config.config.rfc6381()),
            width: Some(*width),
            height: Some(*height),
            channel_count: None,
            sample_rate: None,
        },
        CodecConfig::Vvc {
            config,
            width,
            height,
        } => CodecIdentity {
            codec: "vvc",
            codec_string: Some(config.config.rfc6381()),
            width: Some(*width),
            height: Some(*height),
            channel_count: None,
            sample_rate: None,
        },
        CodecConfig::Av1 {
            config,
            width,
            height,
        } => CodecIdentity {
            codec: "av1",
            codec_string: Some(config.rfc6381()),
            width: Some(*width),
            height: Some(*height),
            channel_count: None,
            sample_rate: None,
        },
        CodecConfig::Vp9 {
            config,
            width,
            height,
        } => CodecIdentity {
            codec: "vp9",
            codec_string: Some(config.rfc6381()),
            width: Some(*width),
            height: Some(*height),
            channel_count: None,
            sample_rate: None,
        },
        CodecConfig::Vp8 { width, height } => CodecIdentity {
            codec: "vp8",
            codec_string: None,
            width: Some(*width),
            height: Some(*height),
            channel_count: None,
            sample_rate: None,
        },
        CodecConfig::Mpeg2Video {
            esds,
            width,
            height,
        } => CodecIdentity {
            codec: "mpeg2video",
            codec_string: oti_codec_string("mp4v", esds),
            width: Some(*width),
            height: Some(*height),
            channel_count: None,
            sample_rate: None,
        },
        CodecConfig::Aac {
            esds,
            channel_count,
            sample_rate,
            ..
        } => CodecIdentity {
            codec: "aac",
            codec_string: aac_rfc6381(esds),
            width: None,
            height: None,
            channel_count: Some(*channel_count),
            sample_rate: Some(*sample_rate),
        },
        CodecConfig::MpegAudio {
            esds,
            channel_count,
            sample_rate,
            ..
        } => CodecIdentity {
            codec: "mpegaudio",
            codec_string: oti_codec_string("mp4a", esds),
            width: None,
            height: None,
            channel_count: Some(*channel_count),
            sample_rate: Some(*sample_rate),
        },
        CodecConfig::Ac3 {
            config,
            channel_count,
            sample_rate,
            ..
        } => CodecIdentity {
            codec: "ac3",
            codec_string: Some(config.rfc6381().to_string()),
            width: None,
            height: None,
            channel_count: Some(*channel_count),
            sample_rate: Some(*sample_rate),
        },
        CodecConfig::Eac3 {
            config,
            channel_count,
            sample_rate,
            ..
        } => CodecIdentity {
            codec: "eac3",
            codec_string: Some(config.rfc6381().to_string()),
            width: None,
            height: None,
            channel_count: Some(*channel_count),
            sample_rate: Some(*sample_rate),
        },
        CodecConfig::Opus {
            config,
            channel_count,
            sample_rate,
            ..
        } => CodecIdentity {
            codec: "opus",
            codec_string: Some(config.rfc6381().to_string()),
            width: None,
            height: None,
            channel_count: Some(*channel_count),
            sample_rate: Some(*sample_rate),
        },
        CodecConfig::Flac {
            config,
            channel_count,
            sample_rate,
            ..
        } => CodecIdentity {
            codec: "flac",
            codec_string: Some(config.rfc6381().to_string()),
            width: None,
            height: None,
            channel_count: Some(*channel_count),
            sample_rate: Some(*sample_rate),
        },
        CodecConfig::Ac4 {
            config,
            channel_count,
            sample_rate,
            ..
        } => CodecIdentity {
            codec: "ac4",
            codec_string: Some(config.rfc6381().to_string()),
            width: None,
            height: None,
            channel_count: Some(*channel_count),
            sample_rate: Some(*sample_rate),
        },
        CodecConfig::Dts {
            codec_fourcc,
            channel_count,
            sample_rate,
            ..
        } => CodecIdentity {
            codec: "dts",
            codec_string: Some(DtsSpecificBox::rfc6381(codec_fourcc).to_string()),
            width: None,
            height: None,
            channel_count: Some(*channel_count),
            sample_rate: Some(*sample_rate),
        },
        CodecConfig::MpegH {
            config,
            channel_count,
            sample_rate,
            ..
        } => CodecIdentity {
            codec: "mpegh",
            codec_string: Some(config.rfc6381()),
            width: None,
            height: None,
            channel_count: Some(*channel_count),
            sample_rate: Some(*sample_rate),
        },
        CodecConfig::Vorbis {
            channels,
            sample_rate,
            ..
        } => CodecIdentity {
            codec: "vorbis",
            codec_string: None,
            width: None,
            height: None,
            channel_count: Some(*channels),
            sample_rate: Some(*sample_rate),
        },
        CodecConfig::Data { .. } => CodecIdentity {
            codec: "data",
            codec_string: None,
            width: None,
            height: None,
            channel_count: None,
            sample_rate: None,
        },
        // `CodecConfig` is `#[non_exhaustive]`: any codec added to transmux in
        // the future surfaces here with a generic label rather than failing to
        // build.
        _ => CodecIdentity {
            codec: "unknown",
            codec_string: None,
            width: None,
            height: None,
            channel_count: None,
            sample_rate: None,
        },
    }
}

/// Decode the AAC `AudioSpecificConfig` out of an `esds` box's
/// `DecoderSpecificInfo` and build its RFC 6381 `mp4a.40.<AOT>` string
/// (ISO/IEC 14496-3 §1.6). `None` if the `esds` is missing the pieces (should
/// not happen for a well-formed AAC track, but this is a Python-facing API —
/// never panic on a malformed input).
fn aac_rfc6381(esds: &EsdsBox) -> Option<String> {
    let dsi = esds
        .es_descriptor
        .decoder_config
        .as_ref()?
        .decoder_specific_info
        .as_ref()?;
    let asc = AudioSpecificConfig::parse(&dsi.data).ok()?;
    Some(asc.rfc6381())
}

/// Build an RFC 6381 codec string from an `esds` `objectTypeIndication` using
/// the plain `{prefix}.{OTI in uppercase hex}` convention (RFC 6381 §3.3;
/// ISO/IEC 14496-1 §7.2.6.6 Table 5 for the OTI values) — used for the legacy
/// codecs (MPEG-1/2 audio, MPEG-2 video) that this crate does not give a
/// dedicated `rfc6381()` helper for, since the OTI alone is already the whole
/// RFC 6381 identifier for these (no profile/level suffix, unlike AVC/HEVC).
fn oti_codec_string(prefix: &str, esds: &EsdsBox) -> Option<String> {
    let oti = esds
        .es_descriptor
        .decoder_config
        .as_ref()?
        .object_type_indication
        .0;
    Some(format!("{prefix}.{oti:02X}"))
}

/// The `transmux_py` extension module.
#[pymodule]
fn transmux_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(demux_ts, m)?)?;
    Ok(())
}
