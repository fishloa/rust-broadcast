# ttml-subtitle 0.4.0

_Released 2026-10-17._

**Breaking release (own API changes plus the `broadcast-common` 10 epoch).** This is a TTML round-trip fidelity release. `to_xml` no longer deletes content it does not model (`<metadata>` including the `<smpte:image>` subtitle images of SMPTE-TT / IMSC image documents, which had been dropped since 0.3.0; `ttm:` title/description/copyright/name, `ittm:altText`, `<initial>`, `<style>` children; unmodeled attributes; a `<span>`'s `<metadata>`/`<set>`), the XML declaration is parsed, validated and re-emitted, unmodeled elements inside `<p>`/`<span>` stay at their original position, and dropping a million-level-deep hand-built tree no longer overflows the stack. The README and crate-root docs are rewritten to the workspace documentation standard.

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

- Move every `broadcast-common` requirement in your tree to `10`; `Serialize::to_bytes` was removed there, use `try_to_bytes()?`.
- `XmlDeclaration::encoding` is `Option<String>` and gains `standalone: Option<bool>`; `Document::parse_str` now populates `Document::xml_declaration` (it was always `None`) and `to_xml` round-trips it. A `None` declaration still serializes `<?xml version="1.0" encoding="UTF-8"?>`; a non-UTF-8 declared encoding is kept in the model but written as `UTF-8`.

  ```rust
  // before
  let enc: &str = &doc.xml_declaration.as_ref().unwrap().encoding;
  // after
  let enc: &str = doc.xml_declaration.as_ref().and_then(|d| d.encoding.as_deref()).unwrap_or("UTF-8");
  ```
- Parsing is stricter about the declaration: a `version` that is not `1.` followed by digits, an invalid `encoding` name, a `standalone` other than `yes`/`no`, or a declaration that is not at the very start of the document (after a comment, whitespace or PI, or a second one) is now a parse error instead of being accepted and silently moved or re-serialized into an unparseable document.
- `InlineContent` gains `Unknown(Box<UnknownElement>)`: the enum is `#[non_exhaustive]`, so existing code keeps compiling, but a wildcard arm will now see unmodeled inline elements; add `InlineContent::Unknown(_) => ..` to handle them explicitly. A parsed `<p>`/`<span>` no longer fills its `unknown_children`; look for unmodeled children in the inline content instead (the field remains for struct-built trees and is written after the content).
- `UnknownElement::prefix` is the prefix the name was actually written with (`None` for an unprefixed, default-namespace name), not a guess.
- `SpanElement`, `MetadataElement`, `EbuttmElement` and `UnknownElement` implement `Drop` iteratively: fields can no longer be moved out of them or used as a `..base` struct-update base by value; clone or `std::mem::take` instead.

  ```rust
  // before
  let SpanElement { content, .. } = span;            // move out (no longer allowed)
  // after
  let content = std::mem::take(&mut span.content);
  ```
- Behavioural: `to_xml` output now contains content that earlier versions silently dropped (`<metadata>`, `ttm:`/`ittm:` items, unmodeled attributes), so golden files you blessed from 0.3.x output will change. Layout whitespace is suppressed inside mixed content, and an unprefixed foreign element with its own `xmlns=` keeps it.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README and crate-root docs rewritten to the workspace documentation standard: what the crate does and does not do, a standards-supported table (edition, scope, status), feature flags, a doctested quick start, and related crates.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Fixed (adversarial review of signalling/captions)
- `to_xml` no longer deletes unmodelled children: `<metadata>` (e.g. the `<smpte:image>` subtitle images of SMPTE-TT / IMSC image documents; broken since 0.3.0), `ttm:title`/`desc`/`copyright`, `ttm:name`, `ittm:altText`, `<initial>` and `<style>` stored them in `unknown_children` but never wrote them, and `ebuttm:documentMetadata` wrote its own in reverse order. Every container is now covered by `tests/unknown_children_roundtrip.rs` (a foreign-namespace marker per container plus every committed W3C IMSC fixture). The golden `edge-metadata-pure-and-impure.xml` pinned the old dropping behaviour and is re-blessed.
- The XML declaration is validated at parse (XML 1.0 §2.8 `VersionNum` `'1.' [0-9]+`, §4.3.3 `EncName`, `standalone` yes|no) and a hand-built non-`VersionNum` version falls back to `1.0` on write, so `<?xml version='1.0" standalone="maybe' ?>` can no longer be accepted and re-serialized into an unparseable document. A declaration anywhere but the very start of the document (after a comment, whitespace or PI, or a second one) is rejected (§2.8) instead of being accepted and silently moved to the front.

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

- docs: correct false 'not vendored' / 'edition unverified' claims (#1311)
- docs: cluster C crate READMEs + crate-root docs (standards tables, feature flags, scope, verified editions) (#1284)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- ttml-subtitle: populate/round-trip xml_declaration; drop stale dvb-si hasher TODO (#1190)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit systemic wave 2: T16 resync, largesize, ttml drop, WHIP/WHEP DELETE/PATCH/ICE restart, mpeg-ts/pes reserved+stuffing preservation, real-capture guards (NO release) (#1175)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
