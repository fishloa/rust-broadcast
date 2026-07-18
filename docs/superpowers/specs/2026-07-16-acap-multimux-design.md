# acap-multimux — On-Camera LL-HLS Origin (Design)

> Design spec for issue #669. Approved 2026-07-16. Two-cycle epic: (1) restructure
> `multimux` into a pure library + a `multimux-cli` binary; (2) build
> `acap-multimux`, an Axis ACAP app that captures via VDO and serves LL-HLS
> **on the camera** (no restream hop), reusing the multimux library.

## Goal

Run the exact LL-HLS origin pipeline that `multimux` ships (SampleSource →
segmenter → RAM window → axum origin with blocking reload) **inside an Axis
camera** as an ACAP app, fed directly by the camera's hardware encoder via VDO —
eliminating the network + restream hop of the standalone `multimux` server.

## Target architecture (3 crates)

- **`multimux` (library, → 0.2.0)** — workspace member, **no binary**. The
  source-agnostic core, all `pub`: `SampleSource` trait, `StreamStore` (RAM
  rolling window + `watch`), `origin` (axum router + handlers + LL-HLS blocking
  reload), `run_pipeline`, `config`, and `RtspSource` (kept in the lib
  **unconditionally**, per decision — the camera build carries an unused
  rtsp-runtime, accepted).
- **`multimux-cli` (new workspace binary crate, 0.1.0)** — thin: clap CLI +
  `serve()`, the native RTSP→LL-HLS binary that today lives in
  `multimux/src/bin/multimux.rs`. Depends on the `multimux` lib.
- **`acap-multimux` (Axis ACAP app, OUTSIDE the main workspace)** — lives under
  a dedicated dir (sibling to `bindings/`), with its own `Cargo.toml`, own
  toolchain, and its own release/build path. Depends on **published**
  `multimux` 0.2.0 + `transmux` 0.17.0 (by version), plus **git-pinned** acap-rs
  crates. Not published to crates.io (it is a deployable `.eap`, not a library).

Reuse flows through the `multimux` library: `acap-multimux` writes only a
`VdoSource` (VDO → `Sample`) and the ACAP shell; everything downstream (store,
playlist, origin, blocking reload) is the shipped, tested multimux code.

## Cycle 1 — multimux 0.2.0 restructure (library + CLI)

Self-contained, releasable. Its own implementation plan.

- Move `multimux/src/bin/multimux.rs` + the `serve()` entrypoint's CLI wiring
  into a new `multimux-cli` crate (`multimux-cli/src/main.rs`); it depends on
  `multimux` (path+version) and re-exposes the same CLI (clap, `--config` /
  `--rtsp`/`--name` + tuning flags) per `docs/CLI-STANDARD.md`.
- `multimux` lib drops its `[[bin]]` + the `cli` feature (clap moves to
  multimux-cli). The `serve(config)` entrypoint stays in the lib (both
  consumers call it). `testsupport` feature stays.
- Version: `multimux` **0.2.0** (breaking — removes the bundled binary);
  `multimux-cli` **0.1.0** (new). Add `release-multimux-cli.yml` lane.
- Gate: the existing multimux tests + integration gate stay green (they test the
  lib); `multimux-cli` gets a smoke test (parses args → builds `Config`).
- RELEASE-DOCS for both: CHANGELOG (`multimux` 0.2.0 "**Breaking:** binary moved
  to the `multimux-cli` crate"), `multimux-cli` README/CHANGELOG/release-note.

## Cycle 2 — acap-multimux (the ACAP app)

Own implementation plan; starts after `multimux` 0.2.0 is published.

### VdoSource (the only new media logic)
`vdo::StreamBuilder::new().channel(0).format(H264|H265).resolution(..).framerate(..).build()`
→ `RunningStream`; loop `next_buffer()` → `StreamBuffer { as_slice(), timestamp()
(µs), sequence_number() }`. Each buffer is a complete coded access unit. Split
it into NALs (reuse `transmux::annexb` / the length-prefix helpers — do NOT
hand-roll), build a `transmux::pipeline::Sample` with real `duration` (from VDO
µs-timestamp deltas mapped to the track timescale), `is_sync` (IDR NAL present),
`composition_offset = 0` (low-delay). Build the `TrackSpec` codec config from
VDO's format + the parameter sets in the stream (H.264 SPS/PPS → `avcC`; H.265
VPS/SPS/PPS → `hvcC` via transmux's `hevc_config`), or VDO metadata. `impl
multimux::SampleSource for VdoSource`.

### Serving (reuse multimux verbatim)
`multimux::run_pipeline(VdoSource) → StreamStore`; `multimux::origin::router` on
`127.0.0.1:<port>`; the camera's own web server reverse-proxies to it.

### ACAP shell
- **`manifest.json`** — `acapPackageConf` (appName `acap-multimux`, version,
  architecture aarch64/armv7hf, required ACAP API version), a **`reverseProxy`**
  array mapping `apiPath`s to `http://localhost:<port>` with `access` levels
  (`viewer` for the HLS playback endpoints; `admin`/`operator` for config),
  and `settingPage: "index.html"`.
- **Auth** — delegated to the device via the reverse-proxy `access` levels
  (VAPIX session); the app builds no auth of its own.
- **Config** — `axparameter` (acap-rs) for persisted app parameters (stream
  channel, resolution/fps, segment duration, part target, window depth, bind
  port).
- **Admin pages** (the four categories, served as the `settingPage` static web
  bundle + small JSON control endpoints behind `admin` access):
  1. **Stream config** — VDO channel/resolution/fps/codec.
  2. **LL-HLS tuning** — segment duration, part target, window depth.
  3. **Status/health** — running/stopped, current segment/part, uptime, last
     error, frames captured.
  4. **Access/security** — surfaces the ACAP access-level mapping (read-only in
     v1; auth is the device's).

### Build / deploy
- Build in the **Axis ACAP Native SDK Docker** image. CI builds against both
  **firmware 11** (SDK `1.15.1-{aarch64,armv7hf}-ubuntu22.04`) and **firmware 12**
  (SDK `12.1.0-{aarch64,armv7hf}-ubuntu24.04`), producing `.eap`s for each.
  Rust toolchain ≥1.86 (acap-rs's 1.85.1 pin is only their dev pin — newer Rust
  compiles their edition-2021 crates). Cross-targets: `aarch64-unknown-linux-gnu`,
  `thumbv7neon-unknown-linux-gnueabihf` (glibc, Yocto sysroots from the SDK
  image). Package with `cargo-acap-build` → `.eap`.
- **acap-rs pin:** uses `fishloa/acap-rs` fork (rev `e9a838d`) instead of the
  upstream `AxisCommunications/acap-rs` (rev `8e58acb`). The fork drops a
  firmware-12-only `VDO_ERROR_NO_VIDEO` diagnostic enum arm so the `vdo` crate
  compiles against the firmware 11 SDK as well.
- Deploy to the target camera with `cargo-acap-sdk install/start` over SSH.

## Constraints

- **acap-rs is experimental + unpublished** (all API crates `0.0.0`, git-only,
  "may become unmaintained"). Pin a **specific commit** of the acap-rs monorepo;
  vendor if it disappears. The sys crates are thin bindgen FFI (low churn).
- **Out of the main workspace** — `acap-multimux` has its own `Cargo.toml`
  (edition 2024, our MSRV), git-deps on acap-rs, version-deps on published
  multimux/transmux, and does NOT participate in the workspace lockstep or the
  `--all-features`/`no_std` workspace gates. Mirrors the `bindings/` pattern.
- **Target SoCs: ARTPEC-6 / 7 / 8 / 9**. ARTPEC-6 supports **H.264 only**;
  ARTPEC-7/8/9 support **both H.264 and H.265** (H.265 encoder not available on
  ARTPEC-6). `VdoSource` builds a `TrackSpec` for the available codec: H.264 →
  `avcC`, H.265 → `hvcC` (transmux's existing `hevc_config`). The default config
  uses H.264, so ARTPEC-6 works out of the box; setting H.265 on ARTPEC-6 will
  fail at VDO stream open (logged as a pipeline error; the HTTP origin stays up).
- macOS cannot cross-compile ACAP — all camera builds happen in the SDK Docker.

## Testing / verification

- **Cycle 1**: multimux lib tests + integration gate green; multimux-cli smoke
  test; full workspace 6-gate.
- **Cycle 2 (library-testable parts, no camera)**: `VdoSource`'s NAL-split +
  Sample-timing logic is unit-tested against **captured VDO buffer fixtures**
  (record a few real `StreamBuffer` byte blobs + timestamps from the camera,
  commit them, assert correct `Sample`s) — the VDO *capture* is mocked, the
  *conversion* bites. The origin/store path is already gated in multimux.
- **Cycle 2 hardware gate (the real "done", per the hard rule)**: cross-build
  the `.eap`, `cargo-acap-sdk install` to the **target ARTPEC-6/7/8/9 camera**,
  start it, and verify end-to-end — a real HLS/LL-HLS player pulls
  `https://<cam>/local/acap-multimux/.../media.m3u8` through the camera's
  reverse-proxy and plays live low-latency video. DONE is not claimed until this
  passes on the device.

## Risks

- acap-rs churn / abandonment → commit-pin + vendor fallback.
- VDO codec-config extraction (H.264 SPS/PPS → `avcC`; H.265 VPS/SPS/PPS →
  `hvcC`) — VDO gives coded AUs; the parameter sets may be in-band (before the
  first IDR) or need a VDO settings query. Cycle 2's first task resolves exactly
  how the param sets reach the `TrackSpec` (in-band scan vs VDO API) for BOTH
  codecs against a real capture from the target camera.
- Cross-compile toolchain assembly (our Rust + Axis Yocto sysroots + the two
  target triples) — a Cycle-2 task-0 spike proves the `.eap` builds + installs a
  hello-world before the media work.

## Decomposition

Two implementation plans, sequential:
1. `docs/superpowers/plans/…-multimux-lib-cli.md` (Cycle 1) — build + release
   multimux 0.2.0 + multimux-cli.
2. `docs/superpowers/plans/…-acap-multimux.md` (Cycle 2) — build acap-multimux on
   the published lib; hardware-verify on the target camera.
