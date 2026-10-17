# dvb-ci-runtime 0.18.0

_Released 2026-10-17._

**Breaking release (its own break, plus the `dvb-ci` 0.10 / `dvb-si` 12 / `mpeg-ts` 0.6 / `broadcast-common` 10 epoch).** Every PID in the managed-CAS API is now `mpeg_ts::pid::Pid` instead of `u16`, and the `dvb-ci` objects built or matched in this crate's APIs carry `dvb-ci`'s new `length_form` field. The release is mostly hardware-driven driver hardening, verified on a Digital Devices CXD2099 `ci0` with an AlphaCrypt One4All V2.5 (CAID 0x0650): `ManagedCa::required_pids()` now always includes the CAT PID (without it the EMM PIDs never descrambled, with it they did from about 55 s), every EMM PID of a CAM-advertised CAID is routed, `CaDescrambler::feed_ts` no longer loses descrambled output on a later error (recover it with `take_undelivered()`), the data plane falls back to two one-direction descriptors when the kernel rejects `O_RDWR`, the post-reset `EINVAL` window is retried for up to 12 s, a CAM that codes `82 00 09` long-form lengths no longer misframes the link, and every device error names the operation, device path and errno.

## Why this version

- **Proposed version: 0.17.2 -> 0.18.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major. `Pid` replaces `u16` on public fields and return types; `dvb-ci`'s `length_form` field reaches host-built objects; device errors are wrapped in `DeviceOpError`.
- **cargo-semver-checks vs live 0.17.2 (all features):** none flagged. It does not see changed field and return types, so the CHANGELOG BREAKING list is the primary evidence.
- **Epoch cascade:** also from `broadcast-common` (9 -> 10), `dvb-ci` (0.9 -> 0.10), `dvb-si` (11 -> 12) and `mpeg-ts` (0.5 -> 0.6).
- **Caret epoch:** 0.17 -> 0.18; every workspace dependent must move with it.

## Dependency changes

```toml
-dvb-ci           = { path = "../dvb-ci",           version = "0.9" }
+dvb-ci           = { path = "../dvb-ci",           version = "0.10" }
-dvb-si           = { path = "../dvb-si",           version = "11.1", default-features = false }
+dvb-si           = { path = "../dvb-si",           version = "12.0", default-features = false }
-broadcast-common = { path = "../broadcast-common", version = "9.4",  default-features = false }
+broadcast-common = { path = "../broadcast-common", version = "10.0", default-features = false }
+mpeg-ts          = { path = "../mpeg-ts",          version = "0.6",  default-features = false, features = ["std"] }   # new dependency (typed Pid)
```

`mpeg-ts` is new in this release.

## Breaking changes and migration

### 1. Typed PIDs (#1159)

`ManagedService::{es_pids, ca_pids, pcr_pid}` and the `emm_pids` / `descramble_pids` / `ca_pids` / `required_pids` accessors on `ManagedCa`, `Driver` and `CaDescrambler` use `mpeg_ts::pid::Pid` instead of `u16`:

```rust
use mpeg_ts::pid::Pid;

// before
// let es_pids: Vec<u16> = service.es_pids.clone();
// let required: Vec<u16> = driver.required_pids();
// after
let es_pids: Vec<Pid> = service.es_pids.clone();
let required: Vec<Pid> = driver.required_pids();
// to get the integer back (e.g. for a DVB demux PID filter):
let raw: Vec<u16> = required.iter().map(|p| p.value()).collect();
```

### 2. `dvb-ci` objects carry `length_form` (#1091)

Follows `dvb-ci` 0.10: the `dvb-ci` objects built or matched in this crate's APIs and tests carry a `length_form` field (`Default` / `Minimal` for host-built values). Struct patterns that listed every field need `..`; struct literals need `length_form: Default::default()`. See the `dvb-ci` 0.10.0 note for the full list of objects.

### 3. Device errors are wrapped in `DeviceOpError`

Every failing Linux device operation (`open`, `read`, `write`, `poll`, `ioctl CA_RESET`, `ioctl CA_GET_SLOT_INFO`) now returns an `io::Error` that wraps a `DeviceOpError` naming the operation, the device path and the errno, for example `/dev/dvb/adapter3/ca0: write (17 byte frame: slot 0, connection 1) failed: Invalid argument (os error 22)`. The `io::ErrorKind` is unchanged, but `err.raw_os_error()` no longer answers directly:

```rust
// before
// if err.raw_os_error() == Some(libc::EINVAL) { .. }
// after
if dvb_ci_runtime::DeviceOpError::errno_of(&err) == Some(libc::EINVAL) { /* ... */ }
```

### 4. Data-plane write contract (implementers of `CiDataDevice`)

`CiDataDevice::write` now documents a contract of whole TS packets only, with `WouldBlock` carrying `dataplane::PartialWrite { written, requested, device }` (read it with `partial_write_progress`) on backpressure. `LinuxCiDataDevice::write` implements it: it writes in 21-packet chunks, completes a short or mid-packet write (waiting up to 2 s for room), and waits on `poll(POLLOUT)` for 50 ms at a packet boundary before returning `WouldBlock`. A custom `CiDataDevice` should follow the same contract; `CaDescrambler::feed_ts` resumes after the accepted bytes and gives up only after 20 consecutive attempts that moved nothing. Likewise `LinuxCaDevice::read` now returns an `InvalidData` error when a TPDU is larger than the caller's buffer (`Driver`'s buffer is 4096 bytes) instead of returning a truncated one as `Ok`.

## Known limitations (recorded in the CHANGELOG and README)

- The periodic entitlement re-query (`Driver::set_requery_interval`) stays opt-in (`REQUERY_DEFAULT` is `Duration::ZERO`). It is hardware-verified on one CAM: an entitled AlphaCrypt One4All V2.5 (CAID 0x0650) in a Digital Devices CXD2099 `ci0`, 180 s runs, 2026-10-16. The 10 s timer sends `update/query` immediately followed by `update/ok_descrambling` (the lone `query` of earlier versions stopped descrambling, #1032); the module answered most queries with `ca_enable = 0x81`, descrambling never gapped, and `feed_ts` stayed under 0.5 ms. Other CAMs are unverified.
- The CAT-PID inclusion in `required_pids()` is hardware-evidenced on the CAM above, not mandated by EN 50221.
- The `CA_RESET` / `CA_GET_SLOT_INFO` request numbers are now built by `rustix::ioctl::opcode`, pinned by compile-time asserts and checked with `cargo check --tests` on x86_64, musl and powerpc64le; the real ioctl path (ENOTTY fallback) runs in the Linux container test run and was not exercised against a real CI adapter.
- `trace::decode_frame` labels are diagnostic only and still assume minimal length forms.

## What changed (from CHANGELOG `[Unreleased]`)

### Fixed
- `ManagedCa::required_pids()` now always includes the CAT PID (0x0001). Hardware-evidenced, not mandated by EN 50221: on a Digital Devices CXD2099 with an entitled AlphaCrypt One4All V2.5 (CAID 0x0650), ORF1, 180 s runs, CAT + EMM PID 0x259 descrambled from about 55 s while all seven EMM PIDs without the CAT never did (#1296).
- `ManagedCa::set_cat` no longer collapses several `CA_descriptor`s with the same CA_system_id to one PID (it keyed a map by CAID); every EMM PID of a CAM-advertised CAID is now routed (#1296).
- `CaDescrambler::feed_ts` no longer discards descrambled TS it had already drained when a later write or drain fails (hard error, or the stalled-write give-up). The bytes are retained and returned first by the next `feed_ts`, or recovered with the new `CaDescrambler::take_undelivered()`. Signature unchanged (not breaking).
- Docs: `required_pids` now names the CAT PID; `emm_pids` describes the `(CAID, PID)` entry list instead of a CAID map.
- `LinuxCiDataDevice::write` never leaves a half-written TS packet in the device
  (adversarial review `review/ci-cam-crypto`). The duplex fd is `O_NONBLOCK`, so
  a burst larger than the device buffer used to stop mid-packet with a bare
  `WouldBlock` and no byte count. Writes now go in 21-packet chunks, a short or
  mid-packet write is completed (waiting up to 2 s for room), and a device that
  is full at a packet boundary is waited on with `poll(POLLOUT)` for 50 ms
  before the call returns `WouldBlock` carrying a new `PartialWrite { written,
  requested, device }` (read with `partial_write_progress`), the number of
  whole-packet bytes accepted. `CaDescrambler::feed_ts` resumes after them,
  draining the device between attempts, and gives up only after 20 consecutive
  attempts that moved nothing.
- `LinuxCaDevice::read` returns an `InvalidData` error when a TPDU is larger than
  the caller's buffer instead of copying a truncated one and returning `Ok`
  (`Driver`'s buffer is 4096 bytes; a CAM that negotiates a larger link buffer is
  now reported, not silently corrupted).
- `Driver::pump`: the first write after the post-`CA_RESET` settle (the `Create_T_C`) that fails `EINVAL` is retried every 250 ms for up to 12 s past the settle window instead of aborting the pump. The kernel's `dvb_ca_en50221` answers `EINVAL` to a write until the slot reaches its RUNNING state, and how long a CAM needs after a reset varies: on a DD CXD2099 with an AlphaCrypt, the 4th re-open within ~12 minutes returned `EINVAL` from the first handshake pump (no CA PMT sent; the kernel logged CAM init OK afterwards). A slot that never comes up is still reported once the window ends (#1032).
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

### Documentation
- README and crate-root docs rewritten to the cluster standard: what it does / does not (no CI Plus, no LSC), standards table, feature table, hardware-verification facts as recorded in this CHANGELOG, stale test-count and roadmap lines removed.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `dvb-ci` 0.9 -> 0.10, `dvb-si` 11 -> 12, `mpeg-ts` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Added
- `CiDataDevice::write` contract: whole packets only, `WouldBlock` + `PartialWrite`
  on backpressure; `dataplane::{PartialWrite, partial_write_progress}` (re-exported).
- `LinuxCiDataDevice::open_split(adapter, ci)` / `LinuxCiDataDevice::from_split(write, read)`: the data plane as two one-direction descriptors (`O_WRONLY` write, `O_RDONLY | O_NONBLOCK` read). The Digital Devices CXD2099 `ci0` (dddvb, Ubuntu 5.15) rejects `O_RDWR` with `EINVAL`, so `LinuxCiDataDevice::open` now tries `O_RDWR` first and falls back to the split open on `EINVAL` and only `EINVAL`. Verified on that hardware with two `from_file`-wrapped fds: 150 s, 111k `feed_ts` calls, max 249 us, 0 errors (#1066).
- `DeviceOpError` (re-exported): every failing Linux device operation (`open`, `read`, `write`, `poll`, `ioctl CA_RESET`, `ioctl CA_GET_SLOT_INFO`) now names the operation, the device path and the errno, e.g. `/dev/dvb/adapter3/ca0: write (17 byte frame: slot 0, connection 1) failed: Invalid argument (os error 22)`. The `io::ErrorKind` is unchanged; the errno is `DeviceOpError::errno_of(&err)` (a wrapped error no longer answers `raw_os_error()` itself) (#1032).

### Changed
- `TransportConnection::init` uses an explicit justified `expect` for the fixed-size `Create_T_C` TPDU.

### Changed (BREAKING)
- **BREAKING (r10-W-4, #1091):** follows `dvb-ci`'s `length_form` fields: the `dvb-ci` objects built or matched in this crate's APIs and tests carry the new field (`Default`/`Minimal` for host-built values).

### Changed (BREAKING)

- **BREAKING (#1159):** `ManagedService::{es_pids,ca_pids,pcr_pid}` and `ManagedCa`/`Driver`/`CaDescrambler` `emm_pids`/`descramble_pids`/`ca_pids`/`required_pids` use `mpeg_ts::pid::Pid` instead of `u16`.
