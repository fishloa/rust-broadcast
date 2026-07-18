# acap-multimux

An **Axis ACAP application** that captures the camera's hardware-encoded
H.264/H.265 stream via **VDO** and serves **LL-HLS on the camera** — no restream
hop. It reuses the [`multimux`](https://crates.io/crates/multimux) library (the
same LL-HLS segmenter + RAM rolling window + axum origin + blocking-reload that
the standalone `multimux-cli` server uses), fed by a `VdoSource` instead of RTSP.

Issue #669. Target SoCs: **ARTPEC-7 / 8 / 9**, both **H.264** and **H.265**.

## Architecture

```
VDO (libvdo) --coded AU + ts--> VdoSource --> transmux Sample/TrackSpec
    --> multimux::run_pipeline --> StreamStore (RAM window)
    --> axum (multimux::origin nested /hls  +  admin routes)  on 127.0.0.1:2999
    <-- camera web server reverse-proxies (manifest.json) with VAPIX access levels
```

- **`src/convert.rs`** — pure (host-testable): VDO Annex-B access unit →
  `transmux::Sample`/`TrackSpec`, in-band SPS/PPS/VPS extraction (avcC / hvcC).
- **`src/vdo_source.rs`** (`device`) — drives `vdo::StreamBuilder` /
  `RunningStream::next_buffer`, implements `multimux::pipeline::SampleSource`.
- **`src/admin.rs`** — `Config` + `ConfigStore` (`axparameter` on device) + the
  `/admin/config` + `/admin/status` routes.
- **`src/bin/acap-multimux.rs`** (`device`) — the ACAP entrypoint: loads config,
  runs the capture pipeline on a dedicated OS thread (VDO `next_buffer` blocks),
  serves the origin + admin on `127.0.0.1:2999`.
- **`manifest.json`** — ACAP `reverseProxy` (`hls` = viewer, `admin` = admin) +
  `settingPage` (`html/index.html`).

## Build model

`acap-multimux` is **out of the main cargo workspace** (its own `Cargo.toml` +
`rust-toolchain.toml`). It depends on published `multimux` 0.2 + `transmux` 0.17
(crates.io) and **git-pinned** acap-rs crates (`vdo`/`axparameter`/`acap-logging`
at rev `8e58acb`). It is **not** published to crates.io — it is a deployable
`.eap`.

- **Host** (macOS/Linux, no `device` feature): compiles + tests the pure lib
  (`convert`, `admin`): `cd acap-multimux && cargo test`.
- **Device** (the `.eap`): the `device` feature pulls the acap-rs crates and
  **only builds inside the Axis ACAP Native SDK Docker** (`vdo-sys` needs
  `pkg-config vdo` from the SDK sysroot; macOS cannot cross-compile ACAP).

### Build the `.eap`

CI (`.github/workflows/acap-multimux.yml`) builds both-arch `.eap`s on every PR
and uploads them as artifacts — grab those, or build locally in the SDK Docker:

```bash
docker run --rm -v "$PWD:/w" -w /w/acap-multimux \
  axisecp/acap-native-sdk:12.1.0-aarch64-ubuntu24.04 bash -lc '
    apt-get update && apt-get install -y --no-install-recommends build-essential curl ca-certificates pkg-config clang libclang-dev
    curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.86 --target aarch64-unknown-linux-gnu
    . "$HOME/.cargo/env"
    # (set the sysroot / PKG_CONFIG / linker env per .github/workflows/acap-multimux.yml)
    cargo install --locked --git https://github.com/AxisCommunications/acap-rs --rev 8e58acb8f0617253ad21fb71ac319fea19454a38 cargo-acap-build
    ACAP_BUILD_IMPL=equivalent cargo-acap-build --target aarch64 -- -p acap-multimux --features device
  '
# → acap-multimux/target/acap/acap-multimux_0_1_0_aarch64.eap
```

(The workflow is the authoritative, working recipe — the container env vars it
sets are required; this snippet elides them for brevity.)

### Deploy to a camera

Install `cargo-acap-sdk` (`cargo install --locked --git https://github.com/AxisCommunications/acap-rs --rev 8e58acb… cargo-acap-sdk`), then:

```bash
export AXIS_DEVICE_IP=<camera-ip> AXIS_DEVICE_USER=root AXIS_DEVICE_PASS=<pw>
cargo-acap-sdk install --host "$AXIS_DEVICE_IP" --user root --pass "$AXIS_DEVICE_PASS"
cargo-acap-sdk start   --host "$AXIS_DEVICE_IP" --user root --pass "$AXIS_DEVICE_PASS"
```

(`install` uploads the `.eap` over VAPIX. Or upload it via the camera's web UI:
Apps → Add app.)

## Hardware verification checklist (the real "done", #669)

On a target ARTPEC-7/8/9 camera:

1. `.eap` installs and the app shows **running** in the camera's Apps list.
2. `curl -u <user>:<pw> https://<cam>/local/acapmultimux/hls/cam/media.m3u8`
   returns an LL-HLS media playlist (`#EXT-X-PART`, `#EXT-X-PART-INF`,
   `#EXT-X-SERVER-CONTROL`).
3. A real LL-HLS player (Safari / hls.js / `ffplay`) plays **live** video via
   `https://<cam>/local/acapmultimux/hls/cam/media.m3u8` at low latency.
4. The admin `settingPage` loads at `https://<cam>/local/acapmultimux/` and a
   config change (e.g. part target) takes effect after restart.
5. Verify with **both H.264 and H.265** stream config.

> **Known verify-on-device detail:** the origin is served nested under `/hls`;
> if the camera's reverse-proxy strips the `apiPath` segment before forwarding,
> the nest prefix may need to be dropped (adjust `src/bin/acap-multimux.rs`'s
> `Router::nest("/hls", …)`). Confirm the actual proxied path on the device and
> adjust if the playlist 404s. Likewise confirm VDO delivers Annex-B (start
> codes) — `convert` assumes it; if VDO frames carry a different framing, adjust
> `VdoSource`'s `data_copy()` handling.

## License

MIT OR Apache-2.0.
