# Provenance

## livesim2 MPDs

| File | Source | Date | Licence |
|---|---|---|---|
| `segtimeline_time_a.mpd`, `segtimeline_time_b.mpd` | Unmodified `GET https://livesim2.dashif.org/livesim2/segtimeline_1/testpic_2s/Manifest.mpd`, two snapshots ~5 s apart (`$Time$` + `SegmentTimeline`, absolute `S@t`, dynamic). Issue #1187. | 2026-10-14 | DASH-IF livesim2, BSD-3-Clause (https://github.com/Dash-Industry-Forum/livesim2/blob/main/LICENSE.md); content from Dash-Industry-Forum/livesim-content, Apache-2.0 |
| `segtimelinenr_a.mpd`, `segtimelinenr_b.mpd` | Unmodified `GET .../livesim2/segtimelinenr_1/testpic_2s/Manifest.mpd`, two snapshots ~8 s apart (`$Number$` + `SegmentTimeline`, sliding `@startNumber`). Issue #1187. | 2026-10-14 | same |

The negative-`S@r` vector (DASH-IF test-assets `dash264/TestCases/1c/qualcomm/1/MultiRate.mpd`) has no stated licence and is **not** committed; `multimux`'s test of it reads `private/fixtures/dash/qualcomm-multirate-1c-1.mpd` (private repo) and skips loudly when `private/` is not checked out.
