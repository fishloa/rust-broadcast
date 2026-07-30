# ll-hls-runtime 0.3.0 — 2026-07-30

**Minor (breaking: `#[non_exhaustive]` on `LlHlsRequest`/`LlHlsBody`).**

See the [lockstep 9.1.0 release note](v9.1.0.md) for the full summary.

## Breaking

- `LlHlsRequest`, `LlHlsBody` (`server::engine`) now carry `#[non_exhaustive]` (#806). A downstream `match` on either now needs a wildcard arm.
- The client's part-prefetch now ignores future `PreloadHintType` variants defensively.

## Added

- `tests/non_exhaustive_coverage.rs` drift guard (#806).
- Now requires `transmux` 0.21.
