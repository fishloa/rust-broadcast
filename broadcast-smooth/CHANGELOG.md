# Changelog

All notable changes to `broadcast-smooth` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Fixed
- `SmoothManifest::to_xml` writes exactly the model: one `c` per `StreamIndex::chunks_list` entry, each attribute only when set. It no longer invents a `c@n` from the entry's position, and no longer expands a `t` + `r` entry into separate elements, so `parse(to_xml(m)) == m` holds for models without `n` and for repeat runs. `C::numbered` sets `@n` explicitly for a writer that wants it.

## [0.1.0]
### Added
- The MS-SSTR client Manifest model, `SmoothManifest`/`StreamIndex`/`QualityLevel`/`C`, with `parse`/`parse_with_limits` and `to_xml`, moved from `transmux` (issue #2356). It covers the live/DVR attributes, `Protection`/`ProtectionHeader`, `StreamIndex@QualityLevels`, and the `c@n` fragment number, none of which the old reader kept, so `transmux`'s writer and this reader are now one model.
- `SmoothManifest::new`/`with_duration`/`live`/`with_protection` and the `StreamIndex`/`QualityLevel`/`C` builders, so a manifest can be constructed without touching a field: the public structs are `#[non_exhaustive]`.
- `StreamIndex::enumerate_chunks` (bounded `c@r` expansion) and `resolve_fragment_url` (fragment-URL token substitution), with the input caps `MAX_CHUNK_RUN` and `MAX_CODEC_PRIVATE_DATA_HEX_LEN`.
