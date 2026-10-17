# transmux 0.27.2

_Released 2026-10-17._

**Patch release.** `Media::degraded` (0.27.1) reported fewer `ContinuityGap` entries than the stream contains whenever a PID repeated an identical packet more than once in a row, and gave a consumer no way to tell which PID an entry belongs to. 0.27.2 counts every excess repeat and adds a per-entry PID.

## Why this version

- **Proposed version:** 0.27.1 -> **0.27.2** (patch: a counting bug fixed and one additive field on a `#[non_exhaustive]` struct).
- **No dependency changes.** `mpeg-ts` stays at 0.6.1.

## Behaviour change to know about

H.222.0 section 2.4.3.3 allows duplicate packets "as two, and only two, consecutive transport stream packets of the same PID": the original plus one byte-identical duplicate (the PCR field excepted). Every further identical repeat is a continuity fault. Until 0.27.1 the check reset the PID's one-duplicate allowance after the first excess repeat, so the next repeat was accepted as legal again and legal and fault alternated. A real ORF III DSM-CC carousel PID (180 excess repeats plus 7 real gaps) reported 97 entries; it now reports 187 (one per excess packet, the run staying spent until a fresh original arrives). The payload of an excess copy is discarded like a duplicate's, so the access unit being reassembled is no longer dropped.

Captures with long identical-repeat runs therefore report more `ContinuityGap` entries than on 0.27.1. A consumer that gates on `Media::degraded.is_empty()` should filter by PID (below): carousel and data PIDs commonly repeat packets and are not the tracks you keep.

## Migration

```rust
let media = TsDemux::new().unpackage(&bytes)?;
// `degraded_at` runs parallel to `degraded` (same order and length) for a TS input.
for (entry, at) in media.degraded.iter().zip(&media.degraded_at) {
    // at.provenance.pid: Option<Pid>, at.provenance.packet_index, at.track
    if at.provenance.pid.is_some_and(|p| kept_pids.contains(&p)) {
        // damage on a PID you actually use
    }
}
```

`degraded_at` is empty for non-TS inputs. `Media` is `#[non_exhaustive]`, so the new field is not a breaking change.

## What changed

### Fixed
- **`InputDegradation::ContinuityGap` under-counted a run of identical repeats.** ISO/IEC 13818-1 §2.4.3.3 allows exactly one byte-identical repeat of a packet (the duplicate); every further repeat is a continuity fault (ETSI TR 101 290 1.4, "duplicate more than twice"). The first fault used to reset the PID's one-duplicate allowance, so the next repeat was accepted as legal again and a run alternated legal/fault: `orf3-dvbs2-raw-zenith-tv.ts` (DSM-CC carousel PID 0x1C90: 118 legal duplicates, 180 repeats beyond the first, 7 real gaps) reported 97 entries instead of 187. Now one fault per excess packet, and the run stays spent until a fresh original arrives. The excess copy's payload is discarded like a duplicate's (nothing was lost, so the access unit under construction is no longer dropped). Behaviour change: captures with long identical-repeat runs report more `ContinuityGap` entries (the other bundled captures are unchanged; their counts were checked per PID against an independent byte walk).

### Added
- `Media::degraded_at: Vec<DegradationRecord>`: for a TS input, parallel to `Media::degraded` (same order and length) with the TS PID, packet index and resolved track of every entry, so a consumer can ignore damage on PIDs it does not keep (a carousel) and act on damage to its tracks. Empty for containers without PID/packet provenance. Not breaking: `Media` and `DegradationRecord` are `#[non_exhaustive]`.

---

Published from tag `transmux-v0.27.2`.
