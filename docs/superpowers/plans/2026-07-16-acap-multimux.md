# acap-multimux — On-Camera LL-HLS ACAP App Implementation Plan (Cycle 2 of #669)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An Axis ACAP app (`acap-multimux`) that captures the hardware-encoded H.264/H.265 stream via VDO and serves LL-HLS **on the camera**, reusing the `multimux` library (segmenter + RAM window + axum origin + blocking reload).

**Architecture:** A pure, host-testable `convert` module (VDO Annex-B access unit + timestamp + frame-type → `transmux::Sample` + `TrackSpec`, with in-band SPS/PPS/VPS extraction) has **no `vdo` dependency**. A `device`-feature-gated `VdoSource` (drives the acap-rs `vdo` crate) and the ACAP `main` bin call into `convert` and into `multimux`. The `vdo` dep + the bin only compile under `--features device` inside the Axis ACAP SDK Docker; the pure `convert` logic compiles + unit-tests on any host with synthetic H.264 fixtures.

**Tech Stack:** Rust (edition 2024, MSRV 1.86), acap-rs `vdo`/`axparameter` (git-pinned), published `multimux` 0.2 + `transmux` 0.17, axum, tokio.

## Global Constraints

- **Out of the main workspace.** `acap-multimux/` has its own `Cargo.toml` + `rust-toolchain.toml` (edition 2024, MSRV 1.86) and is NOT a workspace member (mirrors `bindings/`). It does not participate in the workspace lockstep or the workspace `--all-features`/`no_std` gates.
- **Dependencies:** `multimux = "0.2"` + `transmux = "0.17"` **by published crates.io version** (both live). acap-rs crates **git-pinned**: `rev = "8e58acb8f0617253ad21fb71ac319fea19454a38"`. The `vdo` dep is **optional**, behind the `device` feature — so a host build (no `device`) never pulls `vdo-sys` (whose `build.rs` needs `pkg-config probe("vdo")`, only resolvable in the ACAP SDK sysroot).
- **Two build environments:** host (macOS/Linux, no `device`) compiles + tests the `convert` lib only; the **Axis ACAP SDK Docker** (`axisecp/acap-native-sdk:12.1.0-{aarch64,armv7hf}`) compiles `--features device` + packages the `.eap` via `cargo-acap-build`. macOS cannot cross-compile ACAP.
- **Targets:** ARTPEC-7/8/9; codecs **H.264 (→avcC) and H.265 (→hvcC)**.
- **Manifest:** `schemaVersion "1.5.0"`; `acapPackageConf.setup{appName:"acap-multimux", vendor, runMode, version}` (appName == package name, version == package version); `configuration.reverseProxy` maps LL-HLS paths (`access: viewer`) + admin paths (`access: admin`) to `http://localhost:<port>`; `settingPage: "index.html"`.
- **Auth** is the device's (reverse-proxy `access` levels / VAPIX); the app builds none.
- **"Done" = hardware-verified** (hard rule): the `.eap` installs on the target camera and a real LL-HLS player plays live through the reverse-proxy. Cycle-2 tasks 1–6 are host/CI; task 7 is the hardware verify (user-run, camera available).

## Execution model per task

- Tasks 1–2 (scaffold + `convert`): **host-buildable + unit-tested** by subagents here (no `device`).
- Tasks 3–5 (`VdoSource`, bin, admin/axparameter): **written on host, `device`-gated** — they do NOT compile on macOS; a subagent writes them + confirms the non-device build still passes (`cargo build` without `device` = lib only) + `cargo clippy` of the pure parts; the `device` compile is verified by Task 6's CI.
- Task 6 (CI): the Linux GHA job that actually compiles `--features device` + builds the `.eap` — this is the compile-gate for tasks 3–5.
- Task 7 (docs + hardware verify): the user runs `cargo-acap-sdk install` + the on-camera playback check.

---

## File Structure

- `acap-multimux/Cargo.toml` — lib+bin; `device` feature gates `vdo` + the bin.
- `acap-multimux/rust-toolchain.toml` — `channel = "1.86"`, targets `aarch64-unknown-linux-gnu`, `thumbv7neon-unknown-linux-gnueabihf`.
- `acap-multimux/src/lib.rs` — `pub mod convert;` + `#[cfg(feature="device")] pub mod vdo_source;`.
- `acap-multimux/src/convert.rs` — **pure**: Annex-B AU → `Sample`; SPS/PPS/VPS extraction → `TrackSpec`; timing. No `vdo`.
- `acap-multimux/src/vdo_source.rs` — `#[cfg(feature="device")]` `VdoSource: multimux::pipeline::SampleSource`.
- `acap-multimux/src/admin.rs` — axum admin routes (config get/set via `axparameter`, status/health JSON). `axparameter` calls are `#[cfg(feature="device")]`; the route shapes are host-compilable with a trait-abstracted config store.
- `acap-multimux/src/bin/acap-multimux.rs` — `required-features = ["device"]`: ACAP entrypoint.
- `acap-multimux/manifest.json` — ACAP manifest.
- `acap-multimux/html/index.html` — admin `settingPage` (4 categories).
- `acap-multimux/tests/convert_synthetic.rs` — synthetic-H.264 conversion gate.
- `.github/workflows/acap-multimux.yml` — Linux CI `.eap` build.
- `acap-multimux/README.md` — what it is + build/deploy runbook.

---

### Task 1: Crate scaffold (host-buildable, device-gated vdo)

**Files:** Create `acap-multimux/Cargo.toml`, `acap-multimux/rust-toolchain.toml`, `acap-multimux/src/lib.rs`, `acap-multimux/src/error.rs`

**Interfaces:**
- Produces: the `acap-multimux` crate; host build (`cargo build`, no features) = lib only, no `vdo`. `pub mod convert;` `#[cfg(feature="device")] pub mod vdo_source;`. `error::{AcapError, Result}`.

- [ ] **Step 1: `acap-multimux/Cargo.toml`**

```toml
[package]
name         = "acap-multimux"
version      = "0.1.0"
edition      = "2024"
rust-version = "1.86"
license      = "MIT OR Apache-2.0"
description  = "Axis ACAP app: on-camera VDO capture -> LL-HLS origin (built on the multimux library)."
repository   = "https://github.com/fishloa/rust-broadcast"
publish      = false

[lib]
path = "src/lib.rs"

[[bin]]
name              = "acap-multimux"
path              = "src/bin/acap-multimux.rs"
required-features = ["device"]

[dependencies]
multimux  = "0.2"
transmux  = "0.17"
tokio     = { version = "1", features = ["rt-multi-thread", "net", "macros", "time", "sync"] }
axum      = "0.7"
serde     = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
log       = "0.4"

# device-only (ACAP SDK Docker): acap-rs crates, git-pinned (unpublished 0.0.0).
vdo          = { git = "https://github.com/AxisCommunications/acap-rs", rev = "8e58acb8f0617253ad21fb71ac319fea19454a38", optional = true }
axparameter  = { git = "https://github.com/AxisCommunications/acap-rs", rev = "8e58acb8f0617253ad21fb71ac319fea19454a38", optional = true }
acap-logging = { git = "https://github.com/AxisCommunications/acap-rs", rev = "8e58acb8f0617253ad21fb71ac319fea19454a38", optional = true }

[features]
default = []
# `device` pulls the ACAP-only crates + enables the VdoSource + the bin. It
# ONLY builds inside the Axis ACAP Native SDK sysroot (vdo-sys needs
# pkg-config `vdo`). Host builds omit it and compile just the pure `convert`
# lib + its tests.
device = ["dep:vdo", "dep:axparameter", "dep:acap-logging"]

[dev-dependencies]
```

- [ ] **Step 2: `acap-multimux/rust-toolchain.toml`**

```toml
[toolchain]
channel = "1.86"
components = ["clippy", "rustfmt"]
targets = ["aarch64-unknown-linux-gnu", "thumbv7neon-unknown-linux-gnueabihf"]
```

- [ ] **Step 3: `src/error.rs` + `src/lib.rs`**

`src/error.rs`:
```rust
//! Error type for acap-multimux.
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AcapError {
    /// A VDO access unit could not be converted to an IR sample.
    #[error("convert: {0}")]
    Convert(String),
    /// A transmux error while building codec config / samples.
    #[error("transmux: {0}")]
    Transmux(#[from] transmux::Error),
    /// A multimux error (pipeline/origin).
    #[error("multimux: {0}")]
    Multimux(#[from] multimux::MultimuxError),
}

pub type Result<T> = core::result::Result<T, AcapError>;
```

`src/lib.rs`:
```rust
//! acap-multimux — an Axis ACAP app that captures the camera's hardware-encoded
//! H.264/H.265 stream via VDO and serves LL-HLS on the camera, reusing the
//! `multimux` library. The pure `convert` module (VDO access unit -> IR sample)
//! is host-testable; the `vdo_source` module + the binary are `device`-gated
//! and build only inside the Axis ACAP Native SDK.

pub mod convert;
pub mod error;

#[cfg(feature = "device")]
pub mod vdo_source;

pub use error::{AcapError, Result};
```

(Create an empty `src/convert.rs` with a `//!` doc for now so the lib compiles; Task 2 fills it. `vdo_source.rs` is created in Task 3.)

- [ ] **Step 4: Verify host build (no device)**

```bash
cd acap-multimux
cargo build --locked
cargo fmt --all --check
RUSTFLAGS="-D warnings" cargo clippy --locked -- -D warnings
```
Expected: builds (lib only; no `vdo`/`vdo-sys` pulled — confirm with `cargo tree | grep -c vdo` = 0). If `multimux`/`transmux` aren't yet resolvable by version, they are live on crates.io (0.2.0 / 0.17.0) — `cargo build` fetches them.

- [ ] **Step 5: Commit**

```bash
git add acap-multimux/Cargo.toml acap-multimux/rust-toolchain.toml acap-multimux/src/lib.rs acap-multimux/src/error.rs acap-multimux/src/convert.rs
git commit -m "feat(acap-multimux): crate scaffold, device-gated vdo (#669)"
```

---

### Task 2: `convert` — VDO access unit → IR sample (pure, host-tested)

**Files:** Create/replace `acap-multimux/src/convert.rs`; create `acap-multimux/tests/convert_synthetic.rs`

**Interfaces:**
- Consumes: `transmux::pipeline::{Sample, TrackSpec, CodecConfig}`, `transmux::annexb`, `transmux::{AVCConfigurationBox, ...}` / `hevc_config` (confirm exact names in Step 0), `multimux::pipeline::SampleSource` (for the trait later).
- Produces:
  - `pub enum Codec { H264, H265 }`
  - `pub struct ParamSets { /* raw NAL bytes: H264 sps/pps; H265 vps/sps/pps */ }`
  - `pub fn extract_param_sets(codec: Codec, annexb_au: &[u8]) -> Option<ParamSets>` — scan Annex-B NAL units, collect parameter sets (H.264 types 7/8; H.265 types 32/33/34).
  - `pub fn track_spec(codec: Codec, params: &ParamSets, track_id: u32, clock_rate: u32) -> Result<TrackSpec>` — build `avcC`/`hvcC` → `CodecConfig` → `TrackSpec`.
  - `pub fn au_to_sample(codec: Codec, annexb_au: &[u8], duration_ticks: u32, is_sync: bool) -> Sample` — Annex-B AU → `Sample` (length-prefixed NALs via `Sample::from_annexb`, `composition_offset = 0`).
  - `pub fn duration_ticks(prev_ts_us: u64, ts_us: u64, clock_rate: u32) -> u32` — VDO µs-timestamp delta → track-timescale ticks (`clock_rate` = 90000 for H.264/H.265).

> **Step 0 (read before coding, on host — transmux is a published dep):** open the `transmux` source (crates.io cache or `../transmux/src`) to confirm: `Sample::from_annexb` signature (from #700 it was `Sample::from_annexb(annexb: &[u8], duration: u32, is_sync: bool, composition_offset: i32)`); the H.264 avcC builder (`avc_config_from_sprop` takes base64 — for raw SPS/PPS bytes, build the `AVCConfigurationBox`/`AVCDecoderConfigurationRecord` directly as `rtp_sdp` does, or base64-encode + call it); the **H.265 `hvcC`** builder (`hevc_config::HEVCConfigurationBox` / equivalent — find its constructor + `CodecConfig::Hevc{...}` variant fields); and the `annexb` NAL-iteration helper (start-code split + NAL-type read). Write `convert.rs` against the REAL signatures.

- [ ] **Step 1: Write the failing synthetic-H.264 test** (`tests/convert_synthetic.rs`)

Build a minimal Annex-B H.264 access unit by hand: SPS (NAL type 7) + PPS (type 8) + IDR slice (type 5), each prefixed with the 4-byte start code `00 00 00 01`. Assert:
- `extract_param_sets(Codec::H264, au)` returns `Some` with one SPS + one PPS whose bytes match.
- `track_spec(H264, params, 1, 90_000)` yields a `TrackSpec` with `timescale == 90_000` and a non-empty avcC (SPS/PPS present) — no empty placeholder.
- `au_to_sample(H264, au, 3000, true)` yields a `Sample` with `duration == 3000`, `is_sync == true`, non-empty length-prefixed `data`.
- `duration_ticks(1_000_000, 1_033_333, 90_000)` ≈ `3000` (33.333 ms @ 90 kHz).

```rust
// tests/convert_synthetic.rs — full test scaffolding; fill the exact assertions
// per the API above. Use real minimal SPS/PPS bytes (e.g. the known-good ones
// used in transmux's rtp_sdp tests: SPS 0x67,0x42,0xC0,0x1E,..; PPS 0x68,..)
// wrapped in Annex-B start codes, plus an IDR NAL (0x65, ..).
```

- [ ] **Step 2: Run — `cargo test --test convert_synthetic` → FAIL (not implemented).**

- [ ] **Step 3: Implement `convert.rs`** per the interfaces + Step-0 signatures. `extract_param_sets` splits on Annex-B start codes (`00 00 01` / `00 00 00 01`), reads each NAL's type (H.264: `nal[0] & 0x1F`; H.265: `(nal[0] >> 1) & 0x3F`), collects SPS/PPS(/VPS). `au_to_sample` calls `Sample::from_annexb`. `track_spec` builds avcC (H.264) or hvcC (H.265) from the collected param sets. No `vdo`/`std::process` — pure.

- [ ] **Step 4: Run — `cargo test --test convert_synthetic` → PASS.** Plus `cargo clippy --locked -- -D warnings`, `cargo fmt --all --check`.

- [ ] **Step 5: Commit** — `feat(acap-multimux): pure VDO-AU -> IR sample conversion + synthetic-H.264 gate (#669)`.

> H.265 conversion is exercised by an analogous synthetic test if a minimal hand-built HEVC VPS/SPS/PPS+IDR AU is feasible; otherwise H.265 is validated in the hardware step against a real capture (note it in the test file).

---

### Task 3: `VdoSource` (device-gated)

**Files:** Create `acap-multimux/src/vdo_source.rs`

**Interfaces:**
- Consumes: acap-rs `vdo::{StreamBuilder, Resolution, VdoFormat, RunningStream, StreamBuffer, VdoFrameType}`, `crate::convert`, `multimux::pipeline::SampleSource`, `transmux::pipeline::{Sample, TrackSpec}`.
- Produces (all `#[cfg(feature="device")]`): `pub struct VdoSource { .. }` + `VdoSource::new(codec, channel, width, height, framerate) -> Result<Self>` (builds + starts the stream, captures the first buffers to extract `ParamSets` → `TrackSpec`); `impl SampleSource for VdoSource` (`track_specs()` returns the built spec; `async fn next_samples()` calls `running.next_buffer()`, converts via `crate::convert::au_to_sample` using the frame's `frame_type()` for `is_sync` + `timestamp()` deltas for duration).

> **Step 0:** confirm the exact `vdo` API against the pinned rev (per the acap-rs research): `StreamBuilder::new().channel(u32).format(VdoFormat::VDO_FORMAT_H264|H265).resolution(Resolution::Exact{width,height}).framerate(u32).build() -> Result<Stream>`; `Stream::start() -> RunningStream`; `RunningStream::next_buffer() -> Result<StreamBuffer>`; `StreamBuffer::{as_slice()/data_copy(), frame_type() -> VdoFrameType (check VDO_FRAME_TYPE_H264_IDR / H265 IDR for sync), timestamp() -> u64 µs, sequence_number()}`. NO SPS/PPS accessor → extract in-band from the first buffers via `crate::convert::extract_param_sets` (retry across the first few buffers until the param sets appear, before/at the first IDR).

- [ ] **Steps:** Write the module; since it can't compile on host, the subagent's local verification is: (a) `cargo build` (no device) still succeeds (lib+convert unaffected); (b) `cargo build --features device` is EXPECTED to fail on host (`vdo-sys` pkg-config) — do NOT treat that as a task failure; note it; the real compile-gate is Task 6's CI. The subagent reviews the module by inspection against the Step-0 API. Commit: `feat(acap-multimux): VdoSource (device) — VDO capture -> SampleSource (#669)`.

---

### Task 4: ACAP bin + manifest (device-gated) — wire VDO → multimux origin

**Files:** Create `acap-multimux/src/bin/acap-multimux.rs`, `acap-multimux/manifest.json`, `acap-multimux/html/index.html`

**Interfaces:**
- Consumes: `crate::vdo_source::VdoSource`, `multimux::{store::StreamStore, origin::{AppState, router}, pipeline::run_pipeline}`, `crate::admin` (Task 5), acap-logging.

- [ ] **`src/bin/acap-multimux.rs`** (`required-features = ["device"]`, `#[tokio::main]`): init `acap_logging`; read config (Task 5's axparameter-backed `Config`); build a `StreamStore`; construct `VdoSource`; `tokio::spawn` `run_pipeline(store.clone(), target_duration, part_ms, vdo_source)`; build the axum app = `multimux::origin::router(AppState{streams})` **merged** with `crate::admin::router()`; bind `127.0.0.1:<port>` (the port the manifest reverse-proxies to); `axum::serve`. On pipeline error, log + keep serving.

- [ ] **`manifest.json`** (copy the acap-rs `reverse_proxy` template, adapt):
```json
{
  "schemaVersion": "1.5.0",
  "acapPackageConf": {
    "setup": { "appName": "acap-multimux", "vendor": "rust-broadcast", "runMode": "respawn", "version": "0.1.0" },
    "configuration": {
      "reverseProxy": [
        { "apiPath": "hls",   "target": "http://localhost:2999", "access": "viewer" },
        { "apiPath": "admin", "target": "http://localhost:2999", "access": "admin" }
      ],
      "settingPage": "index.html"
    }
  }
}
```
(LL-HLS playback URLs land at `https://<device>/local/acap-multimux/hls/<stream>/media.m3u8`; the app must serve its HLS routes under an `/hls/..` prefix OR the reverse-proxy strips `apiPath` — confirm the strip behavior in Task 6/7 against the device and adjust the router's base path. Note this as the one manifest↔router detail to verify on hardware.)

- [ ] **`html/index.html`** — the admin `settingPage`: a static page with the 4 sections (Stream config / LL-HLS tuning / Status-health / Access-security) that fetch+post the `admin` JSON endpoints (Task 5). Vanilla HTML/JS, no build step (per the workspace no-inline-style / scoped-style rules if any apply; keep minimal).

- [ ] **Verify:** host `cargo build` (no device) unaffected; device compile via Task 6 CI. Commit: `feat(acap-multimux): ACAP bin + manifest + admin settingPage (#669)`.

---

### Task 5: Admin routes + axparameter config

**Files:** Create `acap-multimux/src/admin.rs`

**Interfaces:**
- Produces: `pub struct Config { channel, width, height, framerate, codec, target_duration_secs, part_target_ms, window_segments, port }` (serde); `pub fn router() -> axum::Router<AppState>`-style admin routes: `GET /admin/config` (current config JSON), `POST /admin/config` (update via axparameter), `GET /admin/status` (running/stopped, current seg/part, uptime, frames, last error). The `axparameter` read/write is `#[cfg(feature="device")]` behind a `ConfigStore` trait so the route handlers + `Config` (de)serialization are host-compilable (a host stub store returns defaults).

> Keep the config store abstracted: `trait ConfigStore { fn load(&self) -> Config; fn store(&self, c: &Config) -> Result<()>; }`. `#[cfg(feature="device")] struct AxParameterStore` (uses `axparameter::Parameter::new("acap-multimux")` + `get::<String>`/`set`); a host `struct DefaultStore` for tests. Admin handlers are generic over `ConfigStore` — host-testable.

- [ ] **Steps:** TDD the admin route handlers on host (with `DefaultStore` + `tower::oneshot`): `GET /admin/config` → 200 + JSON; `POST /admin/config` with a body → 200 + persisted (via the stub); `GET /admin/status` → 200 + status JSON. axparameter wiring is device-gated (inspection + CI). Commit: `feat(acap-multimux): admin config/status routes + axparameter store (#669)`.

---

### Task 6: Linux CI — build the .eap in the ACAP SDK Docker

**Files:** Create `.github/workflows/acap-multimux.yml`

- [ ] **The workflow** (runs on PRs touching `acap-multimux/**` + on `acap-multimux-v*` tags): a job per target arch using `container: axisecp/acap-native-sdk:12.1.0-aarch64-ubuntu24.04` (and `-armv7hf-ubuntu24.04`). Steps: checkout; install the pinned Rust 1.86 toolchain + the target; `cargo install --locked --git https://github.com/AxisCommunications/acap-rs.git --rev 8e58acb… cargo-acap-build`; run `cargo-acap-build --target <arch> -- -p acap-multimux --features device` in `acap-multimux/`; upload the `target/acap/acap-multimux_0_1_0_<arch>.eap` as an artifact. This job is the **compile-gate** proving tasks 3–5's device code builds. Also run the host job: `cd acap-multimux && cargo test --locked` (the pure convert + admin tests) + clippy + fmt.

> **Step 0:** confirm the exact SDK image tag + whether `cargo-acap-build` needs `--` before `-p`, and the sysroot env the image pre-sets (the acap-rs devcontainer Dockerfile is the reference). If the image doesn't pre-set `PKG_CONFIG_*`/linker for the target, replicate the acap-rs `.devcontainer/Dockerfile` env in the workflow. Expect iteration here — it's the riskiest infra task.

- [ ] **Commit:** `ci(acap-multimux): build .eap in ACAP SDK Docker (aarch64 + armv7hf) (#669)`.

---

### Task 7: Docs + hardware verification (the real "done")

**Files:** Create `acap-multimux/README.md`; append a runbook.

- [ ] **README** — what it is; the two-env build model; **exact commands**:
  - Build the `.eap` (CI artifact, or locally in the SDK Docker): `docker run --rm -v "$PWD:/w" -w /w/acap-multimux axisecp/acap-native-sdk:12.1.0-aarch64-ubuntu24.04 bash -c "cargo-acap-build --target aarch64 -- -p acap-multimux --features device"`.
  - Install to the camera: `cargo-acap-sdk install --host <CAM_IP> --user root --pass <PW>` (or `AXIS_DEVICE_IP/USER/PASS` env), then `cargo-acap-sdk start --host <CAM_IP> ...`.
- [ ] **Hardware verify checklist (user runs — camera available):**
  1. `.eap` installs + app starts (ACAP list shows it running).
  2. `curl -u <user>:<pw> https://<CAM_IP>/local/acap-multimux/hls/<stream>/media.m3u8` returns an LL-HLS playlist with `#EXT-X-PART`/`#EXT-X-SERVER-CONTROL`.
  3. A real LL-HLS player (Safari / hls.js / ffplay) plays live video through the reverse-proxy with low latency.
  4. The admin settingPage loads at `https://<CAM_IP>/local/acap-multimux/` and config changes take effect.
  5. Both H.264 and H.265 (where the SoC supports it) verified.
  Record results; only claim DONE when playback works on the device.
- [ ] **Commit:** `docs(acap-multimux): build + hardware-verify runbook (#669)`.

---

## Self-Review

**Spec coverage (Cycle 2):** VdoSource → Tasks 2 (convert) + 3 (vdo). multimux-origin reuse → Task 4. ACAP shell (manifest/reverse-proxy) → Task 4. axparameter + admin pages → Tasks 4–5. Build/deploy (SDK Docker, .eap, targets) → Task 6. Out-of-workspace + git-pin + published multimux/transmux → Task 1. H.264+H.265 avcC/hvcC → Task 2. Hardware verify → Task 7. ✓

**Execution realism:** Tasks 1, 2, 5 are host-testable (subagents here). Tasks 3, 4 are device-gated (written on host, compiled by Task 6 CI). Task 6 is the device compile-gate. Task 7 is the user's hardware verify. This matches the "macOS can't cross-compile / subagents can't build the .eap" constraint.

**Flagged unknowns (resolved in-task against real code/hardware):** exact transmux avcC/hvcC + annexb signatures (Task 2 Step 0, host-readable); exact `vdo` API (Task 3 Step 0, from the pinned rev); in-band SPS/PPS/VPS extraction correctness (Task 2 synthetic + Task 7 real capture); reverse-proxy `apiPath` strip vs the router base path (Task 4/7 note); the SDK Docker env for cross-build (Task 6 Step 0, riskiest infra).

**Placeholders:** none of the "TODO/TBD" kind; the device-gated tasks intentionally defer *compilation* to CI (documented), not content.
