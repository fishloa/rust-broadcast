# dvb-ci-runtime 0.18.1

_Released 2026-10-17._

**Patch release; fixes a production bug in 0.18.0.** `RecordingCaDevice<D>` did not forward `CaDevice::reset_settle()`, so it fell back to the trait default (zero). With a Linux CA device inside it (a Digital Devices CXD2099 needs a settle time after the CA reset), `Driver::init` wrote the first TPDU straight after the reset and the slot answered `EINVAL` (`write (5 byte frame: slot 0, connection 1) failed: Invalid argument (os error 22)`) on every attempt. The `ci-probe` binary uses `RecordingCaDevice<LinuxCaDevice>`, so the released 0.18.0 `ci-probe` was affected too. Reported by zenith from production.

## Why this version

- **Proposed version:** 0.18.0 -> **0.18.1** (patch: a forwarding bug fixed; no API change).
- **No dependency changes.**
- **Hardware:** the fix has a unit test and a lexical guard but could not be exercised on a CAM in CI; the previous hardware runs passed only because the reporter's own logging wrapper forwarded `reset_settle`.

## What to do

Upgrade; no code change is needed. If you worked around the bug by dropping `RecordingCaDevice` you can put it back. A wrapper of your own around `CaDevice` must forward `reset_settle()` explicitly: the trait default is zero.

The crate now carries `tests/wrapper_forwards_defaults.rs`, a behavioural test plus a lexical guard that fails if a defaulted `CaDevice`/`CiDataDevice` method is added to the trait without every wrapper in the crate forwarding it.

## What changed

### Fixed
- `RecordingCaDevice<D>` now forwards `CaDevice::reset_settle()` to the wrapped device. It fell back to the trait default (`Duration::ZERO`), so with a `LinuxCaDevice` inside (CXD2099, `RESET_SETTLE` = 3 s) `Driver::init` wrote the first TPDU straight after `CA_RESET` and every attempt failed with `EINVAL` (reported on 0.18.0; also affected `ci-probe`). An audit of every `CaDevice`/`CiDataDevice` impl in the workspace found no other wrapper dropping a defaulted method; the new `tests/wrapper_forwards_defaults.rs` pins this (a behavioural test plus a lexical guard that fails when a defaulted trait method is added without being forwarded by a wrapper impl). Not exercised on real hardware.

---

Published from tag `dvb-ci-runtime-v0.18.1`.
