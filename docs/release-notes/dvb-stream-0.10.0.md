# dvb-stream 0.10.0

_Released 2026-11-15._

### Changed
- BREAKING: `ResyncStats` is now `#[non_exhaustive]`; it is read from `resync_stats()` and field-assigned, never built by an external struct literal (#2664).
- BREAKING: the datagram bind takes the workspace's one option type, `broadcast_common::net::UdpOptions`, and no longer its own. `udp::bind_standard(SocketSpec)` becomes `udp::bind_udp(SocketAddr, &UdpOptions)`; `SocketSpec`, `MulticastMembership` and `MulticastConfig` are removed. Migration: `MulticastConfig::new(bind_addr, group).with_recv_buffer_size(n).with_reuse_address(on).bind()` becomes `udp::bind_udp(bind_addr.into(), &UdpOptions::new().with_join(MulticastJoin::new(group.into()).with_interface(MulticastInterface::V4(*bind_addr.ip()))).with_recv_buffer_size(n).with_reuse_address(on))` (the builder method is `with_recv_buffer_bytes`). `MulticastConfig`'s IPv4-only `interface` field maps to `MulticastInterface`, whose one value replaces the "address and index never set together" rule. A source-specific join is now real: `MulticastJoin::source` joins with `IP_ADD_SOURCE_MEMBERSHIP` (`join_ssm_v4`); a source-specific IPv6 join is refused as `Unsupported`.
- BREAKING: `UdpSectionStream::bind(&MulticastConfig)` and `UdpT2miStream::bind(&MulticastConfig, pid)` become `bind(bind_addr: SocketAddr, join: &MulticastJoin)` and `bind(bind_addr: SocketAddr, join: &MulticastJoin, pid)` (OS-default socket options; use `udp::bind_udp` + `from_socket` for the rest). `bind_multicast` keeps its signature. A join whose group is not a multicast address is now refused with `io::ErrorKind::InvalidInput` rather than bound unicast.
- `udp::check_udp`: the bind's upfront refusals (a non-multicast join group, a group of the other family than the bind address, an interface of the other family than the group, a source that is not a unicast address of the group's family) as one public check (#2663).

---

Published from tag `dvb-stream-v0.10.0`.
