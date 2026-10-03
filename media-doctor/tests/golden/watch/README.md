# watch goldens

Taken from `main` at commit `182d03f78bd508981c2f2dace5efeebcfe8823d6`, before the
de-hand-roll W1-P metrics rewrite.

- `m6-single.prom` — `WatchState::render_prometheus()` for `fixtures/ts/m6-single.ts`
  (7-packet datagrams, 1 ms synthetic clock step).
- `http-response-head.txt` — status line + headers the old `media-doctor watch` bin
  sends for `GET /metrics`.

The old renderer no longer exists on this branch, so these goldens cannot be
regenerated here. To regenerate one, check out the commit above (or any commit
that still has `WatchState::render_prometheus`) in a scratch worktree and dump
`render_prometheus()` for the same fixture, chunked into 7-packet datagrams
with a 1 ms synthetic clock step. `h264_aac.prom` is the same dump for
`fixtures/ts/h264_aac.ts` (H.264 + AAC PIDs: per-PID `pid="0x0100"` series and
two conformance indicators).

## Live comparators

- `m6-single.prom` and `h264_aac.prom` are compared, semantically (HELP/TYPE per family, every
  series and value), by
  `src/watch_metrics.rs::tests::{real_fixture,per_pid}_exposition_equals_the_main_golden_semantically`.
- `http-response-head.txt` is compared semantically (status line; headers
  lower-cased with `date` and `content-length` removed; `content-length` equals
  the body) by
  `src/metrics_server.rs::tests::response_head_matches_the_main_golden_after_normalisation`.
  The old byte-exact capture test (`tests/watch_metrics_golden.rs`) was deleted
  once the old server was; both files were captured from the old bin.

## Text differences between the old renderer and the exporter

All semantically neutral for a Prometheus scraper; checked on `m6-single` with
`diff <(sort old) <(sort new)`:

1. The `# clauses: Continuity_count_error=TR 101 290 v1.4.1 Table 5.0a indicator 1.4`
   comment line is gone (the clause is now only on `ConformanceSample::clause`).
   This is lost operator-visible information, accepted by the owner.
2. A family with no series yet no longer writes bare `# HELP`/`# TYPE` lines
   (the exporter writes headers only together with a sample). On `m6-single`
   this drops the headers of `media_doctor_codec_signalling_mismatch` and
   `media_doctor_pts_dts_anomaly`, which have no per-PID series for that
   capture. (The plan expected HELP/TYPE to be identical; that cannot hold for
   header-only families, so the comparators drop the old side's header-only
   families before comparing.)
3. Families are separated by a blank line and are in a different order.
4. Every HELP text, TYPE and sample value is otherwise identical.
