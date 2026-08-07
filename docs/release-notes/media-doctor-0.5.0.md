# media-doctor 0.5.0

**Release date:** 2026-07-29

Epoch-pure bump to `broadcast-common` 9 (issue #819). No functional or API
change of this crate's own.

## What changed

- Requires `broadcast-common` 9.

## Migration

**Breaking:** requires `broadcast-common` 9. Staying on 8 caused split-graph
trait-resolution errors when composed with 9-based crates (`transmux` 0.20,
`dvb-si` 9, etc.) — the 9.0.0 wave originally shipped only the crates needed
for the media-server hub, but these crates exist to be composed, and the
breakage only appears in a consumer that mixes them.
