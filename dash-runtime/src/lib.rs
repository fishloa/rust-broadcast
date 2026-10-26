//! Sans-IO MPEG-DASH: the MPD model and parser, segment addressing, the UTC
//! clock and the live-edge arithmetic a DASH client is built on, and
//! [`client::DashClient`], a client state machine for static (VOD)
//! presentations.
//!
//! **Spec:** ISO/IEC 23009-1:2022 (clause 5.3 MPD, 5.3.9 Segment information,
//! 5.6 BaseURL, 5.8.5.7 UTCTiming, Annex A client behaviour, Annex I URL
//! parameters), DASH-IF IOP v4.3 (live timing) and ETSI TS 103 285 (BaseURL
//! priority). Own-words notes and the verified page references are in
//! `docs/specs/dash/` of the repository; decisions where the standard
//! contradicts itself are in `docs/dash-runtime/DESIGN.md` section 11.
//!
//! Nothing here does I/O or reads a clock. Every instant is an argument; the
//! caller fetches the MPD, the `sidx` and the segments, times its own
//! requests, and asks this crate what to fetch and when it becomes available.
//!
//! | Module | What it gives |
//! |---|---|
//! | [`mpd`] | [`mpd::Mpd`]: the consume-only model, a bounded namespace-aware parser, attribute inheritance down Period > AdaptationSet > Representation, BaseURL alternates and URL resolution |
//! | [`address`] | [`address::SegmentPlan`]: the lazy Segment list of a Representation (templates with width tags and `$$`, timelines with negative `@r` and `@n`, `SegmentList`, `SegmentBase` + `sidx`), with availability windows |
//! | [`timing`] | [`timing::clock::UtcClock`], `UTCTiming` sampling, `xs:dateTime` / HTTP `Date` / NTP readers, PeriodStart, availability and live-edge arithmetic |
//! | [`client`] | [`client::config::DashConfig`]: the DASH knobs of a client session (clock ladder, early-request cadence, refresh floor) |
//! | [`selection`] | Pure choice of AdaptationSet and Representation by role, language, codec and bandwidth |
//! | [`client`] | [`client::DashClient`]: plays a static MPD to ordered `Fmp4Demux` samples, with typed failure classes, Period continuity and an origin pin (design step W-d1) |
//!
//! # Format policy
//!
//! The model is consume-only: there is no MPD serializer. The workspace
//! round-trip invariant is met against `transmux`'s DASH generator by a
//! differential test (`tests/differential.rs`): everything the generator
//! writes is parsed, and the two parsers agree on every field both read, the
//! expanded segment numbers and times, and the resolved URLs.
//!
//! # Scope of 0.1
//!
//! The client plays static MPDs; a dynamic MPD is a typed
//! `LiveNotYetSupported` until the live client (design step W-d2).
//!
//! The core handles: static and dynamic MPDs, `SegmentTemplate` (`$Number$`, `$Time$`,
//! `$Bandwidth$`, `$RepresentationID$`), `SegmentTimeline`, `SegmentList`,
//! `SegmentBase` with a `sidx`, BaseURL alternates, multiple Periods,
//! `UTCTiming` (`direct`, `http-xsdate`, `http-iso`, `http-head`,
//! `http-ntp`). Typed `Unsupported` (the Representation is dropped, never
//! misread): segment sequences (`S@k`, `$SubNumber$`), `MPD@type="list"`,
//! `data:` addresses, open-ended byte ranges, hierarchical `sidx`.
//!
//! # Examples
//!
//! * `cargo run -p dash-runtime --example list_segments` prints the segment
//!   URLs of an MPD file.
//! * `cargo run -p dash-runtime --example live_window` prints the live edge
//!   of a dynamic MPD at a simulated clock.
//! * `cargo run -p dash-runtime --example play_static` plays a static MPD from
//!   the file system through `DashClient` and prints what a host receives.
//!
//! ```
//! use dash_runtime::address::{PlanOptions, plan_representation};
//! use dash_runtime::mpd::Mpd;
//! use dash_runtime::selection::RepRef;
//!
//! let xml = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static"
//!                   mediaPresentationDuration="PT6S">
//!   <Period><AdaptationSet mimeType="video/mp4">
//!     <SegmentTemplate timescale="1000" duration="2000" startNumber="0"
//!                      media="$RepresentationID$/$Number%03d$.m4s"/>
//!     <Representation id="v1" bandwidth="500000"/>
//!   </AdaptationSet></Period>
//! </MPD>"#;
//! let mpd = Mpd::parse(xml.as_bytes()).unwrap();
//! let planned = plan_representation(&mpd, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
//! let refs: Vec<_> = planned.plan.iter().map(|s| s.reference).collect();
//! assert_eq!(refs, ["v1/000.m4s", "v1/001.m4s", "v1/002.m4s"]);
//! ```
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod address;
pub mod client;
pub mod error;
pub mod limits;
pub mod mpd;
pub mod selection;
pub mod timing;
