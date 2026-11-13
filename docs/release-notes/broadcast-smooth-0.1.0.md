# broadcast-smooth 0.1.0

_Released 2026-11-14._

Initial release.

Version: 0.1.0 (first publish).

Initial release: Microsoft Smooth Streaming (MS-SSTR) client Manifest syntax, parse and write,
moved out of `transmux` so the manifest has one model shared by the packager, the client and the
origin (#2356). Plain `std` (the shared XML reader is std-only); `manifest` is a named feature, on with `std`.

### Added
- The MS-SSTR client Manifest model, `SmoothManifest`/`StreamIndex`/`QualityLevel`/`C`, with `parse`/`parse_with_limits` (through the bounded `broadcast_common::xml` reader) and `to_xml`. It covers the live/DVR attributes, `Protection`/`ProtectionHeader`, `StreamIndex@QualityLevels` and the `c@n` fragment number. `to_xml` writes exactly the model (one `c` per `chunks_list` entry, each attribute only when set), so `parse(to_xml(m)) == m`.
- `SmoothManifest::new`/`with_duration`/`live`/`with_protection` and the `StreamIndex`/`QualityLevel`/`C` builders; the public structs are `#[non_exhaustive]`.
- `StreamIndex::enumerate_chunks`: bounded `c@r` expansion with MS-SSTR §2.2.2.6 semantics: `r` is one-based (`r = 0` is `SmoothParseError::InvalidRepeatCount`), a `c` without `@d` takes its duration from the next `@t` (`MissingDuration` when there is none; `IndivisibleRun` when a run does not divide the span), and an explicit `@t` that repeats or steps back is `NonMonotonicTime`.
- `resolve_fragment_url` (fragment-URL token substitution) and the input caps `MAX_CHUNK_RUN` and `MAX_CODEC_PRIVATE_DATA_HEX_LEN`.

---

Published from tag `broadcast-smooth-v0.1.0`.
