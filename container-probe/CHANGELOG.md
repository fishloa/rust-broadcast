# Changelog

All notable changes to `container-probe` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-08-14

### Added

- **Core detection API** — `probe(&[u8]) -> Probe` and
  `probe_with_budget(&[u8], usize) -> Probe`, where `Probe` is either
  `Identified { format, confidence, detail }`, `Ambiguous { candidates }`,
  `Insufficient { need_at_least }`, or `Unknown`. One-shot over a caller-owned
  byte slice; no IO, no state.

  `need_at_least` is guaranteed to **exceed the bytes the probe examined**,
  which is `min(len, budget)` — not `len`. That distinction is load-bearing:
  `probe` never reads past `DEFAULT_BUDGET` however long the buffer, so a
  caller that only grows the buffer can stall. See the crate root's "The loop
  that terminates" for the loop that does not; `DEFAULT_BUDGET` is public so a
  caller can implement it.

  The guarantee is enforced centrally rather than trusted from each prober,
  after two probers shipped it wrong. Reporting `region.len() + 1` looks like
  strict progress but is not — `region.len()` saturates at the budget, so the
  answer froze at 65537 and any larger buffer got `need_at_least <= supplied`,
  a fixed point a conforming caller spins on forever. Reachable from a 12-byte
  attacker-chosen prefix.

  The answer is also required to **converge**, not merely to advance. Where a
  prober can name a structural need it reports that — the offset the element
  itself declares — and where none can be named the floor grows geometrically
  rather than by a fixed step. An arithmetic step satisfies "more than
  examined" and still crawls: at `+4` bytes a turn (Annex B) a caller needed
  65 536 reads to cross a 256 KiB file, and at `+188` (TS) 1 394. Terminating
  and usable are different properties, and only a bound on the number of reads
  distinguishes them.
- **Scored confidence model** — the `Confidence` tiers `CERTAIN` (240),
  `STRONG` (192), `STRUCTURAL` (160), `LATTICE_STRONG` (128), `LATTICE_WEAK`
  (96) and `HEURISTIC` (64). All probers always run; the highest score wins;
  two candidates within `TIE_THRESHOLD` (16) yield `Ambiguous`, never an
  arbitrary pick.

  Every adjacent pair of tiers is at least 32 apart — strictly more than
  `TIE_THRESHOLD` — so two candidates sitting on *different* tiers can never be
  reported as tied; only a genuine same-tier tie yields `Ambiguous`. That
  spacing is an invariant rather than an accident of the chosen numbers, and
  the `adjacent_tiers_are_further_apart_than_the_tie_threshold` unit test fails
  if a tier is later added or moved so that neighbours can collide.
- **Detected formats** — MPEG-2 TS (188/192/204/208-byte stride lattice),
  ISOBMFF (box-chain walk), Matroska and WebM (EBML magic + `DocType`), MXF
  (partition-pack key + BER length), MPEG-PS (pack header marker bits), FLV,
  WAV, Ogg, ASF (magic signatures), and the elementary streams ADTS AAC, MP3
  and Annex B H.264 (frame/NAL length chaining).
- **Cross-prober suppression** — a container matched at `LATTICE_STRONG` or
  above zeroes every elementary-stream candidate.
- **`Detail`** — prober-specific findings (TS stride/phase, ISOBMFF major brand
  + box count + `IsobmffLayout`, EBML DocType) so a caller need not re-derive
  them.
- **`IsobmffLayout`** (`Fragmented` / `Progressive` / `Unknown`) on
  `Detail::Isobmff` — the discriminator a consumer needs to choose between a
  fragmented demuxer (`moof` movie fragments) and a progressive one (`moov`
  sample tables). The box walk visits every top-level box anyway, so reporting
  what it saw costs nothing and spares the consumer re-walking the chain.

  `Progressive` is claimed **only** when the walk consumed the whole supplied
  buffer, unclipped by the probe budget. Every fragmented file *opens* with a
  `ftyp` + `moov` init segment and reaches its first `moof` later, so a
  truncated prefix of a fragmented file is indistinguishable in shape from a
  progressive one; anything short of a complete walk reports `Unknown` rather
  than guessing. `Fragmented` is definitive on sight, since only a fragmented
  file carries a `moof`.

  The major brand cannot substitute for this: `fixtures/mp4/cmaf/av_frag.mp4`
  is fragmented yet carries the `isom` brand, identical to every progressive
  fixture. A test pins that shared brand so the box walk is not later
  "optimised" into a brand lookup.
- **`no_std` + `alloc`** — the crate builds without default features; runtime
  dependency is `broadcast-common` only.
- **Tests** — real-fixture verdict tests for every format, a whole-repository
  corpus sweep asserting zero false positives and zero ambiguous results, a
  constant drift guard against `mpeg-ts`/`mpeg-ps`/`st377-1`, mutation proofs
  for the detection discriminators that guard a *verdict* (the chain
  thresholds, the `forbidden_zero_bit` check, the ISOBMFF `largesize` width
  decision, and the TS all-sync rejection), and a `fuzz/` target registered by
  the orchestrator. Not every internal guard is mutation-proven: the ran-out
  decision each prober makes is covered by the exhaustive prefix sweep instead.
