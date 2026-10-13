# rist-runtime 0.3.0

_Released 2026-10-13._

### Changed
- Dependency epoch bump: `rtcp-packet` 0.4 → 0.5 (BREAKING upstream: `SenderReport`/`ReceiverReport` are `#[non_exhaustive]` with constructors and carry `profile_extensions`). `ReportPart::Sr`/`Rr` therefore expose the new types; tests migrated to the constructors. No other behaviour change.

---

Published from tag `rist-runtime-v0.3.0`.
