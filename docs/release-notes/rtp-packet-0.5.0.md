# rtp-packet 0.5.0

_Released 2026-10-16._

**Epoch-forced major-class release; additive feature plus one serializer fix.** `rtp-packet` follows the `broadcast-common` 9 -> 10 caret epoch. New: the `av3a` feature, the AV3A-AATF RTP payload (T/AI 109.7-2024 clause 10.1) with typed `AatfFrame`/`AatfFrameHeader` (T/AI 109.3-2023 Annex A Tables A.4-A.7), `packetize`/`Av3aDepacketizer` (MTU split, `M` marks the last fragment), `is_dynamic_payload_type`, and the SDP `Av3aFmtp`/`CodecNnId` (`rate`/`codec-nn-id`/`config`/`bitrate`). Two stated limits, both quoted from the spec rather than guessed: the AATF `crc_check` is carried but not verified (Tables A.5/A.7 name it `rpchof` but give no polynomial or initial value), and the lossless-frame `channel_number` is not decoded because the spec's only rule ("When audio_codec_id is 1 and the number of channels is less than 16, this field contains 4 bits. 0xf indicates extension, and others indicate the number of channels. When audio_codec_id is 1 and the number of channels is not less than 16, this field contains 8 bits.") does not say whether the 8-bit form replaces the nibble or follows a `0xf` nibble, what it encodes, or how the width is known before the field is read; the bytes after `aatf_error_check()` stay in `AatfFrame::body`. Also fixed: `OneByteElements`/`TwoByteElements::serialize_into` validate every element before writing, so an invalid element no longer leaves a half-serialized buffer.

## Why this version

- **Proposed version:** 0.4.0 -> **0.5.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** minor (additive `av3a` feature; extension-element `serialize_into` now leaves the buffer untouched on error).
- **cargo-semver-checks vs live 0.4.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 0.4 -> 0.5; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

`rtp-packet` has no source-breaking change of its own; the major is the `broadcast-common` 9 -> 10 epoch cascade.

### 1. `broadcast-common` 10

`broadcast-common` 10 removed `Serialize::to_bytes()` (the default method that panicked when `serialize_into` failed). Code that serialises any `rtp-packet` type through it must switch to the fallible form:

```rust
// before
let bytes = value.to_bytes();
// after
let bytes = value.try_to_bytes()?;   // Result<Vec<u8>, _>
```

Your dependency graph must resolve a single `broadcast-common` 10 (a second copy on 9.x is a different `Serialize` trait and the types will not satisfy it).

### 2. Using the new `av3a` feature

Enable it with `rtp-packet = { version = "0.5", features = ["av3a"] }`. `Av3aDepacketizer` is bounded: it caps buffering at `DEFAULT_MAX_FRAME_BYTES` (1 MiB; `raw_frame_length` is 16 bits) or `with_max_frame_bytes(n)`, and `push` returns `Result<Option<Vec<u8>>>`, reporting `InvalidValue` (and dropping the partial frame) on overflow. `Av3aFmtp::parse` rejects a leading `+` in `rate`/`bitrate`/`codec-nn-id`, `rate=0`, and any known parameter given more than once.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- `av3a`: the lossless-header `channel_number` gap is now documented from the rendered T/AI 109.3-2023 pages (Table A.6, Annex A semantics) with the spec sentence quoted and a transcription in `docs/av3a-aatf-frame-header.md`; the previous "vendored text does not make unambiguous" wording is replaced. No behaviour change.
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Fixed (av3a, review of #1235)
- `Av3aDepacketizer` no longer buffers without limit until an `M` packet: it is capped at `DEFAULT_MAX_FRAME_BYTES` (1 MiB; `raw_frame_length` is 16 bits) or `with_max_frame_bytes(n)`. `push` now returns `Result<Option<Vec<u8>>>` and reports `InvalidValue` (dropping the partial frame) on overflow.
- `Av3aFmtp::parse` rejects a leading `+` in `rate`/`bitrate`/`codec-nn-id`, `rate=0`, and any known parameter given more than once.

#### Added (#1235)
- `av3a` feature: AV3A-AATF RTP payload (T/AI 109.7-2024 clause 10.1). The payload has no header of its own; `av3a` provides the typed `AatfFrame`/`AatfFrameHeader` (T/AI 109.3-2023 Annex A Tables A.4-A.7: `audio_codec_id` 0 general high-rate, 1 lossless up to `aatf_error_check()`, 2 general full-rate), `packetize`/`Av3aDepacketizer` (MTU split, `M` marks the last fragment), `is_dynamic_payload_type`, and `Av3aFmtp`/`CodecNnId` for the SDP `rate`/`codec-nn-id`/`config`/`bitrate` parameters (spec examples 0x0000/0x0100/0x0201 tested). CRC bytes are carried, not verified (Table A.5 names `crc_check` as `rpchof` but states no polynomial or initial value).

#### Fixed
- `OneByteElements`/`TwoByteElements::serialize_into` validate every element before writing, so an invalid element leaves the output buffer untouched instead of half-serialized (#1128/RTP-O1).

## Merged changes since `rtp-packet-v0.4.0`

- docs: README coverage gaps (audit 24), media-doctor examples doc (#1317)
- av3a lossless channel_number gap quoted from spec; MPEG-H descriptor from Table 2-121 (#1310)
- docs: correct false 'not vendored' / 'edition unverified' claims (#1311)
- docs: clear per-crate documentation for cluster B (16 crates) (#1291)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- Fix confirmed transport/ancillary review defects (supersedes #1277) (#1280)
- chore: pre-release hygiene (README coverage, snippet tool, docsrs, examples, CLAUDE.md) (#1278)
- feat(rtp-packet): AV3A-AATF RTP payload (C5) (#1240)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
- release: de-hand-roll wave metadata (53 crates: versions, CHANGELOG cut, sibling dep refs, release-note scaffolds)
