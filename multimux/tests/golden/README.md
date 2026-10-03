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
