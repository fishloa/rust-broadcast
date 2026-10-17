# Differential sweep, 2026-10 release wave

The last published crates versus `main`, over every real capture and fixture the
repo tests use. Purpose: find behavioural differences that no in-repo test pins,
before the wave is tagged (the wave's regressions so far were found by a
downstream user on a real lossy capture, not by CI).

Tool and how to rerun: [`tools/differential-sweep/`](../tools/differential-sweep/README.md).
Acknowledged differences (with reasons): [`tools/differential-sweep/expected.json`](../tools/differential-sweep/expected.json).

Three runs are recorded: **run 3** (HEAD = `f5d6aa430`, the release candidate, after #1327 / #1329 / #1330 / #1332) is the current state and is the first section below; **run 2** (HEAD = `ca638fbaf`, after #1319 / #1322 / #1326) and **run 1** (HEAD = `db41ae3c0`) are kept for comparison.

## Run 3 result (HEAD `f5d6aa430`, final pre-release sweep)

Baseline `a9e9694a6` (transmux 0.26.0 tag commit; re-verified: `transmux/Cargo.toml` there is 0.26.0, matching `docs/release-plan-2026-10.md` "Live now"). Same corpus as run 2 (293 files, private submodule `4d876654`), 1273 (fixture, probe) results per tree, 15 probes, 0 panics, 0 timeouts. `diff.py`: **UNEXPLAINED 0, BUG 0, INTENDED 2998 rows over 210 differing records**, exit 0. Exit 0 is earned by fixes, not by acknowledgement: run 2's two BUG rows were deleted from `expected.json` because the code is fixed, and the one new blanket `multimux.ts_ingest` entry is backed by `crosscheck.py` (below).

| class | records | detail |
|---|---|---|
| identical | 1063 of 1273 | |
| INTENDED | 210 | 75 multimux.ts_ingest (improvements, see BUG-2), 72 cli.dvb-tools, 29 transmux.run, 18 dvb-si.sections, 8 transmux.stream, 6 transmux.batch, 1 dvb-t2mi.pump, 1 cli.ts-fix |
| BUG / UNEXPLAINED | 0 | |
| OWNER-DECISION (no diff row: HEAD equals the baseline) | 17 captures | head-of-line live announcement, see below |

### Run 2 items, one by one

| item | run 2 | run 3 | verdict |
|---|---|---|---|
| BUG-1: `TracksResolved` never emitted on `tsduck/input/test-002.ts` | no `tracks_resolved:*` key | `tracks_resolved:at_finish = 1` (as the baseline); 22 tracks, 10 programmes announced, 2004 of 2004 samples published | **fixed** (#1329 `finish()` removes from `skipped`) |
| BUG-2: ingest drops samples emitted before `TracksResolved` | 45,987 (baseline) / 44,478 (HEAD) samples missing, 75 of 92 captures | **baseline 45,987, HEAD 0**; 0 mismatching tracks, per-track count and SHA-256 equal to `transmux.stream` on all 92 captures | **fixed** (#1327 replay hold, #1330, #1332) |
| `test-092` | 2908 emitted / 16 published | 2908 / 2908 | fixed |
| `france-tnt-dvbt-20s` | 3093 / 2443 | 3093 / 3093 | fixed |
| `h264/baseline.ts` (and `h264/main.ts`) | 15 / 13 | 15 / 15 | fixed |
| `france-tnt-uhf32-si-audio` | 20819 / 0, 0 programmes | 20819 / 20819, 5 programmes (26 tracks) | fixed |
| zenith capture | 69 / 0 | 69 / 69, 1 programme | fixed |
| 15 captures publishing 0 programmes | 15 (17 at baseline) | 0 captures with a shortfall. 17 captures still publish nothing, but `transmux.stream` yields no track on them either (T2-MI / BBFrame / PCR-only / PTS-wrap / tsduck PSI-only captures), so nothing is lost | fixed |
| BUG-2b `eac3_dvb_0x06` | 55 of 58 published | 58 of 58 (baseline 56 of 58); `samples_before_first_resolved` 1 -> 2 is the intended hold-back (kept INTENDED, `transmux.stream` row) | fixed |
| single-track trailing AU loss `mpegh_cicp_01_baseline_32kbps_single` | 29 / 27 | 29 / 29 (`h264/main.ts` 15 / 13 -> 15 / 15) | fixed |
| BUG-3 `france-tnt-uhf32-si-audio` announces 0 of 26 tracks live | 0 of 26 | 0 of 26 (identical to the baseline) | **not fixed, OWNER-DECISION** (documented trade-off, see O-1) |
| O-3 ProgramId nondeterminism on an MPTS | random per process | `multimux.ts_ingest` run twice over the 92 captures: output byte-identical, including the new `head_only_program_order` (announce order + `ProgramId`, e.g. `france-tnt-dvbt-20s`: `ProgramId(0..4)` over tracks `[8..]`, `[14..]`, `[1..]`, `[5..]`, `[12..]`) | **fixed** (#1332) |

Totals (92 captures): samples missing between `transmux.stream` and `multimux.ts_ingest`: baseline 45,987 on 75 captures, HEAD **0**. Captures publishing zero tracks: baseline 34, HEAD 17 (all 17 emit zero tracks upstream).

### NEW difference versus run 2 (a regression of run 2's improvements, not versus the baseline)

**Live announcement is back to the baseline's.** Run 2 reported three captures that announced live at HEAD: `fixtures/dvb-si/tnt-5w-12732v-isi6-10s.ts` (19 of 19 tracks live), `private/fixtures/tsduck/input/test-170.ts` (11 of 11) and zenith (7 of 7). At the run-3 HEAD they announce 0 of 19, 9 of 11 and 5 of 7 live, exactly the baseline (`counters.tracks_resolved:at_finish = 1`, `events[*].phase = at_finish`). Per-track samples and SHA-256 are unchanged and, since the ingest fixes, nothing is lost: the whole programme is simply announced and published at end of input instead of live. Cause (by reading the CHANGELOG; not bisected): #1329 replaced the silent-head criterion by "no payload packet at all from the head, and 2 MiB of other-track backlog" and states "a head that has delivered any data keeps blocking exactly as in 0.26.0"; these captures' stalled heads (video PIDs `0x00DC` / `0x0208` on tnt-5w, abandoned at `finish()` as `config unrecoverable`) do deliver data. Captures announcing at least one track only at `finish()`: baseline 17, HEAD **17** (run 2: HEAD 14). The list is identical to the baseline's: `tnt-5w` (0/19), `france-tnt-pcr` and `france-pcr-discontinuity` (0/6), `m6-single/duplicate/discontinuity` (0/6), `scte35-other-pid` (0/1), `csa2-plain` (0/28), `csa2-scrambled` (0/27), zenith (5/7), `france-tnt-uhf32-si-audio` (0/26), `hotbird-mhp-si` (0/6), `hotbird-mhp-video-head-6MB` (1/13), `test-002` (0/22), `test-012` (0/1), `test-062` (0/15), `test-170` (9/11). No diff row exists (equals the baseline), so this is recorded here and **not** acknowledged in `expected.json`: it is the BUG-3 class (head-of-line blocked live announcement), an **OWNER-DECISION** (the CHANGELOG documents the trade-off in favour of "video stays track 1"). For a downstream user on a live lossy capture it means programme announcement waits for end of input or the 4 MiB probe-backlog cap, exactly as in 0.26.0.

No other NEW difference: `transmux.stream` differs from the baseline on 8 captures (7 INTENDED E-AC-3: `m6-*` and zenith +1 frame on the PID 0x82 track, `gw_*` 236 / 235 -> 157, `eac3_dvb_0x06` `samples_before_first_resolved` 1 -> 2; plus `hotbird-mhp-si` event order), down from 13 in run 2 because the three live-announcement improvements and the `test-002` / `test-062` / `hotbird-mhp-video-head` event reordering are gone. Track ids, PID -> track map, abandoned/removed sets and every per-track SHA-256 outside those are identical to the baseline.

### Other crates (unchanged versus run 2)

* `transmux.run` (every capture, CMAF / CMAF-HLS / TS-HLS / DASH / TS / progressive via `cli::run_bytes`; this probe already covers TS -> HLS/CMAF on every capture, so no new probe was added): 29 differing fixtures, the same INTENDED set as run 2 (E-AC-3 frame, MPEG-H descriptor, subtitles).
* `dvb-si.sections`: 18 fixtures, the same INTENDED identity / typed-descriptor gains. `dvb-t2mi`: only `colombia-capital-t2mi.ts`. `cli.ts-fix`: only `test-184.ts` `--regen-psi` / combo. `cli.dvb-tools`: 72 `dump --json` (INTENDED since #1324). `transmux.batch`: the same 6.
* Zero differences: `mpeg-ts.packets` (92), `scte35.sections` (5), `st291.parse` (2), `media-doctor.findings` (235), `hls.playlist` (26), `dash.mpd` (14), `dvb-si.section_bin` (14).
* RTP: the only RTP fixtures are `st291/anc_rtp.bin` (covered, identical) and a webrtc `.pcap` (not read). No RTP-depacketiser probe was added: no fixture carries a depacketiser input worth probing (a coverage gap, below).

### Tooling added in run 3

* `tools/differential-sweep/crosscheck.py <head.jsonl> [base.jsonl]`: per-track comparison of `multimux.ts_ingest` against `transmux.stream` inside one output; exit 1 on any shortfall. HEAD: 92 captures, 0 mismatching tracks. Baseline: 453 mismatching tracks, 45,987 samples. Run it with every sweep; it is the guard behind the blanket `multimux.ts_ingest` entry in `expected.json`.
* `multimux.ts_ingest` records `head_only_program_order` (announce order + `ProgramId`), ignored by `diff.py`, so ProgramId determinism can be diffed between two HEAD runs.

### Rerun determinism and caveats

Two consecutive HEAD runs of `multimux.ts_ingest` are byte-identical. The probe's `programs` lists show only the tracks of the `NewProgram` event: on `test-016` / `test-059` a second programme is announced empty and its tracks arrive through `TracksChanged` (same in the baseline), so a programme-vs-published comparison shows 7 of 8 / 10 of 16; that is a probe limitation, not a loss (published equals emitted).

## Method and coverage (runs 1 and 2)

* **Baseline** = commit `a9e9694a6` (the newest commit any live `<crate>-v*` / `v11.1.0` tag points at). Re-verified for run 2 against `docs/release-plan-2026-10.md` section 3 "Live now": transmux 0.26.0, dvb-si / dvb-t2mi / dvb-tools 11.1.0, ts-fix 0.6.0, multimux 0.12.0, media-plane 0.6.0, hls-runtime 0.8.0, media-doctor 0.10.0, mpeg-ts 0.5.0 all equal the `Cargo.toml` versions at the baseline commit.
* **Fixtures**: 293 files from `fixtures/` + `private/fixtures/` (same files fed to both trees; submodule pinned at `4d876654`), 277 of which at least one probe applies to. No network fetches (the 4.9 GB `russia-t2mi-scte35` capture is **not** covered).
* **Run 2: 1273 (fixture, probe) results per tree**, 15 probes, 0 panics, 0 timeouts in either tree. Run 2 added the `multimux.ts_ingest` probe (92 TS captures) and, to `transmux.stream`, the counters `tracks_resolved:{pre,at}_finish` and `samples_before_first_resolved`.
* `multimux.ts_ingest` drives multimux's `TsIngestSession` (the shared ingest of `ts_udp` / `ts_http` / `srt`) with 7-packet feeds and records the programmes announced and, per track, the samples it publishes (count, bytes, first / last dts, SHA-256). The programmes are recorded as sorted track-id sets: `ProgramTracker` assigns `ProgramId`s by iterating a `HashMap`, so ids and announce order are random per process on an MPTS (observation O-3).

## Run 2 result (HEAD `ca638fbaf`; BUG-1, BUG-2, BUG-2b and O-3 below are fixed in run 3, see its table)

| class | fixtures x probes | detail |
|---|---|---|
| identical | 1128 of 1273 records | |
| INTENDED (cited in `expected.json`) | 72 records | 29 transmux.run, 18 dvb-si.sections, 13 transmux.stream, 6 transmux.batch, 4 multimux.ts_ingest, 1 dvb-t2mi.pump, 1 ts-fix |
| INTENDED, closed since run 1 | 72 records | dvb-tools `dump --json` (run 1's BUG): now stated in the dvb-tools CHANGELOG (#1324), reclassified INTENDED |
| BUG (red, not acknowledged away) | 2 records | BUG-1 (`test-002` TracksResolved), BUG-2b (`eac3_dvb_0x06` multimux loss) |
| UNEXPLAINED | 0 | |

`diff.py` exits non-zero: two BUG rows remain on purpose.

### What is NEW since run 1 (introduced or revealed by #1319 / #1322 / #1326)

| | fixture | change | class |
|---|---|---|---|
| regression | `private/fixtures/tsduck/input/test-002.ts` | HEAD never emits `TracksResolved` (22 tracks, all announced at `finish()`); the baseline emits it at `finish()` | **BUG-1** |
| regression vs run 1 HEAD (equals the published baseline) | `private/fixtures/ts/france-tnt-uhf32-si-audio.ts` (the downstream user's capture class) | run 1: all 26 tracks announced live; now 0 of 26 live, all at `finish()`. Cause: #1322 restricted the 512-AU head-of-line step-over to non-video heads, and this capture's 4 video PIDs (0x00DC, 0x0208, 0x026C, 0x02D0: 3293 packets, 1547 PES starts each) carry data but never yield an SPS. The CHANGELOG documents the trade-off (video stays track 1), but it re-creates the original failure for exactly this capture | **BUG-3** (design trade-off, owner decision; no diff row because HEAD now equals the baseline) |
| improvement | `fixtures/dvb-si/tnt-5w-12732v-isi6-10s.ts` | 19 tracks: 0 live -> 19 live, `TracksResolved` live | INTENDED |
| improvement | `private/fixtures/tsduck/input/test-170.ts` | 11 tracks: 9 live -> 11 live | INTENDED |
| improvement | `private/fixtures/ts/france-tnt-m6-eac3-zenith.ts` | 5 of 7 live -> 7 of 7 live | INTENDED (already under the zenith entry) |
| event order | `hotbird-mhp-si.ts`, `hotbird-mhp-video-head-6MB.ts`, `test-002.ts`, `test-062.ts` | `finish()` now emits the late promotions before the abandoned events; PID -> track_id map, abandoned set, per-track sample counts and SHA-256 identical | INTENDED |

Disappeared since run 1: the `france-tnt-uhf32-si-audio.ts` row (see BUG-3) and the dvb-tools BUG (closed by #1324). Everything else in run 1's table reproduces unchanged (below).

### Run 2: transmux checks requested for this rerun

* **Live announcement** (tracks announced before `finish()`, 92 TS captures): captures with at least one track announced only at `finish()`: baseline 17, **HEAD 14**. Captures whose `TracksResolved` fires live went 58 -> 60 (tnt-5w, test-170); zenith moved from 5 of 7 to 7 of 7 tracks live; no capture got worse versus the baseline. Still all-late at HEAD (video head with no recoverable SPS in the excerpt, by design): `m6-single/duplicate/discontinuity` (0 of 6), `france-tnt-pcr` and `france-pcr-discontinuity` (0 of 6), `csa2-plain` (0 of 28), `csa2-scrambled` (0 of 27), `hotbird-mhp-si` (0 of 6), `test-002` (0 of 22), `test-062` (0 of 15), `test-012`, `scte35-other-pid`, `france-tnt-uhf32-si-audio` (0 of 26); partly late: `hotbird-mhp-video-head-6MB` (1 of 13).
* **Track ids / order**: for every one of the 92 captures the PID -> (track_id, codec) map and the announce order are identical to the baseline, the abandoned and removed sets are identical, and the video track keeps its baseline track id (checked by script over all `added` events; no `video track N -> M` finding).
* **Per-track samples / SHA-256**: identical to the baseline on every capture except the INTENDED E-AC-3 ones (`m6-*` +1 frame, zenith +1 frame, `eac3-dep/gw_*` 236 -> 157 AUs, #1255).
* **`TracksResolved`**: observable and probed (`counters.tracks_resolved:*`). Baseline: fires only at `finish()` on every capture that has one. HEAD: fires live on 60 captures, at `finish()` on 14, and **never on `test-002`** (BUG-1).
* **dvb-si**: section counts, parse ok/err, descriptor err counts, bad-CRC counts identical on all 92 captures; only the INTENDED identity / typed-descriptor gains (18 fixtures). **dvb-t2mi**: only `colombia-capital-t2mi.ts` PID 0x40 (247 -> 258 ok, 11 -> 0 errors); packet counts, CRC failures and inner-TS SHA-256 identical. **ts-fix**: only `test-184.ts` `--regen-psi`; `--repair-continuity`, `--drop-nulls`, `--restamp-pcr-interpolate` output SHA-256 identical on 102 captures. `mpeg-ts`, `scte35-splice`, `st291`, `media-doctor` (235 records), `broadcast-hls` (26 playlists): zero differences.

### BUG-1: HEAD never emits `TracksResolved` on `test-002.ts` (regression from #1326)

Repro: `transmux.stream` probe on `private/fixtures/tsduck/input/test-002.ts`: baseline `counters.tracks_resolved:at_finish = 1`, HEAD no `tracks_resolved:*` key at all (22 tracks announced at `finish()` in both). Likely cause, by reading `StreamingTsDemux::finish()` (not yet fixed): its abandon loop does `self.resolved.insert(pid)` for a stepped-over PID without `self.skipped.remove(&pid)`, so `resolved.len() + skipped.len() == known` in `maybe_signal_tracks_resolved` can never hold (the PID is counted twice). Consumers that wait on the signal (`multimux`) never announce the programme.

### BUG-2: `multimux` `TsIngestSession` drops samples (pre-existing, unchanged vs baseline; another agent is fixing it)

Measured against `transmux.stream`'s per-track counts on the same 92 captures:

* Samples emitted before the first `TracksResolved` are dropped by `ProgramTracker`'s `track_program` guard. Captures where the signal precedes the data lose only the final access units; captures where it comes late lose almost everything. `france-tnt-dvbt-20s.ts`: 3093 emitted, 2443 published (622 emitted before the signal); `france-tnt-uhf32-video-head-6MB.ts`: 928 -> 266; `test-092.ts`: 2908 -> 16 (2864 before the signal); `h264/baseline.ts`: 15 -> 13.
* **15 captures publish zero samples and zero programmes at HEAD** (17 at the baseline), including the downstream capture `france-tnt-uhf32-si-audio.ts` (20819 samples emitted, 0 published) and `france-tnt-m6-eac3-zenith.ts` (69 -> 0).
* `TsIngestSession::finish()` is `Ok(())` and never calls `StreamingTsDemux::finish()`, so held-back final access units (one-behind video, E-AC-3 / MPEG-H hold-back) are never published: single-track captures with no pre-signal samples still lose 2 (`mpegh_cicp_01_baseline_32kbps_single.ts` 29 -> 27, `h264/main.ts` 15 -> 13).
* Totals: baseline 45,987 samples missing across 75 of 92 captures, HEAD 44,478 across 75 (the HEAD figure drops because tnt-5w and test-170 now resolve live; `gw_*` fewer emitted).
* **BUG-2b (the one diff row)**: `fixtures/ts/dolby/eac3_dvb_0x06.ts` publishes 55 of 58 samples at HEAD (baseline 56): the E-AC-3 track now resolves after 2 AUs instead of 1, so a second sample is lost and `first_dts` moves 5946 -> 7482.

### dvb-tools: closed

Run 1's BUG (`dump --json` grew on 72 of 102 TS fixtures, undocumented) is closed by #1324: `dvb-tools/CHANGELOG.md` `[Unreleased]` now states the schema growth. Reclassified INTENDED in `expected.json`.

### transmux: INTENDED (carried from run 1, unchanged)

| fixture(s) | probe | difference | explained by |
|---|---|---|---|
| `fixtures/ts/m6-single.ts`, `m6-duplicate.ts`, `m6-discontinuity.ts`, `private/.../france-tnt-m6-eac3-zenith.ts` | stream, batch, run | the PID 0x82 E-AC-3 track gains one frame: 7 -> 8 samples, 4096 -> 4608 bytes; every CMAF/DASH/HLS/TS/progressive output of that track changes with it | CHANGELOG "Fixed (E-AC-3 dependent substreams, a real 7.1 stream, #1255)" |
| `private/fixtures/eac3-dep/gw_ffmpeg.ts`, `gw_gpac.ts` | stream, batch, run | 236 / 235 samples, 47 / 43 `timeline reanchored` -> 157 samples, 0 discontinuities | CHANGELOG "Fixed (TS demux, #1186)" + #1255 |
| 8 `private/fixtures/mpegh/*.ts`, 4 `private/fixtures/ts/mpegh-*.ts`, `fixtures/mp4/frag/mpegh_stereo.frag.mp4` | run | TS and TS-HLS output bytes differ | CHANGELOG "Fixed (MPEG-H TS descriptor, H.222.0 Table 2-121)" |
| 9 `fixtures/subtitles/**` mp4s + `fixtures/mp4/stpp.mp4` | run | baseline rejects every packager ("predicate kept no track"); HEAD packages them | CHANGELOG "BREAKING #1186" and "Added (subtitles in manifests, #1243)" |

### dvb-si / dvb-t2mi / ts-fix: INTENDED (carried from run 1, unchanged)

* dvb-si (18 fixtures, 0 parse-result changes): `parse -> serialize` identity gained on `tsduck/input/test-012,016,062,120,170` and `reference/test-109` (CHANGELOG "`section_syntax_indicator` is preserved on every table"); tag 0x3F `Extension_descriptor` typed on the 12 MPEG-H fixtures; tag 0x34 typed on `test-062`.
* dvb-t2mi: `colombia-capital-t2mi.ts` PID 0x40, 11 individual-addressing packets now parse (CHANGELOG "BREAKING (fix): `function_length`").
* ts-fix: `test-184.ts` `--regen-psi`, 5555 -> 5556 packets (CHANGELOG "Fixed (#1288)"). Human look still worthwhile: in both trees the capture's 615 in-position PATs are replaced by nothing / one empty PAT at the very end.

## Observations (not baseline-vs-HEAD differences)

* **O-1. Live announcement is still head-of-line blocked on captures whose video head never resolves**: 14 of 92 captures announce some or all tracks at `finish()` (list above), including the downstream user's `france-tnt-uhf32-si-audio.ts` (BUG-3). A live stream with a video PID that carries data but never an SPS delays every later PID until the 4 MiB probe-backlog cap.
* **O-2.** `transmux.stream` abandons the same PMT-declared PIDs at `finish()` in both trees (e.g. `m6-single.ts`: 0x78, 0x8D, 0xAC `config unrecoverable`), as documented.
* **O-3. `ProgramTracker` ProgramId assignment is nondeterministic on an MPTS**: `resolving` is a `HashMap`, drained with `into_iter()`, so which programme gets `ProgramId(0)` (and the `NewProgram` order) differs between runs of the same input (seen on `france-tnt-dvbt-20s.ts`, 5 programmes). Not a data loss, but anything keyed on `ProgramId` (route names, Trunk ids) is unstable across restarts.

## Run 1 (HEAD `db41ae3c0`), for comparison

1181 results per tree, 14 probes, 0 panics, 0 timeouts. 1047 identical, 62 INTENDED (30 transmux, 18 dvb-si, 1 dvb-t2mi, 1 ts-fix), 72 BUG (dvb-tools `dump --json`), 0 UNEXPLAINED. It reported that `france-tnt-uhf32-si-audio.ts` announced all 26 tracks live at HEAD; run 2 shows that no longer holds (BUG-3). It reported 19 of 92 captures announcing late in both trees (run 2 counts 17 baseline / 14 HEAD by the definition "at least one track announced at `finish()`").

## Coverage gaps (what this sweep cannot see)

* The 4.9 GB `russia-t2mi-scte35` capture and the `.test-streams/` tsduck clone beyond what is mirrored under `private/fixtures/tsduck/`: not fetched (no network).
* TS files above 30 MB are skipped for the `cli.*` subprocess probes; above 100 MB for every probe.
* `multimux.ts_ingest` exercises the session only, not the `IngestDriver` / `Trunk` publication or any transport; network crates (`rtsp-runtime`, `hls-runtime`, `srt-runtime`, `rtmp-runtime`, `webrtc-runtime`, `rist-runtime`), `dvb-ci` / `dvb-ci-runtime`, `ssai-runtime`, `playout-runtime`, `caption-convert`, `ttml-subtitle` have no fixture-driven probe.
* `.pcap` fixtures (rist, st2022, webrtc) are not read.
