#![doc = include_str!("../README.md")]

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/media-plane")]

extern crate alloc;

pub mod byte_merge;
pub mod byte_stage;
pub mod byte_tap;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub mod egress;
pub mod ids;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub mod ingress;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub mod retention;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub mod trunk;

pub use byte_merge::{ByteMerge, MergeError, MergePolicy, SourceId};
pub use byte_stage::ByteStage;
pub use byte_tap::{ByteTap, TapItem, TapPoint};
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub use egress::{
    AwaitPolicy, CachePolicy, EgressResponse, NegotiationOutcome, PushEgress, SegmentEgress,
    ServedEgress, TrackSelection,
};
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub use ingress::{
    AcceptOutcome, DEFAULT_MAX_PROGRAMS, DialAttempt, DialSupervisor, Dialer, HandshakePolicy,
    HealthState, IngestDriver, IngestSession, ListenDriver, Listener, ProgramId, ReconnectPolicy,
    SessionEvent, SessionId, run_dial, run_listen,
};
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub use retention::{Retention, RetentionDriver, SegmentLocation, SegmentSink, SinkOutcome};
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub use trunk::{
    ArchiveOverrun, EventAnchor, EventCursor, EventCursorItem, EventEntry, RetentionClass,
    SampleCursor, SampleCursorItem, SegmentCursor, SegmentCursorItem, SegmentEntry, Trunk,
    TrunkConfig, TrunkWriter,
};
