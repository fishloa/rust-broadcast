# cc-data 0.11.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14 (the jiff 1.0 epoch), which is the breaking part. The work is the audit pass over the CEA-608/708 decoders, one code at a time.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Tests

- The CEA-608 decoder is pinned one code at a time: every PAC row/indent/colour/italics/underline combination, all 16 special characters, all 32 extended characters of both blocks (including the automatic backspace), every mid-row style, the tab offsets, each misc control, doubled-control suppression, channel/colour/mode labels and `push_triplets` routing are asserted against the exact screen cell (#3220).
- The CEA-708 C1 handlers are pinned to their parameter-byte counts and decoded fields: DefineWindow's bit fields and all seven style presets, SWA/SPA/SPC/SPL/CW, the CLW/DSW/HDW/TGW/DLW bitmaps, DLY/DLC, RST, every C0/EXT1 length, the G2/G3 tables, packet-sequence resets and the service-6 boundary (#3221).
- The 2-/3-bit screen attribute enums are table-driven through `from_bits` and `name()`, and `CcData` pins its 31-triplet ceiling, the exact `ReservedBits.found` value and every `CcType` label (#3222).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `cc-data-v0.11.0`.
