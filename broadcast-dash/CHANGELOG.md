# Changelog

All notable changes to `broadcast-dash` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- The MPD read side, extracted from `dash-runtime` so a consumer that only needs manifest syntax does not pull in a client: the `Mpd`/`Period`/`AdaptationSet`/`Representation` model with its three Segment addressing layers, `Mpd::parse`/`parse_with` over the shared bounded `broadcast_common::xml` reader under `limits::Limits`, the descriptor scheme vocabulary and `Role`, the attribute-inheritance rules (`inherit`), the `SegmentTemplate` grammar (`template`), the `UTCTiming` scheme identity (`utc_timing`), the `xs:duration` reader moved out of `transmux` (`duration`) and the `xs:dateTime`/NTP readers (`xs_datetime`) (#2355).
