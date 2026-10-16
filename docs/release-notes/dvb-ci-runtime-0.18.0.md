---
status: DRAFT
---
# dvb-ci-runtime 0.18.0

_Released 2026-10-16._

**Breaking release.** The CAM driver survives CAMs that code long-form lengths on hardware, retries the post-reset `EINVAL` window (a DD CXD2099 with an AlphaCrypt needed up to ~12 s), splits the data plane into two one-direction descriptors when the kernel rejects `O_RDWR`, names operation/device/errno in every device error, and takes typed `Pid`s.

## Why this version

- **Proposed version:** 0.17.2 -> **0.18.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (typed Pid on ManagedService/ManagedCa/Driver APIs; dvb-ci length_form).
- **cargo-semver-checks vs live 0.17.2 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** dvb-ci length_form. Siblings it depends on that move epoch in this wave: broadcast-common, dvb-ci, dvb-si, mpeg-ts.
- **Caret epoch:** 0.17 -> 0.18; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-dvb-ci = "0.9"
+dvb-ci = "0.10"
-dvb-si = "11.1"
+dvb-si = "12.0"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
```

## Breaking changes and migration

```rust
// before
service.es_pids: Vec<u16>      driver.required_pids() -> Vec<u16>
// after
service.es_pids: Vec<Pid>      driver.required_pids() -> Vec<Pid>      // mpeg_ts::pid::Pid
```

`ManagedService::{es_pids,ca_pids,pcr_pid}` and `ManagedCa`/`Driver`/`CaDescrambler` `emm_pids`/`descramble_pids`/`ca_pids`/`required_pids` use `Pid`. The `dvb-ci` objects built or matched in tests and host code carry the new `length_form` field (`Default`/`Minimal`). A failing device operation now returns `DeviceOpError` wrapped in the `io::Error`: `err.raw_os_error()` no longer answers directly; use `DeviceOpError::errno_of(&err)`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Added
- `LinuxCiDataDevice::open_split(adapter, ci)` / `LinuxCiDataDevice::from_split(write, read)`: the data plane as two one-direction descriptors (`O_WRONLY` write, `O_RDONLY | O_NONBLOCK` read). The Digital Devices CXD2099 `ci0` (dddvb, Ubuntu 5.15) rejects `O_RDWR` with `EINVAL`, so `LinuxCiDataDevice::open` now tries `O_RDWR` first and falls back to the split open on `EINVAL` and only `EINVAL`. Verified on that hardware with two `from_file`-wrapped fds: 150 s, 111k `feed_ts` calls, max 249 us, 0 errors (#1066).
- `DeviceOpError` (re-exported): every failing Linux device operation (`open`, `read`, `write`, `poll`, `ioctl CA_RESET`, `ioctl CA_GET_SLOT_INFO`) now names the operation, the device path and the errno, e.g. `/dev/dvb/adapter3/ca0: write (17 byte frame: slot 0, connection 1) failed: Invalid argument (os error 22)`. The `io::ErrorKind` is unchanged; the errno is `DeviceOpError::errno_of(&err)` (a wrapped error no longer answers `raw_os_error()` itself) (#1032).

#### Fixed
- `Driver::pump`: the first write after the post-`CA_RESET` settle (the `Create_T_C`) that fails `EINVAL` is retried every 250 ms for up to 12 s past the settle window instead of aborting the pump. The kernel's `dvb_ca_en50221` answers `EINVAL` to a write until the slot reaches its RUNNING state, and how long a CAM needs after a reset varies: on a DD CXD2099 with an AlphaCrypt, the 4th re-open within ~12 minutes returned `EINVAL` from the first handshake pump (no CA PMT sent; the kernel logged CAM init OK afterwards). A slot that never comes up is still reported once the window ends (#1032).

#### Changed
- Internal: adapted to `broadcast-common`'s removal of the panicking `Serialize::to_bytes` (tests, examples and docs now use `try_to_bytes`); requires the new `broadcast-common` major (umbrella #1143). `TransportConnection::init` uses an explicit justified `expect` for the fixed-size `Create_T_C` TPDU.

#### Changed (BREAKING)
- **BREAKING (r10-W-4, #1091):** follows `dvb-ci`'s `length_form` fields: the `dvb-ci` objects built or matched in this crate's APIs and tests carry the new field (`Default`/`Minimal` for host-built values).

#### Fixed
- A CAM that codes a `length_field` in a long form (`82 00 09`, #337) no longer misframes the link: `SessionLayer::on_spdu` slices the APDU after `SessionNumber::header_len()` (it assumed the 4-byte minimal header, handing the APDU layer two stray session bytes), and `Transport` decodes the `T_SB` status `length_field` and the `C_T_C_Reply` object length instead of assuming `80 02 ..` and a 3-byte object. `trace::decode_frame` labels are diagnostic only and still assume minimal forms.
- Linux backend: the `CA_RESET`/`CA_GET_SLOT_INFO` request numbers are built
  by `rustix::ioctl::opcode` instead of a hand-rolled asm-generic `_IOC`
  encoding. mips/powerpc/sparc use a different layout (3 direction bits, 13
  size bits, `_IOC_NONE = 1`), so the old constants issued the wrong request
  there (r10-W-1, #1092). The ioctls now go through `rustix::ioctl`
  (`NoArg`/`Updater`) rather than `libc::ioctl`. Pinned by compile-time
  asserts, verified with `cargo check --tests` on x86_64, musl and
  powerpc64le; the real ioctl path (ENOTTY fallback) runs in the Linux
  container test run. Not exercised against a real CI adapter.

#### Changed (BREAKING)

- **BREAKING (#1159):** `ManagedService::{es_pids,ca_pids,pcr_pid}` and `ManagedCa`/`Driver`/`CaDescrambler` `emm_pids`/`descramble_pids`/`ca_pids`/`required_pids` use `mpeg_ts::pid::Pid` instead of `u16`.

## Merged changes since `dvb-ci-runtime-v0.17.2`

- dvb-ci-runtime: split-fd CI data device, EINVAL fallback, post-reset write retry, error context (#1066/#1032) (#1232)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
