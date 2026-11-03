# broadcast-client 0.2.1

_Released 2026-11-03._

Patch release: raises the minimum `rtmp-runtime` version. Upgrade if you build `broadcast-client` 0.2.0 against a Cargo.lock that still holds `rtmp-runtime` 0.8.0.

### What was wrong
`broadcast-client` 0.2.0 declared `rtmp-runtime = "0.8"`, which also accepts 0.8.0. Its RTMP pull source uses `rtmp_runtime::client::ClientMode` and the `ClientEvent::Media`, `Playing` and `StreamEnded` variants, which only exist in `rtmp-runtime` 0.8.1. With 0.8.0 locked, `broadcast-client` failed to compile.

### Fixed
- The `rtmp-runtime` requirement is now `0.8.1`. Every other workspace-sibling requirement of `broadcast-client` now names the sibling's full current version, and the `transmux` requirement is `0.29.1`.

### Workaround for 0.2.0
Run `cargo update -p rtmp-runtime` to move the lock to 0.8.1 or later, or upgrade to `broadcast-client` 0.2.1.

### How it was found
A downstream build of the published crates against a Cargo.lock holding the older sibling (`cargo update -p rtmp-runtime --precise 0.8.0`). The in-tree build resolves siblings by path, so it could not show it.

---

Published from tag `broadcast-client-v0.2.1`.
