# dvb-conformance

ETSI TR 101 290 v1.4.1 transport-stream conformance monitor (DVB measurement
guidelines). Feed TS packets with a caller-supplied monotonic clock, drain
structured conformance events for each indicator.

## Indicator coverage

| Priority | Indicator | Status |
|----------|-----------|--------|
| 1 | 1.1 `TS_sync_loss` | ✅ |
| 1 | 1.2 `Sync_byte_error` | ✅ |
| 1 | 1.3.a `PAT_error_2` | ✅ |
| 1 | 1.4 `Continuity_count_error` | ✅ |
| 1 | 1.5.a `PMT_error_2` | ✅ |
| 1 | 1.6 `PID_error` | ✅ |
| 2 | 2.1 `Transport_error` | ✅ |
| 2 | 2.2 `CRC_error` | ✅ |
| 2 | 2.3a `PCR_repetition_error` | ✅ |
| 2 | 2.3b `PCR_discontinuity_indicator_error` | ✅ |
| 2 | 2.4 `PCR_accuracy_error` | ❌ requires hardware arrival timestamps; not computable under the caller-supplied-time model |
| 2 | 2.5 `PTS_error` | ✅ |
| 2 | 2.6 `CAT_error` | ✅ |
| 3 | 3.1–3.10 | ❌ (next) |
| SI | SI_repetition_error | ❌ (next) |

## Caller-supplied time model

`ConformanceMonitor::feed(packet, t)` takes a `Duration` timestamp per packet.
The monitor uses this clock for all presence/absence timeout checks (1.3.a,
1.5.a, 1.6, 2.3a, 2.3b, 2.5). The caller is responsible for providing a
monotonic, roughly-wall-clock-aligned timestamp — the monitor does not enforce
monotonicity, but non-monotonic timestamps will produce spurious timeout events.

Because the monitor has no independent clock, some later-priority indicators
(PCR accuracy, buffer model) that require sub-packet timing resolution or
absolute reference clocks are best-effort under this model.
