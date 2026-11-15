# dvb-stream 0.10.0

_Released 2026-11-15._

Minor-class release (0.9 -> 0.10). The datagram bind takes the workspace's one option type, `broadcast_common::net::UdpOptions`, and no longer its own spec struct: the join, the interface and the receive buffer are one value, so "interface and index never set together" is unrepresentable and a source-specific multicast join is real. A join whose group is not a multicast address is refused instead of bound unicast.

Version: 0.9.0 -> 0.10.0.

## Breaking changes and migration

### `SocketSpec` is `UdpOptions`

`udp::bind_standard(SocketSpec)` is `udp::bind_udp(SocketAddr, &UdpOptions)`; `SocketSpec`, `MulticastMembership` and `MulticastConfig` are removed. The join's interface is one `MulticastInterface` value, which replaces the "address and index never set together" rule. A source-specific join is now real: `MulticastJoin::source` joins with `IP_ADD_SOURCE_MEMBERSHIP` (`join_ssm_v4`); a source-specific IPv6 join is refused as `Unsupported`.

```rust
// before
let socket = MulticastConfig::new(bind_addr, group)
    .with_recv_buffer_size(8 << 20)
    .with_reuse_address(true)
    .bind()?;
// after
let options = UdpOptions::new()
    .with_join(MulticastJoin::new(group).with_interface(MulticastInterface::V4(bind_addr.ip())))
    .with_recv_buffer_bytes(NonZeroU32::new(8 << 20).unwrap())
    .with_reuse_address(true);
let socket = udp::bind_udp(bind_addr, &options)?;
```

### The section and T2-MI streams take `(addr, join)`

`UdpSectionStream::bind(&MulticastConfig)` and `UdpT2miStream::bind(&MulticastConfig, pid)` are `bind(bind_addr: SocketAddr, join: &MulticastJoin)` and `bind(bind_addr: SocketAddr, join: &MulticastJoin, pid)` (OS-default socket options; use `udp::bind_udp` + `from_socket` for the rest). `bind_multicast` keeps its signature. A join whose group is not a multicast address is refused with `io::ErrorKind::InvalidInput` rather than bound unicast.

```rust
// before
let stream = UdpSectionStream::bind(&config).await?;
// after
let stream = UdpSectionStream::bind(bind_addr, &MulticastJoin::new(group)).await?;
```

### `ResyncStats` is `#[non_exhaustive]`

It is read from `resync_stats()` and field-assigned, never built by an external struct literal.

```rust
// before
let stats = ResyncStats { resyncs: 3, bytes_discarded: 188, desyncs: 1 };
// after
let stats = stream.resync_stats();   // produced by the stream
```

## Added

- `udp::check_udp`: the bind's upfront refusals (a non-multicast join group, a group of the other family than the bind address, an interface of the other family than the group, a source that is not a unicast address of the group's family) as one public check.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `dvb-si` 14 -> 15, `dvb-t2mi` 14 -> 15, `mpeg-ts` 0.8 -> 0.9, `rtp-packet` 0.7 -> 0.8.

---

Published from tag `dvb-stream-v0.10.0`.
