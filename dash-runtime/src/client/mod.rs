//! The sans-IO DASH client for static (VOD) presentations
//! (`docs/dash-runtime/DESIGN.md` sections 1, 5 and 7).
//!
//! **Spec:** ISO/IEC 23009-1:2022 5.3.2 (Periods, continuity 5.3.2.4), 5.3.9
//! (segment addressing, `sidx` 5.3.9.5.4), 5.6 (BaseURL), Annex A.9 (playback
//! across Periods); ISO/IEC 14496-12 8.8.4 / 8.16.3 (`moof`/`mdat`, `sidx`).
//! Verbatim text: `private/specs-verified/dash/23009-1/`.
//!
//! [`DashClient`] is the `hls_runtime::client::HlsClient` analogue. The caller
//! drains [`Action`]s, performs the I/O, feeds the answers back and drains
//! [`Output`]s: one [`Manifest`](Output::Manifest) per Period, then the
//! [`Tracks`](Output::Tracks) and ordered [`Samples`](Output::Samples) demuxed
//! with `transmux::Fmp4Demux`. All selected AdaptationSets play on one
//! timeline (design decision D7); Periods play back to back.
//!
//! What it does, and does not:
//!
//! * Static and dynamic MPDs. A dynamic one needs a UTC clock
//!   ([`DashClient::set_wall_clock`], `UTCTiming` through
//!   [`Action::FetchUtcTiming`], or the response `Date` in [`ResponseMeta`]),
//!   gates every request on the segment's availability, refreshes the MPD
//!   and reports [`Output::ClockSynced`]; see `next_wake` for timers.
//! * fMP4/CMAF segments only; encrypted, MPEG-2 TS, WebM, subtitle and
//!   trick-mode sets are reported as [`Output::TrackDropped`], never
//!   misplayed.
//! * Retry policy is the caller's. [`DashClient::on_failure`] classifies a
//!   failure ([`FailureClass`]); the caller owns the backoff arithmetic and
//!   calls [`DashClient::retry`]. A resource that fails
//!   `DashConfig::max_segment_failures` times in a row ends the presentation
//!   with [`Output::Fatal`] (the `broadcast-client` per-segment contract for a
//!   VOD playlist).
//! * Init segments are fetched concurrently per Representation and shared by
//!   identical URL and range; `sidx` indexes are fetched before their
//!   Representation's segments are planned; every request is a typed byte
//!   range when the MPD says so.
//! * With `DashConfig::cross_origin_urls = false` every URL the MPD makes the
//!   client request must have the MPD's origin.
//!
//! No clock is read; the `now_us` arguments are the caller's monotonic clock.
//!
//! ```
//! use dash_runtime::client::{Action, DashClient, DashConfig, PullLimits};
//!
//! let mut client = DashClient::new("https://origin.example/vod/manifest.mpd", DashConfig::default(), PullLimits::default());
//! let Some(Action::FetchMpd { url, .. }) = client.poll(0) else { unreachable!() };
//! assert_eq!(url, "https://origin.example/vod/manifest.mpd");
//! // The MPD is requested once until the caller reports a failure and retries.
//! assert!(client.poll(0).is_none());
//! ```

mod action;
pub mod config;
mod demux;
mod engine;
mod error;
mod meta;
mod output;
mod utc_sampler;

pub use action::{Action, MpdValidators, RepId, ResourceId, UtcId, UtcMethod};
pub use config::{DashClockPolicy, DashConfig, DashConfigError, PullLimits, PullLimitsError};
pub use engine::{DashClient, StateSize};
pub use error::{Error, Failure, FailureClass, FailureKind, FatalKind, Rejection, Result};
pub use meta::{ResponseMeta, UtcResponse};
pub use output::{AudioRendition, DropCause, ManifestSelection, Notice, Output, VariantInfo};
