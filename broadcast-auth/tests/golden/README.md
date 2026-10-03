# broadcast-auth wire goldens

Generated from unmodified `main` at commit 182d03f78bd508981c2f2dace5efeebcfe8823d6 (the W1-T baseline).

Bless command (writes instead of comparing):

    GOLDEN_BLESS=$PWD/broadcast-auth/tests/golden cargo test -p broadcast-auth --locked --test golden_wire

Never regenerate on this branch. A deliberate difference is edited into the file in the commit that causes it and
listed in CHANGELOG `[Unreleased]`.
