# Changelog

All notable changes to `broadcast-dash` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Changed
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
