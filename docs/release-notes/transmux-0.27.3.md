# transmux 0.27.3

_Released 2026-10-17._

**Patch release.** `Media::degraded_at[i].track` was `None` for a PID that is in `Media::tracks` (for example a DSM-CC carousel PID announced only at the end of the stream), so a consumer could not map every degradation record to its track. 0.27.3 resolves each record against the final track list. Reported by zenith-tv on 0.27.2 (issue #1348).

## Why this version

- **Proposed version:** 0.27.2 -> **0.27.3** (patch: a resolution bug in an additive field; no signature changed).
- **No dependency changes.**

## Behaviour

`DegradationRecord::track` is `Some(track id)` whenever the record's PID maps to a track in the finished `Media`; it stays `None` when the PID has no track and for stream-wide damage (`pid: None`). The live `StreamingTsDemux` `InputDegraded` event is unchanged: it carries whatever had resolved when the event fired, so a PID that becomes a track later is still `None` there; only the batch `TsDemux::unpackage` result is resolved after the fact.

## What changed

### Fixed
- **`Media::degraded_at[i].track` was `None` for a PID that is in `Media::tracks` (#1348).** The streaming core stamps `InputDegraded::track` with what had resolved when the event fired, and a Data/DSM-CC PID only becomes a track once its first section completes (or at `finish()`), so on `orf3-dvbs2-raw-zenith-tv.ts` every record for PID 0x1C90 had `track: None` although it is a Data track. The batch `TsDemux` now resolves each still-`None` record against the final tracks (`spec.source_pid == provenance.pid`). A record whose PID never became a track, or stream-wide damage (`provenance.pid == None`), stays `None`; the live `DemuxEvent::InputDegraded::track` is unchanged (only what had resolved at that moment; documented).

---

Published from tag `transmux-v0.27.3`.
