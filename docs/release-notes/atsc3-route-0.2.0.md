# atsc3-route 0.2.0

_Released 2026-10-05._

Breaking (0.x minor) removal release. The two typed header-extension decoders, `ExtRoutePresentationTime` and `ExtTol`, are deleted from the public API, along with their constants. Who must act: only code that imported those items. Packet framing (`RoutePacket`), the FEC Payload ID layouts, the Codepoint table and the three `HET_*` constants are unchanged. The previous published version is 0.1.0. The dependency epochs also move (`rmt-flute` 0.5 to 0.6, `broadcast-common` 9.3 to 9.4); see [rmt-flute-0.6.0.md](rmt-flute-0.6.0.md).

## Why

No publicly available ATSC 3.0 ROUTE capture carries either extension: 14,000+ real packets from three independent sources were scanned and none contained one. This crate requires every implemented type to be exercised by a byte-exact round-trip against a real capture, and these two could not meet that bar, so they were removed rather than left as untested code. The typed decoders will be re-added if a real capture containing these extensions surfaces. The crate-root documentation also now states that ATSC 3.0 work in this workspace is archived with no further development; this release carries no feature work.

## Breaking change: removed items

Removed from the crate root and from `ext`:

- `ExtRoutePresentationTime` (EXT_ROUTE_PRESENTATION_TIME, HET 66, A/331 §A.3.7.1; its `ntp_timestamp: u64` field and parse/serialize/`to_extension` methods)
- `ExtTol` (EXT_TOL, HET 194 fixed or 67 variable, §A.3.8.1; `Bits24` / `Bits48` variants and methods)
- `EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN`, `EXT_TOL_24_CONTENT_LEN`, `EXT_TOL_48_CONTENT_LEN`, `MAX_TOL_24`, `MAX_TOL_48`

Kept: `HET_EXT_ROUTE_PRESENTATION_TIME` (66), `HET_EXT_TOL_24` (194) and `HET_EXT_TOL_48` (67), so a caller walking an LCT extension chain can still recognise the extensions by type.

Migration. `RoutePacket.lct.extensions` is a `Vec<rmt_flute::HeaderExtension>` with public `het` and `content` fields; match on the HET constant and decode the content yourself:

```rust
// 0.1.0
for ext in &pkt.lct.extensions {
    if ext.het == HET_EXT_ROUTE_PRESENTATION_TIME {
        let t = ExtRoutePresentationTime::parse(ext.content)?;
        use_ntp(t.ntp_timestamp);
    }
}
// 0.2.0: the typed decoder is gone; the HET constant and the raw extension remain
for ext in &pkt.lct.extensions {
    if ext.het == atsc3_route::HET_EXT_ROUTE_PRESENTATION_TIME {
        handle_raw(ext.content);   // your own decoding of ext.content
    }
}
```

The exact `parse` call in the "0.1.0" snippet is illustrative of the removed API; the 0.1.0 layouts are in the `atsc3-route-v0.1.0` tag's `src/ext.rs`.

## Dependencies

Verified from the `Cargo.toml` diff against `atsc3-route-v0.1.0`:

```toml
broadcast-common = { version = "9.3" -> "9.4", default-features = false }
rmt-flute        = { version = "0.5" -> "0.6", default-features = false }
```

The `description` metadata now says "header-extension type (HET) constants" instead of naming the typed extensions.

---

Published from tag `atsc3-route-v0.2.0`.
