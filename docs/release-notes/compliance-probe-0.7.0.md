
# compliance-probe 0.7.0

_Released 2026-11-07._

Breaking-class release (0.6 -> 0.7): `Probe::feed_scte35_section`, an unused-`self` passthrough, is removed; the SCTE-35 sanity check is called directly.

Version: 0.6.0 -> 0.7.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- `Probe::feed_scte35_section` (an unused-`self` passthrough). Call `compliance_probe::scte35::check_section` directly.

### Renamed and removed items

```text
probe.feed_scte35_section(&section)
    -> compliance_probe::scte35::check_section(&section)
```

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `dvb-conformance` 12 -> 13
- `media-doctor` 0.13 -> 0.14
- `media-plane` 0.9 -> 0.10
- `mpeg-ts` 0.6 -> 0.7
- `scte35-splice` 4 -> 5
- `timed-metadata` 0.8 -> 0.9

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `compliance-probe-v0.7.0`.
