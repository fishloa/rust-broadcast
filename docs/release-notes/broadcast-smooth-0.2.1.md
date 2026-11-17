# broadcast-smooth 0.2.1

_Released 2026-11-17._

Patch release. `SmoothManifest::to_xml` refuses an attribute value or `ProtectionHeader` text holding a character outside the XML 1.0 §2.2 `Char` production (`ManifestWriteError::InvalidChar`) instead of writing a manifest every reader rejects. There is no API change.

## Fixed

- `SmoothManifest::to_xml` refuses an attribute value or `ProtectionHeader` text holding a character outside the XML 1.0 §2.2 `Char` production (`ManifestWriteError::InvalidChar`, the shared `broadcast_common::xml_chars::WriteRefusal`) instead of writing a manifest every reader rejects (#2843).

---

Published from tag `broadcast-smooth-v0.2.1`.
