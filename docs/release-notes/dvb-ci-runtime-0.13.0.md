# dvb-ci-runtime 0.13.0 — 2026-07-21

Minor. Surfaces CAM + card hot-plug transitions as a typed `HotPlug` sub-enum, adds closure-callback pump variants, plus a hardware-path bug fix. Additive (`Notification` and `HotPlug` are both `#[non_exhaustive]`) — no breaking API change.

## Added — hot-plug `Notification::HotPlug(HotPlug)` (#726)

Consumers (players, descramble subsystems) previously had to re-derive hot-plug transitions from raw notifications. The runtime already sees every signal, so it now emits one typed event, `Notification::HotPlug(HotPlug)`, grouping the five transitions into their own sub-enum:

- **`HotPlug::CamPresent` / `CamRemoved`** — real DVB-CA slot-status edges (`CA_CI_MODULE_PRESENT`). Emitted once per edge (not per poll). On insert the driver re-drives the reset/init handshake on a fresh stack; on removal it tears down session state so a re-insert re-handshakes cleanly. The first observation only sets the baseline, so a CAM already inserted at `init()` does not spuriously fire.
- **`HotPlug::CardInserted` / `CardRemoved` / `CardChanged`** — **best-effort app-layer inference**. EN 50221 CI slots are module-level only; there is no card-detect line (the smartcard sits behind the CAM's MCU, invisible to the host — verified against the Digital Devices ddbridge / cxd2099 drivers). Presence/identity is therefore inferred from:
  - `ca_info` CAID-set transitions (empty→populated = inserted, populated→empty = removed, populated→different = changed),
  - `ca_pmt_reply` `descrambling_ok` false↔true transitions,
  - MMI "no card"/entitlement keyword text,
  - a module READY-cycle (surfaces as `CamPresent` for CAMs that reset on card change).

  Documented as heuristic: some CAMs give a strong reset signal, others only a subtle `ca_info`/CW-flow change.

- `HotPlug` carries the #204 label pair — `HotPlug::name()` (`"cam-present"`, `"cam-removed"`, `"card-inserted"`, `"card-removed"`, `"card-changed"`) and a matching `Display`. `Notification::hotplug(&self) -> Option<HotPlug>` is a cheap classifier for poll-mode consumers that only care about hot-plug edges.
- `SlotInfo` gained a `module_present: bool` field alongside `module_ready`.

## Added — `Driver::pump_with` / `pump_hotplug` closure callbacks

This crate is sync/sans-IO (no channels, no async runtime), so the ergonomic push-style alternative to poll-draining `Driver::take_notifications` yourself is a closure invoked in-line per pump call:

- **`Driver::pump_with(timeout, handler: FnMut(&Notification))`** — pumps once, then invokes `handler` for every notification the cycle produced (drain-and-dispatch). Returns the same `bool` as `pump`.
- **`Driver::pump_hotplug(timeout, handler: FnMut(HotPlug))`** — convenience over `pump_with` that invokes `handler` only for `HotPlug` transitions, ignoring everything else that cycle.

Both wrap the existing `pump`/`take_notifications` — which stay public and unchanged — so callers can mix poll-drain and closure styles as needed. The closure is per-call: nothing is stored, so there are no lifetime constraints beyond the call itself.

## Fixed

- `LinuxCaDevice::slot_info` read `CA_CI_MODULE_READY` from the wrong bit (`1` — which is actually the uapi's `CA_CI_MODULE_PRESENT`) instead of `2`. `module_ready` was reporting module *presence*, not *readiness*. Both bits are now read correctly (`CA_CI_MODULE_PRESENT = 1`, `CA_CI_MODULE_READY = 2`, per `include/uapi/linux/dvb/ca.h`).

## Compatibility

MSRV 1.86. Additive `#[non_exhaustive]` enum variants, two new `Driver` methods, and a new `SlotInfo` field — recompile, no source changes required for existing consumers.
