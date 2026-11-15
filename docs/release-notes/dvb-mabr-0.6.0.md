# dvb-mabr 0.6.0

_Released 2026-11-15._

Minor-class release (0.5 -> 0.6). The parser's one stringly XML error is split into the three failures a caller actually branches on: the lexer's fault, a character/entity/namespace validation, and a document-structure rule. `Error` is no longer `PartialEq`/`Eq`.

Version: 0.5.0 -> 0.6.0.

## Breaking changes and migration

### `Error::XmlParse` is split by cause

`Error::XmlParse(String)` is replaced by `Error::XmlParse(#[source] quick_xml::Error)` (the lexer), `Error::XmlInvalid(#[source] broadcast_common::xml_chars::XmlCharError)` (a character/entity/namespace/attribute validation), and `Error::InvalidStructure(StructureError)` (`Doctype`, `UnexpectedEof`, `CharDataOutsideRoot`, `NoRoot`, `MultipleRoots`, `ContentAfterRoot`), each behind `std`. `Error` is `Clone`/`Debug` but no longer `PartialEq`/`Eq` (`quick_xml::Error` is neither).

```rust
// before
match error {
    dvb_mabr::Error::XmlParse(ref message) => eprintln!("{message}"),
    _ => {}
}
// after
match error {
    dvb_mabr::Error::XmlInvalid(kind) => eprintln!("not valid XML text: {kind}"),
    dvb_mabr::Error::InvalidStructure(dvb_mabr::StructureError::Doctype) => { /* a DOCTYPE was refused */ }
    dvb_mabr::Error::XmlParse(source) => eprintln!("lexer: {source}"),
    _ => {}
}
```

### The pull reader carries `XmlCharError`

The pull reader carries `broadcast_common::xml_chars::XmlCharError` for an element's namespace resolution and for character data (the shared helper is typed now); its findings still surface as `Error::XmlParse` with the byte position.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).

---

Published from tag `dvb-mabr-v0.6.0`.
