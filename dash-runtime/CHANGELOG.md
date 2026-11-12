# Changelog

All notable changes to dash-runtime will be documented in this file.

## [Unreleased]
### Added
- `mpd::base_url` (`resolve`, `resolve_chain`, `first_forbidden_char`): RFC 3986 §5 URL-reference and `BaseURL`-chain resolution over `url::Url::join`, moved from `transmux`, whose only caller was this crate; `mpd::urls` now resolves through it instead of `transmux` (#2355).
### Changed
- BREAKING: `SegmentPlan::index_at_or_before`, `available_from_us` and `available_until_us` return `Result<Option<_>, AddressError>` instead of folding an addressing failure into `None`.
- BREAKING: the MPD model, parser and `SegmentTemplate` grammar live in the new `broadcast-dash` crate. `mpd::model`, `mpd::descriptors`, `mpd::inherit` and `address::template` are re-exports of `broadcast_dash` (so `dash_runtime::mpd::Mpd` and friends keep resolving), and `limits::Limits` is a re-export of `broadcast_dash::limits::Limits`. Code that imported from `dash_runtime::mpd`, `dash_runtime::address::template` or `dash_runtime::limits` keeps compiling; new code should depend on `broadcast-dash` directly. `error::{Limit, MpdError, Unsupported}` are re-exports of `broadcast_dash::error`. The `xs:dateTime`/NTP readers moved to `broadcast_dash::xs_datetime` (still at `timing::datetime`) and the HTTP `Date` reader gained its own module, `timing::http_date`; `TimingError::DateTime` now carries only `broadcast_dash::xs_datetime::DateTimeError`, and a bad `Date` value is `TimingError::HttpDate(..)`.
- The MPD reader's namespace and hostile-input bound tests lost when the reader moved out of transmux are back, against `dash_runtime::mpd::Mpd` (issue #2340), and the DASH validator golden now has a test that a refused MPD is recorded as `dash-parse-error`, never as `clean`.
- BREAKING: `address::template::substitute` returns `Result<String, TemplateError>` and `SegmentPlan::get` returns `Result<Option<SegmentRef>, AddressError>`. A template naming `$Number$`/`$Time$` the caller has no value for, or a template that does not parse, is now an error instead of silently dropping the identifier (yielding a URL such as `chunk-.m4s`) or an empty string, which fetched the wrong segment.
### Fixed
- A `SegmentTimeline` `S` with a negative `@r` followed by an `S@t` that falls inside its last repeat now ends that last Segment at the `S@t` instead of letting it overrun the next run; the overlap made a later Segment's `availability_from` earlier than its predecessor's (found by the `dash_address` fuzz target).
- A segment-address failure after the initial plan (for example an `index * duration` overflow that index 0 does not hit) drops the Representation with `DropCause::Addressing`, or ends the presentation with `FatalKind::UnsupportedAddressing` when it was the only track, instead of the client silently waiting forever.
- A list presentation (`MPD@type="list"`) is refused with `FatalKind::Unsupported(Unsupported::PresentationTypeList)` instead of `MpdInvalid { reason: "presentation_type_list" }`; the refusal now carries the typed error every other unsupported feature uses.
- `init_reference` now propagates a non-`IndexRequired` plan error as `DropCause::Addressing` instead of treating the representation as having no init segment; `note_exhausted` drops the affected lane with the typed cause instead of silently excluding it from the tail-wait computation.
- `@duration` on a parent level and a `SegmentTimeline` on a child level (or the reverse) are refused as `InheritError::DurationAndTimeline`, as two attributes on one level already were: ISO/IEC 23009-1 5.3.9.2.1 forbids both in a Representation's effective Segment information, and the child's timeline used to silently replace the inherited duration.
- A Representation inherits its AdaptationSet's `SegmentTemplate` timeline when it declares none of its own; the segment plan used to see no timeline there (found by pinning media-doctor's DASH validator on the ISO 23009-1 examples, where example G22 produced 18 timeline findings instead of 3).
### Added
- `address::template::match_url`: reverse segment-template matching (an uploaded segment URL to its representation id and `$Number$`/`$Time$`/`$Bandwidth$`), moved here from transmux so dash-runtime is the one MPD reader (#2340).
- `PresentationType::List` and `Period::imported_mpd` (`ImportedMPD`): the `MPD@type="list"` value and the Period element ISO/IEC 23009-1 6th edition adds. `DashClient` refuses a list MPD with its typed unsupported error rather than playing it as static.

## [0.4.0] - 2026-11-10
### Fixed
- A whole-second `xs:dateTime` (an `http-xsdate`/`http-iso` body or a `direct` value) is centred half a second later with a half-second uncertainty, as a whole-second HTTP `Date` already was; a server that floors the time left the clock up to 0.5 s behind. A value with a fraction is taken as stated.
- A `sidx` reference of size 0 still advances the subsegment timeline by its `subsegment_duration`; every later subsegment used to start early by the skipped duration.

## [0.3.0] - 2026-11-07
### Added
- `Limits::xml_limits()`: the XML-level bounds as the shared `broadcast_common::xml_chars::XmlLimits`; `From<XmlLimit> for Limit`.

### Documentation
- README brought to the common section skeleton (what it does, standards, quick start, what is implemented, design notes, related crates, feature flags, MSRV and licence); the licence heading is spelled consistently (c6 C6-10).
### Changed
- The MPD reader's size, depth, element-count and attribute bounds are enforced through `broadcast_common::xml_chars::XmlBudget` instead of hand-kept counters; `Limits` and every default are unchanged.
- `timing::ticks_to_us` and the plan's microsecond-to-tick ceiling use `broadcast_common::timescale::{to_micros, from_micros}` (c6 F7).
- The `live_fuzz_crash_regression` test and the `dash_client_live` fuzz target share one driver module (`tests/it/common/live_fuzz_driver.rs`) instead of a 322-line copy (c3 d:T1).
- The crate-root docs are the README (`#![doc = include_str!("../README.md")]`) instead of a hand-synced `//!` copy (c3 DOC1).
- The MPD reader's attribute loop is `broadcast_common::xml_chars::read_attributes_checked` with the attribute count and size limits as its hook; an over-limit attribute count is now reported after (not before) a malformed value on the same start tag (c2 H8).
- `timing::ticks_to_us` rescales through `broadcast_common::timescale` (same floor rounding) (c2 H6).
- XML 1.0 character-data validation (`Char` production, entity/character references) comes from `broadcast_common::xml_chars` instead of a private copy (dedupe D5).
### Removed
- BREAKING: Unused public items `mpd::is_unsupported_urlparam` and `mpd::url_query_info` (c2 TH1).
### Dependencies
- Sibling crates move to a new epoch in this release wave: `broadcast-common` 10 -> 11, `container-probe` 0.2 -> 0.3, `transmux` 0.29 -> 0.30. The `Cargo.toml` requirements name the new versions; no other manifest change.

## [0.2.0] - 2026-11-03

### Changed
- Builds against `transmux` 0.29 (was 0.28); no functional change in this crate.

## [0.1.0] - 2026-11-01
### Changed
- `TrackId` assignment is now stable across Periods: ids are keyed by (media kind, AdaptationSet identity: id/group/lang/role) and allocated on first encounter, so a track that continues keeps its id and a Period that drops a kind does not hand that kind's id to a different kind in a later Period (previously ids restarted at 1 each Period). Visible numeric ids for multi-Period presentations can therefore differ from the previous per-Period numbering; consumers must key state by `TrackId` as announced in `Output::Tracks`, not by position. BREAKING for callers that hard-coded per-Period ids. Only the current and previous Period's identities are remembered (`StateSize::track_identities`), so a long live presentation that renumbers AdaptationSet `@id` does not grow state; ids are never recycled, and an identity absent for two Periods gets a fresh id when it returns.

### Added
- Initial release: the core of `docs/dash-runtime/DESIGN.md`, the static and live clients below.
- `mpd`: consume-only MPD model and bounded namespace-aware parser (`Limits`), attribute inheritance down Period > AdaptationSet > Representation, BaseURL alternates (DVB priority) and URL resolution, descriptors (Role, EssentialProperty, ContentProtection, UTCTiming, `urn:mpeg:dash:urlparam:2014`).
- `address`: lazy `SegmentPlan` for `SegmentTemplate` (width tags, `$$`), `SegmentTimeline` (run table, negative `@r`, `@n`), `SegmentList` and `SegmentBase` + `sidx`, with per-segment availability windows; `plan_representation` as the one-call entry point.
- `timing`: `UtcClock` and `UTCTiming` sampling, `xs:dateTime` (accepted subset), HTTP `Date` and NTP readers, PeriodStart derivation, availability and live-edge arithmetic.
- `selection`: pure Representation and AdaptationSet choice by role, language, codec and bandwidth.
- `timing::datetime::parse_http_date` accepts the three HTTP-date forms of RFC 9110 5.6.7 (IMF-fixdate, RFC 850, asctime); the `Retry-After` HTTP-date path of `broadcast-client` shares it.
- `client::config`: `DashConfig` (clock ladder, `UTCTiming` timeouts, early-request cadence, MPD refresh floor, LL-DASH and cross-origin switches) with defaults and `validate()`, and `DashClockPolicy` (design step W-b, `docs/dash-runtime/DESIGN.md` 6.4).
- Fuzz targets `dash_mpd_parse` and `dash_address`; examples `list_segments` and `live_window`.
- `client` (design step W-d1, DESIGN sections 1, 5 and 7): `DashClient` for static presentations. Action/output protocol mirroring `HlsClient` (`FetchMpd`/`FetchInit`/`FetchIndex`/`FetchSegment`; `Manifest`, `PeriodChanged`, `Tracks`, `Init`, `Samples`, `Discontinuity`, `TrackDropped`, `TrackSkipped`, `FragmentDamaged`, `Notice`, `Fatal`, `EndOfStream`), one timeline for all selected sets, init and `sidx` fetch with identity sharing, a bounded in-flight window with in-order delivery, multi-Period playback with `period-continuity`, `FailureClass` (retry/give-up/end-of-Period-tail) with the `broadcast-client` per-segment contract, BaseURL alternate rotation, `urn:mpeg:dash:urlparam:2014` query, `cross_origin_urls` pin, and `select_representation`. 
- Live client (design step W-d2, DESIGN sections 3, 5.2-5.5 and 12): `DashClient` plays dynamic MPDs. The clock ladder (`UTCTiming` sampled through `Action::FetchUtcTiming`, `DashClient::set_wall_clock`, the MPD response `Date`; typed `FatalKind::ClockUnavailable`), `Output::ClockSynced`, join at the presentation delay on each Representation's own timeline, availability gating (`Action` is only emitted from a segment's availability start), `FailureClass::{NotYetAvailable, Expired, SegmentSkipped}`, `Output::SegmentLost`, `Notice::{FellBehind, LowLatencyIgnored, UtcSchemeUnsupported, UtcCrossOrigin}`, MPD refresh with `ResponseMeta`, `MpdValidators`, `on_mpd_with`, `on_mpd_unchanged`, `on_utc_response`, `on_utc_failure`, `next_wake`, `awaits`, `state_size`, stale-`publishTime` and byte-identical detection, `Location`/redirect update URL, MPD Reset, dynamic-to-static, Period addition and expiry, Representation switching on a live presentation. `Error::LiveNotYetSupported` and `FatalKind::LiveNotYetSupported` are removed; `Action::FetchMpd` gained a `validators` field and `on_mpd_failure` returns the `FailureClass`.
- Tests: the `World` simulation (`tests/common/world.rs`, `synth.rs`, `capture.rs`) with live ladder, refresh, Period, failure, capture, long-run, matrix and soak suites; live mutation table; fuzz target `dash_client_live`; example `play_live`.
- Dependency on `container-probe` (`segment::media_fragment`/`progressive_movie`).
- Tests: loopback host over `fixtures/dash/**`, demux-oracle equality for every static fixture, period/continuity/tail behaviour, an 11-fault failure matrix, 48-mutant table; example `play_static`; fuzz target `dash_client_static`.

### Fixed
- `TimelineIndex::build` rejects an `S` whose `@d` is zero with `AddressError::ZeroSegmentDuration` (ISO/IEC 23009-1 5.3.9.6.1). The MPD parser already refused it, but `TimelineEntry::new(0)` and the public `d` field let a hand-built timeline reach the run arithmetic and divide by zero.
- An MPD in which two Periods share a `@id` is rejected as invalid (ISO/IEC 23009-1 5.3.2.2: the identifier is unique in the Media Presentation). Found by the `broadcast-client` fault matrix: a refresh that duplicated the newest Period made the client play that Period's segments again and again. A refresh carrying one is handled like any unparsable refresh (the MPD already held stays in force); a first MPD carrying one is retried.
- Adversarial DASH client review fixes: reset presentations now refetch init bytes; a transiently invalid timeline no longer silences a track; timeline `@t` tolerates a one-tick shortfall from nominal durations; codec policy is chosen across adaptation sets; `@endNumber` uses saturating arithmetic; zero-size `sidx` references are skipped; replan marks forward jumps; early-segment bookkeeping is released on give-up/skip/end-of-tail.
- A live segment answered `425` after its availability window closed is expired (`FailureClass::Expired`, the lane skips ahead) instead of being retried at the early-request cadence forever, when the clock is the one the origin announced through `UTCTiming`. A host or `Date` clock keeps the old behaviour, because a `425` is then the evidence that the clock is ahead (a host clock a minute fast still waits and catches up).
- With `cross_origin_urls` off, an MPD `<Location>` and the redirect `final_url` the host reports (live refresh and static open alike) are no longer followed or adopted when they leave the MPD's origin (scheme, host, port); a hostile MPD could otherwise steer every refresh to `http://169.254.169.254/...`. A `Location` or `final_url` whose scheme is not `http`/`https` (`file:`, `data:`, `ftp:`) is never used, with the pin on or off. A refused value falls back to the original MPD URL.
- Init, index and media URLs with a scheme other than `http`/`https` are dropped when the Representation is built: `file:` and the like as `DropCause::UnusableUrl`, `data:` as `DropCause::Unsupported(Unsupported::DataUrl)`.
- With `cross_origin_urls` off, BaseURL alternates on a foreign origin are removed from the candidate list, so a pinned client is not made fatal (`UnsupportedAddressing { reason: "cross_origin" }`) by failing over onto one; a foreign alternate ranked first no longer hides a same-origin one.
