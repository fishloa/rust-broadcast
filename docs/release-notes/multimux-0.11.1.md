# multimux 0.11.1

_Released 2026-10-11._

### Fixed
- Admin runtime install no longer uses `expect` on internal state: rollback ownership moved to a `RollbackGuard`, so the invalid state is unrepresentable. No public API change.
- WHIP media transport mutex is `parking_lot` (no poison panics); `take_transport` returns `Option` and a missing transport skips that session with an error log.

---

Published from tag `multimux-v0.11.1`.
