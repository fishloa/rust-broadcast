# ttml-subtitle 0.8.0

_Released 2026-11-15._

### Changed
- BREAKING: the pull reader now carries `broadcast_common::xml_chars::XmlCharError` for an element's namespace resolution and for character data (the shared helper is typed now, #2646); its findings still surface as `Error::XmlParse` with the byte position.

---

Published from tag `ttml-subtitle-v0.8.0`.
