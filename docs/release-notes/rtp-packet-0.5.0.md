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

#### Added (#1235)
- `av3a` feature: AV3A-AATF RTP payload (T/AI 109.7-2024 clause 10.1). The payload has no header of its own; `av3a` provides the typed `AatfFrame`/`AatfFrameHeader` (T/AI 109.3-2023 Annex A Tables A.4-A.7: `audio_codec_id` 0 general high-rate, 1 lossless up to `aatf_error_check()`, 2 general full-rate), `packetize`/`Av3aDepacketizer` (MTU split, `M` marks the last fragment), `is_dynamic_payload_type`, and `Av3aFmtp`/`CodecNnId` for the SDP `rate`/`codec-nn-id`/`config`/`bitrate` parameters (spec examples 0x0000/0x0100/0x0201 tested). CRC bytes are carried, not verified (polynomial not in the vendored extract).

#### Fixed
- `OneByteElements`/`TwoByteElements::serialize_into` validate every element before writing, so an invalid element leaves the output buffer untouched instead of half-serialized (#1128/RTP-O1).

## Merged changes since `rtp-packet-v0.4.0`

- feat(rtp-packet): AV3A-AATF RTP payload (C5) (#1240)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
- release: de-hand-roll wave metadata (53 crates: versions, CHANGELOG cut, sibling dep refs, release-note scaffolds)
