# caption-convert 0.2.0

_Released 2026-10-05._

### Fixed
- **#1109** (reopens #974): `parse_webvtt` failed the *whole document* when
  the `WEBVTT` header contained any line other than `X-TIMESTAMP-MAP`
  (e.g. a `Kind:`/`Language:` metadata hint some encoders emit) — the line
  fell into the cue-block grouper and was misread as a cue identifier with
  no timing line after it. Now the entire header block (every line
  directly after the signature up to the first blank line, per W3C WebVTT
  SS4.1) is skipped and the document is flagged `lossy`, matching how any
  other unrepresentable construct is already handled.

---

Published from tag `caption-convert-v0.2.0`.
