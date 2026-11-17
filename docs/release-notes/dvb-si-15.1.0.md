# dvb-si 15.1.0

_Released 2026-11-17._

Minor release (15.0 -> 15.1) with one fix: the `SiDemux` version gate now evicts per PID instead of from a single global FIFO. A source flooding distinct `(table_id_extension, section_number)` keys on one PID can no longer evict a legitimate PID's entries and make its unchanged repeats re-emit as changed.

## Fixed

- `demux::SiDemux`'s version gate evicts per PID: at capacity, the oldest key of the PID occupying the most of the gate goes first, and a PID already over its equal share (`capacity / PIDs with entries`) pays first. A source flooding distinct `(table_id_extension, section_number)` keys on one PID can no longer evict a legitimate PID's entries, which the old single global FIFO did — making that PID's unchanged repeats re-emit as "changed".

---

Published from tag `dvb-si-v15.1.0`.
