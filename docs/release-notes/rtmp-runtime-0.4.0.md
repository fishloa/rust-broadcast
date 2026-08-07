# rtmp-runtime 0.4.0

**Release date:** 2026-08-05

Dependency bump to transmux 0.23, which added `TrackSpec::program_number` for MPTS support. No change in this crate's own logic; the bump propagates the pre-1.0 caret boundary (^0.21 to ^0.23).

## What's changed

- Requires `transmux` 0.23 (`TrackSpec` is part of this crate's public ingest API).

## Migration

Requires `transmux` 0.23.
