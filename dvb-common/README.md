# dvb-common

> **DEPRECATED — renamed to [`broadcast-common`](https://crates.io/crates/broadcast-common).**

This crate is a thin re-export shim kept so existing `dvb-common` dependencies keep
building. It re-exports `broadcast-common` in full. New code should depend on
`broadcast-common` directly:

```toml
[dependencies]
broadcast-common = "8.1"
```

No further feature work lands here.

## License

Licensed under either of [MIT](../LICENSE-MIT) or [Apache-2.0](../LICENSE-APACHE), at your option.
