# dvb-tools 16.0.0

_Released 2026-11-22._

Lockstep major (15.1 -> 16.0) with no functional change. The CLI is released because `dvb-si` and `dvb-t2mi` take the `broadcast-common` 14 / `mpeg-ts` 0.10 epoch and the family versions together; the `dump`, `services`, `epg`, `pids` and `t2mi` commands, their flags and their output are unchanged.

## Breaking changes

- Builds against `dvb-si` 16, `dvb-t2mi` 16 and `broadcast-common` 14 (the `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own diff, so the CLI moves a major with the family. A consumer that runs the binary rather than embedding the library needs no change.

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4009 (the empty `[Unreleased]` now names the lockstep epoch).

---

Published from tag `dvb-tools-v16.0.0`.
