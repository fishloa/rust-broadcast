# ts-fix

[![crates.io](https://img.shields.io/crates/v/ts-fix.svg)](https://crates.io/crates/ts-fix)
[![docs.rs](https://img.shields.io/docsrs/ts-fix)](https://docs.rs/ts-fix)
[![CI](https://github.com/fishloa/rust-broadcast/actions/workflows/ci.yml/badge.svg)](https://github.com/fishloa/rust-broadcast/actions)

**MPEG-2 Transport Stream conditioning: a streaming engine and CLI that repairs continuity counters, filters PIDs or extracts a service, regenerates the PAT, restamps PCR and handles stuffing.**

## What it does / what it does not

`ts-fix` is container-layer repair: feed 188-byte packets in, get repaired
packets out. Operations are opt-in builder methods, and the engine applies them
in its own canonical order regardless of registration order
(`filter_pids` -> `regen_psi` -> `repair_continuity` -> `restamp_pcr` ->
`honor_pcr_discontinuity` -> `stuffing`). With no operations registered the
engine is an identity pass-through. It is used to clean up captures, cut a
single programme out of a multiplex, and fix PCR timelines before playout, and
it ships both as a library (`TsFix`) and as the `ts-fix` command-line tool.

It does not parse or re-encode codec bitstreams (samples are opaque), does not
descramble, does not remux between containers (see `transmux`), and does not
grade a stream (see `dvb-conformance` / `media-doctor`; `ts-fix` reuses their
TR 101 290 2.3b threshold and legal-duplicate rule rather than re-deriving
them).

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ITU-T H.222.0 / ISO/IEC 13818-1 | H.222.0 v9 (08/2023) (vendored; verified) | §2.4.3.2 TS packet; §2.4.3.3 continuity_counter and legal duplicate packets; §2.4.3.4/§2.4.3.5 adaptation field and PCR; §2.4.3.6/§2.4.3.7 PES header and PTS/DTS (access-unit framing); §2.4.4 PSI (PAT/PMT regeneration); §2.6.16 CA descriptor (ECM/EMM PIDs kept on service extract) | Full for the operations listed | PES is framed, never decoded. |
| ETSI TR 101 290 | V1.4.1 (vendored; verified) | §5.2.2 Table 5.0b indicator 2.3b (PCR discontinuity classification) | Used via `dvb-conformance` | The 100 ms threshold is reused, never re-derived. |

## Feature flags

| Feature | Default | What it enables |
|---|---|---|
| `std` | yes | Forwards `std` to the sibling crates and `thiserror`; off gives `no_std` + `alloc` (the library builds bare-metal) |
| `cli` | yes | The `ts-fix` binary (`clap`); implies `std` |

```toml
[dependencies]
ts-fix = { version = "0.7", default-features = false } # library only
```

## Library quick start

```rust
use ts_fix::{PidFilter, Stuffing, TsFix};

let input = std::fs::read("../fixtures/ts/m6-single.ts").unwrap(); // committed capture
let mut engine = TsFix::builder()
    .repair_continuity()
    .filter_pids(PidFilter::service(1025)) // program_number 0x0401: PAT + PMT + PCR + ES PIDs
    .stuffing(Stuffing::drop_nulls())
    .build()
    .unwrap();

let mut output = Vec::new();
for packet in input.chunks_exact(188) {
    engine.push(packet, |pkt| output.extend_from_slice(pkt)).unwrap();
}
engine.finish(|pkt| output.extend_from_slice(pkt)); // flush buffered state
assert!(!output.is_empty() && output.len() < input.len());
assert!(output.chunks_exact(188).all(|p| p[0] == 0x47));
```

Examples: `cargo run -p ts-fix --example repair_continuity` and
`--example extract_service`.

## Command line

```text
$ ts-fix --help
Reads an input .ts file, applies repair operations, writes the repaired stream.
Operations are applied in the engine's canonical order (filter → psi-regen →
continuity → restamp-pcr/honor-pcr-discontinuity → stuffing) regardless of
flag order.  All operations are opt-in.

Usage: ts-fix [OPTIONS] --input <PATH> --output <PATH>

Options:
  -i, --input <PATH>                  Input TS file
  -o, --output <PATH>                 Output TS file
      --repair-continuity             Renumber per-PID continuity counters to monotonic (mod 16) sequences
      --keep-pids <PID,PID,...>       Comma-separated PIDs to keep (PAT PID 0x0000 always included)
      --service <PROGRAM>             Extract a single programme by program_number (observes PAT/PMT)
      --restamp-pcr-interpolate       Interpolate PCRs between observed anchors (preserves first PCR)
      --restamp-pcr-bitrate <BPS>     Recompute PCRs from a fixed bitrate in bits/sec (e.g. 27000000)
      --honor-pcr-discontinuity       Set discontinuity_indicator on genuine, unflagged PCR breaks (TR 101 290 §5.2.2 2.3b) without rewriting any timestamp
      --regen-psi                     Rebuild PAT from observed PMT PIDs, replacing the PAT in-position as it passes (or emitting one at end-of-stream if the input has no PAT)
      --drop-nulls                    Remove all null packets from the output
      --pad-to <RATE>                 Insert null packets to reach target rate (packets_per_input)
  -h, --help                          Print help (see a summary with '-h')
  -V, --version                       Print version
```

(Condensed from the real `--help`, which prints each option's help on its own
line.) Example invocations on committed fixtures, run from the repository root:

```console
$ ts-fix --input fixtures/ts/m6-single.ts --output out.ts --repair-continuity --drop-nulls
wrote 1264 packets (237632 bytes) to out.ts

$ ts-fix --input fixtures/ts/m6-single.ts --output svc.ts --service 1025
wrote 998 packets (187624 bytes) to svc.ts

$ ts-fix --input fixtures/ts/m6-single.ts --output keep.ts --keep-pids 120,130
wrote 450 packets (84600 bytes) to keep.ts
```

`--keep-pids` takes decimal PIDs (`130` is `0x82`); the PAT is always kept.
`--service` takes the `program_number` (the m6 capture's is `0x0401` = `1025`).

## What is implemented

| Item | Purpose |
|---|---|
| `TsFix` (`builder`, `push(packet, out)`, `finish(out)`), `TsFixBuilder` (`repair_continuity`, `filter_pids`, `regen_psi`, `restamp_pcr`, `honor_pcr_discontinuity`, `stuffing`, `build`) | The engine |
| `PidFilter` (`keep(iter of Pid)`, `service(program_number)`) | PID filter or live PAT/PMT service extract (PAT, PMT, PCR, ES, ECM, CAT and EMM PIDs retained) |
| `PcrRestamp` (`interpolate()`, `from_bitrate(bps)`) | PCR restamp onto one continuous timeline, including across an unflagged break; modular 33-bit/27 MHz arithmetic from `broadcast_common::clock33` |
| `Stuffing` (`drop_nulls()`, `pad_to(packets_per_input)`) | Drop null packets or pad to a rate |
| `discontinuity::{detect_pcr_discontinuities, PcrDiscontinuity}` | Standalone read-only audit of PCR breaks (flagged vs unflagged) |
| `pes::{reconstruct_access_units, AccessUnit}` | PES access-unit framing with PTS/DTS (opaque bytes); an input that is not a multiple of 188 bytes is `Error::UnalignedInput` |
| `Error` (`ShortPacket`, `NoSyncByte`, `UnalignedInput`, ...) | Structured errors |

## Design notes

- **Forward-compatible by construction.** There is no public `Operation` enum or
  trait; operations are builder methods only, and every configuration and error
  enum is `#[non_exhaustive]`, so a new operation is a purely additive change.
- **Canonical order is enforced** by `build()`, not by the caller.
- **Reused judgements.** Legal duplicate packets are `broadcast_common::ts_dup`
  (same rule as `dvb-conformance` and `media-doctor`, issue #956); PCR wrap
  arithmetic is `clock33` (`tests/clock33_guard.rs` bans a local 2^33).
- **`--regen-psi` after a filter that drops every PMT** (e.g. `--keep-pids`
  of ES PIDs only) emits a program-less PAT at end of stream rather than no PAT
  (H.222.0 Table 2-30: the program loop may be empty); it never lists a PMT PID
  absent from the output (#1288).
- **Verified against an oracle.** `tests/it/tsduck_oracle.rs` checks `--regen-psi`
  output against TSDuck as an independent second opinion; `tests/it/scte35_preserve.rs` pins that splice signalling survives.
- The library has no `std` path: with `default-features = false` it is
  `no_std` + `alloc` (a CI no_std lane proves it).

## Related crates and license

Depends on `broadcast-common`, `mpeg-ts`, `mpeg-pes`, `dvb-si`,
`dvb-conformance`. Independently versioned. `dvb-tools pids` is a handy way to
inspect the result. MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
