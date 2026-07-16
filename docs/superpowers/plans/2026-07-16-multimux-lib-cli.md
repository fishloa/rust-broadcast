# multimux → library + multimux-cli Implementation Plan (Cycle 1 of #669)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure `multimux` into a pure library (0.2.0, no binary) and move its CLI into a new `multimux-cli` binary crate (0.1.0), so downstream consumers (`multimux-cli` and, next cycle, `acap-multimux`) depend on the library.

**Architecture:** Delete `multimux`'s `[[bin]]` + `cli` feature (clap moves out); the library keeps its `pub async fn serve(config)` entrypoint + everything else. A new `multimux-cli` crate contains the exact clap CLI that lives in `multimux/src/bin/multimux.rs` today, depends on the `multimux` lib, and installs a `multimux` binary.

**Tech Stack:** Rust, edition 2024, tokio, clap (in multimux-cli), the `multimux` library.

## Global Constraints

- MSRV **1.86**, edition **2024**; build/test `--locked`.
- Both crates are workspace members (add `"multimux-cli"` to root `Cargo.toml` `[workspace] members`). Manifests keep manual column alignment.
- `multimux` **0.2.0** is a **breaking** release (removes the bundled binary) — CHANGELOG entry under a `### Breaking` heading. `multimux-cli` **0.1.0** is new.
- **Publish order** (Cycle-1 release): `multimux` 0.2.0 to crates.io FIRST, then `multimux-cli` 0.1.0 (it depends on `multimux` 0.2). Each has its own `release-<crate>.yml` lane (`multimux-v*` / `multimux-cli-v*`).
- CLI unchanged for users: the installed binary stays named `multimux` (via `multimux-cli`'s `[[bin]] name = "multimux"`), same flags per `docs/CLI-STANDARD.md`.
- No new behavior — this is a move + manifest split. The existing multimux lib tests + integration gates (`tests/origin_llhls.rs`, `tests/rtsp_ingest.rs`) must stay green unchanged.
- RELEASE-DOCS for both crates (CHANGELOG, release note, README).

---

## File Structure

- **Modify** `multimux/Cargo.toml` — remove `[[bin]]`, the `cli` feature, and the `clap` dep; bump to `0.2.0`; default features `["tls"]`.
- **Delete** `multimux/src/bin/multimux.rs` (moves to multimux-cli).
- **Modify** `multimux/CHANGELOG.md` — `[0.2.0]` breaking entry.
- **Create** `multimux-cli/Cargo.toml`, `multimux-cli/src/main.rs`, `multimux-cli/README.md`, `multimux-cli/CHANGELOG.md`.
- **Modify** root `Cargo.toml` — add `"multimux-cli"` to members.
- **Create** `.github/workflows/release-multimux-cli.yml`.
- **Create** `docs/release-notes/multimux-0.2.0.md`, `docs/release-notes/multimux-cli-0.1.0.md`.

---

### Task 1: multimux → library-only (0.2.0)

**Files:**
- Modify: `multimux/Cargo.toml`
- Delete: `multimux/src/bin/multimux.rs`
- Modify: `multimux/CHANGELOG.md`

**Interfaces:**
- Produces: the `multimux` library at 0.2.0 with `pub async fn serve(config: multimux::config::Config) -> multimux::Result<()>`, `pub mod {config, origin, pipeline, source, store, error}`, features `default = ["tls"]`, `tls`, `testsupport` (no `cli`).

- [ ] **Step 1: Delete the binary**

```bash
git rm multimux/src/bin/multimux.rs
```

- [ ] **Step 2: Edit `multimux/Cargo.toml`**

Replace the `[package]` version, remove the `clap` dependency line, drop `cli` from `default` + remove the `cli` feature, and remove the `[[bin]]` block. Resulting file:

```toml
[package]
name         = "multimux"
version      = "0.2.0"
edition      = "2024"
rust-version = "1.86"
license      = "MIT OR Apache-2.0"
description  = "Live RTSP -> LL-HLS just-in-time repackaging HTTP origin (library: tokio + axum)."
repository   = "https://github.com/fishloa/rust-broadcast"
keywords     = ["rtsp", "hls", "ll-hls", "cmaf", "origin"]
categories   = ["multimedia", "network-programming"]

[package.metadata.docs.rs]
all-features = true

[dependencies]
transmux      = { path = "../transmux",     version = "0.17" }
rtsp-runtime  = { path = "../rtsp-runtime",  version = "0.2", features = ["tokio"] }
sdp-types     = "0.1"
tokio         = { version = "1", features = ["rt-multi-thread", "net", "io-util", "macros", "time", "sync"] }
axum          = "0.7"
serde         = { version = "1", features = ["derive"] }
serde_json    = "1"
thiserror     = "2"
url           = "2"
tokio-rustls  = { version = "0.26", optional = true }

[features]
# `tls` is default-on: `rtsps://` sources should work out of the box. It pulls
# rtsp-runtime's `tls` feature (rustls/webpki-roots via its
# `default_tls_client_config` helper) plus `tokio-rustls` directly, needed here
# only to name the connected stream's concrete type
# (`tokio_rustls::client::TlsStream<TcpStream>`).
default     = ["tls"]
tls         = ["rtsp-runtime/tls", "dep:tokio-rustls"]
testsupport = []

[[example]]
name              = "serve_mock"
required-features = ["testsupport"]

[[example]]
name = "serve_rtsp"

[dev-dependencies]
tower = { version = "0.5", default-features = false, features = ["util"] }
```

- [ ] **Step 3: Update `multimux/CHANGELOG.md`**

Cut the current `[Unreleased]` (if any) and add at the top under the header:

```markdown
## [0.2.0] - 2026-07-16

### Breaking
- The bundled `multimux` **binary** (the RTSP→LL-HLS CLI) moved to a new
  dedicated crate, **`multimux-cli`**. `multimux` is now a **library only**
  (its `serve`/`config`/`origin`/`pipeline`/`source`/`store` API is unchanged).
  `cargo install multimux-cli` provides the `multimux` binary as before. The
  `cli` cargo feature (and the `clap` dependency) were removed from `multimux`.
```

- [ ] **Step 4: Verify the library builds + tests (no binary)**

Run each; all must pass:
```bash
cargo build -p multimux --all-features --locked
cargo build -p multimux --no-default-features --locked
cargo test  -p multimux --all-features --locked
cargo build -p multimux --all-features --examples --locked
RUSTFLAGS="-D warnings" cargo clippy -p multimux --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
```
Expected: all green. `cargo build -p multimux` no longer produces a `multimux` binary (lib + examples only). The integration tests (`tests/origin_llhls.rs`, `tests/rtsp_ingest.rs`) still pass.

- [ ] **Step 5: Commit**

```bash
git add multimux/Cargo.toml multimux/CHANGELOG.md
git commit -m "refactor(multimux)!: library-only 0.2.0 — bin moves to multimux-cli (#669)"
```

---

### Task 2: multimux-cli crate (0.1.0)

**Files:**
- Create: `multimux-cli/Cargo.toml`, `multimux-cli/src/main.rs`, `multimux-cli/README.md`, `multimux-cli/CHANGELOG.md`
- Modify: root `Cargo.toml` (add `"multimux-cli"` to `[workspace] members`)

**Interfaces:**
- Consumes: `multimux::config::{Config, Route}`, `multimux::{MultimuxError, Result}`, `multimux::origin::serve`.
- Produces: a `multimux` binary (crate `multimux-cli`) with the CLI unchanged from today.

- [ ] **Step 1: Create `multimux-cli/Cargo.toml`**

```toml
[package]
name         = "multimux-cli"
version      = "0.1.0"
edition      = "2024"
rust-version = "1.86"
license      = "MIT OR Apache-2.0"
description  = "CLI for multimux — a live RTSP -> LL-HLS just-in-time repackaging HTTP origin."
repository   = "https://github.com/fishloa/rust-broadcast"
keywords     = ["rtsp", "hls", "ll-hls", "cmaf", "origin"]
categories   = ["multimedia", "command-line-utilities"]

[dependencies]
multimux = { path = "../multimux", version = "0.2" }
tokio    = { version = "1", features = ["rt-multi-thread", "macros"] }
clap     = { version = "4", features = ["derive"] }

[[bin]]
name = "multimux"
path = "src/main.rs"
```

- [ ] **Step 2: Create `multimux-cli/src/main.rs`**

Move the deleted `multimux/src/bin/multimux.rs` here **verbatim** (its content is already correct — it imports from the `multimux` crate and calls `multimux::origin::serve`). Full file:

```rust
//! CLI for the `multimux` live RTSP -> LL-HLS just-in-time repackaging HTTP
//! origin.
//!
//! Either point it at a JSON config file describing one or more routes, or
//! use the single-route quick start (`--rtsp` + `--name`) for a single
//! source. See `multimux`'s README for the served endpoint table and v1 scope.
//!
//! # Example
//!
//! ```bash
//! multimux --rtsp rtsp://cam.local/stream --name cam1
//! multimux --config routes.json
//! ```

use std::path::PathBuf;

use clap::Parser;
use multimux::config::{Config, Route};
use multimux::{MultimuxError, Result};

#[derive(Parser)]
#[command(
    name = "multimux",
    version,
    about = "Live RTSP -> LL-HLS just-in-time repackaging HTTP origin",
    long_about = "Pulls one or more live RTSP sources and serves each as LL-HLS \
                  (RFC 8216bis) from an in-process HTTP origin.\n\
                  Either point it at a JSON config file (--config) describing one or \
                  more routes, or use the single-route quick start (--rtsp + --name)."
)]
struct Cli {
    /// JSON config file describing routes + segmentation/window/bind parameters.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["rtsp", "name"])]
    config: Option<PathBuf>,

    /// Single-route quick start: RTSP source URL to pull (requires --name).
    #[arg(long, value_name = "URL", requires = "name")]
    rtsp: Option<String>,

    /// Single-route quick start: served stream name, i.e. the URL path
    /// segment (requires --rtsp).
    #[arg(long, value_name = "NAME", requires = "rtsp")]
    name: Option<String>,

    /// `host:port` the HTTP origin binds.
    #[arg(long, value_name = "ADDR", default_value_t = Config::default().bind)]
    bind: String,

    /// Target full-segment duration, in seconds.
    #[arg(long, value_name = "SECS", default_value_t = Config::default().target_duration_secs)]
    target_duration: f64,

    /// LL-HLS part target, in milliseconds.
    #[arg(long, value_name = "MS", default_value_t = Config::default().part_target_ms)]
    part_ms: u32,

    /// Rolling window depth: full segments retained in RAM.
    #[arg(long, value_name = "N", default_value_t = Config::default().window_segments)]
    window: usize,
}

/// Build a [`Config`] from the parsed CLI: `--config <FILE>` if given,
/// otherwise the single-route quick start built from `--rtsp`/`--name` plus
/// the bind/timing/window flags.
fn build_config(cli: Cli) -> Result<Config> {
    if let Some(path) = cli.config {
        return Config::from_json_file(&path);
    }
    let rtsp_url = cli.rtsp.ok_or_else(|| {
        MultimuxError::Config(
            "either --config <FILE> or --rtsp <URL> --name <NAME> is required".into(),
        )
    })?;
    // clap's `requires = "name"` on `--rtsp` guarantees `cli.name` is present
    // whenever `cli.rtsp` is.
    let name = cli
        .name
        .expect("clap requires --name whenever --rtsp is given");

    let config = Config {
        bind: cli.bind,
        target_duration_secs: cli.target_duration,
        part_target_ms: cli.part_ms,
        window_segments: cli.window,
        routes: vec![Route { name, rtsp_url }],
    };
    config.validate()?;
    Ok(config)
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let config = build_config(cli)?;
    multimux::origin::serve(config).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn quick_start_flags_build_a_single_route_config() {
        let cli = Cli::parse_from([
            "multimux",
            "--rtsp",
            "rtsp://cam.local/stream",
            "--name",
            "cam1",
        ]);
        let cfg = build_config(cli).unwrap();
        assert_eq!(cfg.routes.len(), 1);
        assert_eq!(cfg.routes[0].name, "cam1");
        assert_eq!(cfg.routes[0].rtsp_url, "rtsp://cam.local/stream");
    }

    #[test]
    fn cli_definition_is_valid() {
        // Guards against a malformed clap derive (conflicts/requires wiring).
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
```

- [ ] **Step 3: Add to workspace members**

In the root `/Volumes/External/Projects/rust-broadcast/Cargo.toml`, add `"multimux-cli"` to the `[workspace] members` array (preserve formatting).

- [ ] **Step 4: Create `multimux-cli/README.md` + `multimux-cli/CHANGELOG.md`**

`README.md`: one-paragraph description (CLI for the multimux LL-HLS origin library), install (`cargo install multimux-cli`), the two usage forms (`--rtsp`/`--name` quick start; `--config routes.json`), a pointer to the `multimux` crate for the endpoint table + v1 scope, and the flag list (`--bind`/`--target-duration`/`--part-ms`/`--window`).

`CHANGELOG.md`:
```markdown
# Changelog

All notable changes to `multimux-cli` will be documented in this file.

## [0.1.0] - 2026-07-16

### Added
- Initial release: the `multimux` CLI binary, extracted from the `multimux`
  crate (which is now a library). `--config <FILE>` (JSON routes) or the
  single-route quick start `--rtsp <URL> --name <NAME>`, plus `--bind`,
  `--target-duration`, `--part-ms`, `--window`.
```

- [ ] **Step 5: Verify multimux-cli builds, the binary works, tests pass**

```bash
cargo build -p multimux-cli --locked
cargo test  -p multimux-cli --locked
cargo run   -p multimux-cli -- --help
RUSTFLAGS="-D warnings" cargo clippy -p multimux-cli --all-targets --locked -- -D warnings
cargo fmt --all --check
```
Expected: builds; `--help` prints the usage (binary named `multimux`); both unit tests pass.

- [ ] **Step 6: Commit**

```bash
git add multimux-cli/ Cargo.toml Cargo.lock
git commit -m "feat(multimux-cli): new CLI crate wrapping the multimux library (#669)"
```

---

### Task 3: Release lane + release notes + full-workspace gate

**Files:**
- Create: `.github/workflows/release-multimux-cli.yml`
- Create: `docs/release-notes/multimux-0.2.0.md`, `docs/release-notes/multimux-cli-0.1.0.md`

**Interfaces:** none (release infra + docs).

- [ ] **Step 1: Create `.github/workflows/release-multimux-cli.yml`**

Copy `.github/workflows/release-ts-fix.yml` and adapt: name "Release multimux-cli"; trigger `tags: ["multimux-cli-v*"]`; the dependency note states multimux-cli depends on the `multimux` library (publish `multimux` 0.2.0 FIRST) + is a std crate (drop the no_std gate); gate steps `cargo test -p multimux-cli --locked` + `cargo clippy -p multimux-cli --all-targets --locked -- -D warnings`; the tag-vs-version check uses `multimux-cli-v` prefix + package `multimux-cli`; publish `cargo publish -p multimux-cli --locked`. Full file:

```yaml
name: Release multimux-cli

# Publishes the multimux-cli crate to crates.io on a `multimux-cli-v*` tag.
# multimux-cli is versioned/released INDEPENDENTLY. CI is the only source of
# published artifacts — never `cargo publish` from a workstation.
#
# DEPENDENCY ORDER: multimux-cli depends on the `multimux` library. Publish
# `multimux` (>= the referenced 0.2) FIRST, or crates.io will reject the
# publish. multimux-cli is a std (tokio) CLI crate — no no_std gate.

on:
  push:
    tags: ["multimux-cli-v*"]

env:
  CARGO_TERM_COLOR: always

jobs:
  publish:
    name: publish multimux-cli to crates.io
    runs-on: ubuntu-latest
    environment: crates-io
    steps:
      - uses: actions/checkout@v5
      - uses: Swatinem/rust-cache@v2

      - name: Gate — multimux-cli tests
        run: cargo test -p multimux-cli --locked

      - name: Gate — multimux-cli clippy
        run: cargo clippy -p multimux-cli --all-targets --locked -- -D warnings

      - name: Gate — tag matches multimux-cli version
        run: |
          TAG="${GITHUB_REF_NAME#multimux-cli-v}"
          V=$(cargo metadata --no-deps --format-version 1 --locked \
              | python3 -c "import json,sys; print({p['name']: p['version'] for p in json.load(sys.stdin)['packages']}['multimux-cli'])")
          if [ "$V" != "$TAG" ]; then
            echo "::error::multimux-cli is $V but tag is $TAG"; exit 1
          fi

      - name: Publish multimux-cli
        run: |
          if out=$(cargo publish -p multimux-cli --locked 2>&1); then
            echo "$out"; echo "::notice::published multimux-cli"
          elif echo "$out" | grep -qiE "already (been )?uploaded|already exists"; then
            echo "$out"; echo "::notice::multimux-cli already on crates.io at this version — skipping"
          else
            echo "$out"; echo "::error::failed to publish multimux-cli"; exit 1
          fi
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
```

- [ ] **Step 2: Create the release notes**

`docs/release-notes/multimux-0.2.0.md` — headline: library-only restructure; the one breaking change (binary → `multimux-cli`); migration (`cargo install multimux-cli`); note the library API is unchanged since 0.1.0.

`docs/release-notes/multimux-cli-0.1.0.md` — headline: new CLI crate; what it is (the `multimux` binary now lives here); usage forms; depends on `multimux` 0.2 library.

Follow the format of `docs/release-notes/transmux-0.17.0.md`.

- [ ] **Step 3: Full-workspace CI-exact gate**

```bash
RUSTFLAGS="-D warnings" cargo build --workspace --all-features --locked
RUSTFLAGS="-D warnings" cargo build --workspace --no-default-features --locked
cargo test --workspace --all-features --locked
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
rustup run 1.86 cargo build -p multimux-cli --locked   # MSRV
```
Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release-multimux-cli.yml docs/release-notes/multimux-0.2.0.md docs/release-notes/multimux-cli-0.1.0.md
git commit -m "ci+docs(multimux-cli): release lane + release notes for 0.2.0/0.1.0 (#669)"
```

---

## Self-Review

**Spec coverage (Cycle 1 section of the design):**
- multimux → lib-only, drop bin + cli feature → Task 1. ✓
- multimux-cli new bin crate (clap + serve, `multimux` binary name) → Task 2. ✓
- serve() stays in the lib → unchanged (Task 1 keeps origin/mod.rs). ✓
- Add to workspace + release-multimux-cli.yml lane → Tasks 2, 3. ✓
- Versions: multimux 0.2.0 (breaking), multimux-cli 0.1.0 → Tasks 1, 2. ✓
- RELEASE-DOCS both → CHANGELOGs (Tasks 1, 2) + release notes (Task 3). ✓
- Existing multimux tests/gates stay green → Task 1 Step 4, Task 3 Step 3. ✓
- Publish order (multimux 0.2.0 before multimux-cli) → Global Constraints + release lane note. ✓

**Type consistency:** `Config`/`Route`/`MultimuxError`/`Result`/`origin::serve` used in multimux-cli/src/main.rs match the multimux lib's public API (verified against the current bin, which already compiles against these).

**No placeholders:** all files given in full (manifests, main.rs, workflow); release-note/README steps specify exact content to write.
