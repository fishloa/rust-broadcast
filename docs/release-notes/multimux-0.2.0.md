# multimux 0.2.0 — 2026-07-16

**Breaking: library-only restructure.** The `multimux` **CLI binary** (the RTSP
→ LL-HLS repackaging tool) has moved to a dedicated crate, **`multimux-cli`**
(0.1.0), and the `multimux` crate is now a **library only**. The `serve(config)`
entrypoint and all public API (`config`, `origin`, `pipeline`, `source`, `store`,
`error` modules) are unchanged. Downstream libraries depending on `multimux` are
unaffected; downstream tools using the CLI must switch to `cargo install
multimux-cli`.

## Breaking Changes

- The bundled `multimux` binary, `--rtsp`/`--name`/`--config` CLI flags, and the
  `cli` cargo feature have been removed from the `multimux` crate.
- The `clap` dependency is no longer in `multimux` (moved to `multimux-cli`).

## Migration

Users of the `multimux` CLI should install the new `multimux-cli` crate:

```bash
cargo install multimux-cli
```

This provides the same `multimux` binary with the same flags and behavior. See
`multimux-cli`'s documentation for usage.

## Library API

No changes. Callers using `multimux::config::{Config, Route}`,
`multimux::origin::serve`, and error types continue to work as-is.

## Features

- `tls` (default): TLS support for `rtsps://` sources.
- `testsupport`: test fixtures.
