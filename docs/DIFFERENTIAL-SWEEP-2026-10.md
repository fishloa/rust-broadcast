# Differential sweep, 2026-10 release wave

The last published crates versus `main` (db41ae3c0), over every real capture and
fixture the repo tests use. Purpose: find behavioural differences that no
in-repo test pins, before the wave is tagged (the wave's regressions so far were
found by a downstream user on a real lossy capture, not by CI).

Tool and how to rerun: [`tools/differential-sweep/`](../tools/differential-sweep/README.md).
Acknowledged differences (with reasons): [`tools/differential-sweep/expected.json`](../tools/differential-sweep/expected.json).

## Method and coverage

* **Baseline** = commit `a9e9694a6` (the newest commit any live `<crate>-v*` / `v11.1.0` tag points at). Every crate's `Cargo.toml` version there equals the "Live now" column of `docs/release-plan-2026-10.md` section 3 (spot-checked: transmux 0.26.0, dvb-si/dvb-t2mi/dvb-tools 11.1.0, ts-fix 0.6.0 (src identical to its tag), hls-runtime 0.8.0, media-doctor 0.10.0, broadcast-common 9.4.0, mpeg-ts 0.5.0, scte35-splice 3.0.1, st291 0.4.1, broadcast-hls 0.3.1).
* **HEAD** = this branch off `main`.
* **Fixtures**: 293 files from `fixtures/` + `private/fixtures/` (same files fed to both trees; submodule pinned at `4d876654`), 277 of which at least one probe applies to; `.ts/.m2ts/.mpg/.mp4/.m4s/.mkv/.webm/.flv/.ps/.mxf/.bin/.m3u8/.mpd`, up to 100 MB. No network fetches (the 4.9 GB `russia-t2mi-scte35` capture is therefore **not** covered).
* **1181 (fixture, probe) results per tree**, 14 probes, **0 panics, 0 timeouts** in either tree. Probe list: `transmux.stream` (StreamingTsDemux, live `pre_finish` vs `at_finish` track announcement, per-track SHA-256), `transmux.batch` (TsDemux), `transmux.run` (container autodetect + CMAF / CMAF-HLS / TS-HLS / DASH / TS / progressive, per segment SHA-256), `dvb-si.sections` (+ `section_bin`), `dvb-t2mi.pump`, `mpeg-ts.packets`, `scte35.sections`, `st291.parse`, `media-doctor.findings`, `hls.playlist`, `dash.mpd`, and subprocess `cli.dvb-tools` / `cli.ts-fix` (102 TS files each).
* Probes adapted between trees (`cfg` split, see the README): PID argument types for dvb-t2mi (`Pid` vs `u16`), PID printing, and two APIs absent from the baseline (`InnerTsRecovery::plps_seen`, `AnyDescriptor::reserialize`): those two outputs are emitted as `head_only_*` keys that the diff ignores.

## Result

| | records (fixture x probe) | fixtures |
|---|---|---|
| identical | 1047 | 192 of 277 fixtures have no difference in any probe |
| INTENDED (explained, cited) | 62 | 30 transmux, 18 dvb-si, 1 dvb-t2mi, 1 ts-fix |
| BUG | 72 | 72 (`dvb-tools dump --json`, one issue) |
| UNEXPLAINED | 0 | 0 |

`diff.py` against `expected.json` exits non-zero (the one BUG class remains).
Zero differences in `mpeg-ts`, `scte35-splice` (5 sections, all re-serialize
identically in both), `st291`, `media-doctor` (235 records, 47 with findings),
`broadcast-hls` (26 playlists, parse + re-render identical) and `transmux.run`
for every non-subtitle, non-E-AC-3, non-MPEG-H container.

## Differences by crate

### dvb-tools: BUG (1 issue, 72 fixtures) - `dump --json` output grew, undocumented

`dvb-tools dump <file> --json` differs on 72 of 102 TS fixtures (e.g.
`fixtures/ts/m6-single.ts`: 9715 -> 9895 bytes, `ts_204_stride_SYNTHETIC.ts`
1372 -> 1402). Every change is an additive JSON key or a retyped descriptor:

* new `reserved_future_use` / `reserved_future_use_1` / `_3` keys (dvb-si now stores the reserved bits);
* new `"trailing": []` on service / short_event / telephone / audio_preselection descriptors (dvb-si BREAKING `trailing` field);
* tag 0x3F bodies move from `"unknown": {tag: 63, body: [...]}` to the typed `Mpeg2Extension` (all MPEG-H captures).

Cause is intended dvb-si work, but `dvb-tools/CHANGELOG.md` `[Unreleased]` says
of `t2mi --pid` "All other text/JSON output is byte-identical" and lists no
`dump --json` schema change. A consumer parsing that JSON is affected.
Repro: `dvb-tools dump fixtures/ts/m6-single.ts --json` on `v11.1.0` vs HEAD,
`diff` shows 3 added keys. **Action (not done here):** add the schema note to the
dvb-tools CHANGELOG and release note and decide whether it needs its own BREAKING line.

### transmux: INTENDED (30 fixtures)

| fixture(s) | probe | difference | explained by |
|---|---|---|---|
| `fixtures/ts/m6-single.ts`, `m6-duplicate.ts`, `m6-discontinuity.ts`, `private/.../france-tnt-m6-eac3-zenith.ts` | stream, batch, run | the PID 0x82 E-AC-3 track gains one frame: 7 -> 8 samples, 4096 -> 4608 bytes, last dts 3956370328 -> 3956371864; every CMAF/DASH/HLS/TS/progressive output of that track changes with it | CHANGELOG "Fixed (E-AC-3 dependent substreams, a real 7.1 stream, #1255)": a `0B 77` pair parsed as an E-AC-3 header with a garbage length used to swallow a real frame |
| `private/fixtures/eac3-dep/gw_ffmpeg.ts`, `gw_gpac.ts` | stream, batch, run | 236 / 235 samples, 505 kB, **47 / 43 `timeline reanchored` discontinuities** -> 157 samples, 723456 bytes, 0 discontinuities (both producers' files now byte-identical output, one SHA-256) | CHANGELOG "Fixed (TS demux, #1186)" + #1255: dependent substreams join their access unit |
| `private/fixtures/ts/france-tnt-uhf32-si-audio.ts` (the downstream user's class of capture: 26 tracks, video + E-AC-3 + data, 16287 continuity gaps) | stream | identical 26 tracks (ids, PIDs, codecs, per-track sample count/bytes/SHA-256 all equal; the same 4 PIDs abandoned) but all announced **live** (`pre_finish`) instead of all at `finish()` | CHANGELOG "Fixed (StreamingTsDemux E-AC-3 promotion regression, zenith)" and the new `PROBE_HEAD_BLOCK_AUS` step-over (absent from the baseline) |
| 8 `private/fixtures/mpegh/*.ts`, 4 `private/fixtures/ts/mpegh-*.ts`, `fixtures/mp4/frag/mpegh_stereo.frag.mp4` | run | TS and TS-HLS output bytes differ (SHA-256) | CHANGELOG "Fixed (MPEG-H TS descriptor, H.222.0 Table 2-121)": complete 6-byte `MPEG-H_3dAudio_descriptor` |
| 9 `fixtures/subtitles/**` mp4s + `fixtures/mp4/stpp.mp4` | run | baseline: every packager returns `select_tracks_by: predicate kept no track`; HEAD packages CMAF/DASH/progressive and (fragmented inputs) HLS; HLS of a progressive-only input now fails with `cannot render an HLS playlist with no segments` | CHANGELOG "BREAKING #1186" (subtitle `sample_entry`) and "Added (subtitles in manifests, #1243)" |

No fixture lost a track, a sample, or a byte anywhere in transmux: every
difference is a gain or a timing improvement. The tool pins the live behaviour
that the in-repo tests did not: the downstream capture class now announces all
26 tracks before `finish()`.

### dvb-si: INTENDED (18 fixtures, 0 parse-result changes)

Section counts, parse ok/err, descriptor err counts and CRC counts are
identical on every fixture. Differences are only:

* `parse -> serialize` identity gained on `tsduck/input/test-012,016,062,120,170` and `reference/test-109` (e.g. test-012 PAT 0 -> 78 identical of 78, test-120 TDT 0 -> 30 identical of 30): CHANGELOG "`section_syntax_indicator` is preserved on every table" and the typed reserved fields;
* tag 0x3F `Extension_descriptor` typed on the 12 MPEG-H fixtures (7-10 per file; CHANGELOG "tag 0x3F used to parse as `Unknown`; auto-dispatched ... `Mpeg2Extension`");
* tag 0x34 `MPEG2_stereoscopic_video_format` typed on `test-062` (new typed descriptors 0x2D-0x3A).

### dvb-t2mi: INTENDED (1 fixture)

`fixtures/dvb-t2mi/colombia-capital-t2mi.ts` PID 0x40: payload errors 11 -> 0,
payload ok 247 -> 258, individual-addressing payloads 0 -> 11. CHANGELOG
"BREAKING (fix): `function_length` is the length of the whole `function()`
including its 2-byte header ... made the real Colombia capture's 0x21 packets
fail to parse". The same bytes parse in both trees except those 11 packets;
CRC failure counts, packet counts and inner-TS SHA-256 are identical. (This is
the "T2-MI function_length framing" regression class: **fixed at HEAD**, and the
sweep confirms it is the only T2-MI change on the corpus.)

### ts-fix: INTENDED (1 fixture)

`private/fixtures/tsduck/input/test-184.ts`, `--regen-psi` (and the combo that
includes it): 5555 -> 5556 packets. The input's PAT lists 5 PMT PIDs that carry
no packets in the capture; the baseline wrote no PAT at all, HEAD writes one
program-less PAT (tsid 4) at end of stream. CHANGELOG "Fixed (#1288)". Worth a
human look (not a diff): in **both** trees the capture's 615 in-position PATs
are replaced by nothing / one empty PAT at the very end, so `--regen-psi` on a
PAT-only excerpt still destroys the valid PAT.

### Identical (no difference on any fixture)

`mpeg-ts` (packet / PID / PCR / CC / resync counts at three chunkings),
`scte35-splice`, `st291`, `media-doctor` (all ten TS diagnostics + ISOBMFF/HLS/MPD
checkers), `broadcast-hls`, `transmux` MPD parse, `dvb-tools` `services` / `epg` /
`pids` / `t2mi`, `ts-fix` `--repair-continuity` / `--drop-nulls` /
`--restamp-pcr-interpolate` (output SHA-256 on 102 captures).

## Observations (not baseline-vs-HEAD differences, same in both trees)

* **Live announcement is head-of-line blocked on short captures.** 19 of the 92 TS captures announce some or all tracks only at `finish()` in *both* trees (`fixtures/ts/m6-single.ts`: 0 of 6 live; `fixtures/france-tnt-pcr.ts`: 0 of 6; `csa2-plain.ts`: 0 of 28; `hotbird-mhp-video-head-6MB.ts`: 1 of 13 live although 12 tracks carry 36-46 samples each). The mechanism is `try_promote_ready`'s PMT-order promotion held at the first still-probing PID (an AVC PID whose SPS never arrives in the excerpt) until `PROBE_HEAD_BLOCK_AUS` (512 access units) or `finish()`. By design and documented, but it is exactly the live-consumer failure class the downstream user hit; a live stream with one permanently silent early PID delays every later PID by 512 AUs of that PID.
* `transmux.stream` abandons the same PMT-declared PIDs at `finish()` in both trees (e.g. `fixtures/ts/m6-single.ts`: 0x78, 0x8D, 0xAC `config unrecoverable`), as documented.

## Coverage gaps (what this sweep could not see)

* The 4.9 GB `russia-t2mi-scte35` capture and the `.test-streams/` tsduck clone beyond what is mirrored under `private/fixtures/tsduck/`: not fetched (no network).
* TS files above 30 MB are skipped for the `cli.*` subprocess probes; above 100 MB for every probe.
* Network crates (`rtsp-runtime`, `hls-runtime`, `multimux`, `srt-runtime`, `rtmp-runtime`, `webrtc-runtime`, `rist-runtime`) and `dvb-ci`/`dvb-ci-runtime`, `ssai-runtime`, `playout-runtime`, `caption-convert`, `ttml-subtitle` have no fixture-driven probe here (the task scoped these as optional); their wave risk is the typed-id churn, caught by the compiler, plus the behaviour changes in their CHANGELOGs, not by this tool.
* `.pcap` fixtures (rist, st2022, webrtc) are not read.
