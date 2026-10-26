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
