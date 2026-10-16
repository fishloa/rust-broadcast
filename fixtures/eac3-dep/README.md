# eac3-dep (pointer only)

The real E-AC-3 stream with a dependent substream used by
`transmux/tests/eac3_real_dependent_substreams.rs` has an unstated licence, so it
lives in the private submodule: `private/fixtures/eac3-dep/` (source URL, retrieval
date, structure and the regeneration script are in its `PROVENANCE.md` / `make.sh`).
The tests skip loudly when `private/` is not checked out.
