//! The one table between the seam's vocabulary and libav's.
//!
//! Every direction of every mapping the adapter needs lives here: the coded
//! format, the pixel and sample formats, the channel layout, the codec profile
//! and the implementation-name candidates libav may report on a host. A value
//! with no entry is reported as such by the caller; nothing here guesses.

use core::num::{NonZeroU16, NonZeroU32};

use broadcast_codec::{
    ChannelLayout, CodecId, CodecProfile, ColourDescription, PixelFormat, SampleFormat,
    SampleLayout,
};
use ffmpeg_next::ChannelLayout as LibavChannelLayout;
use ffmpeg_next::codec::profile::{AAC, H264, HEVC, MPEG2, VP9};
use ffmpeg_next::codec::{Id, Profile};
use ffmpeg_next::format::{Pixel, Sample};

/// The coded formats this adapter maps, and libav's own id for each.
///
/// This is the whole list: a [`CodecId`] outside it has no libav codec and is
/// [`CodecError::Unsupported`](broadcast_codec::CodecError::Unsupported).
pub(crate) const CODECS: &[(CodecId, Id)] = &[
    (CodecId::H264, Id::H264),
    (CodecId::Hevc, Id::HEVC),
    (CodecId::Vvc, Id::VVC),
    (CodecId::Av1, Id::AV1),
    (CodecId::Vp9, Id::VP9),
    (CodecId::Mpeg2Video, Id::MPEG2VIDEO),
    (CodecId::Aac, Id::AAC),
    (CodecId::Ac3, Id::AC3),
    (CodecId::Eac3, Id::EAC3),
    (CodecId::Ac4, Id::AC4),
    (CodecId::Dts, Id::DTS),
    (CodecId::Opus, Id::OPUS),
    (CodecId::Flac, Id::FLAC),
    (CodecId::MpegAudio, Id::MP3),
];

/// Implementations libav registers under a name other than the codec's own
/// default, which this adapter asks for by name.
///
/// A name the linked libav does not register is simply absent, so a hardware
/// codec missing on this host is missing from the descriptors too.
pub(crate) const NAMED_DECODERS: &[(CodecId, &str)] = &[
    (CodecId::H264, "h264_cuvid"),
    (CodecId::H264, "h264_qsv"),
    (CodecId::H264, "h264_v4l2m2m"),
    (CodecId::H264, "h264_vaapi"),
    (CodecId::Hevc, "hevc_cuvid"),
    (CodecId::Hevc, "hevc_qsv"),
    (CodecId::Hevc, "hevc_v4l2m2m"),
    (CodecId::Hevc, "hevc_vaapi"),
    (CodecId::Av1, "libdav1d"),
    (CodecId::Av1, "av1_cuvid"),
    (CodecId::Av1, "av1_qsv"),
    (CodecId::Vp9, "vp9_cuvid"),
    (CodecId::Vp9, "vp9_qsv"),
    (CodecId::Vp9, "vp9_vaapi"),
    (CodecId::Mpeg2Video, "mpeg2_cuvid"),
    (CodecId::Mpeg2Video, "mpeg2_qsv"),
    (CodecId::Mpeg2Video, "mpeg2_v4l2m2m"),
    (CodecId::Aac, "libfdk_aac"),
];

/// Encoder implementations libav registers under a non-default name.
pub(crate) const NAMED_ENCODERS: &[(CodecId, &str)] = &[
    (CodecId::H264, "libx264"),
    (CodecId::H264, "libx264rgb"),
    (CodecId::H264, "h264_videotoolbox"),
    (CodecId::H264, "h264_nvenc"),
    (CodecId::H264, "h264_qsv"),
    (CodecId::H264, "h264_vaapi"),
    (CodecId::Hevc, "libx265"),
    (CodecId::Hevc, "hevc_videotoolbox"),
    (CodecId::Hevc, "hevc_nvenc"),
    (CodecId::Hevc, "hevc_qsv"),
    (CodecId::Hevc, "hevc_vaapi"),
    (CodecId::Av1, "libsvtav1"),
    (CodecId::Av1, "librav1e"),
    (CodecId::Av1, "av1_nvenc"),
    (CodecId::Vp9, "libvpx-vp9"),
    (CodecId::Mpeg2Video, "mpeg2video"),
    (CodecId::Aac, "libfdk_aac"),
    (CodecId::Opus, "libopus"),
    (CodecId::MpegAudio, "libmp3lame"),
];

/// Name fragments that mark a dedicated-hardware implementation.
const HARDWARE_TOKENS: &[&str] = &[
    "videotoolbox",
    "nvenc",
    "cuvid",
    "qsv",
    "vaapi",
    "v4l2m2m",
    "mediacodec",
    "amf",
    "vulkan",
];

/// Whether libav registers `implementation` as a dedicated-hardware codec.
pub(crate) fn hardware(implementation: &str) -> bool {
    implementation
        .split(['_', '-'])
        .any(|token| HARDWARE_TOKENS.contains(&token))
}

/// Whether `codec` is a lossless one, as far as this adapter can know.
pub(crate) fn lossless(codec: CodecId) -> bool {
    codec == CodecId::Flac
}

/// libav's id for `codec`, when this adapter maps it.
pub(crate) fn libav_id(codec: CodecId) -> Option<Id> {
    CODECS
        .iter()
        .find(|(mapped, _)| *mapped == codec)
        .map(|(_, id)| *id)
}

/// The pixel format a frame of `format` is stored in.
pub(crate) fn pixel_format(format: Pixel) -> Option<PixelFormat> {
    Some(match format {
        Pixel::GRAY8 => PixelFormat::Gray8,
        Pixel::GRAY10LE => PixelFormat::Gray10,
        Pixel::GRAY12LE => PixelFormat::Gray12,
        Pixel::YUV420P | Pixel::YUVJ420P => PixelFormat::Yuv420p8,
        Pixel::YUV422P | Pixel::YUVJ422P => PixelFormat::Yuv422p8,
        Pixel::YUV444P | Pixel::YUVJ444P => PixelFormat::Yuv444p8,
        Pixel::NV12 => PixelFormat::Nv12,
        Pixel::YUV420P10LE => PixelFormat::Yuv420p10,
        Pixel::YUV422P10LE => PixelFormat::Yuv422p10,
        Pixel::YUV444P10LE => PixelFormat::Yuv444p10,
        Pixel::YUV420P12LE => PixelFormat::Yuv420p12,
        Pixel::YUV422P12LE => PixelFormat::Yuv422p12,
        Pixel::YUV444P12LE => PixelFormat::Yuv444p12,
        Pixel::P010LE => PixelFormat::P010,
        Pixel::RGB24 => PixelFormat::Rgb24,
        Pixel::RGBA => PixelFormat::Rgba32,
        _ => return None,
    })
}

/// The libav pixel format for a seam one.
pub(crate) fn libav_pixel(format: PixelFormat) -> Pixel {
    match format {
        PixelFormat::Gray8 => Pixel::GRAY8,
        PixelFormat::Gray10 => Pixel::GRAY10LE,
        PixelFormat::Gray12 => Pixel::GRAY12LE,
        PixelFormat::Yuv420p8 => Pixel::YUV420P,
        PixelFormat::Yuv422p8 => Pixel::YUV422P,
        PixelFormat::Yuv444p8 => Pixel::YUV444P,
        PixelFormat::Nv12 => Pixel::NV12,
        PixelFormat::Yuv420p10 => Pixel::YUV420P10LE,
        PixelFormat::Yuv422p10 => Pixel::YUV422P10LE,
        PixelFormat::Yuv444p10 => Pixel::YUV444P10LE,
        PixelFormat::Yuv420p12 => Pixel::YUV420P12LE,
        PixelFormat::Yuv422p12 => Pixel::YUV422P12LE,
        PixelFormat::Yuv444p12 => Pixel::YUV444P12LE,
        PixelFormat::P010 => Pixel::P010LE,
        PixelFormat::Rgb24 => Pixel::RGB24,
        PixelFormat::Rgba32 => Pixel::RGBA,
        _ => Pixel::YUV420P,
    }
}

/// The seam storage of one libav sample format, with its planar or
/// interleaved layout.
pub(crate) fn sample_format(format: Sample) -> Option<(SampleFormat, SampleLayout)> {
    let planar = format.is_planar();
    let layout = if planar {
        SampleLayout::Planar
    } else {
        SampleLayout::Interleaved
    };
    let storage = match format {
        Sample::U8(_) => SampleFormat::U8,
        Sample::I16(_) => SampleFormat::S16,
        Sample::I32(_) => SampleFormat::S32,
        Sample::F32(_) => SampleFormat::F32,
        Sample::F64(_) => SampleFormat::F64,
        Sample::I64(_) | Sample::None => return None,
    };
    Some((storage, layout))
}

/// The libav sample format of a seam storage and layout.
pub(crate) fn libav_sample(format: SampleFormat, layout: SampleLayout) -> Sample {
    let planar = layout == SampleLayout::Planar;
    match (format, planar) {
        (SampleFormat::U8, false) => Sample::U8(ffmpeg_next::format::sample::Type::Packed),
        (SampleFormat::U8, true) => Sample::U8(ffmpeg_next::format::sample::Type::Planar),
        (SampleFormat::S16, false) => Sample::I16(ffmpeg_next::format::sample::Type::Packed),
        (SampleFormat::S16, true) => Sample::I16(ffmpeg_next::format::sample::Type::Planar),
        (SampleFormat::S32, false) => Sample::I32(ffmpeg_next::format::sample::Type::Packed),
        (SampleFormat::S32, true) => Sample::I32(ffmpeg_next::format::sample::Type::Planar),
        (SampleFormat::F32, false) => Sample::F32(ffmpeg_next::format::sample::Type::Packed),
        (SampleFormat::F32, true) => Sample::F32(ffmpeg_next::format::sample::Type::Planar),
        (SampleFormat::F64, false) => Sample::F64(ffmpeg_next::format::sample::Type::Packed),
        (SampleFormat::F64, true) => Sample::F64(ffmpeg_next::format::sample::Type::Planar),
        _ => Sample::I16(ffmpeg_next::format::sample::Type::Packed),
    }
}

/// The seam layout of a libav channel layout: a named one when it matches, a
/// discrete channel count otherwise.
pub(crate) fn channel_layout(layout: LibavChannelLayout) -> ChannelLayout {
    let channels = u16::try_from(layout.channels())
        .ok()
        .and_then(NonZeroU16::new);
    if layout == LibavChannelLayout::MONO {
        return ChannelLayout::Mono;
    }
    if layout == LibavChannelLayout::STEREO {
        return ChannelLayout::Stereo;
    }
    if layout == LibavChannelLayout::_5POINT1 || layout == LibavChannelLayout::_5POINT1_BACK {
        return ChannelLayout::Surround5_1;
    }
    if layout == LibavChannelLayout::_7POINT1 || layout == LibavChannelLayout::_7POINT1_WIDE {
        return ChannelLayout::Surround7_1;
    }
    match channels {
        Some(count) => ChannelLayout::Discrete(count),
        None => ChannelLayout::Mono,
    }
}

/// The libav channel layout for a seam one.
pub(crate) fn libav_channel_layout(layout: ChannelLayout) -> LibavChannelLayout {
    match layout {
        ChannelLayout::Mono => LibavChannelLayout::MONO,
        ChannelLayout::Stereo => LibavChannelLayout::STEREO,
        ChannelLayout::Surround5_1 => LibavChannelLayout::_5POINT1,
        ChannelLayout::Surround7_1 => LibavChannelLayout::_7POINT1,
        ChannelLayout::Discrete(count) => {
            LibavChannelLayout::default(i32::from(count.get().max(1)))
        }
        _ => LibavChannelLayout::STEREO,
    }
}

/// The seam profile a libav one names, when the seam has a variant for it.
pub(crate) fn codec_profile(profile: Profile) -> Option<CodecProfile> {
    Some(match profile {
        Profile::H264(H264::Baseline) => CodecProfile::H264Baseline,
        Profile::H264(H264::ConstrainedBaseline) => CodecProfile::H264ConstrainedBaseline,
        Profile::H264(H264::Main) => CodecProfile::H264Main,
        Profile::H264(H264::Extended) => CodecProfile::H264Extended,
        Profile::H264(H264::High) => CodecProfile::H264High,
        Profile::H264(H264::High10) => CodecProfile::H264High10,
        Profile::H264(H264::High422) => CodecProfile::H264High422,
        Profile::H264(H264::High444 | H264::High444Predictive) => {
            CodecProfile::H264High444Predictive
        }
        Profile::HEVC(HEVC::Main) => CodecProfile::HevcMain,
        Profile::HEVC(HEVC::Main10) => CodecProfile::HevcMain10,
        Profile::HEVC(HEVC::MainStillPicture) => CodecProfile::HevcMainStillPicture,
        Profile::HEVC(HEVC::Rext) => CodecProfile::HevcRext,
        Profile::VP9(VP9::_0) => CodecProfile::Vp9Profile0,
        Profile::VP9(VP9::_1) => CodecProfile::Vp9Profile1,
        Profile::VP9(VP9::_2) => CodecProfile::Vp9Profile2,
        Profile::VP9(VP9::_3) => CodecProfile::Vp9Profile3,
        Profile::MPEG2(MPEG2::Simple) => CodecProfile::Mpeg2Simple,
        Profile::MPEG2(MPEG2::Main) => CodecProfile::Mpeg2Main,
        Profile::MPEG2(MPEG2::High) => CodecProfile::Mpeg2High,
        Profile::MPEG2(MPEG2::_422) => CodecProfile::Mpeg2FourTwoTwo,
        Profile::AAC(AAC::Low) => CodecProfile::AacLc,
        Profile::AAC(AAC::HE) => CodecProfile::AacHe,
        Profile::AAC(AAC::HEv2) => CodecProfile::AacHeV2,
        Profile::AAC(AAC::LD) => CodecProfile::AacLd,
        Profile::AAC(AAC::ELD) => CodecProfile::AacEld,
        _ => return None,
    })
}

/// The libav profile for a seam one, when this adapter can request it.
pub(crate) fn libav_profile(profile: CodecProfile) -> Option<Profile> {
    Some(match profile {
        CodecProfile::H264Baseline => Profile::H264(H264::Baseline),
        CodecProfile::H264ConstrainedBaseline => Profile::H264(H264::ConstrainedBaseline),
        CodecProfile::H264Main => Profile::H264(H264::Main),
        CodecProfile::H264Extended => Profile::H264(H264::Extended),
        CodecProfile::H264High => Profile::H264(H264::High),
        CodecProfile::H264High10 => Profile::H264(H264::High10),
        CodecProfile::H264High422 => Profile::H264(H264::High422),
        CodecProfile::H264High444Predictive => Profile::H264(H264::High444Predictive),
        CodecProfile::HevcMain => Profile::HEVC(HEVC::Main),
        CodecProfile::HevcMain10 => Profile::HEVC(HEVC::Main10),
        CodecProfile::HevcMainStillPicture => Profile::HEVC(HEVC::MainStillPicture),
        CodecProfile::HevcRext => Profile::HEVC(HEVC::Rext),
        CodecProfile::Vp9Profile0 => Profile::VP9(VP9::_0),
        CodecProfile::Vp9Profile1 => Profile::VP9(VP9::_1),
        CodecProfile::Vp9Profile2 => Profile::VP9(VP9::_2),
        CodecProfile::Vp9Profile3 => Profile::VP9(VP9::_3),
        CodecProfile::Mpeg2Simple => Profile::MPEG2(MPEG2::Simple),
        CodecProfile::Mpeg2Main => Profile::MPEG2(MPEG2::Main),
        CodecProfile::Mpeg2High => Profile::MPEG2(MPEG2::High),
        CodecProfile::Mpeg2FourTwoTwo => Profile::MPEG2(MPEG2::_422),
        CodecProfile::AacLc => Profile::AAC(AAC::Low),
        CodecProfile::AacHe => Profile::AAC(AAC::HE),
        CodecProfile::AacHeV2 => Profile::AAC(AAC::HEv2),
        CodecProfile::AacLd => Profile::AAC(AAC::LD),
        CodecProfile::AacEld => Profile::AAC(AAC::ELD),
        _ => return None,
    })
}

/// The colour description libav reports for one decoded picture, when all
/// three code points are ones this adapter knows.
pub(crate) fn colour(video: &ffmpeg_next::frame::Video) -> Option<ColourDescription> {
    let primaries = primaries_code(video.color_primaries())?;
    let transfer = transfer_code(video.color_transfer_characteristic())?;
    let matrix = matrix_code(video.color_space())?;
    let full_range = matches!(video.color_range(), ffmpeg_next::color::Range::JPEG);
    Some(ColourDescription::new(
        primaries, transfer, matrix, full_range,
    ))
}

/// Colour primaries names as ITU-T H.273 code points.
fn primaries_code(primaries: ffmpeg_next::color::Primaries) -> Option<u8> {
    use ffmpeg_next::color::Primaries;
    Some(match primaries {
        Primaries::BT709 => 1,
        Primaries::SMPTE170M => 6,
        Primaries::SMPTE240M => 7,
        Primaries::BT2020 => 9,
        Primaries::SMPTE428 => 10,
        Primaries::SMPTE431 => 11,
        Primaries::SMPTE432 => 12,
        Primaries::BT470M => 4,
        Primaries::BT470BG => 5,
        Primaries::Film => 8,
        Primaries::EBU3213 => 22,
        _ => return None,
    })
}

/// Transfer-characteristic names as ITU-T H.273 code points.
fn transfer_code(transfer: ffmpeg_next::color::TransferCharacteristic) -> Option<u8> {
    use ffmpeg_next::color::TransferCharacteristic;
    Some(match transfer {
        TransferCharacteristic::BT709 => 1,
        TransferCharacteristic::GAMMA22 => 4,
        TransferCharacteristic::GAMMA28 => 5,
        TransferCharacteristic::SMPTE170M => 6,
        TransferCharacteristic::SMPTE240M => 7,
        TransferCharacteristic::Linear => 8,
        TransferCharacteristic::Log => 9,
        TransferCharacteristic::LogSqrt => 10,
        TransferCharacteristic::IEC61966_2_4 => 11,
        TransferCharacteristic::BT1361_ECG => 12,
        TransferCharacteristic::IEC61966_2_1 => 13,
        TransferCharacteristic::BT2020_10 => 14,
        TransferCharacteristic::BT2020_12 => 15,
        TransferCharacteristic::SMPTE2084 => 16,
        TransferCharacteristic::SMPTE428 => 17,
        TransferCharacteristic::ARIB_STD_B67 => 18,
        _ => return None,
    })
}

/// Matrix-coefficient names as ITU-T H.273 code points.
fn matrix_code(space: ffmpeg_next::color::Space) -> Option<u8> {
    use ffmpeg_next::color::Space;
    Some(match space {
        Space::RGB => 0,
        Space::BT709 => 1,
        Space::FCC => 4,
        Space::BT470BG => 5,
        Space::SMPTE170M => 6,
        Space::SMPTE240M => 7,
        Space::YCGCO => 8,
        Space::BT2020NCL => 9,
        Space::BT2020CL => 10,
        Space::SMPTE2085 => 11,
        Space::ChromaDerivedNCL => 12,
        Space::ChromaDerivedCL => 13,
        Space::ICTCP => 14,
        _ => return None,
    })
}

/// A coded width or height as the seam's non-zero value.
pub(crate) fn coded_size(width: u32, height: u32) -> Option<(NonZeroU32, NonZeroU32)> {
    Some((NonZeroU32::new(width)?, NonZeroU32::new(height)?))
}
