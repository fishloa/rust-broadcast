# dvb-mabr 0.6.0

_Released 2026-11-15._

### Changed
- BREAKING: `Error::XmlParse(String)` is split into typed XML failures: `Error::XmlParse(#[source] quick_xml::Error)` (the lexer), `Error::XmlInvalid(#[source] broadcast_common::xml_chars::XmlCharError)` (a character/entity/namespace/attribute validation), and `Error::InvalidStructure(StructureError)` (`Doctype` / `UnexpectedEof` / `CharDataOutsideRoot` / `NoRoot` / `MultipleRoots` / `ContentAfterRoot`), each `#[cfg(feature = "std")]`. `Error` is `Clone`/`Debug` but no longer `PartialEq`/`Eq` (`quick_xml::Error` is neither). Matching `Error::XmlParse(_)` now distinguishes the reader fault instead of a formatted message (#2734).
- BREAKING: the pull reader now carries `broadcast_common::xml_chars::XmlCharError` for an element's namespace resolution and for character data (the shared helper is typed now, #2646); its findings still surface as `Error::XmlParse` with the byte position.

---

Published from tag `dvb-mabr-v0.6.0`.
