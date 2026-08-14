# multimux-cli 0.8.0

_Released 2026-08-14._

Minor. No CLI surface change; the version moves because its `multimux`
dependency crossed a caret epoch.

## Changed — builds against `multimux` 0.10

`multimux` 0.10.0 adds the `file` input scheme: a local media file as a route
source, identified with `container-probe`, demuxed by the matching `transmux`
demuxer, paced to wall clock, with optional looping. Suited to slates, idents
and filler.

Nothing in the binary changed to support it. `file` routes are configured
through `--config`, and the CLI hands routes to `multimux` unaltered, so the
flags and the JSON config schema are identical to 0.7.0.

## Why this is a minor, not a patch

`multimux-cli` 0.7.0 is published requiring `multimux ^0.9`. Shipping the new
dependency as 0.7.1 would leave the `0.7` compatibility bucket containing both
`^0.9` and `^0.10` requirements — two incompatible epochs a consumer's
`cargo update` could move between without any version change they would notice.

The workspace rule is that **every published bucket stays epoch-pure**: changing
which caret-epoch of a sibling a crate builds against is a major-class bump
(minor for `0.x`, major for `>=1.0`). See `docs/RELEASE-AUDIT.md` §2.

This one was missed when the rest of the wave was staged, and was caught by
`tools/check-published-dep-consistency.py`:

```text
multimux-cli 0.7.0 requires multimux ^0.9, but multimux-cli 0.7.0 in-tree
requires multimux ^0.10 — an epoch change (^0.9 → ^0.10) while staying in the
0.7 bucket; requires 0.8.0
```

## Publish order

`multimux-cli` depends on `multimux`, which depends on the new `container-probe`.
Publish in dependency order or the later tags fail to resolve:

1. `container-probe-v0.1.0`
2. `multimux-v0.10.0`
3. `multimux-cli-v0.8.0`

## Upgrading

`cargo install multimux-cli` as before. No configuration or flag changes.

Published from tag `multimux-cli-v0.8.0`.
