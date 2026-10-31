# ssai-runtime 0.4.0

_Released 2026-10-31._

### Changed (BREAKING: epoch)
- Builds against new caret epochs of workspace siblings (`broadcast-hls` 0.4 -> 0.5). A caret-epoch change is major-class for a 0.x crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` section 2), whatever its own API diff, so this release is a minor bump; consumers must resolve a single copy of each of those crates.
### Changed
- `InterstitialDateRange::parse_tag_line` now propagates malformed attribute-list errors from `broadcast_hls::parse_attribute_list_strict` (unterminated quoted-string, duplicate attribute name) as `Error::HlsAttrValue` instead of silently half-parsing them (#1485 R15).

---

Published from tag `ssai-runtime-v0.4.0`.
