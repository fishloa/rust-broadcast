# Changelog

All notable changes to `broadcast-dash` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- `Mpd::to_xml`: the MPD writer, the exact inverse of the parser (quick-xml events, no hand-built XML). Every real MPD in the corpus round-trips parse -> write -> parse, a model with every field set round-trips, and the writer reproduces every current `transmux` MPD golden byte-identical; the model gained `cenc:pssh`, `InbandEventStream`, `codingDependency` and `ServiceDescription` (`Latency`, `PlaybackRate`), and `ByteRange` implements `Display` (#2355).
- Two runnable examples (`parse_mpd`, `build_mpd`) under `examples/`, plus a `broadcast_dash_round_trip` fuzz target covering parse -> `to_xml` -> re-parse.
- The MPD read side, extracted from `dash-runtime` so a consumer that only needs manifest syntax does not pull in a client: the `Mpd`/`Period`/`AdaptationSet`/`Representation` model with its three Segment addressing layers, `Mpd::parse`/`parse_with` over the shared bounded `broadcast_common::xml` reader under `limits::Limits`, the descriptor scheme vocabulary and `Role`, the attribute-inheritance rules (`inherit`), the `SegmentTemplate` grammar (`template`), the `UTCTiming` scheme identity (`utc_timing`), the `xs:duration` reader moved out of `transmux` (`duration`) and the `xs:dateTime`/NTP readers (`xs_datetime`) (#2355).

### Fixed
- `Mpd::to_xml` wrote child elements in orders the ISO/IEC 23009-1 ed6 schema forbids (`ServiceDescription` before `BaseURL`, `EssentialProperty` before `Period`, `SegmentBase` before `BaseURL`/`ContentProtection` in a Representation, `AudioChannelConfiguration` after `Role`), so a parsed real MPD was not valid on the way out. Children now follow the XSD sequence for `MPD`, `Period`, `AdaptationSet` and `Representation`, and a new test validates every corpus MPD (parse -> write) with `xmllint --schema`.
- `Mpd::to_xml` no longer writes empty `PatchLocation`, `EventStream`, `Preselection`, `ProducerReferenceTime`, `ContentSteering` and `Resync` elements at the end of `<MPD>` for the matching `features` flags; those elements were fabricated and invalid where they appeared. The six flags are documented as read-only (the model keeps no element for them), so a parse -> write -> parse clears them.
