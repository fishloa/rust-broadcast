# ttml-subtitle 0.3.0

_Released 2026-10-05._

Breaking release (0.2 -> 0.3). Three independent things force it: the XML layer moves from `roxmltree` plus a hand-written serializer to `quick-xml`, which makes XML support `std`-only; content in namespaces the crate does not model (vendor extensions, TTML2 embedded resources) is now kept and re-emitted instead of dropped, which adds fields to the element types and removes `other_attributes`; and a few public types and signatures change. You must act if you build with `--no-default-features`, if you read `other_attributes` or `TtElement::text`, if you call `Document::to_xml` through a shared reference, or if you build or match `WallclockForm::DateTime`. Parsed documents and serialized output are identical to 0.2 for every committed fixture, and a number of timing-grammar and validator bugs are fixed.

## Breaking changes

### 1. XML support requires the `std` feature

`document`, `foreign` and `validation`, their crate-root re-exports and the `parse` function are now behind `std`. A `--no-default-features` build exposes only `error` and `time` (the time-expression parser stays `no_std` + `alloc`). `std` is a default feature, so a default build is unaffected. The three `from_scratch`, `parse_document` and `validate_document` examples now declare `required-features = ["std"]`.

```toml
# before (0.2): document/validation built without std
ttml-subtitle = { version = "0.2", default-features = false }
# after (0.3): enable std to get Document, parse, Validator, ...
ttml-subtitle = { version = "0.3" }                        # default features include std
# or, explicitly:
ttml-subtitle = { version = "0.3", default-features = false, features = ["std"] }
```

Cargo.toml delta for the crate itself: `roxmltree = { version = "0.20", default-features = false }` removed; `quick-xml = { version = "0.42", default-features = false, optional = true }` added; `std = ["broadcast-common/std", "thiserror/std", "dep:quick-xml"]` (was `... "roxmltree/std"`).

### 2. Foreign and embedded content is modeled; element structs gain fields (#1110 TT-W1, TT-W2)

Attributes and child elements in namespaces the crate does not model used to be dropped. They are now kept as `ForeignAttribute` values (fields `prefix`, `namespace`, `local_name`, `value`, `scoped_namespaces`) and as `UnknownElement` subtrees, and are re-emitted (TTML2 sections 7.2, 7.3). Each element's `unknown_children` keep their relative document order, but their position between modeled children, `metadata` and `animations` is not tracked: each group lives in its own `Vec` and is emitted in a fixed group order.

TTML2 section 9 embedded and resource elements are now typed: `AudioElement`, `ChunkElement`, `DataElement`, `FontElement`, `ResourcesElement`, `SourceElement`, the `InlineContent::Image` and `InlineContent::Audio` variants, `HeadElement::resources` and `DivElement::audio`. Section 9 elements without a typed struct still go through the foreign-content mechanism.

Source-level consequences:

- Every element type gained `foreign_attributes` and `unknown_children`; metadata-like types also gained `scoped_namespaces`; `MetadataChild` gained an `Unknown` variant. New fields `xml_base`, `ttm_role` / `ttm_role_source` and `xlink_href` / `xlink_role` (and siblings) were added to the element types that carry them. All of these structs and enums are `#[non_exhaustive]`, so downstream code that builds them with `Type::default()` plus field assignment, and matches with a `_` arm, keeps compiling. (The CHANGELOG says struct-literal construction "must use `..Default::default()`"; for a downstream crate, `#[non_exhaustive]` forbids struct literals altogether, so that advice only applies inside this crate.)
- Removed: the `other_attributes: BTreeMap<(String, String), String>` field on every element type (use `foreign_attributes`, a `Vec<ForeignAttribute>`) and `TtElement::text: Option<String>`.
- `Document::to_xml` now takes `&mut self` (was `&self`) because it assigns fallback prefixes for outer-scope vendor namespaces. A `Document` you only hold by shared reference can no longer be serialized without cloning it first.

```rust
// before (0.2)
fn dump(doc: &Document) -> String { doc.to_xml() }
// after (0.3)
fn dump(doc: &mut Document) -> String { doc.to_xml() }
```

### 3. `WallclockForm::DateTime.seconds` is `Option<u8>` (#1108 TT-W5)

It was `u8`. `None` now means the `hhmm-time` form (no seconds component), distinct from an explicit `:00`, so `format_time_expression` no longer inserts a `:00` that was not in the source.

```rust
// before: WallclockForm::DateTime { seconds: 0, .. }
// after:  WallclockForm::DateTime { seconds: Some(0), .. }   // or None for hh:mm
```

## Behaviour changes (not API breaks)

The new parser is a single pass over `quick_xml::NsReader` events straight into the typed structs, with an explicit stack of partially-built spans and no document tree, so hostile nesting cannot overflow the stack. Serialization writes `quick_xml::Writer` events, so the old `replace`-chain escaper is gone. Malformed input is still a structured `Error::XmlParse`; a `DOCTYPE`, undefined entity, unbound namespace prefix, second root element, or content after the root is rejected. The differences from the `roxmltree` parser all follow XML 1.0:

- A root `<tt>` is the document even when it has a `<tt>` child. For `<tt xmlns="http://www.w3.org/ns/ttml"><tt xmlns="http://www.w3.org/ns/ttml"/></tt>` the outer element is now parsed (the inner one used to be) and the inner `<tt>` is kept as an unknown child. A non-`tt` wrapper root still yields its first `<tt>` child.
- A numeric reference to a control character (`<p>&#x1;</p>`), or a literal one, in text or in an attribute is rejected with `Error::XmlParse`.
- An entity reference or CDATA section before the root element is rejected.
- More than 128 namespace declarations in scope at once is rejected (quick-xml's namespace-binding limit, per the CHANGELOG).
- On output, an attribute value containing `\t`, `\n` or `\r` is written as `&#9;`, `&#10;`, `&#13;` (`region="x\ny"` renders `region="x&#10;y"`), and a `\r` in text as `&#13;`, so they survive a re-parse.

## Fixes

Serialization and namespaces (#1110 TT-W1):
- An inner-scope `xmlns:` prefix override no longer silently re-points an outer-scope vendor attribute at the wrong namespace; the outer URI keeps its own declaration under a generated `ttmfallbackN` prefix.
- `<br>` and `<span>` attributes, `<p>` child ordering and the `xml:space` / `xml:base` attributes are no longer dropped by the serializer.

Time expressions (#1108):
- **TT-W4:** the default `ttp:tickRate` (section 7.2.11) always multiplied the (possibly defaulted-to-30) frame rate by the sub-frame rate, instead of defaulting to 1 tick per second when no `ttp:frameRate` was specified, and ignored `ttp:frameRateMultiplier` in the effective frame rate. The computation is now also checked/saturating `u64` rather than unchecked `u32`.
- **TT-W5:** the clock-time and wallclock-time grammars (section 12.3.1) were too lenient. A seconds fraction and a `:frames` term were accepted together (the grammar makes them exclusive), a single-digit `frames` term was accepted (two or more digits required), `"wallclock(10:00)junk"` was accepted (trailing content after the closing paren was discarded), and 2-digit wallclock hours, minutes and seconds accepted a leading `+`. All are now rejected.

Profile validator (#1108 TT-W3):
- Its module doc claimed the full 159-row Feature/Extension disposition table and all of sections 8 and 9; it now documents exactly the smaller set of checks that are implemented. Treat a pass as "none of those checks failed", not as full IMSC conformance.
- `body_has_frame_usage` checked `<body>`'s own `begin`/`dur`/`end` inside the `<div>` loop, so a `<body>` with no `<div>` was never checked; it now also covers `<div>`'s own timing.
- `begin`/`dur`/`end` values now go through `time::parse_time_expression` (section 12.3.1), so `begin="garbage"` is rejected instead of validating.

## Dependencies

`broadcast-common` `9.3` -> `9.4` (see `broadcast-common-9.4.0.md`); `roxmltree` removed; `quick-xml` `0.42` added behind `std`. quick-xml is the workspace's single XML dependency for the crates that read or write XML (`transmux`, `dvb-mabr`, `ttml-subtitle`, `multimux`).

---

Published from tag `ttml-subtitle-v0.3.0`.
