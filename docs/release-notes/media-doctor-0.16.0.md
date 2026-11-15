# media-doctor 0.16.0

_Released 2026-11-15._

Minor-class release (0.15 -> 0.16). A malformed SCTE-35 cue is now reported instead of dropped, a report names the stream it is about with a typed `StreamLocator` instead of one overloaded integer, the UDP bind takes the workspace's shared `UdpOptions`, and a bad socket option is refused before a socket exists.

Version: 0.15.0 -> 0.16.0.

## Breaking changes and migration

### `cli` is no longer a default feature

`default = ["std"]`, and the binary declares `required-features = ["cli"]`, so a plain dependency on the library no longer compiles `clap`, `hyper`, `tokio` or the Prometheus exporter. Install the binary with `cargo install media-doctor --features cli` (previously the bare `cargo install media-doctor`).

```toml
# before
[dependencies]
media-doctor = "0.15"
# after (the library, without the CLI)
media-doctor = "0.16"
# after (the binary)
# cargo install media-doctor --features cli
```

### `Location::pid` is a `StreamLocator`

`Location::pid: u32` (one field carrying either a 13-bit TS PID or a 32-bit track id, with `0` for "none") is `Location::stream: StreamLocator`, the `Ts(Pid) | Track(TrackId) | Unknown` enum. `Location::at_pid(packet_index, Pid)` and `Location::at_track(packet_index, TrackId)` keep the typed location. Reports still print the bare number (`pid=0x0100`, `0` when unknown) and JSON still carries the bare integer under the `pid` key, so existing goldens are unchanged.

```rust
// before
let location = Location::new(packet_index).at_pid(0x0100);     // a raw 13-bit pid
if location.stream == StreamLocator::Ts(Pid::from_const(0x0100)) { .. }
// after
let location = Location::at_pid(packet_index, Pid::from_const(0x0100));
if location.stream == StreamLocator::Ts(Pid::from_const(0x0100)) { .. }
```

### The UDP bind takes `broadcast_common::net::UdpOptions`

`udp::UdpConfig` and `udp::MulticastInterface` are removed in favour of `broadcast_common::net::UdpOptions`; `udp::bind_udp(SocketAddr, &UdpOptions)` binds through `dvb_stream::udp::bind_udp`, and the new `udp::udp_options(recv_buffer, reuse_addr, interface)` builds the options `--udp-rcvbuf`/`--udp-reuse-addr`/`--udp-interface` describe, with `udp::join_on` for a join's interface.

```rust
// before
let config = UdpConfig { recv_buffer: None, reuse_addr: true, interface: None };
let socket = udp::bind_udp(addr, &config)?;
// after
let options = UdpOptions::new().with_reuse_address(true);
let socket = udp::bind_udp(addr, &options)?;
```

### The CLI helpers are typed

`cli::WatchArgs::udp_config` returns `Result<UdpOptions, broadcast_common::net::InterfaceError>` (was `Result<_, String>`), the error `MulticastInterface::parse` already produces, and the binary renders it next to `--udp-interface`. `cli::WatchArgs::metrics_io_timeout` is a `std::time::Duration` (was `metrics_io_timeout_ms: u64`); the `--metrics-io-timeout-ms` flag still takes whole milliseconds and converts at parse.

```rust
// before
let config: UdpConfig = args.udp_config()?;
let timeout_ms: u64 = args.metrics_io_timeout_ms;
// after
let options: UdpOptions = args.udp_config()?;
let timeout: Duration = args.metrics_io_timeout;
```

### `Scte35Check` reports a malformed cue

The new finding (an `Error` of kind `scte35-malformed`) reports a reassembled `splice_info_section` that does not parse (CRC-32 mismatch, impossible length, invalid field): `SpliceTracker::feed` used to drop such a section silently.

## Fixes

- A socket option that cannot apply to the bind address (a multicast group that is not a group, or an interface of the other address family) is refused with `InvalidInput` before a socket exists, instead of reaching the OS as a malformed join.
- `--udp-interface` also accepts a system interface name (an IPv4 address, an IPv6 interface index, or a name such as `eth0`); a value written like an IPv4 address but not one is still refused at startup, now with the reason naming what was wrong with it.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `broadcast-dash` 0.1 -> 0.2, `broadcast-hls` 0.7 -> 0.8, `container-probe` 0.4 -> 0.5, `dvb-conformance` 14 -> 15, `dvb-si` 14 -> 15, `dvb-stream` 0.9 -> 0.10.
- `mpeg-pes` 0.8 -> 0.9, `mpeg-ts` 0.8 -> 0.9, `scte35-splice` 6 -> 7, `timed-metadata` 0.10 -> 0.11, `transmux` 0.31 -> 0.32.
- The dead optional `dash-runtime` dependency is removed (never enabled by any feature; the DASH validator reads MPDs through `broadcast-dash`).

---

Published from tag `media-doctor-v0.16.0`.
