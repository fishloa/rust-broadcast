# ttml-subtitle

[![Crates.io](https://img.shields.io/crates/v/ttml-subtitle.svg)](https://crates.io/crates/ttml-subtitle)
[![docs.rs](https://img.shields.io/docsrs/ttml-subtitle)](https://docs.rs/ttml-subtitle)
[![License](https://img.shields.io/crates/l/ttml-subtitle)](https://github.com/fishloa/rust-broadcast#license)

W3C TTML2 / IMSC 1.1 timed-text subtitle parser, serializer and (partial) IMSC profile validator.

## What it does / what it does not

**Does.** Parses a TTML2 document (the XML subtitle format used by IMSC, EBU-TT-D
and DASH/CMAF text tracks) into a typed tree, serializes the tree back to XML,
and checks a documented subset of the IMSC 1.1 Text and Image Profile
constraints. Parsing and validation are separate passes, so you can inspect a
non-conformant document before deciding to reject it. Users are subtitle
packagers, QC tools and converters that need the model rather than a rendering.

**Does not.**
- It does not render, lay out or burn in subtitles, and does not resolve style
  inheritance or region geometry.
- It is not a full IMSC conformance checker: `valid: true` means only that the
  implemented checks passed (see "Standards supported").
- It does not convert TTML to WebVTT/SRT (`caption-convert` lists TTML/IMSC
  source conversion as a documented gap) and does not decode image data
  carried in `<image>` / `smpte:image`.
- It does not package TTML into ISO BMFF (`transmux` does that).
- Comments, processing instructions and attribute order are not preserved (see
  "Round-Trip Guarantee"); round-trip is semantic, not byte-exact.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| W3C TTML2, Timed Text Markup Language 2 | W3C Recommendation 08 Nov 2018 (`specs/w3c_ttml2.html`) | Document structure: `tt`, `head`, `styling`, `layout`, `body`, `div`, `p`, `span`, `br`, `region`, `style`, `set`, §9 resource elements (`image`, `audio`, `font`, `data`, `chunk`, `source`, `resources`) | Partial | Typed element tree; unmodelled elements and foreign attributes are preserved as `UnknownElement` / `ForeignAttribute` |
| | | Style attributes (`tts:*`) | Partial | Held in `StyleAttributes`; no inheritance or computation |
| | | `<time-expression>` grammar (clock-time, offset-time, wallclock-time) with frame/tick/SMPTE constraints | Full | `time::parse_time_expression`, `no_std` |
| | | Animation, metadata vocabulary, content profiles/features (`ttp:profile`, `ttp:contentProfiles`) | Typed-opaque | Metadata and unknown children kept, not interpreted |
| W3C IMSC 1.1, TTML Profiles for Internet Media Subtitles and Captions 1.1 | W3C Recommendation 08 Nov 2018, edited 27 Apr 2020 (`specs/w3c_imsc1.1.html`) | Text Profile and Image Profile validation: §7.9 profile claim, §7.12.1.3 region count, §7.12.4/5 aspect-ratio exclusion, §7.12.7 frameRate with frame terms, §8.4.11 textShadow count, §9.4.1 Image Profile `div`, §12.3.1 time-expression syntax | Partial | The full 159-row feature disposition table is transcribed in `docs/imsc11-profiles.md` but not enforced (#1108) |
| | | Region overlap/extent, `<region>` constraints, Image Profile `<image>` and `smpte:backgroundImage` checks, `<span>`-level timing | Not implemented | Stated explicitly in the `validation` module docs |
| W3C IMSC 1.0 / 1.0.1 | `ImscVersion::V1_0` selector | Same checker with the IMSC 1.1-only frame-rate rule disabled | Partial | IMSC 1.2 is not implemented |
| XML 1.0 | declaration rules §2.8, §4.3.3 | XML declaration version/encoding/standalone validated at parse | Full | Input is UTF-8 text |

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `std` | yes | Enables `Document`, foreign-content types and `validation` (they are built on `quick-xml`, which needs `std`). Without it only `error` and the `time` expression parser remain (`no_std` + `alloc`). |
| `serde` | no | `serde::Serialize` on the model types. |

## Quick start

```rust
use ttml_subtitle::Document;

let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xml:lang="en" xmlns="http://www.w3.org/ns/ttml"
   xmlns:ttp="http://www.w3.org/ns/ttml#parameter"
   ttp:contentProfiles="http://www.w3.org/ns/ttml/profile/imsc1.1/text">
  <body><div><p begin="0s" end="5s">Hello</p></div></body>
</tt>"#;

let doc = Document::parse_str(xml).unwrap();
let body = doc.tt.body.as_ref().unwrap();
assert_eq!(body.divs[0].paragraphs[0].begin.as_deref(), Some("0s"));
```

Runnable examples:

```bash
cargo run -p ttml-subtitle --example parse_document
cargo run -p ttml-subtitle --example validate_document
cargo run -p ttml-subtitle --example from_scratch
```

## What is implemented

- **Model:** `Document`, `TtElement`, `HeadElement`, `BodyElement`,
  `DivElement`, `PElement`, `SpanElement`, `BrElement`, `RegionElement`,
  `StyleElement`, `SetElement`, `StyleAttributes`, and the resource elements;
  `InlineContent` keeps text and spans in document order. All types implement
  `Default`, so documents can be authored from scratch and `Document::to_xml`
  serializes them.
- **Time:** `TimeExpression` and `time::parse_time_expression`.
- **Foreign content:** `ForeignAttribute`, `UnknownElement`, `UnknownNode`.
- **Validation:** `Validator::new(Profile, ImscVersion)` returning a
  `ValidationResult` listing every violation found (not just the first).
- **Errors:** `Error` / `Result`.
- **Real-fixture tests:** 11 W3C IMSC conformance documents under
  `tests/fixtures/` (provenance in `tests/fixtures/PROVENANCE.md`) plus golden
  files, truncation and XML-escaping tests.

## Design notes

- Parse then validate: two independent passes.
- XML reading and writing go through `quick-xml` only; the serializer generates
  output from typed fields (no raw passthrough), guarded by `tests/it/no_dom_guard.rs`.
- Semantic round-trip: parse -> serialize -> re-parse yields an equal document.
- Public enums are `#[non_exhaustive]`; errors are structured (`thiserror`).
- `no_std` + `alloc` only for `time` and `error`.

## Related crates

Depends on `broadcast-common` (error/length helpers) and `quick-xml`.
`caption-convert` documents TTML/IMSC source conversion as not yet supported;
`transmux` packages subtitle tracks; `dvb-subtitle` handles DVB bitmap
subtitles (a different format).

## MSRV and licence

Rust 1.95.0. MIT OR Apache-2.0.

## Round-Trip Guarantee

The crate provides **semantic** round-trip fidelity, not byte-identity. Parsing a document,
serializing the typed model, and re-parsing yields an equal document (`parse → serialize →
re-parse → equal`). But the serialized XML will differ from the input in the following
ways (each verified against the crate's `quick-xml`-based reader, not assumed):

| Category | Status | Reason |
|---|---|---|
| XML declaration | **Lost** (parser) | `quick-xml` reports the XML declaration as a `Decl` event and the parser ignores it; the serializer always emits `<?xml version="1.0" encoding="UTF-8"?>` |
| Comments | **Lost** (parser) | `quick-xml` *does* report comments (`Event::Comment`), but the parser does not store them |
| Processing instructions | **Lost** (parser) | `quick-xml` *does* report PIs (`Event::PI`), but the parser does not store them |
| Foreign attributes (attributes in namespaces this crate does not model, e.g. vendor or `ebuttm:` extensions) | **Preserved** (TT-W1) | Kept as `ForeignAttribute` triples `(namespace URI, local name, value)` with the `xmlns:` prefix holding the value was resolved through |
| Foreign/unknown child elements (elements not in this crate's explicitly modeled set, whether in a foreign namespace or an unrecognized `tt:` element) | **Preserved** (TT-W1) | Kept as an `UnknownElement` subtree (name, namespace, attributes, children, text) on the element that carried them; their relative order among themselves is preserved, but their position between the element's modeled children/metadata/animations is not (each group is emitted in a fixed order). Allowed by TTML2 §7.2/§7.3 |
| Attribute order | **Deterministic, not preserved** (parser) | `quick-xml` *does* preserve attribute document order, but the parser collects known attrs into named fields and serializes them in a fixed code order (preserved foreign attributes keep their document order) |
| Namespace prefix spelling | **Deterministic, not preserved** (quick-xml + serializer) | `quick-xml`'s `NsReader` resolves QNames to `(namespace_uri, local_name)` pairs; the prefix a *modeled* attribute used is not exposed, so the serializer uses the fixed conventional prefixes (`tts:`, `ttp:`, …). A *preserved* namespace is re-declared under the prefix it originally used; when two URIs need the same prefix the later one gets a generated `ttmfallbackN` binding (XML forbids one prefix bound to two URIs at once), so the URI round-trips even when its spelling does not |
| `xmlns:` scope | **Widened to `<tt>`** | A `xmlns:` declaration written on an inner element is re-emitted on `<tt>`; widening a namespace scope leaves every resolved name unchanged, but it does mean an inner-scope override of a prefix is only reproduced through a `ttmfallbackN` binding for the outer URI |
| Whitespace / indentation | **Deterministic, not preserved** | Indentation is always 2-space, closing tags always on new lines; interstitial whitespace-only text nodes are preserved by the parser but may differ in placement on re-serialization |

The invariant that *must* hold: a document constructed entirely via the public types (no
parsed input) will serialize and re-parse to an equal document. This is tested by the
`from_scratch` example.

## Profile Validation

```rust
use ttml_subtitle::{Document, validation::{Validator, Profile, ImscVersion}};

let doc = Document::parse_str(xml).unwrap();

let validator = Validator::new(Profile::Text, ImscVersion::V1_1);
let result = validator.validate(&doc);
if result.valid {
    println!("Valid IMSC 1.1 Text Profile document");
} else {
    for err in &result.errors {
        println!("{}: {}", err.constraint, err.detail);
    }
}
```
