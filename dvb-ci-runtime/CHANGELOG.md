# Changelog

All notable changes to `dvb-ci-runtime` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
  `ca_pmt_reply`).
- **`Driver<D: CaDevice>`**: pumps the device against the stack
  (`init`/`send_ca_pmt`/`pump`/`take_notifications`).
- Spec mds: `docs/en50221-{transport,session,resources}.md` (clean-room).
- `#![forbid(unsafe_code)]`; 23 tests, no hardware required.

### Not yet (roadmap)

- `date_time` / `host_control` / `mmi` resource handlers.
- A Linux `/dev/dvb/adapterN/caM` `CaDevice` implementation.
- A differential test harness against an external C reference.
