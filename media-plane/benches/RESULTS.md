# Trunk contention benchmark results

Benchmark: `benches/trunk_contention.rs` — one publisher thread publishing
`188*7`-byte timed samples, N reader threads each spinning on a
`SampleCursor::poll()` (worst case; production readers park on `listen()`).
Metric: median publisher ns per `publish` (middle value of criterion's
`time: [low mid high]`).

Machine: macOS 27.0.1 (Darwin 27.0.0), 24 logical cores (`sysctl -n hw.ncpu`).
N = 64 spinning readers oversubscribes 24 cores; that point is a stress
figure, not a production shape.

Command: `cargo bench --locked -p media-plane --bench trunk_contention`

## std `Mutex` (baseline, this commit)

| N readers | median ns/publish | low | high |
|---|---|---|---|
| 1  | 45.9   | 42.9   | 50.1   |
| 4  | 342.1  | 333.7  | 347.8  |
| 16 | 440.3  | 397.4  | 476.3  |
| 64 | 2244.6 | 1990.3 | 2512.7 |

## after parking_lot

| N readers | median ns/publish | low | high |
|---|---|---|---|
| 1  | 38.4   | 34.0   | 42.4   |
| 4  | 124.9  | 119.3  | 129.7  |
| 16 | 574.0  | 565.7  | 581.8  |
| 64 | 2646.7 | 2392.0 | 2982.2 |

Single-thread (N = 1) cost and the 4-reader case improve; the 16- and
64-reader points are no better than the std `Mutex` (run-to-run noise at those
points is large with spinning readers: the baseline N = 16 run read 440 ns,
this one 574 ns).

## Reviewer re-run (3 runs each, median ns/publish, same machine; std = commit 49b2607b, pl = parking_lot)

| N | std runs | std median | parking_lot runs | pl median |
|---|---|---|---|---|
| 1  | 44 / 54 / 64        | 54   | 26.5 / 43 / 29      | 29   |
| 4  | 349 / 311 / 301     | 311  | 137 / 135 / 145     | 137  |
| 16 | 312 / 1062 / 586    | 586  | 637 / 611 / 575     | 611  |
| 64 | 1727 / 5694 / 2193  | 2193 | 2415 / 4192 / 2196  | 2415 |

Variance: std spans 312-1062 ns at N = 16 and 1.7-5.7 us at N = 64 across three
runs, and parking_lot lands inside those ranges, so the apparent regression at
N = 16 / 64 in the single-run table above is noise. The reproducible effect is
at N = 1 (about 1.9x faster) and N = 4 (about 2.3x faster). Verdict: KEEP
parking_lot.

## Decision (Task 4)

Rule (fixed before looking at the numbers): SPLIT iff `t(16) >= 3.0 x t(1)` or
`t(64) >= 6.0 x t(1)` on the after-parking_lot table; the split is accepted
only if afterwards `t'(16) <= 0.7 x t(16)` and the full suite and the ordering
stress test pass.

- `t(16) / t(1)` = 574.0 / 38.4 = 14.9  (threshold 3.0)
- `t(64) / t(1)` = 2646.7 / 38.4 = 68.9 (threshold 6.0)

VERDICT: SPLIT (attempted in Task 5; reverted, see "after split")

(Task 5 attempts it and applies the acceptance bar; see "after split".)

## Not changed, and why

- **Condvar back-pressure:** moved to `parking_lot::Condvar` in Task 3
  because the `Mutex` moved (they must pair); it simplified the deadline
  arithmetic (`wait_until(&mut guard, Instant)`) and removed poison handling.
- **`Trunk::listen` CAS waiter cap** (a `compare_exchange` loop over
  `waiter_count`): **left as is.** The spec's replacement ("a
  `Semaphore`-equivalent") would be `tokio::sync::Semaphore`, which would make
  this runtime-agnostic crate depend on tokio (the module docs require it to
  work "under any executor or none"); `parking_lot` has no counting
  semaphore; `std` has none; `AtomicUsize::fetch_update` would shorten the
  loop by a few lines without removing it, so it does not clear the "only where
  they simplify" bar. Evidence:
  `grep -n 'Semaphore' ~/.cargo/registry/src/*/parking_lot-0.12.5/src/lib.rs`
  returns nothing.

## after split (attempted, then reverted)

Split tried: `Trunk::samples: Mutex<SampleState>` (`timed`, `sparse`,
`handovers`) and `Trunk::rest: Mutex<RestState>` (`segments`, `events`, `parts`,
`tracks`, `track_generation`), `lock_samples()`/`lock_rest()`, the
`StallIngest` `Condvar` paired with `rest`, no method holding both locks. All
28 `lock_state()` call sites classified cleanly (none touched both groups);
the full media-plane suite and the new ordering guard
(`segment_never_overtakes_its_samples_under_a_split_lock`, 20/20 runs) passed.

| N readers | median ns/publish | low | high |
|---|---|---|---|
| 1  | 54.5   | 53.4   | 55.6   |
| 4  | 188.0  | 169.0  | 205.4  |
| 16 | 1104.1 | 1075.5 | 1128.4 |
| 64 | 5104.7 | 4577.5 | 5690.2 |

Acceptance bar `t'(16) <= 0.7 x t(16)` = 0.7 x 574.0 = 401.8 ns. Measured
t'(16) = 1104.1 ns. **Below the 30 % gain bar: reverted.** A re-run of the
unsplit (`parking_lot`) code immediately after gave 28.0 / 144.6 / 614.1 /
2408.8 ns for N = 1 / 4 / 16 / 64, so the unsplit figure at N = 16 is stable
at roughly 570-615 ns and the split is not a win inside the noise.

Why a split cannot help this benchmark, recorded so the verdict is not
over-read: the benchmark's publisher and all N readers touch only the sample
group (`publish` + `SampleCursor::poll`), so splitting *off* the
segment/event/part group leaves them contending on exactly one lock either
way. The benchmark measures sample-ring fan-out contention; a lock split by
ring group addresses segment/part/event traffic contending with sample
traffic, which this benchmark does not exercise. Relieving sample-ring reader
contention itself would need a different design (per-reader state or a
lock-free ring), which is out of scope for this work.

The reverted diff is kept out of tree; the ordering guard test stays (it also
passes on the single-lock code by construction and will bite any future split).

(Task 5)
