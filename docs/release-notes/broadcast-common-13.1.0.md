# broadcast-common 13.1.0

_Released 2026-11-17._

### Added
- `xml_chars::WriteRefusal::InvalidChar`, and `xml_chars::{open_element, empty_element, write_text, text_element}` refuse an attribute value or a text node holding a character outside the XML 1.0 §2.2 `Char` production. `quick-xml` escapes only the markup characters, so U+0000-U+0008, U+000B, U+000C, U+000E-U+001F, U+FFFE and U+FFFF used to reach the document verbatim and every reader rejects it; every writer built on these helpers now fails with `InvalidChar` instead (#2843).
- `url_ref::resolve_http`: [`resolve`](url_ref::resolve) followed by a scheme requirement, `http` or `https`, with an `https`-to-`http` downgrade refused as the new `url_ref::Error::ForbiddenScheme`. A manifest is remote input, so a playlist, key, init or Smooth fragment reference naming `file:`, `ftp:` or downgrading the connection must not become a fetch the caller performs (#2853, audit 20 finding 42).

---

Published from tag `broadcast-common-v13.1.0`.
