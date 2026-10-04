# Golden outputs

Byte-for-byte expected outputs of `render_manifest` (`multimux/src/output/smooth.rs`),
generated from the **pre-quick-xml code on `origin/main` commit `b383d298`**
(`refactor!: consolidate duplicated implementations (#1141)`), not from the quick-xml branch:
`smooth-video-audio.xml` (a video + audio window) and `smooth-empty.xml` (an empty window).

`render_manifest` is private, so the test is a unit test. To regenerate, build **main's own
`smooth.rs`** and add ONLY the test function to it (never copy the branch's `smooth.rs`
over main's):

```bash
git worktree add --detach ../qxml-main b383d298
python3 - <<'PY'
import re
branch = open("multimux/src/output/smooth.rs").read()        # run from the branch worktree
a = branch.index("    /// Byte-for-byte golden: the rendered manifest equals")
b = branch.index("    /// Every attribute value in the rendered manifest is escaped")
test_fn = branch[a:b]                                         # only the test fn
p = "../qxml-main/multimux/src/output/smooth.rs"
main = open(p).read()
marker = "    /// `CodecPrivateData` must carry the H.264 parameter sets in Annex-B form,"
assert marker in main
open(p, "w").write(main.replace(marker, test_fn + marker, 1))
PY
cd ../qxml-main
GOLDEN_BLESS=<this directory> cargo test -p multimux --lib render_manifest_matches_golden --locked
```

The test compares the current output to these files and fails on any byte difference.
Intended differences from main are listed in the multimux CHANGELOG; none is exercised by
these inputs.

## `origin_response_headers.golden`

Byte-for-byte expected HTTP response headers of the origin's shared router
(`multimux/src/origin/mod.rs::add_response_headers` plus the axum route stack), one line per
probe: `path<TAB>status` followed by one `path<TAB>status<TAB>name: value` line per header
(names sorted). Generated from `origin/main` commit `9dd1e31e` (the W2a plan commit) **before**
the axum 0.8 / tower-http 0.7 bump, because `CorsLayer` changes the exact `Access-Control-*`
set on the wire. The HlsOrigin instance token in the instance-named init URI is normalised to
`{instance}` (it is a fresh wall-clock-seeded number per build), and `/metrics`' varying
`Content-Length` (a process-global counter exposition) is omitted. Regenerate:

```bash
GOLDEN_BLESS=multimux/tests/golden cargo test -p multimux --all-features --locked \
  --lib origin::tests::origin_response_headers_match_golden
```

## `whip_response_headers.golden` / `whep_response_headers.golden`

Byte-for-byte expected response HEADERS of the WHIP (`source/whip.rs`) and WHEP
(`output/whep.rs`) signalling endpoints — the `201 Created` / `204 No Content` (OPTIONS
preflight) / `413 Payload Too Large`, plus WHEP's `401 Unauthorized`. Only the status line and
header lines are pinned (the SDP body and its `Content-Length`, the `WWW-Authenticate`
challenge value, the wall-clock `Date`, and the random `ETag` are not — the last two are
normalised to `{date}`/`{etag}`). Generated from `origin/main` commit `9dd1e31e` **before** the axum
0.8 / tower-http 0.7 bump and the WP2.1 move of both endpoints onto an axum router with a
`CorsLayer`. Regenerate:

```bash
GOLDEN_BLESS=multimux/tests/golden cargo test -p multimux --all-features --locked --lib \
  source::whip::tests::whip_response_headers_match_golden \
  output::whep::tests::whep_response_headers_match_golden
```

## `whip_answer.golden` / `whep_answer.golden`

Byte-for-byte expected SDP ANSWER bodies of the WHIP (`source/whip.rs`) and WHEP
(`output/whep.rs`) endpoints, as rendered by `sdp_types::Session::write`. The offer, local
address, ICE credentials, codec config (WHEP), fingerprint and candidate lines are all
caller-supplied through the `render_*_answer_for_test` hooks, so the golden pins only the
`o=`/`c=`/`m=`/attribute LINE ORDERING — the random certificate fingerprint and ephemeral
candidate values never appear.

The offer is **multi-payload-type** (`m=video … 96 97 98`, two H.264 entries plus a VP8 entry)
so the golden pins the WHIP `m=` fmt-list change: main echoed the offer's whole `fmt` list
(`m=video 9 UDP/TLS/RTP/SAVPF 96 97 98`), while the sdp-types answer lists only the chosen
payload type (`… 96`) per RFC 3264 §6.1. Captured (fingerprint normalised to the deterministic
value) from `origin/main` `9dd1e31e` in a throwaway `w2-main-golden` worktree; the WHEP answer
is byte-identical to main (main already listed only the selected PT). See the CHANGELOG.
Regenerate:

```bash
cd multimux
GOLDEN_BLESS=tests/golden cargo test -p multimux --all-features --locked \
  --test whip_whep_sdp --test whep_sdp_golden
```

## `dash_mpd.golden` / `ll_dash_mpd.golden`

Byte-for-byte expected DASH (`output::dash::render_mpd_at`) and LL-DASH
(`output::ll_dash::render_ll_dash_mpd_at`) MPD bodies with the `now`-dependent
`@availabilityStartTime` frozen to `2023-11-14T22:13:20Z` (`UNIX_EPOCH +
1_700_000_000s`). Captured before the SP5 jiff migration; the golden is the
witness that no wire byte changed. Regenerate:

```bash
cd multimux
GOLDEN_BLESS=tests/golden cargo test -p multimux --all-features --locked \
  --lib matches_frozen_now_golden
```

## `dash_mpd_main.golden` / `dash_mpd_window60_main.golden` / `dash_mpd_window3600_main.golden`

Capture-of-record from `origin/main` for the deliberate `xs:duration` change in
W2b-1. Main spells `@timeShiftBufferDepth` as `PT{secs}S`; this branch uses the
balanced jiff spelling (`jiff::fmt::temporal::SpanPrinter`). Below 60 s the two
agree byte-for-byte (`dash_mpd_main.golden` equals `dash_mpd.golden` after
normalising `@availabilityStartTime`); at/above it they differ:

- 60 s: main `PT60S` -> ours `PT1M` (pinned against `dash_mpd_window60_main.golden`
  by `the_over_60s_window_differs_from_main_by_exactly_the_balanced_token`).
- 3600 s: main `PT3600S` -> ours `PT1H` (pinned against
  `dash_mpd_window3600_main.golden` by
  `the_over_an_hour_window_differs_from_main_by_exactly_the_balanced_token`).

Each pinning test asserts the two MPDs differ by EXACTLY that one token, so a
future change that alters any other byte (or silently reverts to `PT60S`) fails.
Captured in a throwaway `origin/main` worktree: add the capture test to main's
own `dash.rs`, run with `GOLDEN_BLESS=<dir>`, then normalise
`@availabilityStartTime` to the frozen value. `ll_dash_mpd.golden` is unaffected
(sub-60 s) and byte-identical to main.
