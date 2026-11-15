# multimux 0.19.0

_Released 2026-11-15._

Minor-class release (0.18 -> 0.19). The origin has one way in (`Server::new(ServeRequest).serve()`), the remaining `f64` seconds and millisecond integers in the runtime config are `Duration` (the JSON keys and units are unchanged), the reconnect policy is the workspace's shared `ReconnectPolicy`, and the push transports' string errors are typed. Several routing and reload bugs are fixed.

Version: 0.18.0 -> 0.19.0.

## Breaking changes and migration

### One entry point: `Server::new(ServeRequest)`

The eight `serve*` functions (`serve`, `serve_config_file`, `serve_config_file_with_registry`, `serve_config_file_with_registry_on_admin`, `serve_config_file_with_registry_on_admin_prebound`, `serve_with_registry`, `serve_with_registry_on`, `serve_with_registry_on_admin`) are removed. `ServeRequest::new(config)` replaces `serve`/`serve_with_registry`, `ServeRequest::from_json_file(path)?` replaces `serve_config_file`/`serve_config_file_with_registry(path, ..)` (it keeps the `POST /admin/reload` path), and `.with_registry(r)` carries what the `_with_registry` variants did. The pre-bound-listener seams are setters on the request (`ServeRequest::with_prebound_media`, `with_prebound_listeners`, `with_prebound_binds`), `test-seams` only.

```rust
// before
multimux::serve(config).await?;
multimux::serve_config_file(path)?;
// after
multimux::Server::new(multimux::ServeRequest::new(config)).serve().await?;
multimux::Server::new(multimux::ServeRequest::from_json_file(path)?).serve().await?;
```

### Seconds are `Duration` (config keys unchanged)

The remaining `f64` seconds in the runtime config are `std::time::Duration`, renamed to drop the now-contradictory `_secs` suffix while every JSON key and unit stays unchanged through a serde adapter: `Config::target_duration_secs`/`request_timeout_secs`/`concurrency_queue_timeout_secs`/`ingest_connect_timeout_secs`/`ingest_read_timeout_secs` are `target_duration`/`request_timeout`/`concurrency_queue`/`ingest_connect_timeout`/`ingest_read_timeout`; `RouteConfig::target_duration_secs`/`RouteHandle::target_duration_secs()` are `target_duration`/`target_duration()`; and `Config::part_target_ms: u32` is `Config::part_target: Duration` (`RouteConfig::part_target`, `RouteHandle::part_target()`, the registry's `InputCtx.part_target`). The setters are `with_target_duration`/`with_request_timeout`/`with_concurrency_queue_timeout`/`with_ingest_connect_timeout`/`with_ingest_read_timeout`/`with_part_target`. A `Duration` cannot be NaN, negative or infinite, so validation now only rejects a zero or an over-bound value.

```rust
// before
let config = Config::default().with_target_duration_secs(4.0).with_request_timeout_secs(30.0);
let part_target = route.part_target_ms();
// after
let config = Config::default().with_target_duration(Duration::from_secs(4)).with_request_timeout(Duration::from_secs(30));
let part_target = route.part_target();   // Duration
```

### `ReconnectPolicy` is `broadcast_common::backoff::ReconnectPolicy`

`config::ReconnectPolicy` (the old `initial_backoff_ms`/`max_backoff_ms` struct) is the shared type: a `backoff: BackoffSteps` (`steps` and `cap`) plus `max_attempts: Option<NonZeroU32>`. The origin supervisor's `Backoff` is gone too (its pacing is `broadcast_common::backoff::RetrySchedule` directly). `ReconnectPolicy::doubling(min, max)` rebuilds the old doubling schedule.

```rust
// before
let policy = ReconnectPolicy { initial_backoff_ms: 500, max_backoff_ms: 30_000 };
// after
let policy = ReconnectPolicy::new(BackoffSteps {
    steps: vec![Duration::from_millis(500), Duration::from_secs(1), Duration::from_secs(2)],
    cap: Duration::from_secs(30),
}).with_max_attempts(NonZeroU32::new(10).unwrap());
```

### The UDP routes take `UdpOptions`

`source::udp::UdpBindOptions` is removed in favour of `broadcast_common::net::UdpOptions`, and `TsUdpRoute::new(name, addr)`/`RtpUdpRoute::new(name, addr, sdp)` no longer take a `multicast_group` (the group is the route's socket options' join). `with_multicast_group` is removed and `with_socket_options` takes `UdpOptions`. `routes.input.multicast_interface` accepts a system interface name as well as an IPv4 address or an IPv6 interface index, and a value that looks like an IPv4 address but is not is a config error at load rather than at the first bind.

```rust
// before
let route = TsUdpRoute::new("bbc", addr, Some(group));
let options = UdpBindOptions { recv_buffer_bytes: Some(4 << 20), reuse_address: true, multicast_interface: Some("eth0".into()), reuse_port: false };
// after
let route = TsUdpRoute::new("bbc", addr)
    .with_socket_options(UdpOptions::new().with_join(MulticastJoin::new(group.parse()?)));
let join = MulticastJoin::new(group).with_interface(MulticastInterface::parse("eth0")?);
let options = UdpOptions::new().with_recv_buffer_bytes(NonZeroU32::new(4 << 20).unwrap()).with_reuse_address(true).with_reuse_port(false).with_join(join);
```

### Typed errors

`MultimuxError::ConfigInvalid { field, reason }` and `DvrError::Config { field, reason }` carry the typed `crate::error::ConfigField` (one variant per config path, with `name()` returning the dotted path); a `broadcast-client` sub-config failure is `ConfigField::RoutesInput` with the sub-field name in `reason`. `MultimuxError::Protocol` carries `phase: Phase` and `kind: ProtocolFailure` (typed status/timeout/source) instead of `phase: &'static str, reason: String`; `Sdp` and `Auth` are tuple variants wrapping `SdpError` and `broadcast_auth::Error`; `OutputKind::build()`/`build_with_playlist_name()` return `Result<Arc<dyn Output>, MultimuxError>`; `DvrConfig::validate()`, `DvrRecorder::feed_si`, `poll_and_persist` and `poll_and_persist_inits` return `DvrError`; and the dead `MultimuxError::Other(String)` is removed. The push transports' errors are typed: `RtmpPushError::Connect`/`Protocol` carry `RtmpConnectError`/`RtmpProtocolError`, `RtspPushError::Connect`/`Protocol`/`SessionLost` carry `RtspConnectError`/`RtspProtocolError`/`rtsp_runtime::Error`, and `push::SendMediaError::Mux` carries `push::MuxError` (`Package(transmux::Error)`, `Flv(transmux::FlvError)`, `Transport(Box<dyn Error + Send + Sync>)`, `NoCarriableTrack`). `PushTransportEgress::new` takes only the transport.

```rust
// before
if let MultimuxError::Protocol { phase, ref reason } = err { eprintln!("{phase}: {reason}"); }
// after
if let MultimuxError::Protocol { phase, kind, .. } = err { eprintln!("{phase:?}: {kind}"); }
```

### The public config types are `#[non_exhaustive]`

`config::Config`, `config::ReconnectPolicy`, `config::AdminSpec`, `config::UdpSocketSpec`, `config::SignedUrlKeySpec`, `dvr::DvrConfig`, `push::RtmpTransportConfig`, `push::SrtTransportConfig`, `push::RtspTransportConfig` and `output::whep::WhepAnswerParams` are `#[non_exhaustive]`; build them with `Default::default()` + `with_*` (or `AdminSpec::new`, `SignedUrlKeySpec::new`, `WhepAnswerParams::new`). `origin::admin::ReloadSummary` is `#[non_exhaustive]` too.

### Every test hook is behind `test-seams`

Every remaining test-only hook (the WHIP/WHEP harnesses, the push `*_for_test` seams, `RouteHandle::first_trunk_for_test` and the rest) is reachable only through `crate::test_seam`, behind `test-seams`/`test-hooks`; the retired `multimux::webrtc_ice` module is folded into it, and `push::srt::srt_url_outcome_for_test` is replaced by `test_seam::parse_srt_url_for_test`.

### The remaining spans are `Duration`/`Timestamp` and rates `Timescale`

The `Srt` input's `latency` is a `Duration` (the JSON key stays `latency_ms` in whole milliseconds) and `SrtRoute::with_latency` takes `Option<Duration>`. `dvr::IndexEntry::start` is a `broadcast_common::Timestamp` and `duration` a `Duration` (the `pN.idx` sidecar keeps its whole-nanosecond keys, so an existing sidecar still parses). `source::TrackInit::clock_rate` is a `Timescale`, and `webrtc_session::resource_url`/the WHIP/WHEP `Location` are parsed `url::Url`s (now absolute), with every `now` handed to `MediaTransport`/`apply_ice_ops` a `broadcast_common::stage::Timestamp`.

```rust
// before
let route = SrtRoute::with_latency_ms(Some(120));
let start_ns = entry.start_pts_ns;
// after
let route = SrtRoute::with_latency(Some(Duration::from_millis(120)));
let start = entry.start;   // broadcast_common::Timestamp
```

## Fixes

- A split-audio rendition's LL-HLS part target is applied to its own rendition: followers carry their rendition index instead of being paired with audio servings by position, so an earlier rendition whose selector matched nothing no longer shifts every later part target one slot.
- A failed reload that restores a changed route keeps every DASH Period the route still listed: continuing from a route no longer drains its retained programs, so a second continuation from the same route sees them too.
- A configured multicast interface of the group's other address family is refused when the route is resolved, instead of failing in the bind with a raw OS error.

## Added

- `source::udp::bind_options`: the `UdpOptions` a route bind asks for (the multicast group as the join, the crate's default `SO_RCVBUF` filled in), validating the join first.
- `dvb-si` is built with its new `legacy-charsets` feature, so KS X 1001 / GB-2312 / Big5 EIT event names keep decoding.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`); `broadcast-auth` 0.7 -> 0.8, `broadcast-client` 0.5 -> 0.6, `broadcast-dash` 0.1 -> 0.2, `broadcast-hls` 0.7 -> 0.8, `broadcast-smooth` 0.1 -> 0.2.
- `container-probe` 0.4 -> 0.5, `dash-runtime` 0.4 -> 0.5, `dvb-conformance` 14 -> 15, `dvb-si` 14 -> 15, `dvb-stream` 0.9 -> 0.10, `hls-runtime` 0.13 -> 0.14, `media-doctor` 0.15 -> 0.16, `media-plane` 0.11 -> 0.12.
- `mpeg-ts` 0.8 -> 0.9, `rtmp-runtime` 0.10 -> 0.11, `rtp-packet` 0.7 -> 0.8, `rtsp-runtime` 0.11 -> 0.12, `scte35-splice` 6 -> 7, `smooth-runtime` 0.1 -> 0.2, `srt-runtime` 0.9 -> 0.10, `timed-metadata` 0.10 -> 0.11, `transmux` 0.31 -> 0.32, `webrtc-runtime` 0.7 -> 0.8.
- `tower-http` 0.6 -> 0.7 (timeout, limit, cors, gzip compression and decompression layers; no behaviour change); `rtsp-types` follows `rtsp-runtime` to 0.2.

---

Published from tag `multimux-v0.19.0`.
