# scte35-splice goldens

Generated from unmodified `main` at commit 182d03f78bd508981c2f2dace5efeebcfe8823d6 (the W1-T baseline).

Bless command (writes instead of comparing):

    GOLDEN_BLESS=$PWD/scte35-splice/tests/golden cargo test -p scte35-splice --all-features --locked --test golden_base64

Never regenerate on this branch; a deliberate difference is edited into the file in the commit that causes it and listed in CHANGELOG `[Unreleased]`.
