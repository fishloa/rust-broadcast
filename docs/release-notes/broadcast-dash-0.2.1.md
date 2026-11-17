# broadcast-dash 0.2.1

_Released 2026-11-17._

### Added
- `broadcast_common::xml_chars::WriteRefusal::InvalidChar` (see that crate's changelog): an attribute value or text node holding a character outside the XML 1.0 §2.2 `Char` production is refused instead of written (#2843).

### Fixed
- `Mpd::to_xml` refuses a document holding a character outside the XML 1.0 §2.2 `Char` production (`MpdError::Invalid`, naming the code point) rather than emitting a manifest every player rejects. The writer escapes with `quick-xml`, which escapes only the markup characters, so a control character in a stored label, `BaseURL` or id passed straight through (#2843).

---

Published from tag `broadcast-dash-v0.2.1`.
