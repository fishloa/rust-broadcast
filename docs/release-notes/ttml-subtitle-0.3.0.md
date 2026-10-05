# ttml-subtitle 0.3.0

_Released 2026-10-05._

### Changed (breaking)
- **BREAKING: XML support now requires the `std` feature; hand-rolled XML
  replaced by `quick-xml`; `roxmltree` dropped.** `document`, `foreign` and
  `validation` (and their crate-root re-exports and `parse`) are gated behind
  `std`; a `--no-default-features` build exposes only `error` and `time`.
  Parsing is a single pass over `quick_xml::NsReader` events straight into
  the typed structs (an explicit stack of partially-built spans, no document
  tree, so hostile nesting cannot overflow the stack); serialization writes
  `quick_xml::Writer` events, so the `replace`-chain escaper is gone and every
  attribute value and text node on output is escaped by quick-xml (an attribute value containing `\t`/`\n`/`\r`
  is now written as a character reference so it survives a re-parse). Parse
  results and serialized output are identical to the previous release for every
  committed fixture. Malformed input stays a structured `Error::XmlParse` (a
  `DOCTYPE`, undefined entity, unbound namespace prefix, second root element, or
  content after the root is rejected).
- **Behaviour differences from the roxmltree-based parser (all follow XML 1.0):**
  - a root `<tt>` is the document even when it has a `<tt>` child element: for
    `<tt xmlns="http://www.w3.org/ns/ttml"><tt xmlns="http://www.w3.org/ns/ttml"/></tt>` the outer
    element is parsed (previously the inner one was; the inner `<tt>` is now kept as an unknown
    child). A non-`tt` wrapper root still yields its first `<tt>` child;
  - a numeric reference to a control character (`<p>&#x1;</p>`), or a literal one, in text or in
    an attribute is rejected with `Error::XmlParse`;
  - an entity reference or CDATA section before the root element is rejected;
  - a document with more than 128 namespace declarations in scope at once is rejected (quick-xml's
    namespace-binding limit);
  - an attribute value containing `\t`, `\n` or `\r` is written as `&#9;`, `&#10;`, `&#13;`
    (`region="x\ny"` renders `region="x&#10;y"`), and a `\r` in text as `&#13;`, so they survive
    a re-parse.
- **#1110 (TT-W1)**: attributes and child elements in namespaces the crate
  does not model are no longer dropped. They are kept as
  `ForeignAttribute` triples `(namespace URI, local name, value)` (with the
  `xmlns:` prefix binding each was resolved through) and as `UnknownElement`
  subtrees, and re-emitted (TTML2 §7.2/§7.3) — each element's
  `unknown_children` keep their relative document order, but their position
  *between* modeled children, `metadata` and `animations` is not tracked
  (each group lives in its own `Vec` and is emitted in a fixed group order). Every element
  type gained `foreign_attributes` and `unknown_children` fields, each
  metadata-like type also a `scoped_namespaces` field, and `MetadataChild`
  gained an `Unknown` variant — struct-literal construction must now use
  `..Default::default()`.
- **#1110 (TT-W2)**: TTML2 §9 embedded/resource elements are modeled instead
  of dropped: `AudioElement`, `ChunkElement`, `DataElement`, `FontElement`,
  `ResourcesElement` and `SourceElement`, plus the `InlineContent::Image` and
  `InlineContent::Audio` variants (Embedded.class is legal in `<p>`/`<span>`)
  and `HeadElement::resources` / `DivElement::audio`. TTML2 §9 elements
  without a typed struct are still preserved through the foreign-content
  mechanism rather than dropped.
- **#1108 (TT-W5)**: `WallclockForm::DateTime.seconds` is now `Option<u8>`
  (was `u8`), to preserve the `hhmm-time`-vs-`hhmmss-time` distinction
  (`date-time`'s `wall-time` grammar allows omitting seconds) so
  `format_time_expression` doesn't reinsert a `:00` that wasn't in the
  source.

- **#1110 (TT-W1)** also changed these public signatures/fields:
  `Document::to_xml` now takes `&mut self` (was `&self`; it assigns fallback
  prefixes for outer-scope vendor namespaces), the `other_attributes:
  BTreeMap<(String, String), String>` field was removed from every element
  type in favour of `foreign_attributes`, and `TtElement::text: Option<String>`
  (spec-empty content) was removed. New fields `xml_base`, `ttm_role`
  / `ttm_role_source` and `xlink_href` / `xlink_role` (and siblings) were
  added to the element types that carry them; struct-literal construction
  must use `..Default::default()`.

### Fixed
- **#1110 (TT-W1)**: an inner-scope `xmlns:` prefix override no longer
  silently re-points an outer-scope vendor attribute at the wrong namespace;
  the outer URI keeps a declaration of its own under a generated
  `ttmfallbackN` prefix.
- **#1110 (TT-W1)**: `<br>` and `<span>` attributes, `<p>` child ordering and
  the `xml:space`/`xml:base` attributes are no longer dropped by the
  serializer.
- **#1108 (TT-W4)**: the default `ttp:tickRate` (§7.2.11) always multiplied
  the (possibly-defaulted-to-30) frame rate by the sub-frame rate, instead
  of defaulting to 1 tick/second when no `ttp:frameRate` was actually
  specified, and ignored `ttp:frameRateMultiplier` when computing the
  effective frame rate. The multiply is now also a checked/saturating `u64`
  computation instead of an unchecked `u32` one.
- **#1108 (TT-W5)**: the clock-time and wallclock-time grammars (§12.3.1)
  were too lenient: a seconds-fraction and a `:frames` term were both
  accepted together (grammar makes them mutually exclusive), a single-digit
  `frames` term was accepted (grammar requires >= 2 digits),
  `"wallclock(10:00)junk"` was accepted (trailing content after the closing
  paren was silently discarded via `rfind` instead of rejected), and
  2-digit wallclock hours/minutes/seconds accepted a leading `+` (Rust's
  `u8::from_str` allows it; the grammar doesn't).
- **#1108 (TT-W3)**: the profile validator's module doc claimed the full
  159-row Feature/Extension disposition table and full §8/§9 provisions;
  it now documents exactly the (much smaller) set of checks actually
  implemented. Two real bugs in that implemented subset are also fixed:
  `body_has_frame_usage`'s check of `<body>`'s own `begin`/`dur`/`end` was
  nested inside the `<div>` loop, so it never ran on a `<body>` with no
  `<div>` at all (now also covers `<div>`'s own timing); and
  `begin`/`dur`/`end` values are now run through
  `time::parse_time_expression` (§12.3.1), so `begin="garbage"` is rejected
  instead of validating.

---

Published from tag `ttml-subtitle-v0.3.0`.
