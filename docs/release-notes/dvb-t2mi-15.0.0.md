# dvb-t2mi 15.0.0

_Released 2026-11-15._

Major-class release (14 -> 15) for the family's `chrono` -> `jiff` move and for epoch purity: the crate follows `broadcast-common` 12 -> 13, `dvb-bbframe` 14 -> 15 and `mpeg-ts` 0.8 -> 0.9, which is a major-class change on its own (docs/RELEASE-AUDIT.md section 2).

Version: 14.0.0 -> 15.0.0.

## Breaking changes and migration

The `chrono` feature is replaced by `jiff` (no alias) and `chrono` is no longer a dependency: `T2TimestampPayload::emission_time_utc` returns and `set_emission_time_utc` takes a `jiff::Timestamp` instead of `chrono::DateTime<Utc>`.

```toml
# before
dvb-t2mi = { version = "14", features = ["chrono"] }
# after (jiff is the default timestamp feature)
dvb-t2mi = "15"
```

```rust
// before
let t: chrono::DateTime<chrono::Utc> = payload.emission_time_utc()?;
// after
let t: jiff::Timestamp = payload.emission_time_utc()?;
```

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `dvb-bbframe` 14 -> 15, `mpeg-ts` 0.8 -> 0.9.

---

Published from tag `v15.0.0`.
