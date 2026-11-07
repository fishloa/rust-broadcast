use multimux::MultimuxError;
use std::error::Error as _;

#[test]
fn an_ingest_flv_failure_keeps_its_transmux_source() {
    let err = MultimuxError::from(transmux::FlvError::NoSupportedTrack);
    assert!(matches!(
        err,
        MultimuxError::Flv(transmux::FlvError::NoSupportedTrack)
    ));
    assert_eq!(
        err.source().map(ToString::to_string),
        Some(transmux::FlvError::NoSupportedTrack.to_string())
    );
}
