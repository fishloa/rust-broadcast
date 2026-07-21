# Story 55c — GSE handoff documentation (dvb-bbframe)

GitHub issue #55(c). Make the MATYPE→GSE seam explicit in docs. **No runtime
dependency**, no new code paths. Touch only `dvb-bbframe/src/lib.rs`,
`dvb-bbframe/README.md`, `dvb-bbframe/CHANGELOG.md`. Run gates; do not commit.

## What
When `Bbheader::parse` yields `matype.ts_gs == TsGs::Gse`, the data field holds
GSE packets (out of scope for this crate). Add a doc example showing the dispatch
and where to hand the data field to the third-party `dvb-gse` crate.

1. **`dvb-bbframe/src/lib.rs`** crate-root doc: add a `## Generic Stream (GSE) handoff`
   section with an example fenced as ```ignore (NOT ```rust / ```no_run — the
   `dvb_gse` crate is not a dependency, so the block must not be compiled by
   `cargo test --doc`). The example must show real dvb-bbframe API:
   ```ignore
   use dvb_bbframe::header::{Bbheader, TsGs, BBHEADER_LEN};
   let hdr = Bbheader::parse(df_bytes)?;
   let data_field = &df_bytes[BBHEADER_LEN..];
   match hdr.matype.ts_gs {
       TsGs::Ts => { /* TS user packets: dvb_bbframe::packet::up_iter(data_field, &hdr) */ }
       TsGs::Gse => { /* GSE packets: hand `data_field` to the `dvb-gse` crate */ }
       other => { /* GFPS / GCS — generic continuous/packetized */ }
   }
   ```
   Keep it accurate to the actual enum variants of `TsGs` (read header.rs).
2. **README.md**: one sentence under the existing "Generic Stream" mention
   pointing at the `dvb-gse` crate for GSE payload parsing (the seam this crate
   stops at).
3. **CHANGELOG.md** `## [Unreleased]` → add a `### Added`/docs note: GSE handoff
   example (#55).

## Constraints
- The doc example is ```ignore — verify `RUSTDOCFLAGS="-D warnings" cargo doc`
  and `cargo test --doc -p dvb-bbframe` stay green (ignored block is not compiled).
- No new dependencies (runtime or dev). No src logic changes.

## Gates (run ALL; fix until green)
```
cargo build --workspace --all-features --locked
cargo test  --workspace --all-features --locked
cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
```
## Boundaries
Only dvb-bbframe/src/lib.rs, dvb-bbframe/README.md, dvb-bbframe/CHANGELOG.md. Do not commit.
