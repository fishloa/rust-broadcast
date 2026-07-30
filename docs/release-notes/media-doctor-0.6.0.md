# media-doctor 0.6.0 — 2026-07-30

**Minor (breaking: `#[non_exhaustive]` on `cli::Cli`).**

See the [lockstep 9.1.0 release note](v9.1.0.md) for the full summary.

## Breaking

- `cli::Cli` now carries `#[non_exhaustive]` (#806). Not expected to affect real consumers — this is the binary's own top-level subcommand enum.

## Added

- HLS manifest validator `check_hls_playlist()` (#756): structured M3U8 parse via `transmux::MediaPlaylist::parse`, plus 5 LL-HLS rules. 12 deferred rules documented in README.
- DASH MPD validator `check_dash_manifest()`: structured MPD parse with 9 rules.
- New `check-hls` and `check-dash` CLI subcommands.
- `tests/non_exhaustive_coverage.rs` drift guard (#806).
- Now requires `transmux` 0.21.
