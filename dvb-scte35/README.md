# dvb-scte35 (deprecated)

This crate has been renamed to [`scte35-splice`](https://crates.io/crates/scte35-splice). The `dvb-scte35` crate is now a thin re-export shim — all types and functions are identical to `scte35-splice 1.0.0`. Please update your dependency to `scte35-splice` directly; this shim will not receive new features.

```toml
# Replace this:
dvb-scte35 = "7"

# With this:
scte35-splice = "1"
```

In `use` statements, replace `dvb_scte35::` with `scte35_splice::`. All types and module paths are identical.
