# dvb-cc (deprecated)

This crate is a compatibility shim. It re-exports everything from
[`cc-data`](https://crates.io/crates/cc-data) 0.2, which is the new home for
the closed-caption `cc_data()` carriage (CEA-608/708) implementation,
per ETSI TS 101 154 Table B.9.

**Migrate:** replace `dvb-cc` with `cc-data` in your `Cargo.toml` and rename
`dvb_cc::` to `cc_data::` in your source.
