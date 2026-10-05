# dvb-bbframe 11.0.0

_Released 2026-10-05._

### Fixed
- Normal-Mode (NM) CRC-8 mismatches are now detected and flagged with the
  Transport Error Indicator (TEI, ISO/IEC 13818-1 §2.4.3.2) to alert
  downstream receivers of corruption (EN 302 755 §5.1.6). Previously the
  CRC-8 chain was checked but only counted diagnostically; corrupted packets
  went undetected. Stats now includes `crc8_mismatches` (#1094).
- `BbframePump` now allocates its 256-element extractor array on the heap
  (via `Box`) instead of the stack, preventing a 59 KB stack overflow on
  embedded or small-stack contexts (#1094).
- `Bbheader::serialize_into` now validates `dfl <= DFL_MAX_BITS` and rejects
  mode-field inconsistencies (NM cannot carry `issy_in_header`, HEM cannot
  carry non-zero `upl`/`sync`), preventing silent data loss from serializing
  into a stream the parser refuses (#1094).
- Normal-Mode user-packet extraction (`NmTsIter`/`up_iter`/
  `CarryOverExtractor::feed_nm`/`feed_nm_into`) always cut every 188 bytes,
  ignoring `UPL`; a real off-air capture with ISSYI=1 (190-byte stride) was
  misframed after the first UP, and NPD/DNP framing was never accounted for
  either. The per-UP stride is now derived from `UPL`/ISSYI/NPD per EN 302
  755 §5.1.8 (new `packet::nm_stride_bytes`), and the per-UP CRC-8 chain (EN
  302 755 §5.1.6) is now checked, exposed via new `CarryOverStats` fields
  `nm_upl_invalid` and `crc8_mismatches` (#1033).

### Changed (breaking)
- `packet::NmTsIter::new` now takes an explicit `stride: usize` (previously
  assumed a fixed 188 bytes) — part of the #1033 fix above — and is now
  fallible: a non-zero stride below 188 returns the new
  `Error::InvalidStride` (it previously panicked in `next()` on short input);
  the iterator also no longer overflows on `pos + stride`.

---

Published from tag `dvb-bbframe-v11.0.0`.
