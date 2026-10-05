# Changelog

All notable changes to `dvb-ci-runtime` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.17.0] - 2026-10-05
### Changed (breaking)
- `resource::Resource::on_open`, `on_apdu` and `tick` now return
  `dvb_ci::Result<ResourceOut>` (were `ResourceOut`), so an APDU that fails to
  serialize or a malformed CAM APDU is an `Err` instead of being dropped; every
  `Resource` implementation must be updated.
- `session::SessionLayer::create_session`, `send_apdu` and `close` now return
  `dvb_ci::Result<Vec<u8>>` (were `Vec<u8>`), and `SessionLayer::on_spdu`
  returns `dvb_ci::Result<SessionOut>` (was `SessionOut`) — a malformed SPDU
  is surfaced rather than dropped silently (#1092).

### Changed
- **`managed::REQUERY_DEFAULT` (the entitlement re-query cadence's default)
  is now `Duration::ZERO` (disabled), was 10s.** The periodic re-query is
  opt-in now: call `Driver::set_requery_interval` explicitly to enable it.
  No CAM this crate has been verified against actually answers the `query`
  half (`stack.rs`'s `descramble` already documents a live AlphaCrypt never
  replying to `query` at all), so it stayed off by default until that is
  confirmed on real hardware (issue #1032).
- (`linux` feature) device readiness polling uses `rustix::event::poll` instead of `libc::poll`; sub-millisecond timeouts are no longer truncated to zero. New dependency `rustix` (feature `linux` only). `libc` stays for the CA ioctls. No public API change.
- Dev: `tests/no_handroll_guard.rs`; Linux tests for the readiness poll (timeout, zero timeout, sub-millisecond timeout, hang-up without data).

### Fixed
- A CAM-originated APDU or SPDU that fails to parse (e.g. a padded `tune`, a
  truncated MMI `enq`, a malformed `open_session_response`) was dropped
  silently by the resource and session layers' `if let Ok(..) = parse(..)`
  guards; it now surfaces as `Notification::Error` (#1092).
- `HostRequest`'s `Debug` redacts the text of `MmiEnquiryAnswer` (what the user
  typed at an enquiry, often a PIN) instead of printing it (audit #1142).
- `Driver::pump` handed each received frame to the stack through a fresh
  `to_vec()`; it now borrows the receive buffer in place (a counting-allocator
  test pins the largest allocation at 416 bytes for a 4096-byte frame, was
  4096) (audit r10-O-11, #1092). `CaDescrambler::feed_ts`'s per-feed PID set
  is left as is: caching it needs invalidation on every `add_service`/
  `set_cat`/re-query, with no measured cost.
- `CiStack` looked up the session for a resource by probing all 65 535
  session numbers; `SessionLayer::session_for` walks only the open sessions
  (audit r10-O-9, #1092).
- The MMI card-keyword heuristic (`HotPlug::CardInserted`/`CardRemoved`) is now
  edge-triggered: a CAM that re-sends the same "insert card" menu no longer
  repeats the notification (audit r10-O-13, #1092).
- The `ser`/`ser_apdu` helper was copied into `resource`, `session` and
  `stack`; they share one (audit r10-O-12, #1092). `trace::decode_frame` now
  names Delete/D_T_C_Reply/Request/New T_C TPDUs (they printed as `T_?`) and
  its docs, and `HotPlug::CamPresent`'s ("and ready"; the driver keys the edge
  off `module_present` alone), match the code (audit r10-W-23, #1092).
  r10-O-10 (`LinuxCaDevice::reset`'s 3 s settle `sleep` in the pump) is not
  changed: making it a sans-IO timer alters the reset-to-Create_T_C timing on
  real hardware, which could not be verified here without a CAM.
- `CaDescrambler::feed_ts` no longer hangs on the real `ciM` data-plane
  device: `LinuxCiDataDevice::open` now opens `O_NONBLOCK`, so the drain loop
  sees `WouldBlock` (mapped to "no more data") instead of blocking forever on
  a second read (issue #1066).
- The periodic entitlement re-query (`Driver::set_requery_interval`) no
  longer sends `list_management = only/first` + `cmd_id = query` on a
  timer — per EN 50221 §8.4.3.4/§8.4.3.5 that replaces the active programme
  list and bars descrambling until an `ok_descrambling` that was never sent,
  silently stopping descrambling ~`interval` after it started. The resend is
  now a `list_management = update` pair: `cmd_id = query` (to still solicit
  a fresh `ca_pmt_reply` from a CAM that answers it) immediately followed,
  unconditionally, by `cmd_id = ok_descrambling` (so descrambling is never
  left barred even when the CAM never answers the query) (issue #1032).
- `Driver::add_service` and the entitlement re-query now CAID-filter the
  `ca_pmt` they send to the CAM's advertised CAIDs (once known), matching the
  filter the raw `descramble` path already applied — previously they sent
  every `CA_descriptor` unfiltered, which a CICAM rejects outright when it
  carries a `CA_system_id` the CAM doesn't support (issue #1067).
- `linux::LinuxCaDevice`'s `CA_RESET`/`CA_GET_SLOT_INFO` ioctls now use
  `libc::Ioctl` (the per-target request type: `c_ulong` on glibc, `c_int` on
  musl/uclibc/Android) instead of a hard-coded `c_ulong`, which failed to
  compile at all on musl (#1092).
- `LinuxCaDevice::slot_info` now falls back to "present + ready" only on
  `EINVAL`/`ENOTTY` (the documented "driver doesn't implement
  `CA_GET_SLOT_INFO`" case); any other ioctl error (`EIO`, `ENODEV`, …) is
  now propagated instead of being masked as a healthy slot (#1092).
- `LinuxCaDevice::read` no longer silently truncates a kernel frame wider
  than its 4096-byte scratch buffer and returns it as if it were the whole
  TPDU; the buffer is now sized to the largest legal TPDU
  (`MAX_CA_FRAME` = 65,539 bytes) and a still-full read is reported as
  `io::ErrorKind::InvalidData` (#1092).
- `Transport`'s `Active`-state poll cadence no longer sends another poll
  (or data block) while a previously-sent C_TPDU is still awaiting its
  reply — EN 50221's link is half-duplex — and that in-flight wait now has
  its own reply timeout (`TransportError::ReplyTimeout`), matching the
  timeout `Creating`'s `Create_T_C` already had (#1092).
- `Transport::send_spdu` now rejects an SPDU longer than `MAX_SPDU_LEN`
  (`TransportError::SpduTooLarge`) instead of queueing it and later
  panicking deep inside `flush`/`tick` when the resulting `CommandTpdu`
  failed to serialize (#1092).
- `Transport::send_spdu`'s outbound queue is now capped
  (`MAX_OUTBOUND_QUEUE`, `TransportError::OutboundQueueFull`) instead of
  growing without limit when the caller enqueues faster than the
  half-duplex link drains; the queue (and any in-flight reassembly) is also
  cleared on a setup timeout / malformed frame / wrong-`t_c_id` frame,
  instead of surviving stale into a later connection or chain (#1092).
- `SessionLayer::alloc` now probes past any `session_nb` still open before
  handing it out (relevant after the 65535-allocation wraparound), and a
  module-chosen `session_nb` (from `open_session_response`/
  `create_session_response`) that collides with an already-open, different
  resource — or is `0`, the reserved value — is now rejected instead of
  silently aliasing/overwriting the existing binding (#1092).
- `CiStack`'s `Init` now clears the session table and cached CAM CAIDs (not
  just the transport connection), and `Shutdown` — previously a complete
  no-op — now resets the device and clears the same stack-level state, so a
  later `Init` starts genuinely clean (#1092). Per-resource internal state
  is now included too: `Resource` gained a `reset()` hook, called on `Init`
  and `Shutdown`, and the stateful resources implemented it
  (`ResourceManager` clears the latched profile/handshake state, `DateTime`
  clears its resend timer), so `CamReady` fires again after a re-`Init`
  (closing the follow-up noted under #1092).
- `Driver::add_service` and the entitlement re-query timer no longer panic
  when the `ca_pmt` projected from a caller's (possibly corrupt or
  CAM-supplied) PMT has no valid wire encoding: `CaError::Serialize` (and
  `CaError::PmtParse` for stored raw PMT bytes that no longer re-parse) is
  returned from the public entry point instead, a rejected PMT is never
  recorded, and a corrupt service's failure is reported on every later pump
  instead of silently swallowing the healthy services' resend.
  `CiStack`'s raw `descramble` path surfaces the same class of failure as
  `Notification::Error` and sends nothing.
- `CaDescrambler::feed_ts` no longer rejects an entire TS batch because one
  packet in it has a bad sync byte (a single bit-error-corrupted packet is
  not evidence the whole batch is misaligned); the bad packet is now
  skipped and counted (`CaDescrambler::bad_sync_packets`) instead (#1092).
- `Driver::pump` now advances the stack's timers (reply timeout, poll
  cadence, entitlement re-query) by the real wall-clock time elapsed since
  the previous call, rather than trusting its `timeout` argument as if it
  were a measurement — `timeout` is only how long that call's `poll` was
  willing to wait, and could diverge from the real elapsed time in either
  direction (`Driver::with_clock` overrides the clock source, e.g. for
  tests) (#1092).
- Fixed stale documentation: `lib.rs`'s roadmap no longer lists the
  `host_control` resource as outstanding (it has been implemented since);
  `stack.rs`'s module doc no longer describes the resource state machines
  as still landing — all six are implemented (#1092).

## [0.16.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Changed (Breaking)
- `DeviceOp` (`device`), `LinkEvent` (`device`), and `TcState` (`transport`)
  now carry `#[non_exhaustive]` (issue #806's non_exhaustive drift-guard
  audit). A downstream `match` on any of these now needs a wildcard arm.

### Added
- `tests/label_coverage.rs` + `tests/non_exhaustive_coverage.rs` drift guards
  (issue #806).

## [0.15.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9** (issue #819). No functional or API change of
  this crate's own.

  Staying on `broadcast-common` 8 was not neutral: this crate's types implement
  `Parse`/`Serialize` from whichever major it links, so a consumer that used it
  alongside a 9-based crate (`transmux` 0.20, `dvb-si` 9, …) got **both majors
  in one graph**, and the trait methods resolved against the wrong one —
  surfacing as `no method named to_bytes found` / `no function named parse
  found` on types that plainly have them, with the compiler pointing at
  `broadcast-common-8.x/src/traits.rs`.

  The 9.0.0 wave originally shipped only the crates needed to publish
  `transmux`/`media-plane`/`multimux`, on the reasoning that everything else
  stayed coherent on its own 8 line. That reasoning was wrong: these crates
  exist to be composed, and the breakage only appears in a consumer that mixes
  them.

## [0.14.1] - 2026-07-25
### Fixed
- Entitlement re-query (#763's `set_requery_interval`) now rebuilds each
  service's `ca_pmt` list-management (`First`/`More`/`Last`/`Only`, EN 50221
  §8.4.3.4 Table 25) against the **current** active set on every tick, instead
  of resending the value frozen at that service's `add_service` time (#765).
  A sole surviving service — after its siblings were `remove_service`'d — now
  correctly re-queries with `Only` rather than a stale `Add`, which a
  strictly-conformant CAM could reject. The internal per-service
  `requery_ca_pmt` byte-copy is removed; re-query is rebuilt from the already-
  stored `pmt_raw` instead.

## [0.14.0] - 2026-07-24
### Added
- **Single-slot managed CAS layer** (#763) — a CA orchestration layer over the
  raw `ca_pmt`/`ca_pmt_reply` surface, fed **parsed `dvb-si` structs** (never
  raw bytes). The raw `send_ca_pmt(&[u8])` + notification surface is unchanged;
  the managed API sits alongside it (opt-in):
  - `Driver::add_service(&PmtSection)` / `remove_service(program_number)` build
    and send the `ca_pmt` via `dvb_ci::builder::build_ca_pmt` (EN 50221
    §8.4.3.4 Table 25) and track the slot's active service set — list-management
    (`Only`/`Add`, `Update`/`NotSelected`) auto-selected from the tracked set.
    `add_service` rejects a PMT with no `CA_descriptor` (`CaError::NoCaDescriptor`).
  - `Driver::set_cat(&CatSection)` — sets the CAT (ISO/IEC 13818-1 §2.4.4.5);
    the EMM-PID feed is the CAT's EMM PIDs ∩ the CAM's advertised `ca_info`
    CAIDs (only what this CAM can use). Not an error before `ca_info` arrives.
  - `Driver::emm_pids()` / `descramble_pids()` / `ca_pids()` / `required_pids()`
    — the PIDs to route into the `ci0` data plane: EMM PIDs, ES PIDs, ECM PIDs,
    and their union (`required_pids` = ES ∪ ECM ∪ EMM).
  - `Driver::set_requery_interval(Duration)` — periodic `ca_pmt` re-query with
    `cmd_id = query` (EN 50221 §8.4.3.5 Table 26 — only `query`/`ok_mmi` solicit
    a `ca_pmt_reply`) so a card entitled *after* the initial `ca_pmt` still
    refreshes; `Duration::ZERO` disables it. Default 10 s (`REQUERY_DEFAULT`).
  - `Notification::Entitlement { program_number, ca_enable, descrambling_ok }`
    — **edge-triggered** per programme, fired only on a status transition
    detected by the re-query. Complements #726 `HotPlug` (coarse module/card
    layer) with the fine-grained per-service layer.
  - `CaDescrambler<D: CaDevice, C: CiDataDevice>` — turnkey wrapper that also
    owns the `ci0` data plane: `feed_ts(scrambled)` filters a TS chunk to
    `required_pids()` and writes only those packets to `ci0`, returning the
    descrambled TS; `add_service`/`set_cat`/`take_notifications`/`required_pids`
    delegate to the wrapped `Driver`. One `CaDescrambler` = one CI slot = one
    TS path (multi-tuner ⇒ one per slot; cross-mux merge is a remux, out of
    scope here).
  - `Notification::CaPmtReply` gains a typed `ca_enable: Option<CaEnable>` field
    (`dvb_ci::objects::ca_pmt_reply::CaEnable`, EN 50221 §8.4.3.5 Table 26) —
    distinguishes not-entitled/technical-failure/purchase-dialogue rather than
    the boolean-only `descrambling_ok` (which stays, now derived from
    `ca_enable`). `None` means the programme `CA_enable_flag` bit was clear (no
    programme-level status given), plumbed straight through from the `dvb_ci`
    `CaPmtReply` object's own `Option<CaEnable>` — never collapsed to a sentinel.
  - New public types: `ManagedCa`, `ManagedService`, `CaError`, `REQUERY_DEFAULT`
    (re-exported at the crate root); `CaDescrambler`. `Notification` is
    `#[non_exhaustive]`, so the new variant + `CaPmtReply` field are additive.

### Fixed
- `required_pids()` (and therefore `CaDescrambler::feed_ts`) now includes each
  active service's PMT `PCR_PID` (ISO/IEC 13818-1 §2.4.4.8) — previously only
  ES ∪ ECM ∪ EMM were routed, so a service carrying its PCR on a **dedicated**
  PID (not equal to any ES/component PID — a legitimate DVB configuration) had
  those packets filtered out of `ci0`'s feed, leaving the descrambled TS with
  no clock reference. `PCR_PID == 0x1FFF` ("no PCR" per ISO/IEC 13818-1) is
  still excluded. `ManagedService` gains a `pcr_pid: u16` field (additive,
  `#[non_exhaustive]`).

## [0.13.0] - 2026-07-21
### Added
- **CAM + card hot-plug `Notification`s** (#726): `Notification::HotPlug(HotPlug)`
  carries the transition — `HotPlug::CamPresent` / `CamRemoved` are real DVB-CA
  slot-status edges (`CA_CI_MODULE_PRESENT`), emitted once per edge; the driver
  re-drives the reset/init handshake on insert and tears down session state on
  removal. `HotPlug::CardInserted` / `CardRemoved` / `CardChanged` are
  best-effort app-layer inference from `ca_info` CAID-set changes,
  `ca_pmt_reply` `descrambling_ok` transitions, and MMI "no card"/entitlement
  keyword text (EN 50221 CI slots have no card-detect line). `HotPlug` gets the
  #204 `name()`/`Display` label pair; `Notification::hotplug()` is a cheap
  `Option<HotPlug>` classifier for poll-mode consumers. `SlotInfo` gained a
  `module_present` field alongside the existing `module_ready`.
- **`Driver::pump_with`/`pump_hotplug`** — closure-callback pump variants
  (this crate is sync/sans-IO, so a per-call closure is the push-style
  alternative to poll-draining `Driver::take_notifications` yourself):
  `pump_with(timeout, |note: &Notification| ...)` invokes `handler` for every
  notification the pump cycle produced; `pump_hotplug(timeout, |hp: HotPlug|
  ...)` filters to just `HotPlug` transitions. Both wrap the existing
  `pump`/`take_notifications` (still public, unchanged) — purely additive.
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
