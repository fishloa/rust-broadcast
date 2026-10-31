# Release runbook - the 2026-11 wave

The ordered tag list for the wave prepared in the `release/2026-11` PR. The pair list is
[`release-plan-2026-11.pairs`](release-plan-2026-11.pairs): 26 crates, 22 tags (21 independent
+ the lockstep `v12.0.1`). Nothing here runs until the owner gives the tag go. Never
`cargo publish` from a workstation; CI is the only publisher. The general procedure (private
fixtures, gates, package check, post-publish verification) is
[`RELEASE-RUNBOOK-2026-10.md`](RELEASE-RUNBOOK-2026-10.md) sections 0-6; this file only fixes
what differs for this wave.

```bash
PAIRS=docs/release-plan-2026-11.pairs
```

## What the wave contains

| Class | Crates |
|---|---|
| Lockstep patch (12.0.0 -> 12.0.1; yoke 0.8 is non-breaking, the `yoke` feature is off by default) | `dvb-si`, `dvb-t2mi`, `dvb-bbframe`, `dvb-conformance`, `dvb-tools` |
| 0.x breaking (minor) | `broadcast-hls` 0.5.0, `hls-runtime` 0.10.0, `transmux` 0.28.0, `multimux` 0.14.0 |
| Epoch-forced (0.x minor) | `timed-metadata` 0.8.0, `ssai-runtime` 0.4.0, `playout-runtime` 0.4.0, `media-plane` 0.8.0, `caption-convert` 0.4.0, `media-doctor` 0.12.0, `compliance-probe` 0.5.0, `multimux-cli` 0.12.0 |
| Additive or internal (0.x patch) | `container-probe` 0.2.1, `broadcast-auth` 0.5.1, `rtsp-runtime` 0.8.1, `dvb-ci-runtime` 0.18.2, `srt-runtime` 0.6.1, `webrtc-runtime` 0.4.1 |
| New crates | `broadcast-client` 0.1.0, `dash-runtime` 0.1.0, `tshttp-runtime` 0.1.0 |
| Not moving | `broadcast-common` 10.0.0 (no `broadcast-common-v*` tag), every other crate; docs-only `[Unreleased]` entries (`cc-data`, `dvb-ci`, `dvb-csa`, `dvb-subtitle`, `mpeg-ts`, `rtmp-runtime`, `scte35-splice`, `buffering-runtime`) stay unreleased |

Minimum-version requirements that matter at publish time: `hls-runtime` and `dash-runtime`
require `container-probe` `0.2.1` (segment-structure checks), `broadcast-client` requires
`broadcast-auth` `0.5.1` and `rtsp-runtime` `0.8.1`. `multimux` requires `dash-runtime` and
`tshttp-runtime`, so both publish before it.

## Tag order

Push one tag at a time (`tools/push-release-tags.sh --plan $PAIRS`, which waits for the
crates.io index and the lane to finish before the next). Batches of at most three; wait for every
tag of a batch to be live before the next batch. The lockstep tag is pushed by hand and
approved in the `crates-io` environment.

| Batch | Tags (in this order) |
|---|---|
| 1 | `v12.0.1`, `broadcast-auth-v0.5.1`, `broadcast-hls-v0.5.0` |
| 2 | `container-probe-v0.2.1`, `srt-runtime-v0.6.1`, `webrtc-runtime-v0.4.1` |
| 3 | `rtsp-runtime-v0.8.1`, `ssai-runtime-v0.4.0`, `timed-metadata-v0.8.0` |
| 4 | `dvb-ci-runtime-v0.18.2`, `transmux-v0.28.0`, `playout-runtime-v0.4.0` |
| 5 | `caption-convert-v0.4.0`, `dash-runtime-v0.1.0`, `tshttp-runtime-v0.1.0` |
| 6 | `media-plane-v0.8.0`, `media-doctor-v0.12.0`, `hls-runtime-v0.10.0` |
| 7 | `compliance-probe-v0.5.0`, `multimux-v0.14.0`, `broadcast-client-v0.1.0` |
| 8 | `multimux-cli-v0.12.0` |

Dependency justification: `transmux` needs `broadcast-hls` 0.5 (batch 1); `media-plane`
needs `timed-metadata` 0.8 and `transmux` 0.28; `hls-runtime` needs `media-plane`,
`timed-metadata`, `container-probe` 0.2.1 and `transmux`; `media-doctor` needs `timed-metadata`
and `transmux`; `compliance-probe` needs `media-doctor`, `media-plane` and `timed-metadata`;
`multimux` needs `hls-runtime`, `caption-convert`, `dash-runtime`, `tshttp-runtime` and (as
dev-dependencies) `media-doctor` and `media-plane`; `broadcast-client` needs `hls-runtime`,
`dash-runtime`, `tshttp-runtime`, `broadcast-auth` 0.5.1 and `rtsp-runtime` 0.8.1;
`multimux-cli` needs `multimux`.

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
published versions; bump `transmux-py` to `transmux` 0.28 only after `transmux-v0.28.0` is live.
