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

`#![deny(unsafe_code)]` — the Linux device leaf is the sole `#[allow]`; the
sans-IO core is unsafe-free. 27 tests, no hardware required.

Roadmap: the `host_control` resource, MMI answering (`menu_answ`/`answ`), and a
differential test harness against an external reference.

## Example

```rust
use std::time::Duration;
use dvb_ci_runtime::{Driver, MockCaDevice, Notification};
use dvb_ci_runtime::dvb_ci::tpdu::tags;

// Script a module that accepts the transport connection.
let dev = MockCaDevice::new([vec![tags::C_T_C_REPLY, 0x01, 0x01]]);
let mut driver = Driver::new(dev);

driver.init()?;                       // reset + open the transport connection
for _ in 0..4 {
    driver.pump(Duration::from_millis(100))?;   // read frames / advance poll cadence
}
for note in driver.take_notifications() {
    if let Notification::CamReady = note { /* now safe to send a ca_pmt */ }
}
# Ok::<(), std::io::Error>(())
```

See `examples/` for a runnable version.

## License

MIT OR Apache-2.0.
