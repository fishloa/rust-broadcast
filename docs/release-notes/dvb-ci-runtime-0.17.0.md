# dvb-ci-runtime 0.17.0

_Released 2026-10-05._

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

---

Published from tag `dvb-ci-runtime-v0.17.0`.
