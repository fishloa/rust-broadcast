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
//! # What it does / what it does not
//!
//! **Does:** decides where a break splices (cue aligned to real boundaries, with
//! the actual drift reported), remembers which break each viewer session is in,
//! and renders what that viewer's playlist should say.
//!
//! **Does not:**
//!
//! - **No HTTP client, no VAST/VMAP.** [`decision::AdDecisionProvider`] is the
//!   only extension point for "which ad to play"; any network I/O is the
//!   caller's job.
//! - **No media manipulation.** Splicing CMAF/TS bytes is `transmux`'s job; this
//!   crate decides *where* and *what the manifest says*.
//! - **No per-viewer media cursor.** What differs per session is a small
//!   [`session::BreakState`] record, never a `media_plane::Trunk` cursor.
//! - **No DASH period insertion, no `multimux` wiring** (future adapter), and no
//!   SCTE-35 parsing: the core takes plain tick counts.
//!
//! # Standards supported
//!
//! | Standard | Edition | Scope | Status |
//! |---|---|---|---|
//! | IETF draft-pantos-hls-rfc8216bis Appendix D (Interstitials) | draft dated 1 May 2026 (vendored); an Internet-Draft | `EXT-X-DATERANGE` Interstitial schema: `CLASS`, `X-ASSET-URI`/`X-ASSET-LIST` (URI only), `X-RESUME-OFFSET`, `X-PLAYOUT-LIMIT`, `X-SNAP`, `X-RESTRICT` | Partial (no skip-button control D.3, no asset-list JSON schema) |
//! | ANSI/SCTE 35 2023r1 | 2023r1 | cue semantics as inputs ([`decision::BreakContext`]); 33-bit `pts_time` wrap handling | Partial (ticks in, no wire decoding; see `scte35-splice`) |
//!
//! # Features
//!
//! | Feature | Default | Effect |
//! |---|---|---|
//! | `std` | yes | Links `std`; off gives `#![no_std]` + `alloc` |
//! | `serde` | no | `serde` derives on the decision and break-state types |
//!
//! # Quick start
//!
//! ```rust
//! use ssai_runtime::{
//!     AdBreakDecision, BreakState, InterstitialDateRange, SessionStore, condition_splice_point,
//! };
//!
//! // A cue targets tick 900_000 (90 kHz); the real segment boundaries nearest it
//! // are at 897_000 and 904_500. Allow up to 5_000 ticks of drift.
//! let boundaries = [810_000_u64, 897_000, 904_500];
//! let point = condition_splice_point(900_000, &boundaries, 5_000).unwrap();
//! assert_eq!(point.snapped_pts, 897_000);
//! assert_eq!(point.delta_ticks, 3_000);
//!
//! // Whatever ad-decision service the caller uses hands back one decision.
//! let decision = AdBreakDecision::single_asset("ad-1", "https://ads.example/ad-1.m3u8");
//!
//! // Track the break for one viewer session ...
//! let mut store = SessionStore::new();
//! store
//!     .begin_break("viewer-42", BreakState::new(decision.clone(), point))
//!     .unwrap();
//! assert_eq!(store.len(), 1);
//!
//! // ... and render the interstitial EXT-X-DATERANGE tag line for their playlist.
//! let tag = InterstitialDateRange::from_decision(&decision, "2026-01-01T00:00:10.000Z", Some(30.0))
//!     .to_tag_line()
//!     .unwrap();
//! assert!(tag.contains("CLASS=\"com.apple.hls.interstitial\""));
//! assert!(tag.contains("X-ASSET-URI=\"https://ads.example/ad-1.m3u8\""));
//! ```
//!
//! # What is implemented
//!
//! - [`session`]: [`SessionStore`] maps a session id to one [`BreakState`].
//! - [`decision`]: [`AdDecisionProvider`], [`AdBreakDecision`], [`AssetSource`],
//!   [`SnapMode`], [`RestrictMode`], [`BreakContext`].
//! - [`splice`]: [`condition_splice_point`] and
//!   [`condition_splice_point_wrapping`] (33-bit clock, [`PTS_MODULUS_33`]),
//!   reporting a [`ConditionedSplicePoint`] with a [`SnapDirection`]. Real cues
//!   are frequently not IDR-aligned; the caller picks the tolerance.
//! - [`playlist`]: [`InterstitialDateRange`] renders and parses the tag;
//!   [`render_session_playlist`] and [`SessionPlaylistBase`] (render the base
//!   once, splice the tag line per viewer) both refuse a base without
//!   `EXT-X-PROGRAM-DATE-TIME`.
//!
//! # Design notes
//!
//! Sans-IO, `no_std` + `alloc`, `#![forbid(unsafe_code)]`, structured
//! [`Error`]s. Ad-decision-supplied strings containing `"`, CR or LF are rejected
//! (no tag injection). 33-bit arithmetic goes through
//! `broadcast_common::clock33`.
//!
//! # Related crates
//!
//! Depends on `broadcast-common` and `broadcast-hls`; media splicing is
//! `transmux`; the HTTP-facing origin is `multimux`. MSRV 1.95.0;
//! MIT OR Apache-2.0.
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
pub use session::{BreakState, SessionStore};
pub use splice::{
    ConditionedSplicePoint, PTS_MODULUS_33, SnapDirection, condition_splice_point,
    condition_splice_point_wrapping,
};
