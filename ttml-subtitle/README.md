# ttml-subtitle

[![Crates.io](https://img.shields.io/crates/v/ttml-subtitle.svg)](https://crates.io/crates/ttml-subtitle)
[![docs.rs](https://img.shields.io/docsrs/ttml-subtitle)](https://docs.rs/ttml-subtitle)
[![MSRV](https://img.shields.io/badge/rustc-1.86+-blue.svg)](https://blog.rust-lang.org/2025/02/20/Rust-1.86.0.html)
[![License](https://img.shields.io/crates/l/ttml-subtitle)](https://github.com/fishloa/rust-broadcast#license)

TTML2 / IMSC 1.1 timed-text subtitle parser for Rust.

Parses W3C Timed Text Markup Language 2 (TTML2) documents and validates them against IMSC 1.1 Text Profile and Image Profile constraints. Parse a document, then validate it separately — the two passes are independent.

- **Spec**: W3C TTML2 Recommendation (08 Nov 2018) + IMSC 1.1 Recommendation (08 Nov 2018, edited 27 Apr 2020)
- **MSRV**: 1.86
- **License**: MIT OR Apache-2.0

## Quick Start

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

## Features

- Full TTML2 element tree (26 element types, 56 style properties)
- Exhaustive `<time-expression>` grammar (clock-time, offset-time, wallclock-time) with frame/tick/SMPTE constraint enforcement
- IMSC 1.1 profile validation (159-row feature disposition table, §7.12 "must reject" constraints)
- Parse/validate split — inspect non-conformant documents before deciding to reject
- Semantic round-trip (parse → serialize → re-parse → equal); byte-exact round-trip is infeasible for XML
- No raw-passthrough in the serializer — output is generated from typed fields
- Real-fixture tested against 11 W3C IMSC conformance suite documents
- `#[no_std]` + `alloc` compatible (with `std` feature)
- Optional `serde` support

## Examples

```bash
cargo run -p ttml-subtitle --example parse_document
cargo run -p ttml-subtitle --example validate_document
```

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
