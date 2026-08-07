# dvb-stream 0.4.0

Released 2026-07-29.

### Changed (BREAKING)

**Requires `dvb-si` 9 and `dvb-t2mi` 9** (issue #819). No functional change.
The published 0.3.1 still required `^8` of both as normal dependencies, so a
consumer combining it with `dvb-si` 9 got two majors in one graph. Found by the
published-dependency consistency check (#821) on its first run.
