# caption-convert 0.7.0

_Released 2026-11-15._

Minor-class release (0.6 -> 0.7). A malformed WebVTT or SRT document is reported as a typed reason instead of a formatted message, so a caller can branch on the failure kind rather than its text.

Version: 0.6.0 -> 0.7.0.

## Breaking changes and migration

### The parse errors are typed

`Error::InvalidWebVtt(String)` carries a typed `WebVttError` (`MissingSignature`, `IdentifierWithoutTiming`, `MalformedTiming { line }`, `EndBeforeStart`), `Error::InvalidSrt(String)` a typed `SrtError` (`EmptyBlock`, `BadSequenceNumber { line }`, `SequenceWithoutTiming`, `NoArrow { line }`, `EndBeforeStart`), and `Error::InvalidTimestamp(String)` is the struct variant `InvalidTimestamp { timestamp }`. Both new enums are re-exported at the crate root.

```rust
// before
if let Error::InvalidWebVtt(ref message) = err { eprintln!("{message}"); }
// after
match err {
    Error::InvalidWebVtt(WebVttError::MalformedTiming { line }) => eprintln!("bad timing on line {line}"),
    Error::InvalidTimestamp { timestamp } => eprintln!("unparseable timestamp {timestamp}"),
    _ => {}
}
```

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `cc-data` 0.9 -> 0.10, `dvb-vbi` 0.7 -> 0.8, `timed-metadata` 0.10 -> 0.11.

---

Published from tag `caption-convert-v0.7.0`.
