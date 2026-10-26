# Changelog

All notable changes to dash-runtime will be documented in this file.

## [Unreleased]

### Added
- Initial core (design step W-c, `docs/dash-runtime/DESIGN.md`), not yet published.
- `mpd`: consume-only MPD model and bounded namespace-aware parser (`Limits`), attribute inheritance down Period > AdaptationSet > Representation, BaseURL alternates (DVB priority) and URL resolution, descriptors (Role, EssentialProperty, ContentProtection, UTCTiming, `urn:mpeg:dash:urlparam:2014`).
- `address`: lazy `SegmentPlan` for `SegmentTemplate` (width tags, `$$`), `SegmentTimeline` (run table, negative `@r`, `@n`), `SegmentList` and `SegmentBase` + `sidx`, with per-segment availability windows; `plan_representation` as the one-call entry point.
- `timing`: `UtcClock` and `UTCTiming` sampling, `xs:dateTime` (accepted subset), HTTP `Date` and NTP readers, PeriodStart derivation, availability and live-edge arithmetic.
- `selection`: pure Representation and AdaptationSet choice by role, language, codec and bandwidth.
- `timing::datetime::parse_http_date` accepts the three HTTP-date forms of RFC 9110 5.6.7 (IMF-fixdate, RFC 850, asctime); the `Retry-After` HTTP-date path of `broadcast-client` shares it.
- `client::config`: `DashConfig` (clock ladder, `UTCTiming` timeouts, early-request cadence, MPD refresh floor, LL-DASH and cross-origin switches) with defaults and `validate()`, and `DashClockPolicy` (design step W-b, `docs/dash-runtime/DESIGN.md` 6.4).
- Fuzz targets `dash_mpd_parse` and `dash_address`; examples `list_segments` and `live_window`.
- `client` (design step W-d1, DESIGN sections 1, 5 and 7): `DashClient` for static presentations. Action/output protocol mirroring `HlsClient` (`FetchMpd`/`FetchInit`/`FetchIndex`/`FetchSegment`; `Manifest`, `PeriodChanged`, `Tracks`, `Init`, `Samples`, `Discontinuity`, `TrackDropped`, `TrackSkipped`, `FragmentDamaged`, `Notice`, `Fatal`, `EndOfStream`), one timeline for all selected sets, init and `sidx` fetch with identity sharing, a bounded in-flight window with in-order delivery, multi-Period playback with `period-continuity`, `FailureClass` (retry/give-up/end-of-Period-tail) with the `broadcast-client` per-segment contract, BaseURL alternate rotation, `urn:mpeg:dash:urlparam:2014` query, `cross_origin_urls` pin, and `select_representation`. A dynamic MPD is a typed `LiveNotYetSupported` (temporary, removed by W-d2).
- Dependency on `container-probe` (`segment::media_fragment`/`progressive_movie`).
- Tests: loopback host over `fixtures/dash/**`, demux-oracle equality for every static fixture, period/continuity/tail behaviour, an 11-fault failure matrix, 48-mutant table; example `play_static`; fuzz target `dash_client_static`.
