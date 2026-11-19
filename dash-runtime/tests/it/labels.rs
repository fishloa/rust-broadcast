//! The stable label vocabulary of the client: every `name()` that a host, a
//! metric or an event log matches on, pinned to its exact string.

use dash_runtime::client::{DropCause, FailureKind, FatalKind, Notice, Rejection, UtcMethod};
use dash_runtime::error::Unsupported;
use dash_runtime::ingest::{
    Addressing, IngestMethod, ManifestFormat, ObjectKind, SegmentDisposition,
};
use dash_runtime::selection::{DropReason, SetClass};
use dash_runtime::timing::clock::{ClockSource, RejectReason};
use dash_runtime::timing::period::PeriodKind;

#[test]
fn ingest_labels() {
    assert_eq!(IngestMethod::Post.name(), "POST");
    assert_eq!(IngestMethod::Put.name(), "PUT");
    assert_eq!(ManifestFormat::Dash.name(), "dash");
    assert_eq!(ManifestFormat::Hls.name(), "hls");
    assert_eq!(Addressing::Number.name(), "number");
    assert_eq!(Addressing::Time.name(), "time");
    assert_eq!(ObjectKind::Probe.name(), "probe");
    assert_eq!(
        ObjectKind::Manifest(ManifestFormat::Dash).name(),
        "manifest"
    );
    assert_eq!(
        ObjectKind::Header {
            representation_id: "V".to_owned()
        }
        .name(),
        "header"
    );
    assert_eq!(
        ObjectKind::Segment {
            representation_id: "V".to_owned(),
            number: Some(1),
            time: None,
        }
        .name(),
        "segment"
    );
    assert_eq!(SegmentDisposition::Accepted.name(), "accepted");
    assert_eq!(SegmentDisposition::Duplicate.name(), "duplicate");
    assert_eq!(SegmentDisposition::Regression.name(), "regression");
}

#[test]
fn selection_labels() {
    assert_eq!(SetClass::Video.name(), "video");
    assert_eq!(SetClass::Audio.name(), "audio");
    assert_eq!(SetClass::Text.name(), "text");
    assert_eq!(SetClass::Other.name(), "other");
    assert_eq!(DropReason::Text.name(), "text");
    assert_eq!(DropReason::NotMedia.name(), "not_media");
    assert_eq!(DropReason::Mp2tSegments.name(), "mp2t_segments");
    assert_eq!(DropReason::WebM.name(), "webm");
    assert_eq!(DropReason::TrickMode.name(), "trick_mode");
    assert_eq!(
        DropReason::ProtectedAlternativeExists.name(),
        "protected_alternative_exists"
    );
    assert_eq!(
        DropReason::UnsupportedEncryption.name(),
        "unsupported_encryption"
    );
    assert_eq!(
        DropReason::UnknownEssentialProperty.name(),
        "unknown_essential_property"
    );
    assert_eq!(
        DropReason::DependencyRepresentation.name(),
        "dependency_representation"
    );
    assert_eq!(DropReason::UnresolvedXlink.name(), "unresolved_xlink");
    assert_eq!(
        DropReason::InvalidSegmentInfo.name(),
        "invalid_segment_info"
    );
}

#[test]
fn request_labels() {
    assert_eq!(UtcMethod::Get.name(), "GET");
    assert_eq!(UtcMethod::Head.name(), "HEAD");
}

#[test]
fn failure_labels() {
    assert_eq!(FailureKind::Missing.name(), "missing");
    assert_eq!(FailureKind::TooEarly.name(), "too_early");
    assert_eq!(FailureKind::Server.name(), "server");
    assert_eq!(FailureKind::Transport.name(), "transport");
    assert_eq!(FailureKind::Invalid.name(), "invalid");
    assert_eq!(Rejection::NotMedia.name(), "not_media");
    assert_eq!(Rejection::InitUndecodable.name(), "init_undecodable");
    assert_eq!(Rejection::DemuxFailed.name(), "demux_failed");
    assert_eq!(Rejection::BadIndex.name(), "bad_index");
    assert_eq!(Rejection::WrongLength.name(), "wrong_length");
    assert_eq!(Rejection::TooLarge.name(), "too_large");
    assert_eq!(Rejection::NonFragmentedMp4.name(), "non_fragmented_mp4");
}

#[test]
fn fatal_labels() {
    assert_eq!(FatalKind::ClockUnavailable.name(), "clock_unavailable");
    assert_eq!(
        FatalKind::MpdInvalid { reason: "empty" }.name(),
        "mpd_invalid"
    );
    assert_eq!(
        FatalKind::Unsupported(Unsupported::DataUrl).name(),
        "data_url"
    );
    assert_eq!(
        FatalKind::UnsupportedAddressing {
            reason: "cross_origin"
        }
        .name(),
        "unsupported_addressing"
    );
}

#[test]
fn drop_cause_labels() {
    assert_eq!(DropCause::Selection(DropReason::Text).name(), "text");
    assert_eq!(DropCause::Selection(DropReason::WebM).name(), "webm");
    assert_eq!(DropCause::NoInitialization.name(), "no_initialization");
    assert_eq!(DropCause::UnboundedPlan.name(), "unbounded_plan");
    assert_eq!(
        DropCause::Addressing("index_required").name(),
        "index_required"
    );
    assert_eq!(DropCause::UnusableUrl.name(), "unusable_url");
    assert_eq!(DropCause::CrossOrigin.name(), "cross_origin");
    assert_eq!(
        DropCause::Unsupported(Unsupported::HierarchicalSidx).name(),
        "hierarchical_sidx"
    );
}

#[test]
fn notice_labels() {
    assert_eq!(
        Notice::UnsupportedEncryption.name(),
        "unsupported_encryption"
    );
    assert_eq!(Notice::EventStreamIgnored.name(), "event_stream_ignored");
    assert_eq!(Notice::PreselectionIgnored.name(), "preselection_ignored");
    assert_eq!(Notice::XlinkSkipped.name(), "xlink_skipped");
    assert_eq!(Notice::PatchIgnored.name(), "patch_ignored");
    assert_eq!(Notice::LowLatencyIgnored.name(), "low_latency_ignored");
    assert_eq!(Notice::FellBehind.name(), "fell_behind");
    assert_eq!(
        Notice::UtcSchemeUnsupported.name(),
        "utc_scheme_unsupported"
    );
    assert_eq!(Notice::UtcCrossOrigin.name(), "utc_cross_origin");
}

#[test]
fn clock_labels() {
    assert_eq!(ClockSource::MpdDate.name(), "mpd_date");
    assert_eq!(ClockSource::Host.name(), "host");
    assert_eq!(ClockSource::UtcTiming.name(), "utc_timing");
    assert_eq!(RejectReason::LowerTrust.name(), "lower_trust");
    assert_eq!(RejectReason::LessPrecise.name(), "less_precise");
}

#[test]
fn period_labels() {
    assert_eq!(PeriodKind::Regular.name(), "regular");
    assert_eq!(PeriodKind::EarlyTerminated.name(), "early_terminated");
    assert_eq!(PeriodKind::EarlyAvailable.name(), "early_available");
}

#[test]
fn display_is_the_label() {
    assert_eq!(SetClass::Audio.to_string(), "audio");
    assert_eq!(Notice::FellBehind.to_string(), "fell_behind");
    assert_eq!(ClockSource::Host.to_string(), "host");
}
