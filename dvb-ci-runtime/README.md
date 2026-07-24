# dvb-ci-runtime

Pure-Rust **EN 50221 DVB Common Interface runtime** — the driver loop over the
[`dvb-ci`](https://crates.io/crates/dvb-ci) wire codecs.

`dvb-ci` is `no_std` and owns the **wire** layer (TPDU / SPDU / APDU
parse+serialize, CA_PMT building, CI Plus extensions). `dvb-ci-runtime` adds the
**runtime**: device I/O, the TPDU poll loop, SPDU session management, and the
per-resource state machines that drive a physical CAM (ETSI EN 50221, TS 101 699).

## Design

Everything is written against the `CaDevice` trait, so the runtime runs against
either a real Linux CA device (`/dev/dvb/adapterN/caM`, the `linux` feature) or an
in-memory `MockCaDevice`. The mock makes the state machines testable without
hardware and enables differential testing against an external reference — drive
both with the same scripted mock CAM, assert the emitted `write`/ioctl byte
sequences match.

Implemented from the EN 50221 specification.

## What's implemented

- **Transport** (TPDU, §A.4): `Create_T_C` handshake, poll cadence, `T_SB`
  data-available → `T_RCV`, `T_Data_More/Last` reassembly, reply timeout.
- **Session** (SPDU, §7.2): session table; `open_session`/`create_session`/
  `close`; `session_number` + APDU routing.
- **Resources** (§8): Resource Manager handshake → `CamReady`,
  application_information, conditional_access (`ca_pmt`/`ca_pmt_reply`),
  date_time (MJD + BCD), mmi (surfaces module menus/enquiries).
- **Descramble helper**: `Driver::descramble(pmt)` / `HostRequest::Descramble`
  runs the full `ca_pmt` query → reply → `ok_descrambling` sequence, filtered to
  the CAM's advertised CAIDs (from `ca_info`).
- **Devices**: in-memory `MockCaDevice` + `MockCiDataDevice`; Linux
  `/dev/dvb/adapterN/caM` (control plane) and `ciM` (TS data plane, `CiDataDevice`
  — scrambled-in / descrambled-out for separate-CI hardware) behind the `linux`
  feature (`libc`).
- **Diagnostics**: `RecordingCaDevice` captures the link both ways;
  `trace::decode_frame` / `decode_log` annotate a capture (TPDU → SPDU → APDU)
  for live-CAM debugging.
- **MMI answering**: `Driver::mmi_menu_answer` / `mmi_enquiry_answer` /
  `mmi_cancel` send `menu_answ` / `answ` back to the module.
- **Hot-plug** (#726): `Notification::HotPlug(HotPlug)` groups the five
  transitions — `CamPresent`/`CamRemoved` are real DVB-CA slot-status edges
  (the driver re-drives the handshake on insert, tears down on remove);
  `CardInserted`/`CardRemoved`/`CardChanged` are best-effort EN 50221
  app-layer inference (CI slots have no card-detect line). React with a
  closure via `Driver::pump_hotplug(dt, |hp| …)` (or `pump_with` for every
  notification), or classify a drained note with `Notification::hotplug()`.
- **Managed CAS layer** (#763): a single-slot orchestration layer fed parsed
  `dvb-si` structs (never raw bytes). `Driver::add_service(&PmtSection)` /
  `remove_service(pn)` build + send the `ca_pmt` and track the slot's active
  service set; `set_cat(&CatSection)` computes the EMM-PID feed (CAT EMM PIDs
  ∩ the CAM's `ca_info` CAIDs); `emm_pids()`/`descramble_pids()`/`ca_pids()`/
  `required_pids()` (EMM ∪ ES ∪ ECM ∪ PCR) give the PIDs to route into `ci0`.
  `set_requery_interval(dt)` periodically re-queries (`cmd_id = query`) so a
  card entitled *after* the initial `ca_pmt` refreshes — a per-programme
  status change surfaces as an edge-triggered `Notification::Entitlement`.
  `CaDescrambler<D, C>` is the turnkey wrapper that also owns the `ci0`
  `CiDataDevice`: `feed_ts(scrambled)` filters to `required_pids()` and returns
  the descrambled TS. One `CaDescrambler` = one CI slot = one TS path
  (multi-tuner ⇒ one per slot).

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

`#![deny(unsafe_code)]` — the Linux device leaf is the sole `#[allow]`; the
sans-IO core is unsafe-free. 27 tests, no hardware required.

Roadmap: the `host_control` resource, MMI answering (`menu_answ`/`answ`), and a
differential test harness against an external reference.

## Example

```rust
use std::time::Duration;
use dvb_ci_runtime::{Driver, HotPlug, MockCaDevice, Notification};
use dvb_ci_runtime::dvb_ci::tpdu::tags;

// Script a module that accepts the transport connection.
let dev = MockCaDevice::new([vec![tags::C_T_C_REPLY, 0x01, 0x01]]);
let mut driver = Driver::new(dev);

driver.init()?;                       // reset + open the transport connection

// Poll-drain style: pump, then match drained notifications.
for _ in 0..4 {
    driver.pump(Duration::from_millis(100))?;   // read frames / advance poll cadence
}
for note in driver.take_notifications() {
    if let Notification::CamReady = note { /* now safe to send a ca_pmt */ }
}

// Or closure style — react to CAM/card hot-plug edges directly:
driver.pump_hotplug(Duration::from_millis(100), |hp| match hp {
    HotPlug::CamPresent => { /* CAM (re)inserted — (re)send any pending ca_pmt */ }
    HotPlug::CamRemoved => { /* CAM removed — stop descrambling */ }
    _ => {}                 // Card{Inserted,Removed,Changed} — best-effort inference
})?;
# Ok::<(), std::io::Error>(())
```

See `examples/` for a runnable version.

## License

MIT OR Apache-2.0.
