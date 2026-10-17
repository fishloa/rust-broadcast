# dvb-ci-runtime

[![crates.io](https://img.shields.io/crates/v/dvb-ci-runtime.svg)](https://crates.io/crates/dvb-ci-runtime)
[![docs.rs](https://img.shields.io/docsrs/dvb-ci-runtime)](https://docs.rs/dvb-ci-runtime)
[![MSRV](https://img.shields.io/badge/MSRV-1.95.0-blue.svg)](https://blog.rust-lang.org/)
[![license](https://img.shields.io/crates/l/dvb-ci-runtime.svg)](#msrv-and-license)

Pure-Rust **EN 50221 DVB Common Interface driver runtime**: device I/O, the TPDU
poll loop, SPDU session management and the per-resource state machines that drive
a physical CAM, over the [`dvb-ci`](https://crates.io/crates/dvb-ci) wire codecs.

## What it does / what it does not

**Does.** Drives a Common Interface module end to end: resets the slot, opens the
transport connection, runs the Resource Manager handshake, answers the module's
session opens for the six host-provided resources, surfaces `application_info`,
`ca_info`, `ca_pmt_reply`, MMI and host-control events as `Notification`s, builds
and sends `ca_pmt` for the services you add, and (with a `ci0` data device)
pushes a scrambled TS through the CAM and returns the descrambled TS. It is
sync and sans-IO at its core: the caller owns the clock and the loop.

**Does not.**

- No CI Plus: the runtime does not run CI+ authentication, the Content Control
  resource, multi-stream or any TS 103 205 resource (the `dvb-ci` crate types
  those APDUs; nothing here drives them).
- No Low-Speed Communications, no TS 101 699 extension resources at runtime
  (Resource Manager v2, Power Manager, Download and so on); only the six
  EN 50221 resources listed below are served.
- It does not re-tune on a `host_control` `tune`/`replace` request: it surfaces
  the request and the host acts on it.
- It does not descramble itself; the CAM does. For software CSA see
  [`dvb-csa`](https://crates.io/crates/dvb-csa).
- Linux only for real hardware (the DVB CA/CI devices); other platforms get the
  portable core and the in-memory mocks.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| CENELEC EN 50221 — Common Interface Specification for CA and other DVB Decoder Applications | February 1997 (vendored text) | Transport (Annex A TPDU poll loop), session layer, and the host-provided resources: Resource Manager, Application Information, Conditional Access Support, Date-Time, MMI, Host Control | Partial | Low-Speed Communications not served; Host Control surfaced, not acted on |
| ETSI TS 101 699 — Extensions to the Common Interface Specification | V1.1.1 (1999-11) | None at runtime (wire objects are typed in `dvb-ci`) | Not implemented | Cited by the crate docs for context only |
| Linux DVB CA API (`linux/dvb/ca.h`, kernel `dvb_ca_en50221`) | n/a (kernel ABI) | `CA_RESET`, `CA_GET_SLOT_INFO`, `[slot, connection_id, ...]` frame read/write, `ciM` TS data plane | Full for the calls used | `linux` feature, Linux only |

## Features

| Feature | Default | Effect |
|---|---|---|
| (none) | yes | Portable sync core: `Driver`, `CiStack`, `ManagedCa`, `CaDescrambler`, `MockCaDevice`, `MockCiDataDevice`, `RecordingCaDevice`, `trace`. Needs `std` (no `no_std` claim). |
| `linux` | no | `LinuxCaDevice` (`/dev/dvb/adapterN/caM`), `LinuxCiDataDevice` (`ciM`), `DeviceOpError`, and the `ci-probe` binary (pulls `libc`, `rustix`, `clap`). |

## Quick start

```rust
use std::time::Duration;
use dvb_ci_runtime::{Driver, HotPlug, MockCaDevice, Notification};
use dvb_ci_runtime::dvb_ci::tpdu::tags;

fn main() -> std::io::Result<()> {
    // Script a module that accepts the transport connection.
    let dev = MockCaDevice::new([vec![tags::C_T_C_REPLY, 0x01, 0x01]]);
    let mut driver = Driver::new(dev);

    driver.init()?; // reset + open the transport connection

    // Pump the device: reads frames when readable, otherwise advances the
    // EN 50221 poll cadence by the timeout.
    for _ in 0..4 {
        driver.pump(Duration::from_millis(100))?;
    }

    // Host-facing events surface here, poll-drained:
    for note in driver.take_notifications() {
        match note {
            Notification::CamReady => { /* now safe to send a ca_pmt */ }
            other => { let _ = other; }
        }
    }

    // ...or delivered by a closure per pump:
    driver.pump_hotplug(Duration::from_millis(100), |hp| match hp {
        HotPlug::CamPresent => { /* CAM (re)inserted: re-send any pending ca_pmt */ }
        HotPlug::CamRemoved => { /* CAM removed: stop descrambling */ }
        _ => {}
    })?;
    Ok(())
}
```

Runnable versions: `examples/mock_cam_session.rs`, `examples/sans_io_core.rs`.

## What is implemented

- **Transport** (EN 50221 Annex A TPDU): `Create_T_C` handshake, poll cadence
  (one `T_Data_Last` per module `T_SB`, half-duplex), `T_SB` data-available to
  `T_RCV`, `T_Data_More/Last` reassembly, reply timeout.
- **Session** (SPDU): session table, `open_session`/`create_session`/`close`,
  `session_number` APDU routing.
- **Resources**, all host-provided (the module opens the session; the host
  advertises them in its Resource Manager `profile`): Resource Manager
  (handshake to `CamReady`), Application Information, Conditional Access
  (`ca_pmt` / `ca_pmt_reply`), Date-Time (MJD + BCD), MMI (menus and enquiries
  surfaced as `Notification::Mmi`; `Driver::mmi_menu_answer` / `mmi_enquiry_answer`
  / `mmi_cancel` answer them), Host Control (`tune` / `replace` /
  `clear_replace` / `ask_release` surfaced as `Notification::HostControl`).
- **Descramble helper**: `Driver::descramble(pmt)` / `HostRequest::Descramble`
  runs the `ca_pmt` query, reply and `ok_descrambling` sequence, filtered to the
  CAM's advertised CAIDs (from `ca_info`).
- **Managed CAS layer**: a single-slot orchestration layer fed parsed `dvb-si`
  structs (never raw bytes). `Driver::add_service(&PmtSection)` /
  `remove_service(pn)` build and send the `ca_pmt` and track the active service
  set; `set_cat(&CatSection)` computes the EMM-PID feed (CAT EMM PIDs intersected
  with the CAM's `ca_info` CAIDs; the CAT keeps one `(CAID, EMM PID)` entry per
  `CA_descriptor`, so a CAID may have several); `emm_pids()` / `descramble_pids()` /
  `ca_pids()` / `required_pids()` give the PIDs to route into `ci0`.
  `required_pids()` always includes the CAT PID (0x0001): hardware-evidenced on a
  CXD2099 with an AlphaCrypt One4All (CAID 0x0650), not mandated by EN 50221.
  `set_requery_interval(dt)` is **opt-in** (default `REQUERY_DEFAULT` is
  `Duration::ZERO`) because no CAM verified so far answers the `query` half; a
  per-programme change surfaces as an edge-triggered `Notification::Entitlement`.
  `CaDescrambler<D, C>` is the turnkey wrapper that also owns the `ci0`
  `CiDataDevice`: `feed_ts(scrambled)` filters to `required_pids()` and returns
  the descrambled TS. One `CaDescrambler` is one CI slot and one TS path.
- **Hot-plug**: `Notification::HotPlug(HotPlug)` with `CamPresent` / `CamRemoved`
  (real DVB-CA slot-status edges; the driver re-drives the handshake on insert
  and tears down on remove) and `CardInserted` / `CardRemoved` / `CardChanged`
  (best-effort EN 50221 app-layer inference; CI slots have no card-detect line).
  React with `Driver::pump_hotplug(dt, |hp| ...)` or `pump_with`.
- **Devices**: in-memory `MockCaDevice` + `MockCiDataDevice`; Linux
  `LinuxCaDevice` (control plane) and `LinuxCiDataDevice` (TS data plane, whole
  packets only, `WouldBlock` + `PartialWrite` on backpressure) behind `linux`.
  `LinuxCiDataDevice::open` tries `O_RDWR` and falls back to a split
  `O_WRONLY` / `O_RDONLY | O_NONBLOCK` open on `EINVAL` (`open_split` /
  `from_split` are public).
- **Named device errors**: every failing Linux device operation returns a
  `DeviceOpError` naming the operation, device path and errno
  (`DeviceOpError::errno_of(&err)`); the `io::ErrorKind` is unchanged. A first
  `Create_T_C` write that fails `EINVAL` while the kernel slot is still coming up
  is retried every 250 ms for up to 12 s past the settle window.
- **Diagnostics**: `RecordingCaDevice` captures the link both ways;
  `trace::decode_frame` / `decode_log` annotate a capture (TPDU, SPDU, APDU).

## Hardware verification

Only what the CHANGELOG records:

- An AlphaCrypt / Irdeto module (live): Resource Manager handshake,
  `application_information` ("AlphaCrypt") and `ca_info` with 18 CA_system_ids
  were verified against the real module (#337, #340). That module rejects a host
  `create_session` for the host-provided resources and sends no
  `resource_identifier`s in its first `profile`; the runtime handles both. It
  also drops a second link block sent before it answers the first.
- A Digital Devices CXD2099 `ci0` (dddvb, Ubuntu 5.15): the split-fd data plane
  was run for 150 s, 111k `feed_ts` calls, max 249 us, 0 errors (#1066); the
  device rejects `O_RDWR` with `EINVAL` and `CA_GET_SLOT_INFO` may return `EINVAL`
  (the slot is then assumed ready).
- Not verified on hardware: the periodic `ca_pmt` re-query (no CAM tested answers
  the `query` half), CardInserted/Removed/Changed inference, and every path not
  listed above.

## `ci-probe` — discover and engage an installed CAM

A command-line tool that drives a real Common Interface module end-to-end over a
Linux DVB CA device. It is the reference consumer of this crate: it wires
`Driver` + `LinuxCaDevice` + `RecordingCaDevice` + `trace` together behind a
`clap` CLI (the workspace CLI standard — `docs/CLI-STANDARD.md`).

### Build / install

Linux only; gated behind the `linux` feature (it opens `/dev/dvb/...` and uses
`libc` ioctls):

```bash
cargo build  -p dvb-ci-runtime --features linux --release   # target/release/ci-probe
cargo install dvb-ci-runtime   --features linux             # ~/.cargo/bin/ci-probe
```

Device access usually needs membership of the `video` group (or root).

### Commands

All device-addressed commands take `-a/--adapter <N>` and `-c/--ca <N>` (both
default `0`). `--help` and `--version` are auto-generated; `<command> --help`
lists that command's flags.

| Command | What it does |
|---|---|
| `ci-probe list` | Enumerate `/dev/dvb/adapterN/caM` and print each slot's number + `module_ready`. |
| `ci-probe info [--adapter N] [--ca N]` | Run the EN 50221 handshake and print `application_info` + the CAM's `CA_system_id`s. |
| `ci-probe descramble [--adapter N] [--ca N] --pmt <FILE>` | Read a PMT section (raw bytes), then run the `ca_pmt` query → reply → `ok_descrambling` sequence and report the result. |
| `ci-probe mmi [--adapter N] [--ca N]` | Interactive MMI: display the module's menus / enquiries and send your answers back (`menu_answ` / `answ`). |

Append `--trace` to **any** command to dump an annotated link trace on exit
(every TPDU/SPDU/APDU both directions) — invaluable for diagnosing a CAM that
won't complete a handshake.

### Examples

Discover what's installed:

```console
$ ci-probe list
/dev/dvb/adapter0/ca0  slot 0  module_ready=true
```

Identify the module and its CA systems:

```console
$ ci-probe info --adapter 0 --ca 0
CAM ready (resource-manager handshake complete)
application_info: type=0x01 manufacturer=0x1234 code=0x5678 menu="Irdeto CAM"
ca_info: 18 CA_system_id(s): 0x0604, 0x0606, 0x0608, ...
```

Diagnose a stuck handshake (annotated trace):

```console
$ ci-probe info --adapter 0 --ca 0 --trace
...
--- link trace ---
  reset()
  slot_info() -> ready=true
W Create_T_C tcid=1
R C_T_C_Reply tcid=1
R T_Data_Last tcid=1 · open_session_request
W T_Data_Last tcid=1 · open_session_response
W T_Data_Last tcid=1 · session 1 · profile_enq (9F8010)
R T_Data_Last tcid=1 · session 1 · profile (9F8011)
...
```

Request descrambling for a service (PMT extracted with e.g. `dvbsnoop` or the
`dvb-si` tools):

```console
$ ci-probe descramble --adapter 0 --ca 0 --pmt service.pmt
CAM ready (resource-manager handshake complete)
ca_info: 18 CA_system_id(s): ...
ca_info received → sending descramble request
ca_pmt_reply: program 1019 descrambling_ok=true
```

### Hardware notes

- The device `read`/`write` exchange one whole kernel link frame
  `[slot, connection_id, <TPDU + T_SB>]`; the `LinuxCaDevice` handles that framing.
- EN 50221's link is **polled half-duplex** — one `T_Data_Last` per module `T_SB`.
  The transport enforces this (a real CAM drops a second block sent before it
  answers the first — see #337).
- If `info` times out before `ca_info`, re-run with `--trace` and read the last
  few lines — they show exactly which step the module stopped responding at.

The EN 50221 link is polled half-duplex — the host sends one `T_Data_Last` per
module `T_SB`. The transport queues outbound SPDUs and releases one per turn (a
real CAM drops a second block sent before it answers the first — #337).

## Design notes

- Everything is written against the `CaDevice` trait, so the runtime runs against
  a real Linux CA device or the in-memory `MockCaDevice`, which makes the state
  machines testable without hardware.
- `#![deny(unsafe_code)]`: the Linux device leaf is the sole `#[allow]`; the
  sans-IO core is unsafe-free. Linux ioctl request numbers come from
  `rustix::ioctl::opcode`; readiness polling is `rustix::event::poll`.
- Long-form `length_field` safe: follows `dvb-ci`'s preserved `LengthForm`, so a
  CAM coding `82 00 09` does not misframe the session or transport layer. PID
  accessors are typed `mpeg_ts::pid::Pid`.
- `Debug` redacts MMI enquiry answers (whatever the user typed).
- `thiserror` error types; dispatch enums are `#[non_exhaustive]`.

## Related crates

Depends on [`dvb-ci`](https://crates.io/crates/dvb-ci) (wire codecs, re-exported
as `dvb_ci_runtime::dvb_ci`), `dvb-si` (PMT / CAT inputs), `mpeg-ts` and
`broadcast-common`.

## MSRV and license

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
