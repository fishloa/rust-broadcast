# Changelog

All notable changes to `broadcast-dash` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking

- The wire-format contract types now come from `broadcast-core`.

## [0.3.0] - 2026-11-22
### Breaking

- **BREAKING:** `jiff` 0.2 -> 1.0. The `Mpd` availability and publish times and the `xs_datetime` readers and writers take and return jiff 1.x `Timestamp`s.
- **BREAKING:** builds against `broadcast-common` 14 (the `jiff` 1.0 epoch). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever its own API diff, so this release is a major-class bump; consumers must resolve a single `broadcast-common` 14.

### Tests

- `mpd-parse.golden` refuses to re-record under `CI=true` (`REGENERATE_MPD_PARSE_GOLDEN`), with a regression test, and `fixtures/golden/dash/PROVENANCE.md` names the `xmllint --schema` validation of the same corpus as its independent oracle (audit 21 finding 62).
- The 15 Table 34 role tokens and their round trip through `Role::name`, `descriptors::find`'s match/absence/first-of-equal rules, and every error enum's label and `Display` are pinned (audit 22 findings 25, 26).
- The `SegmentTemplate` identifier grammar, format-tag and width bounds, media/initialization rules, `$Bandwidth$` substitution, and the inverse `match_url` (longest-first id choice, width agreement, repeated-identifier uniqueness, `None` on ambiguity) are tabled, with a substitution/matching value grid (audit 22 findings 27, 28).
- `xs:dateTime`'s zone sign, range and hour-24 rules, `resolution_us`, the RFC 5905 1900-epoch NTP reader, the `UTCTiming` scheme policy and `@value` list are asserted against literal instants (audit 22 findings 29, 30).
- The writer's per-element "has content" branches (BaseURL, SegmentBase, SegmentList, SegmentTemplate) are exercised both ways against the emitted text and the re-parsed model; the configured parser ceilings are asserted at exactly max and max+1, and every shared-XML failure maps to its typed variant (audit 22 findings 31, 32, 35).
- Attribute edges are pinned: a non-finite `xs:double`, an inverted byte range, the resolve-to-zero xlink, and which namespace+name a `pssh`/`UrlQueryInfo` child must carry; the inheritance helpers are asserted field by field across Period/AdaptationSet/Representation (audit 22 findings 33, 34).

### Changed

- Internal: error types derive `thiserror::Error` in place of hand-written `Display`/`Error` impls (messages unchanged) (#3974, #4020).
- **Behaviour change:** `MpdFeatures::xlink_resolve_to_zero` and `MpdFeatures::inband_event_stream` are derived from what the model keeps, not from any element anywhere in the tree: `xlink_resolve_to_zero` no longer means "a Period or AdaptationSet was removed by resolve-to-zero"; it now reports an xlink the model carries (a `SegmentList`, `EventStream` or `RequestParam`), not the Period or AdaptationSet that the xlink removes, and the `InbandEventStream` flag reports only a stream on a retained AdaptationSet, Representation or Preselection. Content the parser drops no longer raises a flag the writer cannot reproduce, so `parse -> to_xml -> parse` is an identity for both (#3139).

### Fixed

- `MpdFeatures::service_description` is derived from the `ServiceDescription` elements the model keeps (MPD, Period, `Event`), not from any element named `ServiceDescription` anywhere in the tree, so an element the model drops no longer raises a flag the writer cannot reproduce and `parse -> to_xml -> parse` is an identity (#3138).
- `xs_duration`'s doc claimed a hand-rolled `PT{secs}S` spelling while the function delegates to `jiff`'s ISO 8601 duration printer; the doc now states the printed form, the exact output for the common shapes is asserted, and three dead `let _` statements (here and in the MPD reader) are removed (audit 22 finding 69).

## [0.2.1] - 2026-11-17
### Added
- `broadcast_common::xml_chars::WriteRefusal::InvalidChar` (see that crate's changelog): an attribute value or text node holding a character outside the XML 1.0 §2.2 `Char` production is refused instead of written (#2843).

### Fixed
- `Mpd::to_xml` refuses a document holding a character outside the XML 1.0 §2.2 `Char` production (`MpdError::Invalid`, naming the code point) rather than emitting a manifest every player rejects. The writer escapes with `quick-xml`, which escapes only the markup characters, so a control character in a stored label, `BaseURL` or id passed straight through (#2843).

## [0.2.0] - 2026-11-15
### Changed
- BREAKING: the MPD wall-clock fields are `jiff::Timestamp`, not Unix microsecond integers (#2730). `Mpd` is `{ availability_start_time: Option<jiff::Timestamp>, availability_end_time: Option<jiff::Timestamp>, publish_time: Option<jiff::Timestamp> }` (the `_us`-suffixed `i64` fields are removed), `xs_datetime::XsDateTime::unix` is a `jiff::Timestamp` (was `unix_us: i64`), `format_xs_datetime` takes a `jiff::Timestamp`, and `parse_ntp_timestamp` returns a `jiff::Timestamp`. Migration: `Some(unix_us)` -> `jiff::Timestamp::from_microsecond(unix_us)`, `t.unix_us` -> `t.unix.as_microsecond()`.
- `MpdError` gains `From<broadcast_common::xml_chars::XmlCharError>`, the typed character-data and namespace failure the shared XML helper now reports (#2646).
- BREAKING: `BaseInfo::timescale` and `EventStream::timescale` are `Option<Timescale>` (was `Option<u64>` / `Option<u32>`), and `inherit::ResolvedMulti::timescale` and `inherit::DEFAULT_TIMESCALE` are `Timescale`. A present `@timescale="0"` (or one above `u32::MAX`) is `MpdError::Invalid { what: "timescale" }` on every element that carries it, never a silent 1 Hz; an absent one stays `None` in the model, serialises as absent, and resolves to the spec default 1 through the new `BaseInfo::effective_timescale()`. Migration: `Some(90_000)` -> `Some(Timescale::from_nonzero(90_000))`, `multi.timescale` is read with `.get()` (#2657).

### Added
- `BaseInfo::effective_timescale`: `@timescale` with its absent default (1) applied.

## [0.1.0] - 2026-11-14
Initial release: MPEG-DASH (ISO/IEC 23009-1) MPD manifest syntax, parse and write, below
`transmux` (writer) and `dash-runtime` (client), so a consumer that only needs manifest syntax
does not pull in a client or the muxing hub (#2355).

### Added
- The MPD model: `Mpd`/`Period`/`AdaptationSet`/`Representation` with the three Segment addressing layers (`SegmentBase`, `SegmentList`, `SegmentTemplate` with `SegmentTimeline`), the descriptor scheme vocabulary and `Role`, `Label`/`GroupLabel` (`id`, `lang`, text), `cenc:pssh`, `InbandEventStream`, `codingDependency`, `ServiceDescription` (`Latency`, `PlaybackRate`), `EventStream` with its `StreamEvent` children and `RequestParam`s (5.10.2), `ProducerReferenceTime` (5.12, with its `UTCTiming`), `Resync` (5.3.13), `PatchLocation` (5.15), `Preselection` (5.3.11, the full `RepresentationBaseType`) and `ContentSteering`; `Mpd::has_event_streams`/`has_preselections`/`has_patch_location`.
- `Mpd::parse`/`parse_with`: the bounded reader over `broadcast_common::xml` under `limits::Limits` (DOCTYPE and entities refused).
- `Mpd::to_xml`: the writer, the exact inverse of the parser (quick-xml events, no hand-built XML), emitting child elements in ISO/IEC 23009-1 ed6 XSD order and declaring exactly the namespaces the model uses. Every real MPD in the corpus round-trips parse -> write -> parse, a model with every field set round-trips, and the writer reproduces the `transmux` MPD goldens byte-identically. An `Event` whose content is XML (`SelectionInfo`, `InsertPresentation`, `ReplacePresentation`, a foreign-namespace payload such as an SCTE 35 `Signal`) parses, and `to_xml` refuses it with the typed `Unsupported::EventXmlPayload` rather than dropping the payload.
- The attribute-inheritance rules (`inherit`), the `SegmentTemplate` identifier grammar (`template`), the `UTCTiming` scheme identity (`utc_timing`), `duration` (`xs:duration` reader and the one `xs_duration` writer `transmux` and `multimux` use, #2465) and `xs_datetime` (`xs:dateTime` and NTP readers); `availabilityTimeOffset` is written as `INF`/`-INF`/decimal; `ByteRange` implements `Display`.
- Two runnable examples (`parse_mpd`, `build_mpd`) and a `broadcast_dash_round_trip` fuzz target (parse -> `to_xml` -> re-parse).
