# caption-convert 0.7.0

_Released 2026-11-15._

### Changed
- BREAKING: `Error::InvalidWebVtt(String)` carries a typed `WebVttError` (`MissingSignature`/`IdentifierWithoutTiming`/`MalformedTiming { line }`/`EndBeforeStart`), `Error::InvalidSrt(String)` a typed `SrtError` (`EmptyBlock`/`BadSequenceNumber { line }`/`SequenceWithoutTiming`/`NoArrow { line }`/`EndBeforeStart`), and `Error::InvalidTimestamp(String)` is a struct variant `InvalidTimestamp { timestamp }`, so a caller branches on the typed kind rather than the message text (#2734). Both new enums are re-exported at the crate root.

---

Published from tag `caption-convert-v0.7.0`.
