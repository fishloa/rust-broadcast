# Changelog

All notable changes to dash-runtime will be documented in this file.

## [Unreleased]

### Fixed
- `timing::utc_timing::sample_from_exchange`: saturate the exchange midpoint to `u64::MAX` instead of overflowing on extreme inputs.

### Added
- Initial core (design step W-c, `docs/dash-runtime/DESIGN.md`), not yet published.
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
- An MPD in which two Periods share a `@id` is rejected as invalid (ISO/IEC 23009-1 5.3.2.2: the identifier is unique in the Media Presentation). Found by the `broadcast-client` fault matrix: a refresh that duplicated the newest Period made the client play that Period's segments again and again. A refresh carrying one is handled like any unparsable refresh (the MPD already held stays in force); a first MPD carrying one is retried.
- A live segment answered `425` after its availability window closed is expired (`FailureClass::Expired`, the lane skips ahead) instead of being retried at the early-request cadence forever, when the clock is the one the origin announced through `UTCTiming`. A host or `Date` clock keeps the old behaviour, because a `425` is then the evidence that the clock is ahead (a host clock a minute fast still waits and catches up).
