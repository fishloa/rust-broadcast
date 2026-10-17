# ts-fix 0.7.0

_Released 2026-10-16._

**Breaking release (typed PIDs, two behavioural changes, and the `broadcast-common` 10 epoch).** Four themes: (1) PCR restamping fixes: `PcrRestamp::Interpolate` kept extrapolating from the pre-wrap rate forever after a legal 33-bit PCR wrap, and its extrapolation rate aliased by a whole ~26.5 h PCR wrap on very long runs; (2) `--regen-psi` fixes: a filter that dropped every PMT (for example `--keep-pids 120,130 --regen-psi`) used to emit no PAT at all, `TsFix::finish` discarded the flush output of the last op in a multi-op pipeline, and the original PAT is now reassembled across TS packets before its `transport_stream_id`/`network_PID` hints are read; (3) PID parameters are `mpeg_ts::pid::Pid`, and `--keep-pids` rejects a PID above `0x1FFF`; (4) `pes::reconstruct_access_units` returns a `Result` and reports unaligned input as an error instead of panicking. The README is rewritten to the workspace standard and is now the crate-root documentation.

## Why this version

- **Proposed version:** 0.6.0 -> **0.7.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (PidFilter/AccessUnit take Pid).
- **cargo-semver-checks vs live 0.6.0 (all features):** MAJOR (1 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, dvb-conformance, dvb-si, mpeg-pes, mpeg-ts, scte35-splice.
- **Caret epoch:** 0.6 -> 0.7; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-dvb-conformance = "11.1"
+dvb-conformance = "12.0"
-dvb-si = "11.1"
+dvb-si = "12.0"
-mpeg-pes = "0.5"
+mpeg-pes = "0.6"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
-scte35-splice = "3.0" # dev-dependency
+scte35-splice = "4.0" # dev-dependency
```

## Breaking changes and migration

Typed PIDs (#1159). `Pid::new` masks to 13 bits; use `Pid::try_new` for user-supplied numbers:

```rust
use mpeg_ts::pid::Pid;
// before
PidFilter::keep([0x100u16, 0x101]);   reconstruct_access_units(ts, &[0x100]);
// after
PidFilter::keep([Pid::new(0x100), Pid::new(0x101)]);   reconstruct_access_units(ts, &[Pid::new(0x100)])?;
```

`AccessUnit::pid` and `PcrDiscontinuity::pid` are `Pid` (use `.value()` for the raw `u16`); `AccessUnit` is `#[non_exhaustive]`, so construct it with `AccessUnit::new(pid, pts, dts, data)` rather than a struct literal. CLI: `--keep-pids` now rejects a PID above `0x1FFF` (it used to be accepted and never matched).

`reconstruct_access_units` now returns `Result<Vec<AccessUnit>, Error>`:

```rust
// before
let aus = reconstruct_access_units(&ts, &pids);            // panicked (assert_eq!) on len % 188 != 0
// after
let aus = reconstruct_access_units(&ts, &pids)?;           // Err(Error::UnalignedInput { len })
```

Behavioural (no signature change): a `--regen-psi` pipeline whose filter dropped every PMT now emits, at end of stream, a program-less PAT carrying the original `transport_stream_id` and any `network_PID` (ITU-T H.222.0 Table 2-30 leaves the program loop unconstrained); output that previously had no PAT now has one. When the original PAT never parses, one PAT with `transport_stream_id` 0 is emitted at end of stream, and only if programs were discovered; a PMT that began but never reassembled is a parse failure, not "no programs", so no program-less PAT is emitted in that case. Golden outputs of such pipelines will change.

Finally, move every `broadcast-common` requirement to `10` (and `mpeg-ts`/`mpeg-pes` to `0.6`, `dvb-si`/`dvb-conformance` to `12`) so one copy of each resolves.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `dvb-conformance` 11 -> 12, `dvb-si` 11 -> 12, `mpeg-pes` 0.5 -> 0.6, `mpeg-ts` 0.5 -> 0.6, `scte35-splice` 3 -> 4). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Fixed (audit 2026-10, `--regen-psi`)
- The original PAT is now reassembled across TS packets (`SectionReassembler`) before its `transport_stream_id` / `network_PID` hints are read. Previously only a single packet was parsed, so a multi-packet PAT yielded no hints and the regenerated PAT silently got `transport_stream_id` 0 and lost its `network_PID`.
- `transport_stream_id` is no longer invented in position: while PAT slots exist but the original has not parsed, no PAT is emitted in position; if it never parses, one PAT (tsid 0, documented) is emitted at end of stream, and only if programs were discovered. A program-less PAT with an unknown tsid is never emitted.
- A PMT that began but never reassembled/parsed is treated as a parse failure, not "no programs": the flush-time program-less PAT is no longer emitted in that case.
- Docs: `current_entries`/`rebuild_pat` no longer claim "Returns None if no programs" unconditionally.

#### Fixed (#1288)
- `--regen-psi` after a filter that dropped every PMT (e.g. `--keep-pids 120,130 --regen-psi`) emitted no PAT at all. Two causes: (1) the regen op built no PAT when no PMT was seen; it now emits, at end of stream, a program-less PAT (ITU-T H.222.0 (08/2023) Table 2-30, PDF p. 76 (printed p. 54; verified on the rendered page): the program loop `N` is unconstrained, `network_PID` optional) carrying the original `transport_stream_id` and any `network_PID`, and never lists a PMT PID absent from the output; (2) `TsFix::finish` discarded the flush output of the *last* op in a multi-op pipeline, so any trailing `regen_psi` flush (also the late-PMT fallback PAT) was lost. **BREAKING (behavioural):** output of such pipelines now contains a PAT it previously lacked. Tests: `tests/psi_regen.rs` (`keep_pids_dropping_every_pmt_*`, `two_program_keep_*`).

#### Fixed (adversarial review of signalling/captions)
- `PcrRestamp::Interpolate`: the extrapolation rate (used after a corrupt or frozen observation) was the *modular* forward distance from the PID's first anchor to its last observation divided by the packets between them, which aliases by a whole PCR wrap (~26.5 h) once a clean run outgrows one wrap; the anchor-to-observation span is now a running unwrapped total, so it holds for arbitrarily long runs. The in-crate test drives the op with a 1M-packet 99 ms-step run in one reused buffer and fails on the old code.
- **BREAKING (behavioural):** `pes::reconstruct_access_units` returns `Result<Vec<AccessUnit>, Error>` and reports a buffer that is not a multiple of 188 bytes as the new `Error::UnalignedInput { len }` instead of panicking (`assert_eq!`).

#### Fixed
- `PcrRestamp::Interpolate` stopped adopting observed PCRs after a legal 33-bit PCR wrap: the forward distance was computed as `obs.wrapping_sub(last) % (2^33 * 300)`, which is garbage whenever `obs < last` (2^64 is not a multiple of the non-power-of-two PCR modulus), so every post-wrap observation was discarded and the output was extrapolated from the pre-wrap rate forever. The constant-rate `pcr_wrap` fixture passed only by coincidence; `tests/pcr_wrap_observed_after_wrap.rs` (post-wrap rate change) fails on the old code and passes now. Both distance computations use `broadcast_common::clock33::pcr_forward_distance` (#1206).

#### Changed (BREAKING)

- **BREAKING (#1159):** `PidFilter::keep` takes `IntoIterator<Item = Pid>`; `reconstruct_access_units(ts, &[Pid])`, `AccessUnit::pid` and `PcrDiscontinuity::pid` are `Pid`; `AccessUnit` is now `#[non_exhaustive]` with a new `AccessUnit::new` constructor. CLI: `--keep-pids` now rejects a PID above `0x1FFF` (was silently accepted and never matched).

## Merged changes since `ts-fix-v0.6.0`

- fix: pre-release audit findings in dvb-ci-runtime and ts-fix (#1306)
- fix(ts-fix): --regen-psi after a PMT-dropping filter keeps a PAT (#1288) (#1305)
- docs: cluster A crate READMEs to the crate-README standard (what/not, standards table, features, doctested quick start, API, design notes) (#1289)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- chore: pre-release hygiene (README coverage, snippet tool, docsrs, examples, CLAUDE.md) (#1278)
- test: gate binary-spawning tests on the cli feature (required-features) (#1258)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Audit critical-bug verification pass: 72 issues re-verified against main; fixes for #1012 #1022 #1034 #1035 #1056 + regression tests (NO release) (#1180)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
- Audit systemic wave 1: reserved-bit spec cleanup, push mux, T9 clock33, rtcp profile extensions (release set: lockstep 11.1.0 + cascade) (#1173)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
