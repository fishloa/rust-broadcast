# Golden outputs

Byte-for-byte expected outputs, generated from the **pre-quick-xml code on `origin/main`
commit `b383d298`** (`refactor!: consolidate duplicated implementations (#1141)`), not from
the quick-xml branch. DASH MPD (default/dynamic/timeline/protected/trick), LL-DASH MPD (plain/rate/utc/timeline), the Smooth manifest of `fixtures/ts/h264_aac.ts`, PlayReady WRMHEADER (with specials), and `Debug` dumps of the MPD/Smooth parse results.

Regenerate (only ever against that main commit, in a separate worktree, never on the
quick-xml branch):

```bash
git worktree add --detach ../qxml-main b383d298
cp transmux/tests/golden.rs ../qxml-main/transmux/tests/golden.rs   # the test file is API-identical on main
cd ../qxml-main
GOLDEN_BLESS=<this directory> cargo test -p transmux --test golden --all-features --locked
```

The tests (`transmux/tests/golden.rs`) compare the current output to these files and fail on any
byte difference. Intended, documented differences from main are listed in the crate
CHANGELOG `### Changed` section; none of them is exercised by these inputs.
