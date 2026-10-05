# dvb-tools 11.0.0

_Released 2026-10-05._

### Fixed
- `pids` bitrate estimate: pick one PCR PID and use only its own first/last
  PCR readings, instead of mixing the first PCR seen on any PID with the
  last PCR seen on any (possibly different) PID — a multiplex carries one
  independent PCR clock per program (ISO/IEC 13818-1 §2.4.4.9), so comparing
  two different PCR PIDs' timestamps is meaningless and previously reported
  "n/a" on any multi-program capture with more than one PCR PID (issue #1100).
  Also unwrap the PCR's own ~26.5h rollover (`2^33 * 300` 27 MHz ticks,
  ISO/IEC 13818-1 §2.4.2.2/§2.4.3.5) across samples on the selected PID, so
  a capture whose PCR crosses that wrap no longer reports "n/a" either.
- `services`: key services and logical channel numbers by
  `(original_network_id, transport_stream_id, service_id)` instead of
  `service_id` alone — `service_id` is only unique within one transport
  stream's namespace (ETSI EN 300 468 §5.2.2/§5.2.3), so a numerically
  identical `service_id` on a second transport stream (SDT-other, or a
  second TS entry in the NIT) previously overwrote the first service and
  could be shown with the wrong LCN (issue #1100).

---

Published from tag `dvb-tools-v11.0.0`.
