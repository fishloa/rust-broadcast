# Benchmark Results

One local run on **Apple M4 Max (macOS 26.0 Darwin 25.5.0), aarch64**, rustc stable
(`cargo bench --workspace`).  Numbers are Criterion `thrpt` / `time` medians;
use `cargo bench` locally to reproduce.  Only the `[bench]` profile runs here —
CI runs `cargo bench -- --test` (compile + smoke only, no timing).

---

## dvb-common — CRC-32 MPEG-2

| Payload   | Throughput     | Time       |
|-----------|----------------|------------|
| 188 B     | 435 MiB/s      | 412 ns     |
| 4 096 B   | 359 MiB/s      | 10.9 µs    |
| 65 536 B  | 362 MiB/s      | 173 µs     |

---

## dvb-si — Hot paths

### `SiDemux::feed` — fixture throughput

> **Headline: 59.9 Gbit/s on m6-single.ts · 20.5 Gbit/s on tnt-10s.ts**
>
> These are the rates at which `SiDemux::feed` can process TS bytes; broadcast
> streams are typically 1–50 Mbit/s, so the demux is not the bottleneck.

| Fixture                             | Size      | Time      | Throughput   |
|-------------------------------------|-----------|-----------|--------------|
| `m6-single.ts`  (1 260 pkts)        | 232 KiB   | 31.7 µs   | 6.97 GiB/s   |
| `tnt-5w-12732v-isi6-10s.ts` (13 515 pkts) | 2.42 MiB  | 991 µs    | 2.39 GiB/s   |

### `AnyTableSection::parse` dispatch

| Corpus              | Total bytes | Time    | Throughput  |
|---------------------|-------------|---------|-------------|
| m6 (200 sections)   | ~7 KiB      | 2.35 µs | 1.03 GiB/s  |

### `parse_loop` — descriptor-loop walk

| Scenario                              | Bytes | Time    | Throughput  |
|---------------------------------------|-------|---------|-------------|
| 10× ShortEvent + 10× Unknown (130 B)  | 130 B | 199 ns  | 622 MiB/s   |

### `DvbText::decode`

| Input variant         | Time    |
|-----------------------|---------|
| ASCII (no selector)   | 10.4 ns |
| UTF-8 (0x15 selector) | 92.5 ns |
| ISO 6937 combining    | 162 ns  |

---

## dvb-t2mi — `T2miPump::feed_ts`

| Fixture                    | Size     | Time    | Throughput   |
|----------------------------|----------|---------|--------------|
| `colombia-capital-t2mi.ts` | 1.08 MiB | 2.98 ms | 360 MiB/s    |

---

## dvb-bbframe — `BbframePump::feed` and `up_iter`

| Benchmark                   | Size / desc                     | Time    | Throughput   |
|-----------------------------|---------------------------------|---------|--------------|
| `BbframePump::feed` (TNT)   | all NM BBFrames from tnt fixture | 53.9 µs | 16.9 GiB/s   |
| `up_iter` (8 UPs × 10 iter) | synthetic, 15 040 B / call      | 2.14 µs | 6.54 GiB/s   |
| `NmTsIter` direct           | same corpus, no header parse    | 2.24 µs | 6.27 GiB/s   |
