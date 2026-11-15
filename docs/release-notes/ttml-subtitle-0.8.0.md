# ttml-subtitle 0.8.0

_Released 2026-11-15._

The pull reader now carries the shared typed `XmlCharError` for an element's namespace and character data: its findings still surface as `Error::XmlParse` with the byte position, so matching a malformed document is unchanged.

Version: 0.7.0 -> 0.8.0.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).

---

Published from tag `ttml-subtitle-v0.8.0`.
