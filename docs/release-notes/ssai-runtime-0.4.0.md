# ssai-runtime 0.4.0

_Released 2026-10-31._

### Changed
- `InterstitialDateRange::parse_tag_line` now propagates malformed attribute-list errors from `broadcast_hls::parse_attribute_list_strict` (unterminated quoted-string, duplicate attribute name) as `Error::HlsAttrValue` instead of silently half-parsing them (#1485 R15).

---

Published from tag `ssai-runtime-v0.4.0`.
