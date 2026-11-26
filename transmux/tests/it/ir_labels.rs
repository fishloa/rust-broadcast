//! The `name()` spec label of every IR field enum (the #204 convention) and
//! its `Display` delegation.
//!
//! These labels are part of the logged and serialized surface (and the
//! `impl_spec_display!` macro is label-free: it only delegates to `name()`),
//! so a blanked or changed label must fail here, not ship silently.

use broadcast_bitstream::facts::{ActiveFormatDescription, TransferCharacteristics};
use broadcast_bitstream::interlace::{PicStruct, SourceScanType, VideoScan};
use transmux::hdr10plus::OverlapProcessOption;
use transmux::ir::{
    AbandonReason, AfdPersistence, AudioRole, AudioType, DataCarriage, DiscontinuityKind,
    EditorialClassification, InputDegradation, MixType, SubtitleFormat, UserDataCarriage,
};

/// Assert both `name()` and its `Display` form (which delegates to it; the
/// byte-bearing catch-alls render as `"{name}(0x..)"`).
macro_rules! label {
    ($value:expr, $name:expr) => {{
        let value = $value;
        assert_eq!(value.name(), $name);
        assert_eq!(format!("{value}"), $name);
    }};
    ($value:expr, $name:expr, $display:expr) => {{
        let value = $value;
        assert_eq!(value.name(), $name);
        assert_eq!(format!("{value}"), $display);
    }};
}

#[test]
fn audio_type_labels() {
    label!(AudioType::Undefined, "undefined");
    label!(AudioType::CleanEffects, "clean effects");
    label!(AudioType::HearingImpaired, "hearing impaired");
    label!(
        AudioType::VisualImpairedCommentary,
        "visual impaired commentary"
    );
    label!(AudioType::Other(0x04), "other", "other(0x04)");
    label!(AudioType::Other(0x7F), "other", "other(0x7F)");
}

#[test]
fn mix_type_labels() {
    label!(MixType::Dependent, "dependent");
    label!(MixType::Independent, "independent");
}

#[test]
fn editorial_classification_labels() {
    label!(EditorialClassification::MainAudio, "main audio");
    label!(
        EditorialClassification::AudioDescription,
        "audio description"
    );
    label!(
        EditorialClassification::CleanAudioHearingImpaired,
        "clean audio for the hearing impaired"
    );
    label!(EditorialClassification::SpokenSubtitles, "spoken subtitles");
    label!(
        EditorialClassification::DependentParametricData,
        "dependent parametric data"
    );
    label!(
        EditorialClassification::GeneralAudience,
        "general audience supplementary audio"
    );
    label!(
        EditorialClassification::UserDefined(0x18),
        "user defined",
        "user defined(0x18)"
    );
    label!(
        EditorialClassification::Reserved(0x05),
        "reserved",
        "reserved(0x05)"
    );
}

#[test]
fn audio_role_labels() {
    label!(AudioRole::Main, "main");
    label!(AudioRole::AudioDescription, "audio description");
    label!(AudioRole::HearingImpaired, "hearing impaired");
    label!(AudioRole::Unknown, "unknown");
}

#[test]
fn video_scan_labels() {
    label!(VideoScan::Progressive, "progressive");
    label!(VideoScan::Paff, "PAFF");
    label!(VideoScan::Mbaff, "MBAFF");
    label!(VideoScan::FieldSequence, "field_seq");
}

#[test]
fn pic_struct_labels() {
    label!(PicStruct::Frame, "frame");
    label!(PicStruct::TopField, "top field");
    label!(PicStruct::BottomField, "bottom field");
    label!(PicStruct::TopBottom, "top, bottom");
    label!(PicStruct::BottomTop, "bottom, top");
    label!(PicStruct::TopBottomTop, "top, bottom, top repeated");
    label!(PicStruct::BottomTopBottom, "bottom, top, bottom repeated");
    label!(PicStruct::FrameDoubling, "frame doubling");
    label!(PicStruct::FrameTripling, "frame tripling");
    label!(
        PicStruct::TopPairedWithPreviousBottom,
        "top paired with previous bottom"
    );
    label!(
        PicStruct::BottomPairedWithPreviousTop,
        "bottom paired with previous top"
    );
    label!(
        PicStruct::TopPairedWithNextBottom,
        "top paired with next bottom"
    );
    label!(
        PicStruct::BottomPairedWithNextTop,
        "bottom paired with next top"
    );
}

#[test]
fn source_scan_type_labels() {
    label!(SourceScanType::Interlaced, "interlaced");
    label!(SourceScanType::Progressive, "progressive");
    label!(SourceScanType::Unknown, "unknown");
}

#[test]
fn overlap_process_option_labels() {
    label!(OverlapProcessOption::WeightedAverage, "weighted average");
    label!(OverlapProcessOption::Layering, "layering");
}

#[test]
fn data_carriage_labels() {
    label!(DataCarriage::Pes, "pes");
    label!(DataCarriage::Sections, "sections");
}

#[test]
fn subtitle_format_labels() {
    label!(SubtitleFormat::Ttml, "TTML (stpp)");
    label!(SubtitleFormat::WebVtt, "WebVTT (wvtt)");
    label!(SubtitleFormat::DvbBitmap, "DVB bitmap subtitle");
    label!(SubtitleFormat::Teletext, "Teletext");
}

/// One assertion per arm: a constant replacement of the whole function would
/// otherwise only have to match one row.
#[test]
fn transfer_characteristics_labels() {
    label!(
        TransferCharacteristics::Reserved(3),
        "reserved",
        "reserved(0x03)"
    );
    label!(TransferCharacteristics::Bt709, "Rec. ITU-R BT.709-6");
    label!(TransferCharacteristics::Unspecified, "Unspecified");
    label!(
        TransferCharacteristics::Gamma22,
        "Assumed display gamma 2.2"
    );
    label!(
        TransferCharacteristics::Gamma28,
        "Assumed display gamma 2.8"
    );
    label!(
        TransferCharacteristics::Bt601,
        "Rec. ITU-R BT.601-7 525 or 625"
    );
    label!(
        TransferCharacteristics::Smpte240M,
        "SMPTE 240M (1999, historical)"
    );
    label!(
        TransferCharacteristics::Linear,
        "Linear transfer characteristics"
    );
    label!(
        TransferCharacteristics::Log100,
        "Logarithmic transfer characteristic (100:1 range)"
    );
    label!(
        TransferCharacteristics::Log316,
        "Logarithmic transfer characteristic (100 * Sqrt(10) : 1 range)"
    );
    label!(TransferCharacteristics::Iec61966_2_4, "IEC 61966-2-4");
    label!(
        TransferCharacteristics::Bt1361Extended,
        "Rec. ITU-R BT.1361-0 extended colour gamut system (historical)"
    );
    label!(
        TransferCharacteristics::Iec61966_2_1,
        "IEC 61966-2-1 sRGB (with matrix_coefficients equal to 0) / IEC 61966-2-1 sYCC \
         (with matrix_coefficients equal to 5)"
    );
    label!(
        TransferCharacteristics::Bt2020_10Bit,
        "Rec. ITU-R BT.2020-2 (10 bit system)"
    );
    label!(
        TransferCharacteristics::Bt2020_12Bit,
        "Rec. ITU-R BT.2020-2 (12 bit system)"
    );
    label!(
        TransferCharacteristics::SmpteSt2084,
        "SMPTE ST 2084 (2014) / Rec. ITU-R BT.2100-2 PQ system"
    );
    label!(
        TransferCharacteristics::SmpteSt428_1,
        "SMPTE ST 428-1 (2006)"
    );
    label!(
        TransferCharacteristics::AribStdB67,
        "ARIB STD-B67 / Rec. ITU-R BT.2100-2 HLG system"
    );
}

#[test]
fn active_format_description_labels() {
    label!(ActiveFormatDescription::Unknown, "AFD unknown");
    label!(ActiveFormatDescription::Box16x9Top, "box 16:9 (top)");
    label!(ActiveFormatDescription::Box14x9Top, "box 14:9 (top)");
    label!(
        ActiveFormatDescription::BoxWiderThan16x9Centre,
        "box > 16:9 (centre)"
    );
    label!(
        ActiveFormatDescription::SameAsCodedFrame,
        "same as coded frame"
    );
    label!(ActiveFormatDescription::Centre4x3, "4:3 (centre)");
    label!(ActiveFormatDescription::Centre16x9, "16:9 (centre)");
    label!(ActiveFormatDescription::Centre14x9, "14:9 (centre)");
    label!(
        ActiveFormatDescription::Centre4x3ShootProtect14x9,
        "4:3 (with shoot and protect 14:9 centre)"
    );
    label!(
        ActiveFormatDescription::Centre16x9ShootProtect14x9,
        "16:9 (with shoot and protect 14:9 centre)"
    );
    label!(
        ActiveFormatDescription::Centre16x9ShootProtect4x3,
        "16:9 (with shoot and protect 4:3 centre)"
    );
    label!(
        ActiveFormatDescription::Reserved(5),
        "reserved",
        "reserved(0x05)"
    );
}

#[test]
fn user_data_carriage_labels() {
    label!(UserDataCarriage::AvcSei, "AVC SEI");
    label!(UserDataCarriage::HevcPrefixSei, "HEVC prefix SEI");
    label!(
        UserDataCarriage::Mpeg2PictureUserData,
        "MPEG-2 picture user data"
    );
}

#[test]
fn afd_persistence_labels() {
    label!(AfdPersistence::UntilReplaced, "until replaced");
    label!(
        AfdPersistence::ResetAtRandomAccess,
        "reset at random access"
    );
}

#[test]
fn discontinuity_kind_labels() {
    label!(DiscontinuityKind::Signalled, "signalled");
    label!(DiscontinuityKind::TimelineReanchored, "timeline reanchored");
    label!(
        DiscontinuityKind::BudgetExceeded { bytes: 4 },
        "budget exceeded"
    );
}

#[test]
fn abandon_reason_labels() {
    label!(AbandonReason::ConfigUnrecoverable, "config unrecoverable");
    label!(AbandonReason::BudgetExceeded, "budget exceeded");
}

#[test]
fn input_degradation_labels() {
    label!(InputDegradation::TransportError, "transport error");
    label!(
        InputDegradation::ContinuityGap {
            expected: 1,
            got: 3
        },
        "continuity gap"
    );
    label!(
        InputDegradation::InvalidParameterSet,
        "invalid parameter set"
    );
    label!(
        InputDegradation::MalformedUnit { position: 7 },
        "malformed unit"
    );
    label!(
        InputDegradation::SyncLoss {
            offset: 0,
            dropped_bytes: 188
        },
        "sync loss"
    );
    label!(
        InputDegradation::TruncatedPacket {
            offset: 0,
            dropped_bytes: 3
        },
        "truncated packet"
    );
    label!(InputDegradation::Scrambled, "scrambled");
    label!(
        InputDegradation::PesUnitTruncated {
            declared: 10,
            available: 4
        },
        "truncated PES unit"
    );
}
