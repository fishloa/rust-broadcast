#![doc = include_str!("../README.md")]
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod decision;
pub mod error;
pub mod playlist;
pub mod session;
pub mod splice;

pub use decision::{
    AdBreakDecision, AdDecisionProvider, AssetSource, BreakContext, RestrictMode, SnapMode,
};
pub use error::{Error, Result};
pub use playlist::{
    INTERSTITIAL_CLASS, InterstitialDateRange, SessionPlaylistBase, render_session_playlist,
};
pub use session::{BreakState, DEFAULT_MAX_SESSIONS, SessionStore};
pub use splice::{
    ConditionedSplicePoint, PTS_MODULUS_33, SnapDirection, condition_splice_point,
    condition_splice_point_wrapping,
};
