# differential-sweep

Runs the **last published** crates and **HEAD** over every real capture/fixture
the repo's tests use and reports every behavioural difference, so each one is
explained (intended, spec-driven) or fixed *before* a release. Built after the
2026-10 wave shipped regressions a downstream user found on a real lossy capture
(`StreamingTsDemux` dropped E-AC-3/subtitle tracks; T2-MI `function_length`
framing failed) that the in-repo tests did not.

Not a workspace member (own `[workspace]`, own `Cargo.lock`, `publish = false`).
Triage of the 2026-10 runs (run 1 and the rerun): [`docs/DIFFERENTIAL-SWEEP-2026-10.md`](../../docs/DIFFERENTIAL-SWEEP-2026-10.md).

## How it works

One source file set (`src/main.rs`, `src/probes.rs`) is compiled twice:

| build | where | feature |
|---|---|---|
| `sweep-baseline` | a checkout of the commit the live `<crate>-v*` / `v11.1.0` tags point at | `--features baseline` |
| `sweep-head` | the tree under test | `--features head` |

Each prints one JSON line per `(fixture, probe)` to stdout. `diff.py` flattens
both outputs to dotted leaf paths, classifies every difference and exits
non-zero while any `UNEXPLAINED` or `BUG` difference remains.

Fixtures: everything under `fixtures/` and `private/fixtures/` (the same two
trees `test_fixtures` resolves), extensions `ts m2ts mpg mp4 m4s mkv webm flv ps
mxf bin m3u8 mpd`, files up to `--max-mb` (default 100). Nothing is fetched from
the network. Both binaries read the SAME fixture files (`--root` = the tree
under test), so a difference is the code, never the corpus.

Every probe runs in its own thread under `catch_unwind` and a per-probe timeout
(`--timeout`, default 300 s): a panic or timeout is itself a result.

## Probes

| probe | crate(s) | records (per fixture) |
|---|---|---|
| `transmux.stream` | transmux | `StreamingTsDemux` fed 188 B packets, drained after every feed: ordered `TrackAdded` (id, PID, program, codec identity + RFC 6381), `TrackRemoved`/`TrackAbandoned`, **phase = `pre_finish` vs `at_finish`**, per-track sample count/bytes/dts range/SHA-256, event counters, `TracksResolved` per phase (`tracks_resolved:*`) and `samples_before_first_resolved` |
| `multimux.ts_ingest` | multimux, media-plane | `TsIngestSession` (the shared ts_udp/ts_http/srt ingest) fed 7-packet datagrams: programmes announced (sorted track-id sets: `ProgramId`s are `HashMap`-order random), per-track published samples/bytes/dts/SHA-256. Compare with `transmux.stream`: any shortfall is a sample the session dropped |
| `transmux.batch` | transmux | `TsDemux::demux` tracks + sample SHA-256, PCR count |
| `transmux.run` | transmux | container autodetect + `cli::run_bytes` to CMAF, CMAF-HLS, TS-HLS, DASH, TS, progressive MP4: size + SHA-256 per output, per segment name |
| `dvb-si.sections` | dvb-si | `SiDemux` per `(PID, table_id)`: sections, parse ok/err, typed, `parse -> serialize` identity/mismatch, bad CRC; descriptor typed/unknown/err per tag |
| `dvb-si.section_bin` | dvb-si | standalone section `.bin` fixtures through `AnyTableSection` |
| `dvb-t2mi.pump` | dvb-t2mi | every PID with T2-MI-looking traffic: packet counts, per-type counts, parse errors, individual addressing, `InnerTsRecovery` output SHA-256 |
| `mpeg-ts.packets` | mpeg-ts | packet/PID/PCR/CC counts, `TsResync` counts at three chunkings |
| `scte35.sections` | scte35-splice | every SCTE-35 section (PMT 0x86 PIDs, or a `.bin`): parse ok/err, re-serialize identity |
| `st291.parse` | st291 | `anc.bin`, `anc_rtp.bin` |
| `media-doctor.findings` | media-doctor | the ten TS diagnostics over TS files; ISOBMFF/HLS/MPD checkers: findings by rule + SHA-256 of the list |
| `hls.playlist` / `dash.mpd` | broadcast-hls, transmux | `.m3u8` parse + re-render identity, `.mpd` parse |
| `cli.dvb-tools` / `cli.ts-fix` | dvb-tools, ts-fix | only with `--bin-dir`: subprocess runs (`dump --json`, `services`, `epg`, `pids`, `t2mi`; `--repair-continuity`, `--regen-psi`, `--drop-nulls`, combo, `--restamp-pcr-interpolate`) with exit code + stdout/output SHA-256, for TS files up to 30 MB |

### API adaptations between the trees

Only these call sites are `cfg`-split (everything else is one source):

* `dvb_t2mi` PID arguments: HEAD takes `mpeg_ts::pid::Pid`, the baseline a `u16` (`mkpid`).
* `PidRaw`: `u16` (baseline `TrackSpec::source_pid`, `TsHeader::pid`, ...) vs `Pid` (HEAD) both print as the same 4-digit hex.
* `InnerTsRecovery::plps_seen` and `AnyDescriptor::reserialize` do not exist in the baseline: PLP lists are empty there, and the descriptor re-serialize counts are emitted as `head_only_*` keys, which `diff.py` ignores.

## Rerun

```bash
SCR=/tmp/sweep && mkdir -p $SCR
# 1. baseline checkout (the commit the live tags point at)
git worktree add --detach $SCR/baseline a9e9694a6        # newest base tag; check `git tag --contains`
cp -R tools/differential-sweep $SCR/baseline/tools/
cp $SCR/baseline/Cargo.lock $SCR/baseline/tools/differential-sweep/Cargo.lock
# (private fixtures: the sweep reads them from the tree under test via --root)

# 2. build both (offline: the lock files pin what is already in the cargo cache)
cargo build --release --offline --features baseline \
    --manifest-path $SCR/baseline/tools/differential-sweep/Cargo.toml --target-dir $SCR/t-base
cargo build --release --offline --features head \
    --manifest-path tools/differential-sweep/Cargo.toml --target-dir $SCR/t-head
# optional CLI probes: build the binaries in each tree
cargo build --release --locked -p ts-fix -p dvb-tools --target-dir $SCR/b-head
cargo build --release --locked -p ts-fix -p dvb-tools \
    --manifest-path $SCR/baseline/Cargo.toml --target-dir $SCR/b-base

# 3. run (SAME --root for both) and diff
$SCR/t-base/release/differential-sweep --root "$PWD" --bin-dir $SCR/b-base/release > $SCR/base.jsonl
$SCR/t-head/release/differential-sweep --root "$PWD" --bin-dir $SCR/b-head/release > $SCR/head.jsonl
python3 tools/differential-sweep/diff.py $SCR/base.jsonl $SCR/head.jsonl \
    --expected tools/differential-sweep/expected.json --json $SCR/diff.json
git worktree remove --force $SCR/baseline
```

`--only <probe-prefix>` restricts to matching probes (`--only transmux.stream`).

## Acknowledging a difference: `expected.json`

A list of `{"probe", "fixture", "path", "class", "reason"}` objects (`fnmatch`
globs, `reason` required). A matching difference becomes `INTENDED` (cite the
CHANGELOG `[Unreleased]` BREAKING line or the spec page); `"class": "BUG"`
records a triaged-but-unfixed defect and still fails the run. Anything not
matched is `UNEXPLAINED` and fails the run.

`python3 -m pytest tools/tests/test_differential_sweep_diff.py` covers `diff.py`.
