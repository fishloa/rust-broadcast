# mp4-emsg 0.4.1

_Released 2026-10-05._

### Fixed
- `EmsgBox::serialize_into` now rejects `scheme_id_uri`/`value` containing an
  embedded NUL byte with `Error::InvalidString` (#1104). Previously it wrote
  the byte verbatim, which silently misframed the box on reparse (the
  null-terminated string ended early and every following field shifted).

### Added
- `EmsgBox::parse_with_flags`/`serialize_into_with_flags` (#1104): preserve a
  non-conformant non-zero `flags` value byte-exactly. The plain
  `parse`/`serialize_into` pair is unchanged and still always reads/writes 0.

---

Published from tag `mp4-emsg-v0.4.1`.
