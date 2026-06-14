# cargo-fuzz harness for rust-dvb

This directory is a standalone cargo workspace (`fuzz/`), excluded from the
parent `rust-dvb` workspace. It runs under nightly (libFuzzer requires it) and
does **not** affect the parent `cargo build --workspace --locked` gate.

## Prerequisites

```bash
cargo install cargo-fuzz   # one-time: provides `cargo fuzz`
rustup install nightly      # libFuzzer needs nightly rustc
```

## Targets

| Target                  | What it fuzzes |
|-------------------------|---------------|
| `si_table_section`      | `dvb_si::tables::AnyTableSection::parse(data)` |
| `si_descriptor_loop`    | `dvb_si::descriptors::parse_loop(data)` |
| `si_demux`              | `dvb_si::demux::SiDemux::builder().build()`, fed 188-byte chunks |
| `si_text`               | `dvb_si::text::decode(data)` and `DvbText::new(data).decode()` |
| `carousel`              | `dvb_si::carousel::ModuleReassembler` fed DSI/DII/DDB parse attempts |
| `t2mi_pump`             | `dvb_t2mi::pump::T2miPump` (TS and raw modes) |
| `bbframe`               | `dvb_bbframe::header::Bbheader::parse` + `up_iter` + `CarryOverExtractor` |
| `roundtrip`             | parse → serialize → re-parse: asserts serialized bytes are idempotent |

## Running

```bash
# From the fuzz/ directory:
cd fuzz

# Run a single target (default: infinite, stop with Ctrl+C):
cargo +nightly fuzz run si_table_section

# Run with a time limit (5 minutes):
cargo +nightly fuzz run si_table_section -- -max_total_time=300

# Run the high-value roundtrip invariant check:
cargo +nightly fuzz run roundtrip -- -max_total_time=600

# Build all targets (compile check, no fuzzing):
cargo +nightly fuzz build
```

## Corpus

Seed corpora live under `fuzz/corpus/<target>/`. They are derived from
real broadcast captures in the workspace fixtures:

- `dvb-si/tests/fixtures/m6-single.ts`
- `dvb-si/tests/fixtures/tnt-5w-12732v-isi6-10s.ts`
- `dvb-t2mi/tests/fixtures/colombia-capital-t2mi.ts`
- `dvb-bbframe/tests/fixtures/rai-5w-12606v-bbframe.ts`
- `dvb-bbframe/tests/fixtures/tnt-5w-12732v-bbframe.ts`

### Re-seeding from fixtures

To add more seeds from a `.ts` fixture, split it into 188-byte packets and drop
them into the target's corpus directory:

```bash
INPUT=../dvb-si/tests/fixtures/m6-single.ts
CORPUS=fuzz/corpus/si_table_section
dd if="$INPUT" bs=188 count=50 2>/dev/null | split -b 188 - "$CORPUS/m6_"
```

Fuzzing also grows the corpus automatically — `cargo fuzz` persists
interesting inputs for future runs.

## Crash workflow

If a target crashes (panic, OOM, assert failure):

```bash
# 1. Reproduce with the crashing input:
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<crash-file>

# 2. Minimise the crashing input:
cargo +nightly fuzz fmt -- <target> <crash-file>
cargo +nightly fuzz tmin <target> fuzz/artifacts/<target>/<crash-file>

# 3. Turn the minimised repro into a unit test in the owning crate.
#    Example: dvb-si/src/tables/pat.rs or a new test fixture file.

# 4. File a GitHub issue with the repro. Do NOT fix the crate in this harness.
```

## Notes

- This harness is a **crash oracle**, not a bug-fix delivery mechanism.
  If a target panics, report it — do **not** touch `../dvb-*/src/`.
- `cargo build --workspace --locked` from the repository root must remain
  unaffected (the `fuzz/` directory is excluded from the workspace).
- `libfuzzer-sys` requires nightly Rust, but the parent workspace stays on
  stable 1.81 MSRV. Use `cargo +nightly fuzz ...` for fuzzing only.
