# dvb-t2mi 15.0.0

_Released 2026-11-15._

### Changed
- BREAKING: the `chrono` feature is replaced by `jiff` (no alias): `T2TimestampPayload::emission_time_utc` returns and `set_emission_time_utc` takes `jiff::Timestamp` instead of `chrono::DateTime<Utc>`; `chrono` is no longer a dependency (#2635).

---

Published from tag `dvb-t2mi-v15.0.0`.
