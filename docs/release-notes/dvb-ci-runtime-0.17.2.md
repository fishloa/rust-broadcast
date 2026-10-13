# dvb-ci-runtime 0.17.2

_Released 2026-10-13._

### Changed
- `CaDevice::reset` on Linux no longer sleeps the 3 s settle time (audit r10-O-10): a CAM hot-plug edge used to freeze `Driver::pump` and any `CaDescrambler` data plane for 3 s. New defaulted trait method `CaDevice::reset_settle() -> Duration` (default `ZERO`, so existing implementations whose `reset` already blocks keep working); the `Driver` reads it after `Action::Reset`, holds the slot quiet on its injectable clock (no `Create_T_C`, no reads, no timer advance; slot-status sampling continues), then sends the deferred `Create_T_C`. A second reset restarts the deadline. `pump` returns immediately while settling, so callers' loops pace themselves.

---

Published from tag `dvb-ci-runtime-v0.17.2`.
