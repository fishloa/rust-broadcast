# rust-broadcast CLI standard

Every command-line tool in the workspace follows this so the tools feel like one
suite. New CLIs and new subcommands MUST conform.

## Parser

Use **`clap` (derive API)**, `features = ["derive"]`. No hand-rolled
`std::env::args` parsing. `clap` gives `--help`/`-h` and `--version`/`-V`,
named flags, validation, and consistent error messages for free.

- A binary crate depends on `clap` directly.
- A binary gated behind a feature (e.g. `ci-probe` under `linux`) takes `clap`
  as an **optional** dep enabled by that feature, so the default build stays lean.
- Pin a `clap` version that builds on the workspace MSRV (1.86); `--locked` is
  authoritative.

## Shape

- **Subcommands** are kebab-case verbs (`list`, `info`, `descramble`). Define them
  as a `#[derive(Subcommand)]` enum.
- The top-level `#[command(...)]` sets `version`, `about` (one line), and
  `long_about` where useful. Every subcommand and every field gets a doc comment —
  clap turns it into `--help` text, so write it for the reader.
- **No bare positional magic numbers.** Anything a human would have to remember
  the order of is a named flag. A single obvious input (an input file) may be a
  positional `<FILE>`; everything else is named.

## Conventions

| Concern | Flag | Notes |
|---|---|---|
| DVB adapter | `-a, --adapter <N>` | default `0` |
| CA slot device | `-c, --ca <N>` | default `0` |
| CI data device | `--ci <N>` | default `0` |
| TS PID | `--pid <PID>` | accept `0x` hex and decimal |
| T2-MI PLP | `--plp <N>` | |
| Machine output | `--json` | JSON to stdout instead of human text |
| Diagnostics dump | `--trace` | extra wire/diagnostic output to stderr |
| Input file | positional `<FILE>` | when there is exactly one obvious input |

- Hex-or-decimal numeric args parse `0x`-prefixed and plain decimal.
- Human output to **stdout**; diagnostics/traces/errors to **stderr**.
- Exit `0` on success, non-zero on error (return `Result` from `main`/`run`).
- `--json`, where offered, emits valid JSON and nothing else on stdout.

## Example (`ci-probe`)

```text
ci-probe info --adapter 3 --ca 0 --trace
ci-probe descramble --adapter 3 --ca 0 --pmt service.bin
ci-probe --help          # auto-generated, lists subcommands
ci-probe info --help     # auto-generated, lists this command's flags
```
