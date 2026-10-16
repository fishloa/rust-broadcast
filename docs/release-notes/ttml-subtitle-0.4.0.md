---
status: DRAFT
---
# ttml-subtitle 0.4.0

_Released 2026-10-16._

**Breaking release.** TTML round-trip fidelity: the XML declaration is parsed and re-emitted, unmodeled elements inside `<p>`/`<span>` stay at their position, unmodeled attributes survive `to_xml`, and dropping a million-level-deep hand-built tree no longer overflows the stack.

## Why this version

- **Proposed version:** 0.3.0 -> **0.4.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (InlineContent::Unknown, XmlDeclaration fields, Drop impls).
- **cargo-semver-checks vs live 0.3.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 0.3 -> 0.4; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

- `XmlDeclaration::encoding` is `Option<String>` and gains `standalone: Option<bool>`; `Document::parse_str` now populates `Document::xml_declaration`.
- `InlineContent` gains `Unknown(Box<UnknownElement>)`: add an arm to exhaustive matches (the enum is `#[non_exhaustive]`).
- `UnknownElement::prefix` is the prefix the name was written with.
- `SpanElement`, `MetadataElement`, `EbuttmElement` and `UnknownElement` implement `Drop` iteratively: fields can no longer be moved out of them or used as a `..base` struct-update base by value; clone or `mem::take`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed (BREAKING)
- `Document::parse_str` now populates `Document::xml_declaration` from the XML
  declaration (quick-xml `Decl` event; it was always `None`), and `to_xml`
  round-trips it. `XmlDeclaration::encoding` is now `Option<String>` (the
  declaration may omit it) and gains `standalone: Option<bool>`; `Default` is
  `1.0`/`UTF-8`. A `None` declaration (hand-built or declaration-less input)
  still serializes the default `<?xml version="1.0" encoding="UTF-8"?>`; a
  non-UTF-8 declared encoding is kept in the model but serialized as `UTF-8`
  (the output is UTF-8 text). A `standalone` other than yes/no is a parse error
  (#1189).
- `InlineContent` gains an `Unknown(Box<UnknownElement>)` variant: an unmodeled
  element inside `<p>`/`<span>` is kept at its original position among the text
  and spans instead of being hoisted to the end of `unknown_children` (which a
  parsed `<p>`/`<span>` no longer fills; it remains for struct-built trees and is
  written after the content). Exhaustive matches on `InlineContent` (the enum is
  `#[non_exhaustive]`) gain an arm (#1110 TT-W1/TT-W2).
- `UnknownElement::prefix` is now the prefix the element name was actually
  written with (`None` for an unprefixed, default-namespace name), instead of a
  guess from the scope that gave an unprefixed element the `tt` prefix.

#### Fixed
- `to_xml` no longer drops an attribute the crate does not model on an element,
  even in a namespace it does model (`tts:`/`ttp:`/`ttm:`/`xml:`/unprefixed):
  such attributes are read-tracked and preserved as `ForeignAttribute`s
  (#1110 TT-W1). Pinned by `tests/unknown_attributes.rs`.
- `to_xml` inserted newline/indent text nodes inside `<p>` when it held a hoisted
  `<metadata>`/`<set>` (e.g. the committed `imsc1-animation-001` fixture), changing
  the paragraph's text; layout whitespace is now suppressed inside mixed content,
  and mixed foreign-element content is written verbatim (#1110 TT-W1). Goldens
  for `imsc1-animation-001` and `edge-foreign` re-blessed: the old files encoded
  the bug.
- A `<span>`'s `<metadata>`, `<set>` and unmodeled children were silently dropped
  by `to_xml` (#1110 TT-W2).
- An unprefixed foreign element carrying its own `xmlns="..."` was written
  without it and so changed namespace; the declaration is now emitted, and an
  unknown `xml:`-namespace attribute no longer gets a (forbidden) second prefix
  binding for the XML namespace.

- **BREAKING (minor)**: dropping a deeply nested, hand-built `Document` no longer overflows the stack (#1146). `SpanElement`, `MetadataElement`, `EbuttmElement` and `UnknownElement` now implement `Drop` iteratively (children are moved onto an explicit work stack), so a tree a million levels deep drops on a 256 KiB stack. A type that implements `Drop` can no longer have fields moved out of it or be used as the base of a `..base` struct update by value; clone or `mem::take` the fields instead. Parsed documents were never affected (depth is capped at 64).

## Merged changes since `ttml-subtitle-v0.3.0`

- ttml-subtitle: populate/round-trip xml_declaration; drop stale dvb-si hasher TODO (#1190)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit systemic wave 2: T16 resync, largesize, ttml drop, WHIP/WHEP DELETE/PATCH/ICE restart, mpeg-ts/pes reserved+stuffing preservation, real-capture guards (NO release) (#1175)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
