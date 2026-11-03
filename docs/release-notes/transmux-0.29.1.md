# transmux 0.29.1

_Released 2026-11-03._

Patch release: raises the minimum `cc-data` version. Upgrade if you build `transmux` 0.29.0 against a Cargo.lock that still holds `cc-data` 0.7.0.

### What was wrong
`transmux` 0.29.0 declared `cc-data = "0.7"`, which also accepts 0.7.0. Its caption detection (`caption_detect`) uses `Cea608Channel`'s `Ord`/`Hash` impls, `Cea608Channel::number`, `Cea608Decoder::has_caption_text` and `Cea708Decoder::services_seen`, which only exist in `cc-data` 0.7.1. With 0.7.0 locked, `transmux` failed to compile with 6 errors.

### Fixed
- The `cc-data` requirement is now `0.7.1`. Every other workspace-sibling requirement of `transmux` now names the sibling's full current version, so no declared floor is below a release that has the API in use.

### Workaround for 0.29.0
Run `cargo update -p cc-data` to move the lock to 0.7.1 or later, or upgrade to `transmux` 0.29.1.

### How it was found
A downstream build of the published crates against a Cargo.lock holding the older sibling (`cargo update -p cc-data --precise 0.7.0`). The in-tree build resolves siblings by path, so it could not show it.

---

Published from tag `transmux-v0.29.1`.
