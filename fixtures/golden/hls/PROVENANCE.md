# PROVENANCE


## Folded note: hls-runtime golden README

# HLS client URL golden

`client-urls.golden` is the HLS client's actions and request URLs, generated on
`main` at BASE `182d03f78bd508981c2f2dace5efeebcfe8823d6` (before any
W1-R-low-b change) with:

```text
GOLDEN_UPDATE=1 cargo test --locked -p hls-runtime --all-features --test it golden_client
```

`tests/golden_client.rs` compares the live output to this file byte-for-byte.



## Folded note: timed-metadata golden README

# timed-metadata goldens

Generated from unmodified `main` at commit 182d03f78bd508981c2f2dace5efeebcfe8823d6 (the W1-T baseline).

Bless command (writes instead of comparing):

    GOLDEN_BLESS=$PWD/timed-metadata/tests/golden cargo test -p timed-metadata --all-features --locked --test golden_wire

Never regenerate on this branch; a deliberate difference is edited into the file in the commit that causes it and listed in CHANGELOG `[Unreleased]`.



## Folded note: transmux golden README

# Golden outputs

Byte-for-byte expected outputs, generated from the **pre-quick-xml code on `origin/main`
commit `b383d298`** (`refactor!: consolidate duplicated implementations (#1141)`), not from
the quick-xml branch. DASH MPD (default/dynamic/timeline/protected/trick), LL-DASH MPD (plain/rate/utc/timeline), the Smooth manifest of `fixtures/ts/h264-aac.ts`, PlayReady WRMHEADER (with specials), and `Debug` dumps of the MPD/Smooth parse results.

Regenerate (only ever against that main commit, in a separate worktree, never on the
quick-xml branch):

```bash
git worktree add --detach ../qxml-main b383d298
cp transmux/tests/it/golden.rs ../qxml-main/transmux/tests/golden.rs   # the test file is API-identical on main
cd ../qxml-main
GOLDEN_BLESS=<this directory> cargo test -p transmux --test golden --all-features --locked
```

The tests (`transmux/tests/it/golden.rs`) compare the current output to these files and fail on any
byte difference. Intended, documented differences from main are listed in the crate
CHANGELOG `### Changed` section; none of them is exercised by these inputs.

## RTP SDP and HLS IV (W1-T)

`rtp-sdp-h264-aac.sdp`, `rtp-sdp-conn-v4.sdp`, `rtp-sdp-conn-v6.sdp` and `hls-iv.txt` (test: `tests/it/golden_wire.rs`) were
generated from unmodified `main` at commit 182d03f78bd508981c2f2dace5efeebcfe8823d6 with:

    GOLDEN_BLESS=$PWD/transmux/tests/golden cargo test -p transmux --all-features --locked --test golden_wire

The W1-T `xs:duration` change (Task 12c) intentionally edits the `PT...S` values in the `*.mpd` goldens (listed in the CHANGELOG);
these four files stay byte-identical.

