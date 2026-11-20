use broadcast_client::{
    ErrorKind, PauseReason, ResumeMode, StarveKind, Status, StreamKind, TrackId,
};

#[test]
fn stream_kind_labels_are_the_literal_strings() {
    for (kind, label) in [(StreamKind::Video, "video"), (StreamKind::Audio, "audio")] {
        assert_eq!(kind.name(), label);
        assert_eq!(kind.to_string(), label);
    }
}

#[test]
fn resume_mode_labels_are_the_literal_strings() {
    for (mode, label) in [(ResumeMode::Range, "range"), (ResumeMode::Full, "full")] {
        assert_eq!(mode.name(), label);
        assert_eq!(mode.to_string(), label);
    }
}

#[test]
fn pause_reason_labels_are_the_literal_strings() {
    for (reason, label) in [
        (PauseReason::NoData, "no data"),
        (PauseReason::DataArriving, "data arriving"),
        (PauseReason::Discontinuity, "discontinuity"),
        (PauseReason::Reprime, "reprime"),
    ] {
        assert_eq!(reason.name(), label);
        assert_eq!(reason.to_string(), label);
    }
}

#[test]
fn starve_kind_labels_are_the_literal_strings() {
    for (kind, label) in [(StarveKind::Data, "data"), (StarveKind::NoData, "no-data")] {
        assert_eq!(kind.name(), label);
        assert_eq!(kind.to_string(), label);
    }
}

#[test]
fn status_labels_are_the_literal_strings() {
    for (status, label) in [
        (Status::Establishing, "establishing"),
        (Status::Live, "live"),
        (
            Status::Stalled {
                stalled: core::time::Duration::from_secs(1),
            },
            "stalled",
        ),
        (Status::Reconnecting { attempt: 2 }, "reconnecting"),
        (Status::NotFound, "not-found"),
        (Status::WaitingForCredentials, "waiting-for-credentials"),
        (Status::AwaitingTracks, "awaiting-tracks"),
        (Status::Ended, "ended"),
        (
            Status::RequestStalled {
                resume: ResumeMode::Range,
            },
            "request-stalled",
        ),
    ] {
        assert_eq!(status.name(), label);
        assert_eq!(status.to_string(), label);
    }
}

#[test]
fn ends_connection_is_true_only_for_link_ending_kinds() {
    for kind in [
        ErrorKind::SrtRejected {
            reason: "bad-secret",
        },
        ErrorKind::SrtHandshakeTimeout,
        ErrorKind::SrtPeerIdle,
        ErrorKind::RequestRefused {
            method: "DESCRIBE",
            status: "Service Unavailable".into(),
        },
    ] {
        assert!(kind.ends_connection(), "{kind:?}");
    }
    for kind in [
        ErrorKind::MpdParse { reason: "xml" },
        ErrorKind::SegmentFetchFailed { status: None },
        ErrorKind::BufferInputsDropped,
        ErrorKind::UnknownRenderedTrack {
            track_id: TrackId::new(9),
        },
    ] {
        assert!(!kind.ends_connection(), "{kind:?}");
    }
}
