# Changelog

All notable changes to `dvb-ci-runtime` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- **CAM + card hot-plug `Notification`s** (#726): `Notification::CamPresent` /
  `CamRemoved` — real DVB-CA slot-status edges (`CA_CI_MODULE_PRESENT`), emitted
  once per edge; the driver re-drives the reset/init handshake on insert and
  tears down session state on removal. `Notification::CardInserted` /
  `CardRemoved` / `CardChanged` — best-effort app-layer inference from `ca_info`
  CAID-set changes, `ca_pmt_reply` `descrambling_ok` transitions, and MMI
  "no card"/entitlement keyword text (EN 50221 CI slots have no card-detect
  line). `SlotInfo` gained a `module_present` field alongside the existing
  `module_ready`.
### Fixed
- `LinuxCaDevice::slot_info` read `CA_CI_MODULE_READY` from the wrong bit
  (`1`, the uapi `CA_CI_MODULE_PRESENT` value) instead of `2` — `module_ready`
  was actually reporting module presence, not readiness.

## [0.12.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.11.0] — 2026-07-02
### Added
- **Host Control resource** (EN 50221 §8.5.1, `HOST_CONTROL` 0x0020_0041): a `Resource`
  impl decoding incoming `tune` / `replace` / `clear_replace` / `ask_release` APDUs and
  surfacing them as `Notification::HostControl(HostControlEvent{…})` for the host to act
  on out-of-band; advertised in the profile reply (#328).
- Driver-level byte-exact gate tests for MMI answering (`menu_answ` / `answ` on the MMI
  session — send path already existed; this pins its wire bytes).

## [0.10.1] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [0.10.0]

### Added
- **Multi-programme descrambling** (for a capacity manager driving several
  services at once):
  - `Driver::descramble_programs(&[&[u8]])` / `HostRequest::DescramblePrograms`
    — send a CA-PMT list (`list_management` `first`/`more`/`last`, or `only` for
    one), replacing the selected set; each `ca_pmt` is `ok_descrambling`.
  - `Driver::add_program(&[u8])` / `HostRequest::AddProgram` — add one programme
    (`list_management = add`) without re-listing the rest.
  - `Driver::remove_program(&[u8])` / `HostRequest::RemoveProgram` — drop one
    (`list_management = update`, `cmd_id = not_selected`).
  - Per-programme `ca_pmt`s are serialised one-per-module-turn by the transport
    queue. CAID-filtered to the CAM's `ca_info` like single `descramble`.

## [0.9.0]

### Added
- **UI-friendly MMI menu API.** `MmiEvent::Menu`/`List` now carry a typed
  `MmiMenu` { `title`, `subtitle`, `bottom`, `choices` } (the three header lines
  + selectable choices kept separate for direct rendering); `Menu` (selectable)
  and `List` (informational) are distinct variants. Answer via the existing
  `Driver::mmi_menu_answer` / `mmi_enquiry_answer` / `mmi_cancel`.
- **`Driver::enter_menu` / `HostRequest::EnterMenu`** — ask the module to open its
  MMI menu (`enter_menu` on the application_information session).
- Typed **`DisplayReply`** for the high-level MMI `display_control` handshake
  (no hand-rolled magic-byte APDU).

### Changed
- **`descramble` sends `ca_pmt` `cmd_id = ok_descrambling` directly** (no `query`
  first): a real AlphaCrypt/Irdeto module stays silent on a query, stalling the
  prior query→reply→ok flow. The reply still surfaces as
  `Notification::CaPmtReply`. Matches oscam / libdvben50221.

### Fixed (#340 — `ca_info` finally lands on live hardware)
- **app_info / conditional_access / mmi are HOST-provided, not module-provided.**
  Every prior round had the session direction wrong for these resources: the host
  tried to open them itself (0.6.0 `create_session`, then `open_session_request`),
  but a real AlphaCrypt/Irdeto module **rejects `create_session` (status 0xF0)**
  and **ignores a host `open_session_request`** for them. They are host-provided:
  the host advertises all five resources it implements (resource_manager,
  application_information, conditional_access, date_time, mmi) in its RM `profile`
  reply, and the **module** opens a session to each (module → host
  `open_session_request`) — exactly as it already did for resource_manager and
  date_time. The host just accepts; each session's `on_open` drives its enquiry.
  - `CiStack::host_provided` now lists all five.
  - The RM no longer `create_session`s anything after `profile_change`.
  - `SessionLayer::on_spdu` binds a host-opened session on `open_session_response`.
- **`trace::decode_frame`** now annotates session SPDUs with the resource_id
  (+ status/session_nb), so a capture shows *which* resource each open targets.

Verified live: resource_manager → application_information ("AlphaCrypt") →
conditional_access → `ca_info` with 18 CA_system_ids (incl 0x0648/0x0650 ORF).

## [0.7.0]

### Changed
- **`ci-probe` now uses a proper CLI** (the workspace standard — `clap` derive;
  see `docs/CLI-STANDARD.md`). Device addressing is via named flags instead of
  bare positionals, and `--help`/`--version` are auto-generated:
  `ci-probe info --adapter 3 --ca 0`, `ci-probe descramble --adapter 3 --ca 0
  --pmt service.bin`, `--trace` on any subcommand. (`linux` feature now also pulls
  `clap`.)

### Fixed (#340 — fourth live-CAM run)
- **CA session still never opened: the module's `profile` is empty.** A real
  AlphaCrypt returns `profile` with **no** `resource_identifier`s (`9F 80 11 00`),
  so opening only the resources it *enumerates* (0.6.0) opened nothing. The
  Resource Manager now `create_session`s the standard module-provided resources
  (`application_information`, `conditional_access`, `mmi`) **unconditionally**
  after `profile_change`; the module accepts those it provides and refuses the
  rest (ignored). Matches libdvben50221.

## [0.6.0]

### Fixed (#340 — third live-CAM run)
- **CA sessions never opened → no descrambling.** 0.5.0 added the
  `profile_change` gate (correct) but also wrongly stopped the host opening the
  module-provided resource sessions. Hardware confirmed the **direction rule**:
  the *module* opens sessions to *host*-provided resources (`resource_manager`,
  `date_time`); the *host* opens sessions to *module*-provided resources
  (`application_information`, `conditional_access`, `mmi`) with `create_session`.
  The Resource Manager again opens those (alongside `profile_change`), and the
  session layer once more accepts module opens only for host-provided resources.
  The spec mds (`en50221-resources.md`, `en50221-session.md`) are corrected to
  this rule.

## [0.5.0]

### Fixed (#340 — second live-CAM run)
- **Post-`CamReady` stall: the module idled and no CA path opened.** Per
  EN 50221 §8.4.1.1 the module, after sending its `profile` reply, **waits for a
  `profile_change` object** before it may open or accept any session. The host
  never sent one, so the module sat idle. The Resource Manager now sends
  `profile_change` once it has the module's profile — the gate that lets the
  module open its `application_information` / `conditional_access` / `mmi`
  sessions.
- **Module session opens were rejected.** The module opens those sessions itself
  (§7.2.3 — `create_session` is host→module routing for a *second* module only,
  not how a host uses a module's resource). The session layer now accepts an
  `open_session_request` for **any resource the host has a handler for**, not just
  host-provided ones; the host no longer issues `create_session` for them.
- **`LinuxCaDevice` link framing.** The kernel `dvb_ca_en50221` device carries a
  `[slot, connection_id, …]` header on every read/write; the device now adds it on
  write and strips it on read (a raw TPDU write was rejected `EINVAL`). It also
  tolerates `CA_GET_SLOT_INFO` returning `EINVAL` (assume the slot is ready — e.g.
  DD/cxd2099) and settles ~2 s after `CA_RESET` before the handshake.

### Changed
- *(breaking, `linux` feature)* `LinuxCaDevice::from_file` now takes a `slot: u8`.

## [0.4.0]

### Fixed
- **RM handshake stalled one step past the #337 fix on a real CAM.** The stack
  required the module to enquire the host's profile (`host_profiled`) before
  declaring `CamReady`, but a real AlphaCrypt/Irdeto module sends its `profile`
  reply and then idles — it never enquires. `CamReady` now fires on the module's
  profile alone (the host still answers a module `profile_enq` if one arrives).
- **`trace::decode_frame` mis-decoded long-form `length_field`s.** It assumed a
  single length byte, so a `T_Data_Last` with a long-form length (e.g. the
  module's `profile` reply, `A0 82 00 09 …`) read the wrong `t_c_id` and a
  garbled SPDU. It now uses the Table-1 length codec.

### Added
- **`ci-probe` binary** (`linux` feature, Linux-only) — discover and engage an
  installed CAM from the command line: `list` (enumerate `/dev/dvb/adapterN/caM`
  + slot status), `info` (run the handshake, print application-info + CAIDs),
  `descramble <pmt-file>` (query → reply → ok), `mmi` (interactive menus /
  enquiries). `--trace` dumps an annotated link trace on exit.
- **Host MMI answering**: `HostRequest::MmiMenuAnswer(choice_ref)` /
  `MmiEnquiryAnswer(text)` / `MmiCancel`, and the matching
  `Driver::mmi_menu_answer` / `mmi_enquiry_answer` / `mmi_cancel` — send
  `menu_answ` / `answ` back to the module (completes the MMI dialogue, previously
  receive-only).

## [0.3.0]

### Fixed
- **Resource-manager exchange stalled against a real CAM** (#337). The stack
  emitted two `T_Data_Last` blocks back-to-back in one turn (e.g.
  `open_session_response` + `profile_enquiry`), but EN 50221's link is polled
  half-duplex — one data block per module turn. A real module (AlphaCrypt/Irdeto)
  consumed the first and dropped the second, so RM never completed. The transport
  now queues outbound SPDUs and sends one per module `T_SB`.

### Added
- **`RecordingCaDevice<D>`** — a `CaDevice` decorator that captures every frame
  in both directions (+ ioctls) as `LinkEvent`s, for live-CAM diagnostics.
- **`trace::decode_frame` / `trace::decode_log`** — decode raw link frames into
  one-line annotations (TPDU → SPDU → APDU tag names), so a capture reads like a
  bug-report trace without hand-decoding.

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
