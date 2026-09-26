# mpeg-ts 0.4.1

Security patch for `pusi::PusiReassembler`. **Upgrade if you reassemble PUSI-delimited payloads
from streams you do not control.** Drop-in for 0.4.x.

## Security

**GHSA-74gr-mvr4-2qp8** — before this release:
- `PusiReassembler` appended continuation payload before it had seen any
  `payload_unit_start_indicator`. After joining a stream mid-unit, the first emitted unit
  therefore began with the tail of the previous one.
- Its buffer had no limit, so a PID that never set PUSI again grew memory without bound.

## Changes

- Continuation payload before the first PUSI is ignored.
- A unit larger than the reassembler's limit is discarded, and reassembly restarts at the next
  PUSI.
- **Added:** `DEFAULT_MAX_UNIT_SIZE` (16 MiB) is the default limit.
  `PusiReassembler::with_max_unit_size(pid, max)` sets a different one. The default is
  deliberately large: a video PES packet with `PES_packet_length = 0` has no length bound
  (ISO/IEC 13818-1 §2.4.3.7).

MSRV 1.95.0.
