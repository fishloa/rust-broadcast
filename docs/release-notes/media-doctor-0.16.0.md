# media-doctor 0.16.0

_Released 2026-11-15._

### Added
- `Scte35Check` reports a reassembled `splice_info_section` that does not parse (CRC-32 mismatch, impossible length, invalid field) as a `scte35-malformed` Error finding: `SpliceTracker::feed` used to drop such a section silently.

### Dependencies
- `scte35-splice` 6 -> 7: `SpliceTime::pts_time` is a validated `broadcast_common::clock33::Clock33`, and `SpliceInsert` carries its flags and times in one `SpliceMode` (see that crate's changelog).

### Changed
- BREAKING: `cli::WatchArgs::udp_config` returns `Result<UdpOptions, broadcast_common::net::InterfaceError>` (was `Result<_, String>`), the error `MulticastInterface::parse` already produces; the binary renders it next to `--udp-interface`. A caller matching on the message text now matches the typed variant (#2695).
- BREAKING: `cli::WatchArgs::metrics_io_timeout` is a `std::time::Duration` (was `metrics_io_timeout_ms: u64`); the `--metrics-io-timeout-ms` flag still takes whole milliseconds and is converted at parse (#2693).
### Changed
- BREAKING: `udp::UdpConfig` and `udp::MulticastInterface` are removed in favour of `broadcast_common::net::UdpOptions`; `udp::bind_udp(SocketAddr, &UdpOptions)` binds through `dvb_stream::udp::bind_udp`, and the new `udp::udp_options(recv_buffer, reuse_addr, interface)` builds the options `--udp-rcvbuf`/`--udp-reuse-addr`/`--udp-interface` describe, with `udp::join_on` for a join's interface. Migration: `UdpConfig { recv_buffer, reuse_addr, interface }` becomes `udp_options(recv_buffer, reuse_addr, interface)`; `recv_buffer: None, reuse_addr: true` becomes `UdpOptions::new().with_reuse_address(true)` (#2663).
- `--udp-interface` now also accepts a system interface name (an IPv4 address, an IPv6 interface index, or a name such as `eth0`); a value written like an IPv4 address but not one is still refused at startup, now with the reason naming what was wrong with it.
- BREAKING: `Location::pid: u32` (one field carrying either a 13-bit TS PID or a 32-bit track id, with `0` for "none") becomes `Location::stream: StreamLocator`, the new `Ts(Pid) | Track(TrackId) | Unknown` enum; `Location::new(packet)` takes no stream id, and `at_pid`/`at_track` keep the typed location. Reports still print the bare number (`pid=0x0100`, `0` when unknown) and JSON still carries the bare integer under the `pid` key, so existing goldens are unchanged (#2650).
### Fixed
- A socket option that cannot apply to the bind address (a multicast group that is not a group, or an interface of the other address family) is refused with `InvalidInput` before a socket exists, instead of reaching the OS as a malformed join (#2663).
- **BREAKING**: `cli` is no longer a default feature. `default = ["std"]`; the
  binary declares `required-features = ["cli"]`, so a plain dependency on the
  library no longer compiles `clap`, `hyper`, `tokio` or the Prometheus
  exporter. Install the binary with `cargo install media-doctor --features cli`
  (previously the bare `cargo install media-doctor`).
- Removed the dead optional `dash-runtime` dependency (never enabled by any
  feature; the DASH validator reads MPDs through `broadcast-dash`).

---

Published from tag `media-doctor-v0.16.0`.
