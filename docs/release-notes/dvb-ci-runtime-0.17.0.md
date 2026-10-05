# dvb-ci-runtime 0.17.0

_Released 2026-10-05._

Breaking release (0.16 -> 0.17) of the EN 50221 driver runtime. The headline is that errors are no longer swallowed: the resource and session layers used to drop a malformed CAM APDU or SPDU silently, and a failed serialization had no way to surface. Their methods now return `dvb_ci::Result`, and CAM-originated parse failures become `Notification::Error`. The release also changes one default (the periodic entitlement re-query is now off), fixes the re-query so it can no longer silently stop descrambling, hardens the transport and session layers against stalls, runaway queues and aliasing, and fixes several Linux-device defects including a hang in `CaDescrambler::feed_ts`. It builds against `dvb-ci` 0.9 and `dvb-si` 11. You must act if you implement `Resource` or call `SessionLayer` methods directly; if you only use `Driver` / `CaDescrambler`, check the re-query default and the Behaviour changes.

Read with `dvb-ci-0.9.0.md` (its stricter APDU parsers are what produce the new `Notification::Error`s) and `dvb-si-11.0.0.md`.

## Breaking changes

1. **`resource::Resource::on_open`, `on_apdu` and `tick` return `dvb_ci::Result<ResourceOut>`** (were `ResourceOut`), so an APDU that fails to serialize, or a malformed CAM APDU, is an `Err` instead of being dropped. Every `Resource` implementation must be updated.
   ```rust
   // before (0.16)
   fn on_apdu(&mut self, apdu: &[u8]) -> ResourceOut { ... }
   // after (0.17)
   fn on_apdu(&mut self, apdu: &[u8]) -> dvb_ci::Result<ResourceOut> { ... Ok(out) }
   ```
   `Resource` also gained `fn reset(&mut self)` with a default no-op body (called on `HostRequest::Init` and `Shutdown`; see Fixes). Existing implementations need not define it unless they carry state between calls.
2. **`session::SessionLayer`**: `create_session`, `send_apdu` and `close` return `dvb_ci::Result<Vec<u8>>` (were `Vec<u8>`), and `on_spdu` returns `dvb_ci::Result<SessionOut>` (was `SessionOut`). A malformed SPDU is surfaced rather than dropped (#1092).

## Behaviour changes

- **`managed::REQUERY_DEFAULT` is now `Duration::ZERO` (disabled); it was 10 s** (issue #1032). The periodic entitlement re-query is opt-in: call `Driver::set_requery_interval(Duration)` to enable it. No CAM this crate has been verified against answers the `query` half (a live AlphaCrypt never replies to `query` at all, as `stack.rs` documents), so it stays off by default until confirmed on real hardware.
- **CAM-originated parse failures are reported.** An APDU or SPDU that fails to parse (for example a padded `tune`, a truncated MMI `enq`, a malformed `open_session_response`) was dropped by `if let Ok(..) = parse(..)` guards in the resource and session layers; it now surfaces as `Notification::Error` (#1092). Combined with the stricter parsers in `dvb-ci` 0.9, a CAM that pads fixed-layout APDUs will now generate these notifications.
- **`Driver::pump` timing.** It advances the stack's timers (reply timeout, poll cadence, entitlement re-query) by the real wall-clock time since the previous call, not by its `timeout` argument, which is only how long that call's `poll` was willing to wait and could diverge from elapsed time either way. `Driver::with_clock` overrides the clock source (for tests) (#1092).
- **Entitlement re-query wire behaviour** (issue #1032). The re-query no longer sends `list_management = only/first` + `cmd_id = query` on a timer. Per EN 50221 sections 8.4.3.4/8.4.3.5 that replaces the active programme list and bars descrambling until an `ok_descrambling` that was never sent, so descrambling silently stopped about one interval after it started. The resend is now a `list_management = update` pair: `cmd_id = query` (to still solicit a fresh `ca_pmt_reply` from a CAM that answers it) immediately followed unconditionally by `cmd_id = ok_descrambling`.
- **`ca_pmt` CAID filtering** (issue #1067). `Driver::add_service` and the re-query now filter the `ca_pmt` they send to the CAM's advertised CAIDs once known, matching the raw `descramble` path. Previously they sent every `CA_descriptor`, which a CICAM rejects outright when it carries a `CA_system_id` it does not support.
- **`CaDescrambler::feed_ts`** no longer rejects a whole TS batch because one packet has a bad sync byte; the packet is skipped and counted (`CaDescrambler::bad_sync_packets()`) (#1092).
- **`Debug` redaction.** `HostRequest`'s `Debug` redacts the text of `MmiEnquiryAnswer` (what the user typed at an enquiry, often a PIN) (audit #1142).
- **`trace::decode_frame`** now names Delete, D_T_C_Reply, Request and New T_C TPDUs (they printed as `T_?`).

## Fixes: transport and session layers (#1092)

- `Transport`'s `Active`-state poll no longer sends another poll or data block while a C_TPDU awaits its reply (the EN 50221 link is half-duplex), and that wait has its own timeout, `TransportError::ReplyTimeout`, matching `Creating`'s `Create_T_C` timeout.
- `Transport::send_spdu` rejects an SPDU longer than `MAX_SPDU_LEN` with `TransportError::SpduTooLarge` instead of queueing it and panicking later in `flush`/`tick` when the `CommandTpdu` failed to serialize.
- `Transport`'s outbound queue is capped at `MAX_OUTBOUND_QUEUE` (`TransportError::OutboundQueueFull`) instead of growing without limit when the caller outpaces the link. The queue and any in-flight reassembly are also cleared on a setup timeout, malformed frame or wrong-`t_c_id` frame, instead of surviving into a later connection.
- `SessionLayer::alloc` skips any `session_nb` still open (relevant after the 65535-allocation wraparound). A module-chosen `session_nb` that collides with an already-open, different resource, or is the reserved `0`, is rejected instead of aliasing the existing binding.
- `CiStack`'s `Init` now clears the session table and cached CAM CAIDs (not just the transport connection). `Shutdown`, previously a no-op, now resets the device and clears the same state. `Resource::reset()` is called on both, and the stateful resources implement it (`ResourceManager` clears its latched profile/handshake state, `DateTime` its resend timer), so `CamReady` fires again after a re-`Init`.
- `Driver::add_service` and the re-query timer no longer panic when the `ca_pmt` projected from a (possibly corrupt or CAM-supplied) PMT has no valid wire encoding: `CaError::Serialize` (or `CaError::PmtParse` for stored raw PMT bytes that no longer re-parse) is returned, a rejected PMT is never recorded, and a corrupt service's failure is reported on every later pump instead of silently dropping the healthy services' resend. `CiStack`'s raw `descramble` path reports the same class of failure as `Notification::Error` and sends nothing.
- The MMI card-keyword heuristic (`HotPlug::CardInserted` / `CardRemoved`) is edge-triggered: a CAM that re-sends the same "insert card" menu no longer repeats the notification (audit r10-O-13).

## Fixes: Linux device (`linux` feature)

- **`CaDescrambler::feed_ts` hung on the real `ciM` data-plane device** (issue #1066). `LinuxCiDataDevice::open` now opens `O_NONBLOCK`, so the drain loop sees `WouldBlock` (mapped to "no more data") instead of blocking forever on a second read.
- `LinuxCaDevice`'s `CA_RESET` / `CA_GET_SLOT_INFO` ioctls use `libc::Ioctl` (the per-target request type: `c_ulong` on glibc, `c_int` on musl/uclibc/Android) instead of a hard-coded `c_ulong`, which failed to compile on musl (#1092).
- `LinuxCaDevice::slot_info` falls back to "present + ready" only on `EINVAL` / `ENOTTY` (driver does not implement `CA_GET_SLOT_INFO`); any other ioctl error (`EIO`, `ENODEV`, ...) is propagated instead of being masked as a healthy slot.
- `LinuxCaDevice::read` no longer silently truncates a kernel frame wider than its 4096-byte scratch buffer and returns it as a whole TPDU. The buffer is sized to the largest legal TPDU (65,539 bytes) and a still-full read is `io::ErrorKind::InvalidData`.
- Readiness polling uses `rustix::event::poll` instead of `libc::poll`; sub-millisecond timeouts are no longer truncated to zero. No public API change.

## Fixes: efficiency and docs

- `Driver::pump` borrows the receive buffer in place instead of a per-frame `to_vec()`; a counting-allocator test pins the largest allocation at 416 bytes for a 4096-byte frame (was 4096) (audit r10-O-11, #1092).
- `CiStack` found a resource's session by probing all 65,535 session numbers; the new `SessionLayer::session_for(resource) -> Option<u16>` walks only open sessions (audit r10-O-9).
- The duplicated `ser` / `ser_apdu` helper in `resource`, `session` and `stack` is one shared copy (audit r10-O-12). Stale docs fixed: the `lib.rs` roadmap no longer lists `host_control` as outstanding, `stack.rs` no longer describes the resource state machines as still landing, and `HotPlug::CamPresent`'s doc matches the code (the driver keys the edge off `module_present` alone).
- Not changed: `CaDescrambler::feed_ts`'s per-feed PID set (caching needs invalidation on every `add_service` / `set_cat` / re-query, with no measured cost) and `LinuxCaDevice::reset`'s 3 s settle `sleep` in the pump (making it a sans-IO timer alters reset-to-Create_T_C timing, which could not be verified without a CAM) (audit r10-O-10).

## Dependencies

```toml
# before (0.16.0)
dvb-ci           = { ..., version = "0.8" }
dvb-si           = { ..., version = "10", default-features = false }
broadcast-common = { ..., version = "9.3", default-features = false }
libc             = { version = "0.2", optional = true }
# after (0.17.0)
dvb-ci           = { ..., version = "0.9" }
dvb-si           = { ..., version = "11.0", default-features = false }
broadcast-common = { ..., version = "9.4", default-features = false }
libc             = { version = "0.2.108", optional = true }   # libc::Ioctl first appears in 0.2.108
rustix           = { version = "1", default-features = false, features = ["std", "event"], optional = true }
```

The `linux` feature is now `["dep:libc", "dep:rustix", "dep:clap"]` (was `["dep:libc", "dep:clap"]`); `libc` stays for the CA ioctls. A `dvb-ci` and `dvb-si` pair on the new epochs is required; see `dvb-ci-0.9.0.md` and `dvb-si-11.0.0.md`. Dev-only: `tests/no_handroll_guard.rs`, and Linux tests for the readiness poll (timeout, zero timeout, sub-millisecond timeout, hang-up without data).

---

Published from tag `dvb-ci-runtime-v0.17.0`.
