# dvb-ci 0.13.0

_Released 2026-11-15._

Minor-class release (0.12 -> 0.13). The `ca_pmt` builder's serializer is fallible (its panicking twin is gone), the IPv6 `IpConfig` is built through a constructor so a field added later is additive, and the reserved-bit types now live in `broadcast-common` and are re-exported.

Version: 0.12.0 -> 0.13.0.

## Breaking changes and migration

### `CaPmtBuilt::to_bytes` is fallible

`CaPmtBuilt::to_bytes` returns `Result<Vec<u8>>` and the panicking twin is removed: a filtered descriptor loop with no valid `ca_pmt` encoding (for example a loop whose final `descriptor_length` runs past the end of the source PMT section) is reported, so a host forwarding a CAM-supplied PMT can reject such a `ca_pmt` as a value it can act on rather than crash on it.

```rust
// before
let bytes = built.to_bytes();
// after
let bytes = built.to_bytes()?;   // Result<Vec<u8>>
```

### `IpConfig` is `#[non_exhaustive]`

`ci_plus::low_speed_comms_v4::IpConfig` is built with `IpConfig::new(ip_address, network_mask, default_gateway)` plus `with_dhcp_server(..)`/`with_dns_servers(..)` (both default to the Table 79 "absent" encoding) instead of a struct literal.

```rust
// before
let config = IpConfig { ip_address, network_mask, default_gateway, dhcp_server_address, .. };
// after
let config = IpConfig::new(ip_address, network_mask, default_gateway).with_dhcp_server(dhcp);
```

## Changed

- `reserved::Reserved`/`ReservedOnes`/`ReservedZero` live in `broadcast_common::reserved`; `dvb_ci::reserved` re-exports them, so this crate's own paths are unchanged.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `dvb-si` 14 -> 15, `mpeg-ts` 0.8 -> 0.9.

---

Published from tag `dvb-ci-v0.13.0`.
