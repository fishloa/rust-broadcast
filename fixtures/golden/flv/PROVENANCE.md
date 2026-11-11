# PROVENANCE

## FLV tag-walk goldens

`tag-walk.golden` records what `transmux`'s FLV tag walk yields — tag count,
damaged-span offsets, truncated-tail flag and each tag's offset, type,
timestamp and body length — for every committed `fixtures/flv/` capture and for
the damage and truncation shapes built from them inside the test.

Regenerated only by the test that owns it:

    REGENERATE_FLV_WALK_GOLDEN=1 cargo test -p transmux --all-features --locked --lib walk_golden

The record was taken before the walk's logic moved into
`rtmp_runtime::flv::tags_resync`, and the move had to reproduce it byte for
byte. It is never re-blessed to make a change pass.
