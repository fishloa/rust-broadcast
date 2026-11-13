# broadcast-hls 0.7.0

_Released 2026-11-14._

Breaking-class release (0.6 -> 0.7): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

Version: 0.6.0 -> 0.7.0.

## Breaking changes and migration

- Behaviour: `MasterPlaylist::to_m3u8` no longer fails with "group not found" for an `#EXT-X-STREAM-INF` whose group has no `#EXT-X-MEDIA`; parse keeps the reference and render emits it unchanged. Callers relying on `to_m3u8` as a validator must now call `dangling_group_references()`.

## Added

- `MasterPlaylist::dangling_group_references()` lists groups a variant names that no `#EXT-X-MEDIA` declares.
- `OpenSegment::discontinuous`: an `#EXT-X-DISCONTINUITY` ahead of live-edge parts kept on parse/render.

## Fixed

- `MediaPlaylist::parse` no longer drops `#EXT-X-DISCONTINUITY` preceding open segment's parts.

## Also in this release

### Changed
- BREAKING: a malformed `#EXT-X-PROGRAM-DATE-TIME` no longer fails `MediaPlaylist::parse`: the tag is dropped (its segment has no `program_date_time`) and recorded in the new `MediaPlaylist::diagnostics` (`ParseDiagnostic { kind: ParseDiagnosticKind::InvalidProgramDateTime, line_no, line }`, never serialised). `ProgramDateTime::parse` itself stays strict (#2591).
- BREAKING: `ProgramDateTime::parse` rejects a bare date (`2026-10-02`): RFC 8216 §4.3.2.6 asks for an ISO 8601 date-time. A zone-less date-time is still accepted and taken as UTC (#2591).

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

---

Published from tag `broadcast-hls-v0.7.0`.
