# dvb-tools 15.0.0

_Released 2026-11-15._

Major-class release (14 -> 15) for the family's `chrono` -> `jiff` move and for epoch purity: the crate follows `broadcast-common` 12 -> 13, `dvb-si` 14 -> 15, `dvb-t2mi` 14 -> 15 and `mpeg-ts` 0.8 -> 0.9, which is a major-class change on its own (docs/RELEASE-AUDIT.md section 2). The CLI's flags and output shape are unchanged apart from one timestamp format.

Version: 14.0.0 -> 15.0.0.

## Breaking changes and migration

The `epg` subcommand prints an event's start time as RFC 3339 with a `Z` suffix (jiff's `Timestamp`) instead of `+00:00`, and `dvb-si` is built with its `jiff` feature in place of the removed `chrono`. A script that matched the old `+00:00` text must match the new form.

```text
# before
2026-11-15T20:00:00+00:00
# after
2026-11-15T20:00:00Z   (jiff's Timestamp Display)
```

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `dvb-si` 14 -> 15, `dvb-t2mi` 14 -> 15, `mpeg-ts` 0.8 -> 0.9.

---

Published from tag `v15.0.0`.
