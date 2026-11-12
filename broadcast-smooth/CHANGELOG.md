# Changelog

All notable changes to `broadcast-smooth` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Fixed
- The crate builds with `--no-default-features`: it reads XML unconditionally, so its `broadcast-common` dependency always enables `std` and `xml` (it failed with unresolved imports of `broadcast_common::xml`).
- `StreamIndex::enumerate_chunks`: a `c` with no `@d` and `@r > 1` takes `(next.t - t) / r` as each fragment's duration (it took the whole span, so the run overshot the next explicit `t`), and returns the new `SmoothParseError::IndivisibleRun` when the span is not a whole multiple of `r`.
- `StreamIndex::enumerate_chunks`: an explicit `c@t` that repeats or steps back before the end of the preceding entry is `SmoothParseError::NonMonotonicTime` also when `@d` is present; it was only checked when `@d` was absent, so the plan refetched a fragment and emitted samples whose dts jumped back.
- `StreamIndex::enumerate_chunks`: `c@r` is now one-based (MS-SSTR §2.2.2.6): an entry with `r = n` stands for `n` fragments; `r = 0` is rejected with `SmoothParseError::InvalidRepeatCount`. Previously `r` was treated as `r + 1` (DASH SegmentTimeline semantics).
- `StreamIndex::enumerate_chunks`: a `c` with no `@d` now derives its duration from the next entry's `@t` (start-time coding); the last entry without `@d` and without a following `@t` returns `SmoothParseError::MissingDuration`. Previously `d.unwrap_or(0)` emitted zero-duration chunks.
- `SmoothManifest::to_xml` writes exactly the model: one `c` per `StreamIndex::chunks_list` entry, each attribute only when set. It no longer invents a `c@n` from the entry's position, and no longer expands a `t` + `r` entry into separate elements, so `parse(to_xml(m)) == m` holds for models without `n` and for repeat runs. `C::numbered` sets `@n` explicitly for a writer that wants it.

### Changed
- BREAKING: the manifest is read through `broadcast_common::xml` (the bounded reader `dash-runtime`/`broadcast-dash` use) and the crate's own `xml_walk` is deleted. The `XmlLimit` variants and the namespace behaviour are unchanged (`xml_namespaces.rs`); `SmoothParseError` loses `UnexpectedEof`, `UnterminatedTag`, `MalformedAttribute` and `MismatchedEndTag`, which the shared reader reports as `Xml { pos, message }`.
- The crate is plain `std` (like `dash-runtime`): the `no_std` claim is dropped, because the shared XML module the manifest reads through is std-only and the crate could not build without `std`. `manifest` remains a named feature, on with `std`.

## [0.1.0]
### Added
- The MS-SSTR client Manifest model, `SmoothManifest`/`StreamIndex`/`QualityLevel`/`C`, with `parse`/`parse_with_limits` and `to_xml`, moved from `transmux` (issue #2356). It covers the live/DVR attributes, `Protection`/`ProtectionHeader`, `StreamIndex@QualityLevels`, and the `c@n` fragment number, none of which the old reader kept, so `transmux`'s writer and this reader are now one model.
- `SmoothManifest::new`/`with_duration`/`live`/`with_protection` and the `StreamIndex`/`QualityLevel`/`C` builders, so a manifest can be constructed without touching a field: the public structs are `#[non_exhaustive]`.
- `StreamIndex::enumerate_chunks` (bounded `c@r` expansion) and `resolve_fragment_url` (fragment-URL token substitution), with the input caps `MAX_CHUNK_RUN` and `MAX_CODEC_PRIVATE_DATA_HEX_LEN`.
