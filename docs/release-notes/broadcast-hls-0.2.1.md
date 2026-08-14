# broadcast-hls 0.2.1

_Released 2026-08-14._

Patch. One fix, no API change.

## Fixed — `BYTERANGE` integer overflow accepted at parse time

`MediaPlaylist::parse` accepted a `BYTERANGE` whose `offset + length` overflows
`u64`, and an `EXT-X-PRELOAD-HINT` whose `BYTERANGE-START + BYTERANGE-LENGTH`
overflows. Both now fail parsing (issue #958).

Affected every byte-range-bearing tag: media segments, `EXT-X-MAP` and
`EXT-X-PART`.

**Why it mattered.** The pair parsed cleanly, so any downstream arithmetic on it
inherited the overflow — wrapping silently in release builds, panicking in
debug. A playlist is attacker-influenced input for anything pulling a remote
origin, so a wrapped range could address bytes the author never described.

Rejecting at parse time keeps the invariant where it belongs: a
`MediaPlaylist` that exists has ranges whose arithmetic is sound, and no
consumer needs to re-check.

## Upgrading

No API change. A playlist that previously parsed and then misbehaved downstream
now fails at `parse` with a structured error — which is the point, but is worth
knowing if you were relying on the permissive behaviour.

Published from tag `broadcast-hls-v0.2.1`.
