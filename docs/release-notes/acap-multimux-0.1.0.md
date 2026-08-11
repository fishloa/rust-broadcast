# acap-multimux 0.1.0

**Release date:** 2026-08-11

First release. An **Axis ACAP application** that captures a camera's
hardware-encoded H.264/H.265 stream via **VDO** and serves **Low-Latency HLS
from the camera itself** — no restream hop, no intermediate server.

It reuses the [`multimux`](https://crates.io/crates/multimux) library — the same
LL-HLS segmenter, RAM rolling window, axum origin and blocking-reload the
standalone `multimux` server uses — fed by a `VdoIngestSession` instead of RTSP.

## This crate is not on crates.io, by design

`acap-multimux` is `publish = false` and out of the workspace. Its deliverable
is a **`.eap` package** you install on a camera, not a Rust library. Attach it
from the assets below; there is nothing to `cargo add`.

## Downloads

Four packages, one per (firmware, architecture) cell. Match the pair to your
camera — a firmware-12 package will not install on an AXIS OS 11.x device:

| file | AXIS OS | architecture |
|---|---|---|
| `acap-multimux_0_1_0_fw12_aarch64.eap` | 12.x | aarch64 (ARTPEC-8/9) |
| `acap-multimux_0_1_0_fw12_armv7hf.eap` | 12.x | armv7hf (ARTPEC-6/7) |
| `acap-multimux_0_1_0_fw11_aarch64.eap` | 11.x LTS | aarch64 |
| `acap-multimux_0_1_0_fw11_armv7hf.eap` | 11.x LTS | armv7hf |

`SHA256SUMS` covers all four. Verify before installing:

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing
```

Install via the camera's web UI (**Apps → Add app**) or over VAPIX.

The firmware-11 cells (Native SDK 1.15.1) are built by the same CI matrix as
firmware 12 (SDK 12.1.0) but are marked **experimental** in the workflow: they
are not `fail-fast`, so a firmware-11 build failing never blocks the
known-good firmware-12 packages. Treat fw11 accordingly.

## Support

Target SoCs **ARTPEC-6 / 7 / 8 / 9**. **H.264** on all of them; **H.265** on
ARTPEC-7/8/9 only.

## Fixed before first release

Two defects found on a real camera, not in tests — both of the class where the
app appears healthy while doing nothing useful:

- **The config store had never worked on any camera** (issue #955).
  `AxParameterStore::store` called `Parameter::set("Config", …)` on a parameter
  that was never `add`ed; an ARTPEC-8 camera returned `Error -1 getting param
  in group`. `new` now `add`s the parameter with a default if it does not exist,
  following the vendored `axparameter_example`'s add-then-ignore-`ParamAdded`
  idiom, so a restart does not fail merely because the parameter now exists —
  and `manifest.json` sets `runMode: "respawn"`, so every restart took that
  path.
- **A broken config backend was indistinguishable from an unconfigured one.**
  `ConfigStore::load` discarded the backend's error and returned
  `Config::default()` either way. That is precisely why the bug above went
  unnoticed for a month: the app *looked* like it was running fine.

## Distribution changed with this release

Before this, the `.eap` files existed only as `actions/upload-artifact`
uploads from pull-request runs — they expired, and were reachable only from
the Actions run page. There was no versioned, downloadable build of this
application anywhere. An `acap-multimux-v*` tag now builds all four cells and
attaches them here.
