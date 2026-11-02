# Release runbook - the 2026-11b wave

The ordered tag list for the wave prepared in the `release/2026-11b` PR. The pair list is
[`release-plan-2026-11b.pairs`](release-plan-2026-11b.pairs): 18 crates, 14 tags (the lockstep
`v12.1.0` plus 13 independent tags). Nothing here runs until the owner gives the tag go. Never
`cargo publish` from a workstation; CI is the only publisher. The general procedure (private
fixtures, gates, package check, post-publish verification) is
[`RELEASE-RUNBOOK-2026-10.md`](RELEASE-RUNBOOK-2026-10.md) sections 0-6; this file only fixes
what differs for this wave.

```bash
PAIRS=docs/release-plan-2026-11b.pairs
```

## What the wave contains

| Class | Crates |
|---|---|
| Lockstep minor (12.0.1 -> 12.1.0) | `dvb-si`, `dvb-t2mi`, `dvb-bbframe`, `dvb-conformance`, `dvb-tools` |
| 0.x breaking (minor) | `srt-runtime` 0.7.0, `transmux` 0.29.0, `broadcast-client` 0.2.0 |
| 0.x additive (patch) | `cc-data` 0.7.1, `rtmp-runtime` 0.8.1 |
| 0.x additive, epoch-forced to a minor | `hls-runtime` 0.11.0, `multimux` 0.15.0 |
| Epoch-forced only (0.x minor) | `media-doctor` 0.13.0, `media-plane` 0.9.0, `dash-runtime` 0.2.0, `tshttp-runtime` 0.2.0, `compliance-probe` 0.6.0, `multimux-cli` 0.13.0 |

Minimum-version requirements that matter at publish time: `transmux` requires `dvb-si` 12.1
(batch 1 must be live first); `broadcast-client` requires `srt-runtime` 0.7, `rtmp-runtime` 0.8,
`transmux` 0.29, `hls-runtime` 0.11, `dash-runtime` 0.2 and `tshttp-runtime` 0.2; `multimux`
requires all of the above plus `media-plane` 0.9; `multimux-cli` requires `multimux` 0.15;
`compliance-probe` requires `dvb-conformance` 12.1, `media-doctor` 0.13 and `media-plane` 0.9.

## Tag order

Push one tag at a time (`tools/push-release-tags.sh --plan $PAIRS`, which waits for the
crates.io index and the lane to finish before the next). Batches of at most three; wait for every
tag of a batch to be live before the next batch. The lockstep tag is pushed by hand and
approved in the `crates-io` environment.

| Batch | Tags (in this order) |
|---|---|
| 1 | `cc-data-v0.7.1`, `srt-runtime-v0.7.0`, `v12.1.0` |
| 2 | `transmux-v0.29.0`, `dash-runtime-v0.2.0`, `media-doctor-v0.13.0` |
| 3 | `media-plane-v0.9.0`, `rtmp-runtime-v0.8.1`, `tshttp-runtime-v0.2.0` |
| 4 | `compliance-probe-v0.6.0`, `hls-runtime-v0.11.0`, `broadcast-client-v0.2.0` |
| 5 | `multimux-v0.15.0`, `multimux-cli-v0.13.0` |

Dependency justification: `srt-runtime` must be live before `broadcast-client` and `multimux`
(batch 1 before batches 4 and 5); the lockstep `v12.1.0` must be live before `transmux`
(batch 1 before batch 2); `transmux` must be live before `dash-runtime`, `media-doctor`,
`media-plane`, `rtmp-runtime`, `tshttp-runtime`, `hls-runtime`, `broadcast-client`,
`multimux` and `multimux-cli`; `media-plane` and `media-doctor` must be live before
`compliance-probe`, `hls-runtime`, `broadcast-client` and `multimux`; `hls-runtime`,
`dash-runtime`, `tshttp-runtime`, `rtmp-runtime` and `srt-runtime` must be live before
`broadcast-client` and `multimux`; `multimux` must be live before `multimux-cli`.

## Before the first tag

```bash
python3 tools/finalize-release-check.py $PAIRS
python3 tools/prepare-release.py --restamp-check $(cat $PAIRS)
python3 tools/check-published-dep-consistency.py --enforce-dev-cycles
python3 tools/finalize-release-check.py $PAIRS --package
```

`--restamp-check` must report the stamp day; if the first tag is pushed on another day run
`--restamp` and commit first. After each batch: `python3 tools/prepare-release.py --verify
$(cat $PAIRS)` for the crates published so far, and check docs.rs built green.

## After the wave

The out-of-workspace Python bindings (`bindings/transmux-py`, `bindings/dvb-si-py`) consume
published versions; bump `transmux-py` to `transmux` 0.29 and `dvb-si-py` to `dvb-si` 12.1
only after `transmux-v0.29.0` and `v12.1.0` are live.
