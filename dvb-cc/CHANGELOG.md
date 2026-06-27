# Changelog — dvb-cc

## 0.2.1 — 2026-06-27

Compatibility shim release. The crate is now a thin `pub use cc_data::*;`
re-export of [`cc-data`](https://crates.io/crates/cc-data) 0.2.
Migrate by replacing `dvb-cc` → `cc-data` in `Cargo.toml` and
`dvb_cc::` → `cc_data::` in source.

## 0.2.0 — 2026-06-21

See [`cc-data` CHANGELOG](../cc-data/CHANGELOG.md) for the full history through 0.2.0.
