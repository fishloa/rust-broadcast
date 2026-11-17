# PROVENANCE


## Dvb-mabr goldens

# Golden outputs

Byte-for-byte expected outputs, generated from the **pre-quick-xml code on `origin/main`
commit `b383d298`** (`refactor!: consolidate duplicated implementations (#1141)`), not from
the quick-xml branch. The three committed Annex C fixtures (`parse.txt` = `Debug` dump of the parse result, `.xml` = `to_xml()`), plus a specials document (`& < > " '`).

Regenerate (only ever against that main commit, in a separate worktree, never on the
quick-xml branch):

```bash
git worktree add --detach ../qxml-main b383d298
cp dvb-mabr/tests/it/golden.rs ../qxml-main/dvb-mabr/tests/golden.rs   # the test file is API-identical on main
cd ../qxml-main
GOLDEN_BLESS=<this directory> cargo test -p dvb-mabr --test golden --all-features --locked
```

The tests (`dvb-mabr/tests/it/golden.rs`) compare the current output to these files and fail on any
byte difference. Intended, documented differences from main are listed in the crate
CHANGELOG `### Changed` section; none of them is exercised by these inputs.

## Independent oracle

The three Annex C inputs are this workspace's own hand-authored documents
(`fixtures/mabr/PROVENANCE.md`), modelled on the *shape* of ETSI TS 103 769 V1.2.1 Annex C but not
transcribed from it, and no third-party MABR schema validator is available offline. These goldens
therefore pin stability, not correctness; correctness rests on `spec_fidelity.rs` (every field of
the clause 10 documents is parsed) and `round_trip.rs` (`parse -> to_xml -> parse` is
structure-preserving).

`GOLDEN_BLESS` refuses to write when `CI` is set, so a re-bless cannot ride along in a CI run.

