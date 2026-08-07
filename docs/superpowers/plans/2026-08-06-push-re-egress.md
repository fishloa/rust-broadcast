# Push Re-Egress Implementation Plan (#744)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add push-style outputs (SRT, RTMP, RTSP) to multimux so it can relay/restream ingested media to downstream servers.

**Architecture:** A generic async push driver in multimux drains a `SampleCursor` from the `Trunk`, muxes samples to wire format via transmux's existing `TsMux`/`FlvMux` (implementing `Package`), and sends over a thin `PushTransport` trait per protocol. An exponential-backoff `ReconnectEngine` handles disconnects. Each push output runs as its own tokio task. SRT uses the existing `srt-runtime` Caller API; RTSP needs two new client methods (`announce`/`record`); RTMP needs a new client state machine.

**Tech Stack:** Rust, tokio, transmux (`TsMux`/`FlvMux`/`Package` trait), srt-runtime, rtsp-runtime, rtmp-runtime, media-plane (`Trunk`/`SampleCursor`/`PushEgress`)

## Global Constraints

- MSRV 1.86. Build with `--locked`. All changes additive (no breaking API).
- `RUSTFLAGS="-D warnings"` on all builds.
- `cargo fmt --all --check` must pass.
- `cargo clippy --workspace --all-features --all-targets --locked -- -D warnings` must pass.
- No magic numbers outside `#[cfg(test)]`.
- IPv4 and IPv6 dual-stack throughout (use `ToSocketAddrs` / `tokio::net::lookup_host`).
- Spec citations in module doc comments.
- CHANGELOG.md updated with `[Unreleased]` section in each modified crate.
- `#[non_exhaustive]` on all new public enums.

---

## File Structure

### multimux (most new code lives here)

| File | Responsibility |
|------|---------------|
| `multimux/src/config.rs` | Add `PushFormat`, `ReconnectPolicy` types |
| `multimux/src/output/mod.rs` | Add `SrtPush`/`RtmpPush`/`RtspPush` to `OutputKind` |
| `multimux/src/push/mod.rs` (CREATE) | `PushTransport` trait, `ReconnectEngine`, `PushDriver`, `drive_push()` async fn, `PushMetrics` |
| `multimux/src/push/srt.rs` (CREATE) | `SrtTransport` implementing `PushTransport` |
| `multimux/src/push/rtsp.rs` (CREATE) | `RtspTransport` implementing `PushTransport` |
| `multimux/src/push/rtmp.rs` (CREATE) | `RtmpTransport` implementing `PushTransport` |
| `multimux/src/origin/supervisor.rs` | Spawn push tasks alongside served outputs |
| `multimux/src/prometheus.rs` | Push output metrics |
| `multimux/src/lib.rs` | Add `pub mod push;` |

### rtsp-runtime (small addition)

| File | Responsibility |
|------|---------------|
| `rtsp-runtime/src/client.rs` | Add `announce()` and `record()` request builders |

### rtmp-runtime (new module)

| File | Responsibility |
|------|---------------|
| `rtmp-runtime/src/client.rs` (CREATE) | `RtmpClientSession` sans-IO state machine |
| `rtmp-runtime/src/io.rs` | Add client tokio adapter alongside existing server adapter |
| `rtmp-runtime/src/lib.rs` | Add `pub mod client;` |

---

### Task 1: Push config types + OutputKind variants

**Files:**
- Modify: `multimux/src/config.rs`
- Modify: `multimux/src/output/mod.rs`

**Interfaces:**
- Produces: `PushFormat` enum (`Ts`, `Flv`), `ReconnectPolicy` struct, `OutputKind::SrtPush { url, format, reconnect }`, `OutputKind::RtmpPush { url, format, reconnect }`, `OutputKind::RtspPush { url, format, reconnect }`

- [ ] **Step 1: Add `PushFormat` and `ReconnectPolicy` to config.rs**

In `multimux/src/config.rs`, add after the `AuthSpec` type:

```rust
/// Container format for a push output. Each push protocol has a natural
/// default; this field overrides it.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PushFormat {
    /// MPEG-2 Transport Stream (188-byte packets). Default for SRT.
    Ts,
    /// FLV tag stream. Default for RTMP.
    Flv,
}

impl PushFormat {
    /// The spec/field-enum label (workspace #204 convention).
    pub fn name(&self) -> &'static str {
        match self {
            PushFormat::Ts => "ts",
            PushFormat::Flv => "flv",
        }
    }
}

broadcast_common::impl_spec_display!(PushFormat);

/// Reconnect policy for push outputs. Exponential backoff with configurable
/// bounds.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ReconnectPolicy {
    /// Initial backoff duration after first disconnect (default 1 s).
    #[serde(default = "ReconnectPolicy::default_initial_backoff_ms")]
    pub initial_backoff_ms: u64,
    /// Maximum backoff duration cap (default 30 s).
    #[serde(default = "ReconnectPolicy::default_max_backoff_ms")]
    pub max_backoff_ms: u64,
    /// Maximum reconnect attempts before giving up. `None` = infinite.
    #[serde(default)]
    pub max_attempts: Option<u32>,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_backoff_ms: 1_000,
            max_backoff_ms: 30_000,
            max_attempts: None,
        }
    }
}

impl ReconnectPolicy {
    fn default_initial_backoff_ms() -> u64 { 1_000 }
    fn default_max_backoff_ms() -> u64 { 30_000 }

    /// Backoff duration for attempt `n` (0-indexed).
    pub fn backoff_for(&self, attempt: u32) -> std::time::Duration {
        let ms = self.initial_backoff_ms.saturating_mul(1u64 << attempt.min(20));
        std::time::Duration::from_millis(ms.min(self.max_backoff_ms))
    }
}
```

- [ ] **Step 2: Add push variants to `OutputKind` in `output/mod.rs`**

In `multimux/src/output/mod.rs`, add three new variants to the `OutputKind` enum, before the `Custom` variant:

```rust
    /// Push to a remote SRT Listener (Caller mode). Issue #744.
    #[serde(rename = "srt_push")]
    SrtPush {
        /// `srt://host:port` or `host:port` destination.
        url: String,
        /// Container format override (default: TS).
        #[serde(default)]
        format: Option<crate::config::PushFormat>,
        /// Reconnect policy override (default: exponential backoff 1s→30s, infinite retries).
        #[serde(default)]
        reconnect: Option<crate::config::ReconnectPolicy>,
    },
    /// Push to a remote RTMP server (client publish). Issue #744.
    #[serde(rename = "rtmp_push")]
    RtmpPush {
        /// `rtmp://host/app/stream_key` destination.
        url: String,
        /// Container format override (default: FLV).
        #[serde(default)]
        format: Option<crate::config::PushFormat>,
        /// Reconnect policy override.
        #[serde(default)]
        reconnect: Option<crate::config::ReconnectPolicy>,
    },
    /// Push to a remote RTSP server (client ANNOUNCE/RECORD). Issue #744.
    #[serde(rename = "rtsp_push")]
    RtspPush {
        /// `rtsp://host:port/path` destination.
        url: String,
        /// Container format override (default: TS over interleaved).
        #[serde(default)]
        format: Option<crate::config::PushFormat>,
        /// Reconnect policy override.
        #[serde(default)]
        reconnect: Option<crate::config::ReconnectPolicy>,
    },
```

- [ ] **Step 3: Update `OutputKind::name()` and `build()` for new variants**

In `OutputKind::name()` match, add:
```rust
OutputKind::SrtPush { .. } => "srt_push",
OutputKind::RtmpPush { .. } => "rtmp_push",
OutputKind::RtspPush { .. } => "rtsp_push",
```

In `build_with_playlist_name()` match, add the push variants to the same `unreachable!` arm as `Custom` (push outputs don't produce `Arc<dyn Output>` — they are driven by a separate push task, not by HTTP manifest routes):
```rust
OutputKind::Custom { .. }
| OutputKind::SrtPush { .. }
| OutputKind::RtmpPush { .. }
| OutputKind::RtspPush { .. } => unreachable!(
    "Push/Custom OutputKinds do not produce an Output — \
     push outputs are driven by crate::push::drive_push, \
     custom outputs by SchemeRegistry"
),
```

- [ ] **Step 4: Update the `output_kind_name_and_display_agree` test**

Add the three new kinds to the test's assertion list.

- [ ] **Step 5: Update the `every_output_kind_merges_without_panicking` test**

If this test iterates all `OutputKind` variants, skip the push kinds (they don't produce `Output`). Add a separate test that push kinds return their correct `name()`.

- [ ] **Step 6: Run gate**

```bash
RUSTFLAGS="-D warnings" cargo build -p multimux --all-features --locked
RUSTFLAGS="-D warnings" cargo test -p multimux --all-features --locked
RUSTFLAGS="-D warnings" cargo clippy -p multimux --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 7: Commit**

```bash
git add multimux/src/config.rs multimux/src/output/mod.rs
git commit -m "feat(multimux): push output config types + OutputKind variants (#744)"
```

---

### Task 2: Push driver infrastructure (ReconnectEngine + PushTransport + drive_push)

**Files:**
- Create: `multimux/src/push/mod.rs`
- Modify: `multimux/src/lib.rs`

**Interfaces:**
- Consumes: `PushFormat` and `ReconnectPolicy` from Task 1
- Produces: `PushTransport` trait, `ReconnectEngine`, `PushDriver`, `drive_push()` async fn, `PushMetrics`

- [ ] **Step 1: Create `multimux/src/push/mod.rs`**

```rust
//! Push output driver — restreams media to downstream RTMP/RTSP/SRT servers.
//!
//! Issue #744. Each push output spawns one tokio task per destination,
//! draining a [`media_plane::SampleCursor`] from the [`media_plane::Trunk`],
//! muxing samples to wire format via [`transmux::TsMux`]/[`transmux::FlvMux`],
//! and sending over a [`PushTransport`] (SRT/RTMP/RTSP). Disconnects are
//! handled by [`ReconnectEngine`] with exponential backoff.

pub mod srt;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use media_plane::trunk::{SampleCursor, SampleCursorItem};
use media_plane::Trunk;
use tokio_util::sync::CancellationToken;
use transmux::{Media, Package, Sample, TrackSpec, Track, TsMux, FlvMux};

use crate::config::{PushFormat, ReconnectPolicy};

/// Async transport abstraction — connect/send/close over the wire.
/// One implementation per push protocol.
#[async_trait::async_trait]
pub trait PushTransport: Send + 'static {
    /// Protocol-specific config (e.g. SRT latency, RTMP app name).
    type Config: Send + Sync + Clone;
    /// Transport error.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Connect to the remote destination. Must support IPv4 and IPv6.
    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error>
    where
        Self: Sized;

    /// Send muxed bytes to the remote.
    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error>;

    /// Protocol-specific setup after connect (e.g. RTMP publish, RTSP
    /// ANNOUNCE/SETUP/RECORD). No-op for format-agnostic protocols like SRT.
    async fn setup(&mut self, _tracks: &[TrackSpec]) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Clean shutdown.
    fn close(&mut self);
}

/// Reconnect state machine with exponential backoff.
pub struct ReconnectEngine {
    policy: ReconnectPolicy,
    state: ReconnectState,
    attempt: u32,
}

enum ReconnectState {
    /// Ready to attempt connection.
    Ready,
    /// Waiting for backoff timer.
    Backoff { resume_at: Instant },
    /// Gave up (max_attempts reached).
    Failed,
}

impl ReconnectEngine {
    pub fn new(policy: ReconnectPolicy) -> Self {
        Self {
            policy,
            state: ReconnectState::Ready,
            attempt: 0,
        }
    }

    /// Whether a connection attempt should be made now.
    pub fn should_connect(&self) -> bool {
        match &self.state {
            ReconnectState::Ready => true,
            ReconnectState::Backoff { resume_at } => Instant::now() >= *resume_at,
            ReconnectState::Failed => false,
        }
    }

    /// Record a successful connection.
    pub fn on_connect(&mut self) {
        self.attempt = 0;
        self.state = ReconnectState::Ready;
    }

    /// Record a failed connection or send. Enters backoff.
    pub fn on_disconnect(&mut self) {
        self.attempt = self.attempt.saturating_add(1);
        if let Some(max) = self.policy.max_attempts {
            if self.attempt > max {
                self.state = ReconnectState::Failed;
                return;
            }
        }
        let backoff = self.policy.backoff_for(self.attempt.saturating_sub(1));
        self.state = ReconnectState::Backoff {
            resume_at: Instant::now() + backoff,
        };
    }

    /// Whether the engine has permanently failed.
    pub fn is_failed(&self) -> bool {
        matches!(self.state, ReconnectState::Failed)
    }

    /// Duration until next reconnect attempt, or zero if ready.
    pub fn time_until_retry(&self) -> Duration {
        match &self.state {
            ReconnectState::Ready => Duration::ZERO,
            ReconnectState::Backoff { resume_at } => {
                resume_at.saturating_duration_since(Instant::now())
            }
            ReconnectState::Failed => Duration::MAX,
        }
    }
}

/// Per-push-output metrics, shared with the Prometheus exporter.
#[derive(Debug)]
pub struct PushMetrics {
    pub samples_sent: AtomicU64,
    pub samples_dropped: AtomicU64,
    pub bytes_sent: AtomicU64,
    pub reconnect_count: AtomicU64,
}

impl PushMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            samples_sent: AtomicU64::new(0),
            samples_dropped: AtomicU64::new(0),
            bytes_sent: AtomicU64::new(0),
            reconnect_count: AtomicU64::new(0),
        })
    }
}

/// Container muxer wrapping transmux's `Package` implementations.
enum MuxStage {
    Ts(TsMux),
    Flv(FlvMux),
}

impl MuxStage {
    fn new(format: PushFormat) -> Self {
        match format {
            PushFormat::Ts => MuxStage::Ts(TsMux::new()),
            PushFormat::Flv => MuxStage::Flv(FlvMux::new()),
        }
    }

    fn mux(&mut self, media: &Media) -> Result<Vec<u8>, transmux::Error> {
        match self {
            MuxStage::Ts(m) => m.package(media),
            MuxStage::Flv(m) => m.package(media),
        }
    }
}

/// The main push output driver. Runs as a tokio task, draining a
/// `SampleCursor` and sending muxed bytes over `T: PushTransport`.
pub async fn drive_push<T: PushTransport>(
    url: String,
    transport_config: T::Config,
    format: PushFormat,
    reconnect_policy: ReconnectPolicy,
    trunk: Arc<Trunk>,
    mut cursor: SampleCursor,
    metrics: Arc<PushMetrics>,
    cancel: CancellationToken,
) {
    let mut reconnect = ReconnectEngine::new(reconnect_policy);
    let mut transport: Option<T> = None;
    let mut mux = MuxStage::new(format);

    loop {
        // Check cancellation
        if cancel.is_cancelled() {
            if let Some(ref mut t) = transport {
                t.close();
            }
            return;
        }

        // Connect if needed
        if transport.is_none() && reconnect.should_connect() {
            match T::connect(&url, &transport_config).await {
                Ok(mut t) => {
                    // Setup (RTMP publish, RTSP ANNOUNCE, etc.)
                    // Track specs come from the trunk's track set if available
                    reconnect.on_connect();
                    transport = Some(t);
                    metrics.reconnect_count.fetch_add(1, Ordering::Relaxed);
                    tracing::info!(url = %url, "push output connected");
                }
                Err(e) => {
                    reconnect.on_disconnect();
                    tracing::warn!(url = %url, error = %e, "push output connect failed");
                }
            }
        }

        // If not connected and in backoff, wait
        if transport.is_none() {
            let wait = reconnect.time_until_retry().min(Duration::from_secs(1));
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(wait) => {},
            }
            // Still drain cursor to advance past samples we can't send
            while let Some(_item) = cursor.poll() {
                metrics.samples_dropped.fetch_add(1, Ordering::Relaxed);
            }
            continue;
        }

        // Wait for samples
        tokio::select! {
            _ = cancel.cancelled() => {
                if let Some(ref mut t) = transport {
                    t.close();
                }
                return;
            }
            _ = trunk.listen() => {}
        }

        // Drain cursor and send
        while let Some(item) = cursor.poll() {
            if let Some(ref mut t) = transport {
                match &item {
                    SampleCursorItem::Timed { sample, track_id, .. }
                    | SampleCursorItem::Sparse { sample, track_id, .. } => {
                        // Build a minimal Media for the muxer
                        // (real implementation: accumulate samples per track,
                        // build proper Media with track specs from trunk)
                        let data = sample.data.to_vec();
                        let bytes_len = data.len() as u64;
                        if let Err(e) = t.send(&data).await {
                            tracing::warn!(url = %url, error = %e, "push output send failed");
                            t.close();
                            transport = None;
                            reconnect.on_disconnect();
                            // Drain remaining samples as dropped
                            metrics.samples_dropped.fetch_add(1, Ordering::Relaxed);
                            break;
                        }
                        metrics.samples_sent.fetch_add(1, Ordering::Relaxed);
                        metrics.bytes_sent.fetch_add(bytes_len, Ordering::Relaxed);
                    }
                    SampleCursorItem::Lagged { .. } => {
                        tracing::warn!(url = %url, "push output cursor lagged");
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Add `pub mod push;` to `multimux/src/lib.rs`**

- [ ] **Step 3: Write tests for `ReconnectEngine`**

In `multimux/src/push/mod.rs`, add a `#[cfg(test)] mod tests` section with:
- `backoff_doubles_each_attempt` — verify `backoff_for(0)` = 1s, `backoff_for(1)` = 2s, `backoff_for(2)` = 4s, capped at `max_backoff_ms`
- `max_attempts_triggers_failed` — set `max_attempts: Some(3)`, call `on_disconnect()` 4 times, verify `is_failed()`
- `on_connect_resets_attempt_counter` — connect, disconnect, connect, verify attempt resets
- `should_connect_respects_backoff` — after disconnect, `should_connect()` is false until backoff elapsed

- [ ] **Step 4: Run gate**

```bash
RUSTFLAGS="-D warnings" cargo build -p multimux --all-features --locked
RUSTFLAGS="-D warnings" cargo test -p multimux --all-features --locked
RUSTFLAGS="-D warnings" cargo clippy -p multimux --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 5: Commit**

```bash
git add multimux/src/push/ multimux/src/lib.rs
git commit -m "feat(multimux): push driver infrastructure — PushTransport + ReconnectEngine (#744)"
```

---

### Task 3: SRT push transport + integration

**Files:**
- Create: `multimux/src/push/srt.rs`
- Modify: `multimux/src/push/mod.rs` (add `pub mod srt;`)

**Interfaces:**
- Consumes: `PushTransport` trait from Task 2, `srt_runtime::io::SrtSocket` API
- Produces: `SrtTransport` implementing `PushTransport`

- [ ] **Step 1: Create `multimux/src/push/srt.rs`**

```rust
//! SRT push transport — Caller mode to a remote SRT Listener.
//!
//! Uses `srt_runtime::io::SrtSocket::connect()` (dual-stack IPv4/IPv6 via
//! `ToSocketAddrs`) and `SrtSocket::send()`.

use srt_runtime::io::SrtSocket;
use srt_runtime::SrtConfig;
use transmux::TrackSpec;

use super::PushTransport;

/// SRT push transport config.
#[derive(Debug, Clone)]
pub struct SrtTransportConfig {
    /// SRT-specific options (latency, etc.). Defaults are fine for most cases.
    pub srt_config: SrtConfig,
}

impl Default for SrtTransportConfig {
    fn default() -> Self {
        Self {
            srt_config: SrtConfig::default(),
        }
    }
}

/// SRT Caller transport — connects to a remote SRT Listener and sends data.
pub struct SrtTransport {
    socket: SrtSocket,
}

#[async_trait::async_trait]
impl PushTransport for SrtTransport {
    type Config = SrtTransportConfig;
    type Error = srt_runtime::SrtError;

    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        // Parse srt:// URL to get address
        let addr = url.strip_prefix("srt://").unwrap_or(url);
        let socket = SrtSocket::connect(addr, config.srt_config.clone()).await?;
        Ok(Self { socket })
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.socket.send(data).await
    }

    fn close(&mut self) {
        // SrtSocket closes on drop
    }
}
```

- [ ] **Step 2: Write a loopback integration test**

In `multimux/src/push/srt.rs` or a separate test file, write a test that:
1. Spawns an SRT Listener on `127.0.0.1:0` (ephemeral port)
2. Creates an `SrtTransport` connecting to it
3. Sends a few TS packets
4. Verifies the Listener received them byte-for-byte
5. Drops the transport, verifies clean shutdown

Use `#[tokio::test]`.

- [ ] **Step 3: Run gate**

```bash
RUSTFLAGS="-D warnings" cargo test -p multimux --all-features --locked -- push::srt
RUSTFLAGS="-D warnings" cargo clippy -p multimux --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 4: Commit**

```bash
git add multimux/src/push/srt.rs multimux/src/push/mod.rs
git commit -m "feat(multimux): SRT push transport (#744)"
```

---

### Task 4: Wire push outputs into multimux origin supervisor

**Files:**
- Modify: `multimux/src/origin/supervisor.rs`
- Modify: `multimux/src/push/mod.rs` (helper to spawn from OutputKind)
- Modify: `multimux/src/prometheus.rs`

**Interfaces:**
- Consumes: `OutputKind::SrtPush/RtmpPush/RtspPush`, `drive_push()`, `PushMetrics`, `RouteHandle`, `Trunk`
- Produces: Push tasks spawned alongside served outputs per route

- [ ] **Step 1: Add `spawn_push_outputs` helper in `push/mod.rs`**

```rust
/// Inspect a route's `outputs` for push kinds, spawn a `drive_push` task for
/// each one, returning the `JoinHandle`s and `PushMetrics` for metrics collection.
pub fn spawn_push_outputs(
    outputs: &[OutputKind],
    trunk: &Arc<Trunk>,
    cancel: CancellationToken,
) -> Vec<(tokio::task::JoinHandle<()>, Arc<PushMetrics>, String)> {
    let mut handles = Vec::new();
    for output in outputs {
        match output {
            OutputKind::SrtPush { url, format, reconnect } => {
                let push_format = format.unwrap_or(PushFormat::Ts);
                let policy = reconnect.clone().unwrap_or_default();
                let metrics = PushMetrics::new();
                let cursor = trunk.subscribe();
                let trunk = Arc::clone(trunk);
                let metrics_clone = Arc::clone(&metrics);
                let url = url.clone();
                let cancel = cancel.clone();
                let handle = tokio::spawn(async move {
                    drive_push::<srt::SrtTransport>(
                        url, srt::SrtTransportConfig::default(),
                        push_format, policy, trunk, cursor, metrics_clone, cancel,
                    ).await;
                });
                handles.push((handle, metrics, url.clone()));
            }
            // RtmpPush and RtspPush: same pattern, using their respective transports
            _ => {}
        }
    }
    handles
}
```

- [ ] **Step 2: Call `spawn_push_outputs` from the supervisor**

In `multimux/src/origin/supervisor.rs`, find where routes are started and outputs are built. After the served outputs are mounted, call `spawn_push_outputs` with the route's outputs, the route's Trunk, and the cancellation token.

Store the returned handles so they are cancelled on route teardown.

- [ ] **Step 3: Add push metrics to Prometheus endpoint**

In `multimux/src/prometheus.rs`, add gauges/counters for push outputs:
- `multimux_push_samples_sent_total{url, protocol}`
- `multimux_push_samples_dropped_total{url, protocol}`
- `multimux_push_bytes_sent_total{url, protocol}`
- `multimux_push_reconnects_total{url, protocol}`
- `multimux_push_connected{url, protocol}` (gauge: 0 or 1)

Read from the `PushMetrics` atomics.

- [ ] **Step 4: Write integration test**

Create a test that:
1. Sets up a multimux route with `outputs: [OutputKind::SrtPush { url: "srt://127.0.0.1:PORT", .. }]`
2. Spawns an SRT Listener on that port
3. Publishes samples into the route's Trunk
4. Verifies the Listener receives TS-muxed data
5. Verifies metrics are populated

- [ ] **Step 5: Update CHANGELOG.md**

Add `[Unreleased]` section to `multimux/CHANGELOG.md`:
```markdown
## [Unreleased]

### Added
- Push output support: `OutputKind::SrtPush`, `OutputKind::RtmpPush`,
  `OutputKind::RtspPush` — relay/restream ingested media to downstream
  servers (#744).
- `PushFormat` config: selectable container format per push output (TS, FLV).
- `ReconnectPolicy` config: exponential backoff reconnect with configurable bounds.
- Prometheus metrics per push output: samples sent/dropped, bytes, reconnects.
```

- [ ] **Step 6: Run full gate**

```bash
RUSTFLAGS="-D warnings" cargo build --workspace --all-features --locked
RUSTFLAGS="-D warnings" cargo test --workspace --all-features --locked
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 7: Commit**

```bash
git add multimux/
git commit -m "feat(multimux): wire push outputs into supervisor + Prometheus metrics (#744)"
```

---

### Task 5: RTSP client ANNOUNCE/RECORD methods

**Files:**
- Modify: `rtsp-runtime/src/client.rs`
- Modify: `rtsp-runtime/src/state.rs` (verify state transitions)

**Interfaces:**
- Consumes: existing `ClientSession` API, `SessionState::Recording`, `Method::Record`, `Method::Announce`
- Produces: `ClientSession::announce(uri, sdp)` and `ClientSession::record(uri)` methods

- [ ] **Step 1: Verify state machine already supports ANNOUNCE/RECORD transitions**

In `rtsp-runtime/src/state.rs`, confirm that `client_next_state` handles:
- `Method::Announce` — valid from any state (RFC 2326 §A.1: "AS" in all states)
- `Method::Record` — valid from `Ready` → `Recording`

If missing, add the transitions.

- [ ] **Step 2: Add `announce()` method to `ClientSession`**

In `rtsp-runtime/src/client.rs`:

```rust
/// Build an ANNOUNCE request (RFC 2326 §10.3) — client→server media
/// description for recording/push. The `sdp` body describes the media
/// this client will send.
pub fn announce(&mut self, uri: &str, sdp: &str) -> Result<Vec<u8>> {
    self.check_method_allowed(Method::Announce)?;
    let body = sdp.as_bytes().to_vec();
    let mut req = Request::builder(Method::Announce, Version::V1_0)
        .request_uri(uri.parse().map_err(|_| Error::InvalidUri(uri.to_owned()))?)
        .build(body);
    req.insert_header(headers::CONTENT_TYPE, "application/sdp");
    self.send_request(req, uri)
}
```

Follow the exact same `send_request` pattern used by `describe()`, `setup()`, etc.

- [ ] **Step 3: Add `record()` method to `ClientSession`**

```rust
/// Build a RECORD request (RFC 2326 §10.11) — start recording/publishing
/// on the server. Must be in `Ready` state (after SETUP).
pub fn record(&mut self, uri: &str) -> Result<Vec<u8>> {
    self.check_method_allowed(Method::Record)?;
    let req = Request::builder(Method::Record, Version::V1_0)
        .request_uri(uri.parse().map_err(|_| Error::InvalidUri(uri.to_owned()))?)
        .build(Vec::new());
    self.send_request(req, uri)
}
```

- [ ] **Step 4: Write tests**

In `rtsp-runtime/src/client.rs` tests or a test file:
- `announce_builds_request_with_sdp_body` — verify Content-Type header + body
- `record_transitions_to_recording` — after SETUP (Ready state), RECORD succeeds, state transitions to Recording
- `record_fails_from_init_state` — verify `MethodNotValidInState` error

- [ ] **Step 5: Update CHANGELOG.md**

Add `[Unreleased]` section to `rtsp-runtime/CHANGELOG.md`:
```markdown
## [Unreleased]

### Added
- `ClientSession::announce()` — ANNOUNCE request builder for client-side
  media push (RFC 2326 §10.3). Issue #744.
- `ClientSession::record()` — RECORD request builder (RFC 2326 §10.11). Issue #744.
```

- [ ] **Step 6: Run gate**

```bash
RUSTFLAGS="-D warnings" cargo test -p rtsp-runtime --all-features --locked
RUSTFLAGS="-D warnings" cargo clippy -p rtsp-runtime --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 7: Commit**

```bash
git add rtsp-runtime/
git commit -m "feat(rtsp-runtime): client ANNOUNCE + RECORD request builders (#744)"
```

---

### Task 6: RTSP push transport

**Files:**
- Create: `multimux/src/push/rtsp.rs`
- Modify: `multimux/src/push/mod.rs` (add `pub mod rtsp;`, wire into `spawn_push_outputs`)

**Interfaces:**
- Consumes: `PushTransport` trait, `rtsp_runtime::client::ClientSession::announce/record`, `rtsp_runtime::io` TCP adapter
- Produces: `RtspTransport` implementing `PushTransport`

- [ ] **Step 1: Implement `RtspTransport`**

```rust
//! RTSP push transport — client ANNOUNCE/RECORD to a remote RTSP server.
//!
//! Uses rtsp-runtime's `ClientSession` with ANNOUNCE + per-track SETUP +
//! RECORD, then sends TS-muxed data over interleaved TCP framing.

pub struct RtspTransport {
    // rtsp-runtime client session + TCP stream
}

#[async_trait::async_trait]
impl PushTransport for RtspTransport {
    type Config = RtspTransportConfig;
    type Error = RtspPushError;

    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        // TCP connect (dual-stack), OPTIONS, then hold
    }

    async fn setup(&mut self, tracks: &[TrackSpec]) -> Result<(), Self::Error> {
        // ANNOUNCE with SDP built from tracks, SETUP per track, RECORD
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        // Interleaved frame: $channel_len_data over TCP
    }

    fn close(&mut self) {
        // TEARDOWN
    }
}
```

- [ ] **Step 2: Wire into `spawn_push_outputs`**

Add `OutputKind::RtspPush` arm to the match in `spawn_push_outputs`.

- [ ] **Step 3: Write loopback test**

Spawn a minimal RTSP server (using rtsp-runtime's `ServerSession`), connect an `RtspTransport`, verify ANNOUNCE/SETUP/RECORD handshake completes and data arrives.

- [ ] **Step 4: Run gate + commit**

```bash
RUSTFLAGS="-D warnings" cargo test -p multimux --all-features --locked
git add multimux/src/push/
git commit -m "feat(multimux): RTSP push transport (#744)"
```

---

### Task 7: RTMP client session (sans-IO state machine)

**Files:**
- Create: `rtmp-runtime/src/client.rs`
- Modify: `rtmp-runtime/src/lib.rs` (add `pub mod client;`)
- Modify: `rtmp-runtime/src/io.rs` (add client tokio adapter)

**Interfaces:**
- Consumes: existing `crate::handshake::Handshake`, `crate::chunk::{ChunkAssembler, ChunkWriter}`, `crate::amf0`, `crate::message`
- Produces: `RtmpClientSession` state machine with `start_handshake()`, `feed()`, `send_av()`, `send_metadata()`

- [ ] **Step 1: Create `rtmp-runtime/src/client.rs` — the sans-IO client session**

Model it after `server.rs` but with reversed roles:
- Client sends C0/C1, processes S0/S1/S2
- Client sends `connect` command, processes `_result`
- Client sends `createStream`, processes `_result` with stream ID
- Client sends `publish`, processes `onStatus`
- Client sends Audio(8)/Video(9)/Data-AMF0(18) messages as chunks

```rust
//! RTMP client session state machine — `connect` → `createStream` →
//! `publish` (Adobe RTMP 1.0 §7.2, client role).
//!
//! [`RtmpClientSession`] is the sans-IO **publish** (push) engine: build
//! outbound bytes via request methods, feed inbound bytes via
//! [`RtmpClientSession::handle_data`], get back typed [`ClientEvent`]s.
//! Mirrors [`crate::server::ServerSession`] but in the client role.

pub struct RtmpClientSession {
    state: ClientState,
    handshake: crate::handshake::Handshake,
    chunk_reader: ChunkAssembler,
    chunk_writer: ChunkWriter,
    stream_id: Option<u32>,
    next_transaction_id: f64,
    pending_transactions: std::collections::HashMap<u64, PendingCommand>,
    app: String,
    stream_key: String,
    bytes_received: u64,
    window_ack_size: u32,
}

enum ClientState {
    Handshake,
    ConnectSent,
    CreateStreamSent,
    PublishSent,
    Publishing,
    Closed,
}

pub enum ClientEvent {
    /// Bytes to write to TCP socket.
    Transmit(Vec<u8>),
    /// State advanced — diagnostic.
    StateChanged(ClientState),
    /// Server accepted publish. Ready to send A/V data.
    PublishAccepted,
    /// Server rejected with reason.
    Rejected { code: String, description: String },
}
```

Key methods:
```rust
impl RtmpClientSession {
    pub fn new(app: String, stream_key: String) -> Self;
    pub fn start_handshake(&mut self) -> Vec<u8>;  // C0+C1
    pub fn handle_data(&mut self, data: &[u8]) -> Result<Vec<ClientEvent>>;
    pub fn send_audio(&mut self, timestamp: u32, data: &[u8]) -> Result<Vec<u8>>;
    pub fn send_video(&mut self, timestamp: u32, data: &[u8]) -> Result<Vec<u8>>;
    pub fn send_metadata(&mut self, pairs: &[(String, Amf0Value)]) -> Result<Vec<u8>>;
}
```

The `handle_data` method drives state transitions internally:
- Handshake → on S0/S1/S2 complete → send `connect` command → ConnectSent
- ConnectSent → on `_result` success → send Window Ack Size + Set Chunk Size + `createStream` → CreateStreamSent
- CreateStreamSent → on `_result` with stream_id → send `publish` → PublishSent
- PublishSent → on `onStatus` "NetStream.Publish.Start" → Publishing
- Publishing → send_audio/send_video/send_metadata ready

- [ ] **Step 2: Write sans-IO tests**

Test the state machine with pre-recorded server response bytes:
- `handshake_completes_and_sends_connect` — feed S0+S1, verify C2 + connect command emitted
- `connect_result_triggers_create_stream` — feed `_result` success, verify createStream emitted
- `publish_accepted_enters_publishing` — feed onStatus NetStream.Publish.Start, verify PublishAccepted event
- `send_audio_in_publishing_state` — after reaching Publishing, `send_audio()` succeeds
- `send_audio_before_publishing_fails` — in ConnectSent state, `send_audio()` returns error

- [ ] **Step 3: Add client tokio adapter in `io.rs`**

Add a `connect_and_publish` async function:
```rust
pub async fn connect_and_publish(
    addr: impl tokio::net::ToSocketAddrs,
    app: &str,
    stream_key: &str,
) -> Result<RtmpClientConnection, RtmpError> {
    // TCP connect (dual-stack), drive handshake + connect + createStream + publish
    // Return a handle that exposes send_audio/send_video
}
```

- [ ] **Step 4: Update CHANGELOG.md**

```markdown
## [Unreleased]

### Added
- `client` module: `RtmpClientSession` sans-IO state machine for client-side
  `connect` → `createStream` → `publish` (Adobe RTMP 1.0 §7.2 client role).
  Issue #744.
- `io::connect_and_publish()` tokio adapter for driving the client session
  over a real TCP connection. Issue #744.
```

- [ ] **Step 5: Run gate + commit**

```bash
RUSTFLAGS="-D warnings" cargo test -p rtmp-runtime --all-features --locked
RUSTFLAGS="-D warnings" cargo clippy -p rtmp-runtime --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
git add rtmp-runtime/
git commit -m "feat(rtmp-runtime): client publish session — sans-IO + tokio adapter (#744)"
```

---

### Task 8: RTMP push transport + final wiring

**Files:**
- Create: `multimux/src/push/rtmp.rs`
- Modify: `multimux/src/push/mod.rs` (add `pub mod rtmp;`, wire into `spawn_push_outputs`)

**Interfaces:**
- Consumes: `PushTransport` trait, `rtmp_runtime::client::RtmpClientSession`, `rtmp_runtime::io::connect_and_publish`
- Produces: `RtmpTransport` implementing `PushTransport`

- [ ] **Step 1: Implement `RtmpTransport`**

```rust
//! RTMP push transport — client publish to a remote RTMP server.

pub struct RtmpTransport {
    // rtmp-runtime client connection handle
}

#[async_trait::async_trait]
impl PushTransport for RtmpTransport {
    type Config = RtmpTransportConfig;
    type Error = RtmpPushError;

    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        // Parse rtmp://host/app/stream_key URL
        // TCP connect (dual-stack) + handshake + connect + createStream + publish
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        // FLV tag bytes → send_audio/send_video via client session
    }

    fn close(&mut self) {
        // Drop connection
    }
}
```

- [ ] **Step 2: Wire into `spawn_push_outputs`**

Add `OutputKind::RtmpPush` arm.

- [ ] **Step 3: Write loopback test**

Spawn an RTMP server (using rtmp-runtime's `ServerSession` + io adapter), connect an `RtmpTransport`, send FLV-muxed A/V data, verify the server receives `ServerEvent::Audio`/`ServerEvent::Video`.

- [ ] **Step 4: Run full workspace gate + commit**

```bash
RUSTFLAGS="-D warnings" cargo build --workspace --all-features --locked
RUSTFLAGS="-D warnings" cargo test --workspace --all-features --locked
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
git add multimux/
git commit -m "feat(multimux): RTMP push transport (#744)"
```

---

### Task 9: multimux-cli push output flags

**Files:**
- Modify: `multimux-cli/src/main.rs` (or wherever CLI args are defined)

**Interfaces:**
- Consumes: `OutputKind::SrtPush/RtmpPush/RtspPush`
- Produces: CLI flags `--srt-push <url>`, `--rtmp-push <url>`, `--rtsp-push <url>`

- [ ] **Step 1: Add push output CLI flags**

Follow the existing pattern for `--dash`/`--outputs`. Add:
```
--srt-push <URL>     Push to SRT destination (Caller mode)
--rtmp-push <URL>    Push to RTMP destination (client publish)
--rtsp-push <URL>    Push to RTSP destination (ANNOUNCE/RECORD)
```

Each flag appends the corresponding `OutputKind` variant to the route's outputs.

- [ ] **Step 2: Test CLI flag parsing**

- [ ] **Step 3: Run gate + commit**

```bash
RUSTFLAGS="-D warnings" cargo test -p multimux-cli --all-features --locked
git add multimux-cli/
git commit -m "feat(multimux-cli): --srt-push/--rtmp-push/--rtsp-push flags (#744)"
```
