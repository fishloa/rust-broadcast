# dvb-ci-runtime 0.21.1

_Released 2026-11-17._

Patch release from the audit 20 bundle bounding the CA runtime's state. The session table is capped at 32 sessions and one per resource, the pending notification queue at 1024 entries with a dropped counter, and the read buffer is the largest legal CA frame instead of 4096 bytes so a `Tdata_last` block is no longer lost. `MAX_CA_FRAME` is now public.

## Added

- `MAX_CA_FRAME` is public (re-exported from `device`): the largest legal link-layer frame a `CaDevice` can deliver.
- `Driver::notifications_dropped` reports how many of the oldest buffered notifications the new 1024-entry bound discarded.

## Fixed

- The session layer keeps at most 32 sessions and one per resource: a duplicate `open_session_request` for a resource that already has a session, or a request past the bound, is answered `resource_busy` instead of growing the table (previously up to 65 535 sessions, each duplicating that resource's per-tick work). Each resource handler is ticked once per tick regardless of how many sessions reach it.
- `Driver`'s read buffer is `MAX_CA_FRAME` instead of a 4096-byte buffer, so a legal TPDU larger than ~4094 bytes (a `Tdata_last` block) is no longer consumed by the device and then surfaced as an `InvalidData` read error, losing the frame.
- The MMI `menu`/`list` chain cap (`MAX_MMI_CHAIN_LEN`) now has a test that drives a chain one byte past it and asserts the `LengthTooLarge` error; removing the bound no longer passes the suite.
- The pending notification queue is bounded at 1024 entries (oldest discarded, counted); a host that only calls `pump` against a module sending one notification per malformed frame no longer grows it for the life of the process.

---

Published from tag `dvb-ci-runtime-v0.21.1`.
