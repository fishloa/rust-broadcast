---
status: DRAFT
---
# rtp-packet 0.5.0

_Released 2026-10-16._

**Epoch-forced major-class release.** Adds the `av3a` feature: AV3A-AATF RTP payload (T/AI 109.7-2024 clause 10.1) with typed `AatfFrame`/`AatfFrameHeader`, `packetize`/`Av3aDepacketizer`, and SDP `Av3aFmtp`/`CodecNnId`; `OneByteElements`/`TwoByteElements::serialize_into` no longer half-write on an invalid element.

## Why this version

- **Proposed version:** 0.4.0 -> **0.5.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** minor (av3a feature (additive)).
- **cargo-semver-checks vs live 0.4.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 0.4 -> 0.5; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

No API break of its own; the major is the `broadcast-common` 9 -> 10 epoch cascade.

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
