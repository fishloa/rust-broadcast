//! Sans-IO SCTE-35 server-side ad insertion (SSAI) session core — issue #929.
//!
//! Per-session ad-break state ([`session`]), a pluggable ad-decision trait
//! ([`decision`]), SCTE-35 splice-point conditioning against real segment or
//! keyframe boundaries ([`splice`]), and per-session HLS Interstitial
//! (`EXT-X-DATERANGE CLASS="com.apple.hls.interstitial"`,
//! draft-pantos-hls-rfc8216bis Appendix D, transcribed at
//! `broadcast-hls/docs/interstitials.md`) playlist rendering over
//! [`broadcast_hls::MediaPlaylist`] ([`playlist`]).
//!
//! Mirrors the `rtsp-runtime`/`hls-runtime` split the rest of the workspace
//! uses: this crate is the driveable, sans-IO core; an HTTP-facing adapter
//! (e.g. a `multimux` output) is separate, later work that layers on top of
//! it rather than living inside it.
//!
//! ## What this crate is not
//!
//! - **No HTTP client, no VAST/VMAP.** [`decision::AdDecisionProvider`] is
//!   the only extension point for "which ad to play" — implementing it (and
//!   doing whatever network I/O that requires) is entirely the caller's
//!   job. This crate never makes a network call.
//! - **No media manipulation.** Splicing the actual CMAF/TS bytes at the
//!   conditioned splice point is `transmux`'s job (its splice/SSAI IR
//!   transforms and PTS/DTS rebase); this crate only decides *where* that
//!   splice point is and *what* the per-session manifest should say about
//!   it.
//! - **No per-viewer media cursor.** Every viewer in a break watches one of
//!   a small set of pre-conditioned ad assets, and outside a break sees
//!   byte-identical primary content — see the issue's design-decision
//!   comment. What differs per session is a small [`session::BreakState`]
//!   record, never a `media_plane::Trunk` cursor; `media-plane`'s
//!   O(N)-in-cursor-count rule is about the shared media rings, not this.
//! - **No `multimux` HTTP wiring.** That adapter is future work, not this
//!   crate.
//!
//! `no_std` + `alloc`.
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
pub use playlist::{INTERSTITIAL_CLASS, InterstitialDateRange, render_session_playlist};
pub use session::{BreakState, SessionStore};
pub use splice::{ConditionedSplicePoint, SnapDirection, condition_splice_point};
