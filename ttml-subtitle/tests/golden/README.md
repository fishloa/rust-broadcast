# Golden outputs

Byte-for-byte expected outputs, generated from the **pre-quick-xml code on `origin/main`
commit `b383d298`** (`refactor!: consolidate duplicated implementations (#1141)`), not from
the quick-xml branch. All 11 committed `.ttml` fixtures and five edge documents (specials, mixed content, foreign content, metadata pure/impure, prefix rebinding): `parse.txt` = `Debug` dump of the parse result, `.xml` = `to_xml()`.

Regenerate (only ever against that main commit, in a separate worktree, never on the
quick-xml branch):

```bash
git worktree add --detach ../qxml-main b383d298
cp ttml-subtitle/tests/golden.rs ../qxml-main/ttml-subtitle/tests/golden.rs   # the test file is API-identical on main
cd ../qxml-main
GOLDEN_BLESS=<this directory> cargo test -p ttml-subtitle --test golden --all-features --locked
```

The tests (`ttml-subtitle/tests/golden.rs`) compare the current output to these files and fail on any
byte difference. Intended, documented differences from main are listed in the crate
CHANGELOG `### Changed` section; none of them is exercised by these inputs.
