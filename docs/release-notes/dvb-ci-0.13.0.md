# dvb-ci 0.13.0

_Released 2026-11-15._

### Changed
- `reserved::Reserved`/`ReservedOnes`/`ReservedZero` now live in `broadcast_common::reserved`; `dvb_ci::reserved` re-exports them, so this crate's own paths are unchanged (#2738).
- BREAKING: `ci_plus::low_speed_comms_v4::IpConfig` is `#[non_exhaustive]`; build it with `IpConfig::new(ip_address, network_mask, default_gateway)` plus `with_dhcp_server(..)` / `with_dns_servers(..)` (both default to the Table 79 "absent" encoding) instead of a struct literal (#2736).
- BREAKING: `CaPmtBuilder::to_bytes` is now fallible (`Result<Vec<u8>>`); the panicking twin is removed; callers use the fallible one (#2659).

---

Published from tag `dvb-ci-v0.13.0`.
