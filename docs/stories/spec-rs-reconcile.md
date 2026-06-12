# Story — reconcile dvb-si coded enums against the corrected spec md

The `dvb-si/docs/*.md` transcriptions were just corrected verbatim from the
vendored PDFs (commits f018b20, a7a30cd, 348dc2b). This story reconciles the
`.rs` enum codings against them — applying the **exact** changes below. These are
decided; do not invent values, and do **not** mass-rename concise names to verbose
spec sentences (concise `name()` strings are intentional). Touch only the listed
files. Run the full gate suite and fix until green. Do **not** commit.

## 1. `descriptors/service.rs` — `ServiceType` (EN 300 468 Table 89)
Add the missing value **0x20**:
- enum variant `HevcUhdDigitalTelevision` (doc `/// 0x20 — HEVC UHD digital television service.`), placed between `HevcDigitalTelevision` and `VvcDigitalTelevision`.
- `from_u8`: `0x20 => Self::HevcUhdDigitalTelevision,`
- `to_u8`: `Self::HevcUhdDigitalTelevision => 0x20,`
- `name`: `Self::HevcUhdDigitalTelevision => "HEVC UHD digital television service",`
- Add a unit test asserting `from_u8(0x20).name() == "HEVC UHD digital television service"` and `to_u8()==0x20`.

## 2. `descriptors/iso_639_language.rs` — `AudioType` (ISO/IEC 13818-1 §2.6.19 Table 2-63)
- Fix the enum doc cite `/// Audio type — ETSI EN 300 468 §6.2.22.` → `/// Audio type — ISO/IEC 13818-1 §2.6.19 Table 2-63.`
- Add variants (keep existing 0x00–0x03): `Primary` (0x80), `Native` (0x81), `Emergency` (0x82), `PrimaryCommentary` (0x83), `AlternateCommentary` (0x84), and `UserPrivate(u8)` for 0x04–0x7F. `Reserved(u8)` now covers 0x85–0xFF.
- `from_u8`: `0x04..=0x7F => Self::UserPrivate(v), 0x80 => Self::Primary, 0x81 => Self::Native, 0x82 => Self::Emergency, 0x83 => Self::PrimaryCommentary, 0x84 => Self::AlternateCommentary, v => Self::Reserved(v),` (exact arms before the range; keep 0x00–0x03).
- `to_u8`: map each back (`Self::UserPrivate(v) => v`, `Self::Primary => 0x80`, …, `Self::Reserved(v) => v`).
- `name`: `Primary => "primary"`, `Native => "native"`, `Emergency => "emergency"`, `PrimaryCommentary => "primary commentary"`, `AlternateCommentary => "alternate commentary"`, `UserPrivate(_) => "user private"`, `Reserved(_) => "reserved"`.
- Tests: round-trip `to_u8(from_u8(b))==b` for b in `{0x00,0x03,0x40,0x80,0x84,0x85,0xFF}`; assert `from_u8(0x80).name()=="primary"`.

## 3. `descriptors/subtitling.rs` — `SubtitlingType` (EN 300 468 Table 26, stream_content 0x03)
Add the missing plano-stereoscopic HD variants:
- `0x15` → variant `DvbSubtitlesNormalPlanoStereoscopicHd`, name `"DVB subtitles (normal), plano-stereoscopic disparity, HD"`.
- `0x25` → variant `DvbSubtitlesHardOfHearingPlanoStereoscopicHd`, name `"DVB subtitles (hard of hearing), plano-stereoscopic disparity, HD"`.
- Wire them into from_u8/to_u8/name; add round-trip tests for 0x15 and 0x25.

## 4. `descriptors/scrambling.rs` — `ScramblingMode` (EN 300 468 Table 87) — REMOVE fabricated values
The spec marks **0x04–0x0F reserved**; the impl invented `DvbCsa3MinimalEnhanced` (0x04) and `DvbCsa3FullyEnhanced` (0x05). **Remove both variants** entirely (enum decl, from_u8, to_u8, name). 0x04/0x05 must now decode to `Reserved(0x04)`/`Reserved(0x05)`.
- Add a test asserting `from_u8(0x04) == ScramblingMode::Reserved(0x04)` and same for 0x05, and `from_u8(0x03).name()=="DVB-CSA3 (standard)"` unchanged.

## 5. `descriptors/data_stream_alignment.rs` — `AlignmentType` (ISO/IEC 13818-1, Table 2-53)
- Fix the module/enum doc cite: it currently cites **§2.6.14** (Video window descriptor); the data_stream_alignment_descriptor is **§2.6.10** — change the cite to `§2.6.10`.
- Two `name()` strings are missing the spec's comma: `"Slice or video access unit"` → `"Slice, or video access unit"`; `"GOP or SEQ"` → `"GOP, or SEQ"` (verbatim Table 2-53).

## 6. `tables/mod.rs`+`tables/pmt.rs`+`tables/sat.rs`+`tables/ait.rs` — strengthen tautological tests
For `StreamType`, `AssociationType`, `SatTableId`, `InterpolationType`, `ControlCode` (and `AudioType`): ensure at least one test asserts a concrete `(wire_value → name)` pair using a **literal expected string** (independent of the impl's own arms), so a wrong/missing value would fail. Where such a test is missing or merely re-asserts the impl, add/repair it. Use values from the corrected docs/*.md.

## Do NOT change
- The ~concise `name()` strings the audit flagged as "wrong_name" that are just shorter than the spec sentence (e.g. ServiceType `0x07` "FM radio service", CridType "item of content", linkage names). These are intentional. Leave them.
- `StreamType` 0x81/0x86/0x87 (AC-3/SCTE-35/E-AC-3) — these DVB/ATSC/SCTE conventions are correct and deliberate; keep them.

## CHANGELOG
Add to `dvb-si/CHANGELOG.md` `## Unreleased`:
- Added: service_type 0x20 (HEVC UHD), audio_type 0x04–0x7F user-private + 0x80–0x84 (Primary/Native/Emergency/Primary commentary/Alternate commentary), subtitling 0x15/0x25 (plano-stereoscopic HD).
- Fixed: removed fabricated scrambling_mode 0x04/0x05 (spec-reserved); corrected AudioType / data_stream_alignment spec citations and two alignment_type names.

## Gate commands (run ALL; fix until every one is green)
```
cargo build  --workspace --all-features --locked
cargo test   --workspace --all-features --locked
cargo build  --workspace --no-default-features --locked
cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
```

## Boundaries
Touch only: `descriptors/service.rs`, `descriptors/iso_639_language.rs`,
`descriptors/subtitling.rs`, `descriptors/scrambling.rs`,
`descriptors/data_stream_alignment.rs`, the test sites in
`tables/{mod,pmt,sat,ait}.rs`, and `dvb-si/CHANGELOG.md`. Do not commit.
