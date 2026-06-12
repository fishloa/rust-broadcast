# Story 49 — cargo-fuzz harness

GitHub issue **#49**. Add a `cargo fuzz` harness with one target per parser entry
point + a round-trip property target. **Deliverable is the harness + seed corpora
+ README — NOT bug fixes.** If a target finds a crash, do NOT fix the crate; note
it (the orchestrator files a separate issue). Run the gate check below; do not commit.

## Hard constraint: must not perturb the published workspace
- `fuzz/` is its **own** crate, **excluded from the root workspace**. Add
  `exclude = ["fuzz"]` to the root `Cargo.toml` `[workspace]` table.
- `fuzz/Cargo.toml`: `[package]` (edition 2021), `[package.metadata]` cargo-fuzz
  marker, path deps on the workspace crates with their needed features
  (`dvb-si` with `ts`, `dvb-t2mi` with `ts`, `dvb-bbframe`), `libfuzzer-sys`, and
  `arbitrary` if useful. Add `[workspace]` (empty) to `fuzz/Cargo.toml` so it is a
  standalone workspace and never pulled into the parent.
- **Acceptance:** `cargo build --workspace --locked` and the full parent gate suite
  remain byte-for-byte unaffected (the fuzz dir is invisible to them). libfuzzer
  needs nightly — fine, since fuzz is isolated; the parent stays on MSRV 1.75.

## Targets (`fuzz/fuzz_targets/<name>.rs`, each `fuzz_target!`)
Use the real public APIs (verified names):
- `si_table_section` — `dvb_si::tables::AnyTableSection::parse(data)` (ignore Err).
- `si_descriptor_loop` — walk `dvb_si::descriptors` `parse_loop` / `AnyDescriptor` over `data`.
- `si_demux` — `dvb_si::demux::SiDemux::builder().build()`, feed `data` in 188-byte chunks via `.feed()`, drain events (stateful).
- `si_text` — `dvb_si::text::decode(data)` and `DvbText`.
- `carousel` — `dvb_si::carousel::ModuleReassembler` fed fuzzed DSI/DII/DDB (note audit issues #42/#43 live here).
- `t2mi_pump` — `dvb_t2mi::…::T2miPump::new(pid).feed_ts(chunk)` over 188-byte chunks; also `feed_raw`.
- `bbframe` — `dvb_bbframe::header::Bbheader::parse(data)`; on Ok, `dvb_bbframe::packet::up_iter(&data[BBHEADER_LEN..], &hdr)` drained; and `CarryOverExtractor`.
- **`roundtrip`** (highest value) — for each layer where `parse(data)` succeeds, `serialize` it and assert `parse(serialized) == original` (the project's hard invariant). Cover `Bbheader` and at least one `dvb-si` table; a panic/inequality is a finding.

Each target: `#![no_main]`, take `data: &[u8]`, never `unwrap()` on the fuzzed parse (you're testing it doesn't panic). Keep them small.

## Seed corpora
Add `fuzz/corpus/<target>/` seeds derived from the real fixtures:
`dvb-si/tests/fixtures/m6-single.ts`, `tnt-5w-12732v-isi6-10s.ts`,
`dvb-t2mi/tests/fixtures/colombia-capital-t2mi.ts`. A small shell snippet in the
README that `split`s a `.ts` into 188-byte packets into the corpus dir is enough;
commit a handful of representative seeds (do not commit megabytes).

## `fuzz/README.md`
- How to run each target: `cargo +nightly fuzz run <target> -- -max_total_time=300`.
- Where corpora live; how to (re)seed from fixtures.
- Crash workflow: `cargo +nightly fuzz fmt`/`tmin` to minimize → turn the repro
  into a unit test in the owning crate → file an issue → then fix. (Harness here
  does NOT fix crates.)

## CI
**Do not touch `.github/workflows/`** (the required gates must stay as-is).
Document manual/`cargo fuzz` invocation in the README only.

## Verify before finishing
```
cargo build --workspace --locked                      # MUST be unaffected
cargo test  --workspace --all-features --locked        # MUST be unaffected
cargo fmt --all --check                                # parent fmt unaffected
cargo +nightly fuzz build                              # all targets compile (if cargo-fuzz present)
```
If `cargo fuzz` is not installed, state that the targets are written but unbuilt;
do not block on it.

## Boundaries
Create `fuzz/**`; edit root `Cargo.toml` (`exclude`) only. Do **not** edit any
crate `src/`. Do **not** add a CI workflow. Do **not** commit.
