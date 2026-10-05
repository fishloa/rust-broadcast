# mp4-emsg 0.4.1

_Released 2026-10-05._

**Patch with one additive API.** `EmsgBox::serialize_into` now rejects a `scheme_id_uri` or `value` containing an embedded NUL byte with `Error::InvalidString` (#1104). Previously the byte was written verbatim, which silently misframed the box on reparse: the null-terminated string ended early and every following field shifted. If you build `emsg` boxes from external strings (for example a SCTE-35 or ID3 scheme URI taken from network data), a NUL in either string is now an error rather than a corrupt box.

New: `EmsgBox::parse_with_flags` and `EmsgBox::serialize_into_with_flags(flags, out)` (#1104) preserve a non-conformant non-zero `flags` value byte-exactly (`parse_with_flags` returns `(EmsgBox, u32)`). The plain `parse` / `serialize_into` pair is unchanged and still always reads and writes 0.

Dependency: `broadcast-common` `9.3` to `9.4`; no feature change. See [broadcast-common 9.4.0](broadcast-common-9.4.0.md).

---

Published from tag `mp4-emsg-v0.4.1`.
