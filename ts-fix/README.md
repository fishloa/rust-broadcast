# ts-fix — MPEG-2 TS repair / remux

A **forward-compatible streaming engine** for repairing and remultiplexing
MPEG-2 Transport Streams (ISO/IEC 13818-1 §2.4).  Feed 188-byte packets in,
get repaired packets out.

Operations are opt-in, additive, and applied in the engine's canonical order
regardless of registration order:

| Operation | Builder method | What it does |
|---|---|---|
| Continuity repair | `.repair_continuity()` | Renumber per-PID continuity counters to correct monotonic sequences (mod 16) respecting payload-bearing rules (§2.4.3.3). |
| PID filter / service extract | `.filter_pids(PidFilter)` | Keep only specified PIDs — or track the live PAT/PMT to extract a single programme by `program_number`. |
| PAT/PMT regeneration | `.regen_psi()` | Rebuild the PAT from observed PMT PIDs on flush; useful after filtering. |
| PCR restamp | `.restamp_pcr(PcrRestamp)` | Recompute PCR values on the PCR PID onto one continuous timeline (§2.4.3.5), including across a genuine unflagged break (TR 101 290 §5.2.2 2.3b). |
| PCR-discontinuity honor | `.honor_pcr_discontinuity()` | Set `discontinuity_indicator` on genuine, unflagged PCR breaks without rewriting any timestamp — the alternative to restamping. |
| Stuffing | `.stuffing(Stuffing)` | Drop null packets (PID 0x1FFF) or pad to a target packet rate. |

`discontinuity::detect_pcr_discontinuities` is a standalone read-only scan for
auditing PCR breaks (flagged vs. unflagged) without repairing the stream.

All configuration enums are `#[non_exhaustive]`; adding new operations in a
future minor release is purely additive and never breaks callers.

## Quick start

```rust
use ts_fix::{TsFix, PidFilter, Stuffing};

let mut engine = TsFix::builder()
    .repair_continuity()
    .filter_pids(PidFilter::keep([0x0100, 0x0101]))
    .regen_psi()
    .stuffing(Stuffing::drop_nulls())
    .build()
    .unwrap();

let mut output = Vec::new();
for chunk in input.chunks(188) {
    engine.push(chunk, |pkt| output.extend_from_slice(pkt)).unwrap();
}
engine.finish(|pkt| output.extend_from_slice(pkt));
```

## CLI

```text
ts-fix --input in.ts --output out.ts --repair-continuity --drop-nulls
```

Full flags: `--help` for details.

## Examples

```bash
# Repair continuity counters on the test fixture
cargo run --example repair_continuity

# Extract programme 1 from a synthetic multi-program stream
cargo run --example extract_service
```

## Spec

ISO/IEC 13818-1 (= ITU-T H.222.0) — §2.4.3.2 (TS packet), §2.4.3.3
(adaptation field / continuity counter), §2.4.3.4 (PCR), §2.4.4 (PSI). PCR
discontinuity classification reuses ETSI TR 101 290 v1.4.1 §5.2.2 Table 5.0b
indicator 2.3b (`dvb-conformance`).

## License

MIT OR Apache-2.0
