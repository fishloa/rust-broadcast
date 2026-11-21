# PROVENANCE


## dvr-runtime goldens

# dvr-runtime period-playlist golden

`p4.m3u8` is the exact playlist text `dvr_runtime::ArchiveWriter` produces from
a fixed input: period 4 of an fMP4 archive, 812-byte init, two 48 211/51 002-byte
segments, one programme boundary whose title contains a `"` (so it travels as
the lossless `X-TITLE-UTF8` hex attribute) and one discontinuous segment.
`dvr-runtime/tests/it/golden.rs` compares the writer's lines to this file
byte-for-byte; it does not regenerate it. A deliberate format change edits the
file in the commit that causes it and lists it in the crate CHANGELOG.
