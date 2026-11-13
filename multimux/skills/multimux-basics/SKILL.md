---
name: multimux-basics
description: Use when writing Rust code that embeds the multimux crate as a live media origin (configuring routes in JSON, serving LL-HLS/DASH/LL-DASH/Smooth/TS-HLS/catch-up, the runtime admin API, the scheme plugin registry), or when running the multimux-cli binary pointed at a config file.
---

# multimux basics

`multimux` is a multi-input, multi-output **just-in-time repackaging HTTP
origin** (tokio + axum). One process runs many independent *routes*: each route
ingests one live source, re-segments it once, and serves it as any combination
of delivery protocols. It is a thin host: every protocol state machine lives in
the runtime crates this one builds on.

## Scope

**It does:** ingest over `rtsp`, `rtp`, `ts_udp`, `ts_http`, `srt`, `hls_pull`,
`dash_pull`, `smooth_pull`, `rtmp`, `dashif_ingest`, `whip` (feature) and
`file`; serve `llhls`, `dash`, `ll_dash`, `smooth`, `ts_hls`, `catchup` and
push re-egress (`srt_push`/`rtmp_push`/`rtsp_push`) plus `whep` (feature);
one shared output credential across every route; a runtime admin API;
durable DVR segments; a plugin registry for third-party schemes.

**It does not:** transcode (samples are opaque), do per-viewer sessions,
server-side ad insertion, ABR ladders, or serve fMP4 and classic TS from one
route (the `Trunk` has one segment ring per program: `ts_hls` is mutually
exclusive with `llhls`/`dash`/`ll_dash`/`smooth`, enforced by
`Config::validate`). The crate is `std` only.

## Configuring routes

A config is JSON: a handful of process-wide fields plus `routes[]`. Every
field except `routes` has a default from `Config::default()`; a route's
`outputs` defaults to `["llhls"]`.

```json
{
  "bind": "0.0.0.0:8080",
  "output_auth": { "scheme": "bearer", "token": "viewer-secret" },
  "routes": [
    { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" }, "outputs": ["llhls", "dash"] },
    { "name": "ingest", "input": { "type": "rtmp", "listen": "0.0.0.0:1935", "app": "live", "stream_key": "s3cret" },
      "outputs": ["llhls", { "srt_push": { "url": "srt://relay.example:9000" } }] },
    { "name": "slate", "input": { "type": "file", "path": "slate.ts" }, "outputs": ["ts_hls"] }
  ]
}
```
(`multimux/README.md`, "Configuration examples"; `broadcast-origin.json` in
the same directory is the eight-source, seven-transport config with
signed-URL egress and the admin API, and `broadcast-origin.md` walks through
it.)

Load and validate — **always both**, `from_json_file` does not validate:

```rust
use multimux::config::{Config, InputSpec};

let config = Config::from_json_file(std::path::Path::new("examples/multi-output.json")).unwrap();
config.validate().unwrap();
```
(`multimux/README.md`, "Quick start", same lines; loads the
`multimux/examples/multi-output.json` config)

`Config`/`Route` are plain exhaustive structs with `pub` fields, so
in-code construction assigns fields or deserializes JSON:

```rust
let config: Config = serde_json::from_str(json).expect("valid JSON");
    config
        .validate()
        .expect("a Custom input is always structurally valid");
    match &config.routes[0].input {
        InputSpec::Custom { type_tag, .. } => assert_eq!(type_tag, "demo"),
        other => panic!("expected InputSpec::Custom, got {other:?}"),
}
```
(`multimux/examples/custom_scheme.rs`, `main`; there is **no** `Route::with_*`
builder — `outputs`, `dvr`, `ts_hls_cut`, `subtitles` and friends are struct
fields, not builder methods. The only `with_*` builders in this crate are
`RouteConfig::with_*`/`RouteHandle::with_*` in `multimux/src/route.rs`, plus
`DvrConfig` field assignment in `multimux/src/dvr.rs`.)

- **Ingest credentials** ride on the input: `"auth": { "username", "password" }`
  or `{ "bearer_token" }` on `rtsp`/`ts_http`/`hls_pull`/`dash_pull`/
  `smooth_pull` (`InputSpec` in `multimux/src/config.rs`).
- **Output auth** is one `output_auth` for every route and every output
  (`OutputAuthSpec`: `basic`, `digest`, `bearer`, `forwarded`, `signed_url`,
  `custom`). `null` (the default) leaves everything open.
- **DVR / catch-up**: a route needs `"dvr": { "enabled": true, "archive_root":
  …, "retention_periods": N }` (at least one retention axis) **and**
  `"catchup"` in `outputs`; `Config::validate` rejects the mix otherwise.
  `"overrun"` (default `"gap"`) decides what happens when the live ring
  outruns the recorder.

## Embedding it as a library

`serve` blocks for the life of the origin and installs Ctrl-C/SIGTERM
graceful shutdown itself. Call it from `#[tokio::main]`:

```rust
    let rtsp_url = args
        .first()
        .cloned()
        .expect("usage: serve_rtsp <rtsp://host/stream> [--dash | --outputs llhls,dash]");
    let outputs = parse_outputs(&args[1..]);

    let config = Config {
        routes: vec![Route {
            name: STREAM_NAME.to_string(),
            input: InputSpec::Rtsp {
                url: rtsp_url,
                auth: None,
            },
            outputs,
            dvr: DvrConfig::default(),
            ts_hls_cut: Default::default(),
            ts_hls_idr_fallback: Default::default(),
            declared_bandwidth_bps: None,
            audio_renditions: Vec::new(),
            audio_rendition_sources: Vec::new(),
            closed_caption_overrides: Vec::new(),
            audio_profiles: Vec::new(),
            subtitles: true,
        }],
        ..Config::default()
    };
    config.validate().expect("valid config");

    println!(
        "multimux serve_rtsp: http://{}/{STREAM_NAME}/master.m3u8",
        config.bind
    );
    // `OutputKind` no longer derives `PartialEq` (its `Custom` variant
    // carries a `serde_json::Value`), so check by pattern rather than
    // `contains`.
    if config.routes[0]
        .outputs
        .iter()
        .any(|k| matches!(k, OutputKind::Dash))
    {
        println!(
            "multimux serve_rtsp: http://{}/{STREAM_NAME}/manifest.mpd",
            config.bind
        );
    }

    // SNIPPET-BEGIN
    multimux::origin::serve(config)
        .await
        .expect("origin server");
```
(`multimux/examples/serve_rtsp.rs`, `main` between its SNIPPET markers, verbatim)

Choosing the entry point:

| Call | Use when |
|---|---|
| `multimux::serve(config)` | in-memory config, built-in schemes only |
| `multimux::serve_with_registry(config, registry)` | in-memory config, custom schemes registered |
| `multimux::serve_config_file(path)` | config on disk (needed for `POST /admin/reload`) |
| `multimux::serve_config_file_with_registry(path, registry)` | both |

All four are re-exported at the crate root (`multimux/src/lib.rs`) and defined
in `multimux/src/origin/mod.rs`.

**Admin API.** Opt-in via `"admin": { "bind", "auth" }` in the config — the
listener must differ from `bind` and `auth` is mandatory (without it the
config fails to *parse*). Endpoints on that listener: `GET /admin/routes`,
`GET /admin/routes/{name}`, `POST /admin/routes` (JSON `Route`; `409` on a
duplicate name), `DELETE /admin/routes/{name}`, `POST /admin/reload` (only
works when the origin knows its config file path). Each route's status is a
`RouteStatus` (`multimux/src/origin/admin.rs`): `name`, `input_kind`,
`outputs`, `health`, `output_error`, `ts_hls_idr`, `created_at_unix_nanos`.

```bash
curl -u admin:hunter2 http://127.0.0.1:9090/admin/routes
curl -X POST -H 'Content-Type: application/json' \
  -d '{"name":"cam41","input":{"type":"rtsp","url":"rtsp://cam41.local/stream"}}' \
  http://127.0.0.1:9090/admin/routes
curl -X DELETE http://127.0.0.1:9090/admin/routes/cam41
curl -X POST http://127.0.0.1:9090/admin/reload
```
(`multimux/README.md`, "Runtime admin API")

**Operability** is on the media listener itself, never behind `output_auth`:
`GET /healthz` (always `200`), `GET /readyz` (`200` once a route is live),
`GET /metrics` (Prometheus text). Metrics render through
`multimux::prometheus::install()`, which is idempotent
(`multimux/src/prometheus.rs`).

## multimux-cli quick start

The binary is the separate `multimux-cli` crate. Its flags reach a single RTSP
route; everything else goes through `--config`:

```bash
multimux --rtsp rtsp://cam.local/stream --name cam1     # one camera
curl http://0.0.0.0:8080/cam1/master.m3u8
multimux --config routes.json                           # many routes
```
(`multimux/README.md`, "Quick start"; `multimux-cli/src/main.rs` module doc
has the same shape plus the `--srt-push`/`--rtmp-push`/`--rtsp-push`
shorthands.)

## Custom scheme plugin registry

A third-party crate adds an input, output or output-auth scheme by name,
resolved from JSON `{"type": "custom", "type_tag": …, "params": …}`, without
editing multimux. Build a `SchemeRegistry`, register a factory, pass it to
`serve_with_registry`. An input factory spawns `supervise_driver` itself:

```rust
fn build_registry() -> SchemeRegistry {
    let mut registry = SchemeRegistry::new();
    registry.register_input(
        "demo",
        Arc::new(|ctx: InputCtx| {
            Ok(tokio::spawn(supervise_driver(
                run_demo,
                ctx.store,
                Backoff::production_default(),
                ctx.name,
                ctx.cancel,
            )))
        }),
    );
    registry
}
```
(`multimux/examples/custom_scheme.rs`, `build_registry`)

Inside the attempt closure, `multimux::source::advance_route` is the **one**
call a custom source makes after each driver step — it publishes each newly
announced program's `Trunk` into the route and turns samples into servable
segments:

```rust
async fn run_demo(route_handle: Arc<RouteHandle>) -> multimux::Result<()> {
    // SNIPPET-BEGIN
    /* dial, wrap the session in an IngestDriver, feed it … */
    let trunk_config = TrunkConfig::default()
        .with_timed_capacity(nz(64))
        .with_sparse_capacity(nz(16))
        .with_segment_capacity(nz(8))
        .with_event_capacity(nz(64))
        .with_part_capacity(nz(64));
    // A generous handshake bound; a real scheme would derive this from its
    // own configured connect timeout.
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let mut driver: IngestDriver<DemoSession> = IngestDriver::new(
        session,
        media_plane::IngestConfig::new(handshake).with_trunk(trunk_config),
    );

    let mut progress = DriverProgress::new();

    // One feed: announces NewProgram AND queues every sample — mints the
    // driver-side Trunk and publishes its samples in the same batch.
    // `advance_route`'s own segmenting step subscribes via
    // `subscribe_from_backlog` (issue #808), which replays whatever backlog
    // is already resident in the ring rather than starting from "now" — so
    // this single feed's samples are not lost.
    driver.feed(&[], Timestamp::from_nanos(0));
    advance_route(&driver, &route_handle, &mut progress).await;
    // SNIPPET-END

    // Nothing more to send: end the session cleanly and flush the trailing
    // buffered partial segment, exactly as a real scheme does on a clean
    // disconnect.
    driver.finish();
    advance_route(&driver, &route_handle, &mut progress).await;

    Ok(())
}
```
(`multimux/examples/custom_scheme.rs`, `run_demo`, from its first line to the
SNIPPET-END marker; the marked block is what a custom source must keep — read
the whole file, it drives a real `Dialer`/`IngestSession` pair to servable
bytes)

`OutputFactory` and `AuthFactory` (`multimux/src/registry.rs`) are the other
two registration points, via `register_output`/`register_auth`.

`OutputFactory` and `AuthFactory` (`multimux/src/registry.rs`) are the other
two registration points, via `register_output`/`register_auth`.

## Features

| Feature | Default | Effect |
|---|---|---|
| `tls` | on | `rtsps://` sources (rustls via `rtsp-runtime`) |
| `whip` | off | WHIP ingest; pulls the ICE/DTLS-SRTP tree (`webrtc-runtime` `media`) |
| `whep` | off | WHEP viewer egress (independent of `whip`) |
| `test-seams` / `test-hooks` | off | pre-bound listeners / WebRTC fault injection; not public API |

The crate is `std`-only — there is no `no_std` build. Default features are
enough for HTTP outputs; add `whip`/`whep` only if those transports are
wanted.

## Pitfalls

- **`from_json_file` parses, it does not validate.** Call `config.validate()`
  and propagate the error: it is where the secure-by-default rules live (an
  unauthenticated ingest listener on a non-loopback address needs
  `"allow_open_ingest": true`; `admin.bind` must differ from `bind`;
  `request_timeout_secs` must exceed 5.0; `ts_hls` cannot mix with the fMP4
  outputs; `catchup` needs `dvr.enabled`).
- **There is no `Route::with_*` builder.** `Config` and `Route` are
  exhaustive `pub`-field structs; set fields or deserialize. `Route` grew
  fields across releases, so build it from a JSON/`Config::default()` source
  rather than a bare struct literal.
- **`Custom` schemes need `serve_with_registry`.** Under plain `serve` (empty
  registry) an unregistered `type_tag` fails route setup with
  `MultimuxError::UnknownScheme` — it does not fall back to a built-in.
- **`POST /admin/reload` needs a config file path.** An origin started from
  `serve` rejects reload with a clear error; add/remove/list still work.
- **A custom source that skips `advance_route` serves nothing.** Publishing the
  program and segmenting samples are one call; doing neither leaves the route
  permanently `503` with nothing servable.
- **One container per route.** `ts_hls` and the fMP4 outputs cannot share a
  route; run two routes against one source if both are needed.
- **`serve` owns the shutdown signal.** It installs its own Ctrl-C/SIGTERM
  handler and returns only on bind failure or fatal server error, so call it
  last (from `#[tokio::main]`) rather than wrapping it in a `select!` that
  expects it to return.
- **`ts_hls_cut`/`ts_hls_idr_fallback` are `ts_hls`-only.** They are accepted
  on every route and simply ignored by the fMP4 outputs; setting them does
  not change LL-HLS/DASH segmenting.
