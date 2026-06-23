# Changelog

All notable changes to `dvb-ci-runtime` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0]

### Added
- **`HostRequest::Descramble(pmt_section)`** + **`Driver::descramble(pmt)`** — a
  high-level descramble helper (#334). The stack remembers the CAM's CAIDs from
  `ca_info`, filters the PMT's `CA_descriptor`s to them, sends a `ca_pmt` with
  `cmd_id = query`, and — when the `ca_pmt_reply` reports descrambling is
  possible — automatically sends `cmd_id = ok_descrambling`. The outcome surfaces
  as `Notification::CaPmtReply`.
- **`CiDataDevice`** trait + **`MockCiDataDevice`** + Linux **`LinuxCiDataDevice`**
  (`linux` feature) — the CI **TS data-plane** device (`/dev/dvb/adapterN/ciM`)
  for separate-CI (host-fed) hardware: the host writes scrambled TS and reads the
  descrambled TS back, in whole 188-byte packets (#333). Parallels `CaDevice`
  (the control plane); the mock supports scripted-descramble differential tests.

### Changed
- New dependency on `dvb-si` (to parse a `PmtSection` for `descramble`) and
  `dvb-ci` ≥ 0.5 (the CAID-filtered `ca_pmt` builder).

## [0.1.1]

### Documentation

- Refresh the crate-root and README **status** to reflect the shipped surface
  (transport / session / resources incl. date_time + mmi / Linux device) — the
  0.1.0 text still described it as an incremental foundation.
- Add a crate-level doctest and two runnable examples (`mock_cam_session`,
  `sans_io_core`) showing the `Driver` loop and the pure sans-IO core.

## [0.1.0]

### Added

- New crate: a pure-Rust **EN 50221 DVB Common Interface runtime** over the
  `dvb-ci` no_std codecs — the driver loop the codec crate omits.
- **`CaDevice`** trait (the hardware-abstraction boundary) + an in-memory,
  op-recording **`MockCaDevice`**.
- **Sans-IO core**: `Event` → `Action` + `Notification`; every layer is a pure
  state machine (no device/threads/clock), so all logic — including the EN 50221
  timing (poll cadence, reply timeout) — is deterministic and testable without
  hardware.
- **Transport** (TPDU, §A.4): `Create_T_C` handshake, empty-`T_Data_Last` poll
  cadence, `T_SB` Data-Available → `T_RCV`, `T_Data_More/Last` reassembly, reply
  timeout.
- **Session** (SPDU, §7.2): session table; `open_session_request`/response,
  host-initiated `create_session`, `close_session`; `session_number` + APDU
  routing.
- **Resource layer** (§8): `Resource` trait + registry; **Resource Manager**
  handshake (profile exchange → `CamReady`, then opens the module-provided
  resources); **application_information** (→ `ApplicationInfo`); **conditional
  access** (`ca_info` → `CaInfo`; host `ca_pmt` via `send_ca_pmt`; decodes
  `ca_pmt_reply`); **date_time** (host-provided; answers `date_time_enquiry`,
  resends on the module's requested interval; DVB UTC = MJD + BCD encoding);
  **mmi** (decodes module `Menu`/`Enquiry`/`Close` → `Notification::Mmi`).
- **`Driver<D: CaDevice>`**: pumps the device against the stack
  (`init`/`send_ca_pmt`/`pump`/`take_notifications`).
- **Linux `CaDevice`** (`linux` feature, Linux-only): a `/dev/dvb/adapterN/caM`
  device via `libc` (read/write/poll + `CA_RESET`/`CA_GET_SLOT_INFO` ioctls,
  request numbers computed from the standard `_IOC` encoding). The one place the
  crate uses `unsafe`; the portable core stays unsafe-free.
- Spec mds: `docs/en50221-{transport,session,resources}.md` (clean-room).
- `#![deny(unsafe_code)]` (the Linux device leaf is the sole `#[allow]`);
  27 tests, no hardware required (the Linux device is compile-checked).

### Not yet (roadmap)

- `host_control` resource; MMI answering (`menu_answ`/`answ`).
- A differential test harness against an external C reference.
