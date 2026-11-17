# broadcast-common 13.1.0

_Released 2026-11-17._

Minor release (13.0 -> 13.1) adding two shared hardening helpers from the audit 20 bundle. The XML writer helpers refuse a character outside the XML 1.0 §2.2 `Char` production, and `url_ref::resolve_http` refuses a non-web scheme or an `https`-to-`http` downgrade in a manifest reference. Both are additive; existing callers are unchanged.

## Added

- `xml_chars::WriteRefusal::InvalidChar`, and `xml_chars::{open_element, empty_element, write_text, text_element}` refuse an attribute value or a text node holding a character outside the XML 1.0 §2.2 `Char` production. `quick-xml` escapes only the markup characters, so U+0000-U+0008, U+000B, U+000C, U+000E-U+001F, U+FFFE and U+FFFF used to reach the document verbatim and every reader rejects it; every writer built on these helpers now fails with `InvalidChar` instead (#2843).
- `url_ref::resolve_http`: [`resolve`](https://docs.rs/broadcast-common/13.1.0/broadcast_common/url_ref/fn.resolve.html) followed by a scheme requirement, `http` or `https`, with an `https`-to-`http` downgrade refused as the new `url_ref::Error::ForbiddenScheme`. A manifest is remote input, so a playlist, key, init or Smooth fragment reference naming `file:`, `ftp:` or downgrading the connection must not become a fetch the caller performs (#2853, audit 20 finding 42).

---

Published from tag `broadcast-common-v13.1.0`.
