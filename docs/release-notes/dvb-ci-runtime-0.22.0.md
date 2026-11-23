# dvb-ci-runtime 0.22.0

_Released 2026-11-22._

Minor for a 0.x crate. No public API change: the `linux` feature no longer depends on `libc` (the CA-device ioctls, errno values and `O_NONBLOCK` go through `rustix`), and the crate builds at the wave's epochs. Error types derive `thiserror::Error` with unchanged messages.

## Breaking changes

- Builds against new caret epochs of workspace siblings (`broadcast-common` 13 -> 14, `dvb-ci` 0.13 -> 0.14, `dvb-si` 15 -> 16, `mpeg-ts` 0.9 -> 0.10). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch, and the `CA_PMT`/section types it takes are those versions' types.

## Changed

- The `linux` feature no longer depends on `libc`: the CA-device ioctls, errno values and `O_NONBLOCK` go through `rustix` (#3972, #4019). No public API change.
- Internal: error types derive `thiserror::Error` (messages unchanged) (#3974, #4020).

## Tests

The Linux CA-device poll path, the TPDU/SPDU poll loop's state machines, the CAM/card hot-plug notifications and the managed CAS layer's descrambler transitions are covered by the crate's suites; the device-level tests skip loudly when no CA device is present.

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4019 (`rustix` in the CHANGELOG), #4020 (`thiserror`), #4021 (test bullets under `### Tests`).

---

Published from tag `dvb-ci-runtime-v0.22.0`.
