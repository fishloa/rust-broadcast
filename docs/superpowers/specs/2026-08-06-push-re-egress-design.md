# Push Re-Egress Design (#744)

## Summary

Turn multimux from an HTTP-only origin into a relay/gateway by adding push-style
outputs that restream ingested media to downstream RTMP, RTSP, and SRT servers.
Reuses existing `rtmp-runtime`, `rtsp-runtime`, and `srt-runtime` crates.

**Approach:** Generic `PushDriver<P: PushProtocol>` — shared reconnect/mux/drain
logic with thin per-protocol transport adapters.

## Requirements

1. SRT push output (Caller mode → remote Listener).
2. RTMP push output (client publish → remote RTMP server).
3. RTSP push output (client ANNOUNCE/RECORD → remote RTSP server).
4. Configurable container format per output (TS, FLV, RTP) with protocol-native
   defaults (SRT→TS, RTMP→FLV, RTSP→RTP).
5. Exponential backoff reconnect on disconnect. Samples dropped during backoff.
6. Multiple push destinations per route (fan-out).
7. IPv4 and IPv6 dual-stack support throughout.
8. Push outputs configured inline in `outputs: Vec<OutputKind>` alongside
   existing served outputs.
9. Prometheus metrics per push output (state, samples sent/dropped, reconnects,
   bytes sent, last error).
10. All changes additive — no breaking API changes in any crate.

## Architecture

### Layer Diagram

```
multimux Route
  └── outputs: Vec<OutputKind>
        ├── LlHls { ... }                          ← existing (ServedEgress)
        ├── Dash { ... }                            ← existing (ServedEgress)
        ├── SrtPush { url, format, reconnect }      ← NEW
        ├── RtmpPush { url, format, reconnect }     ← NEW
        └── RtspPush { url, format, reconnect }     ← NEW

Each push OutputKind spawns:

  SampleCursor (from Trunk)
       │
       ▼
  PushDriver<P: PushProtocol>         ← implements PushEgress
       │
       ├── MuxStage (transmux muxer)  ← IR samples → wire bytes
       │     selectable: TS, FLV, RTP
       │
       ├── ReconnectEngine            ← exponential backoff FSM
       │
       └── P: PushProtocol            ← transport connect/send/close
             ├── SrtPush   (srt-runtime SrtSocket::connect + send)
             ├── RtmpPush  (rtmp-runtime client publish — NEW)
             └── RtspPush  (rtsp-runtime ANNOUNCE/RECORD — NEW)
```

### Threading / Async Drive Model

Each push output gets its own tokio task, spawned when the route starts,
cancelled on route teardown or config removal.

```
Route::start()
  ├── spawn(serve_ll_hls(...))        ← existing
  ├── spawn(serve_dash(...))          ← existing
  ├── spawn(drive_push(srt_push_1))   ← NEW: one task per push output
  ├── spawn(drive_push(srt_push_2))   ← fan-out = more tasks
  └── spawn(drive_push(rtmp_push_1))
```

Drive loop per task:

```rust
async fn drive_push<P: PushProtocol>(
    trunk: &Trunk,
    mut cursor: SampleCursor,
    mut driver: PushDriver<P>,
) {
    loop {
        // Wait for samples (Trunk::listen() — bounded async wake)
        trunk.listen().await;

        // Drain cursor, mux, send
        while let Some(item) = cursor.poll() {
            match driver.state {
                Connected => {
                    let bytes = driver.mux(&item);
                    if driver.protocol.send(&bytes).await.is_err() {
                        driver.enter_reconnect();
                    }
                }
                Reconnecting => {
                    // samples dropped — cursor advances, nothing sent
                    driver.reconnect_tick().await;
                }
            }
        }
    }
}
```

No shared mutable state. Each task owns its cursor, muxer, protocol instance,
and reconnect FSM exclusively. No locks, no contention.

Cancellation via `tokio::select!` with existing multimux `CancellationToken`.
On cancel → `protocol.close()` → task exits.

Backpressure: if `send()` blocks (TCP buffer full on RTMP/RTSP), that task's
cursor falls behind. If it lags past ring capacity, cursor reports `Lagged` —
driver logs it, continues. No effect on other tasks or the Trunk.

## Detailed Design

### PushProtocol Trait

Lives in `media-plane/src/push.rs` (new module).

```rust
#[async_trait]
pub trait PushProtocol: Send + 'static {
    type Config: Send;
    type Error: std::error::Error + Send;

    /// Establish connection to remote. Dual-stack IPv4/IPv6.
    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error>
    where Self: Sized;

    /// Negotiate tracks at connection time. Some protocols need this
    /// (RTSP DESCRIBE/SETUP, RTMP createStream/publish). SRT: no-op.
    async fn setup(&mut self, tracks: &[TrackSpec]) -> Result<(), Self::Error>;

    /// Send muxed bytes to remote.
    async fn send(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;

    /// Clean shutdown.
    fn close(&mut self);
}
```

### PushDriver\<P\>

Lives in `media-plane/src/push.rs`.

Implements `PushEgress` (from `media-plane/src/egress.rs`). Owns:
- `P: PushProtocol` instance (or None when disconnected)
- `MuxStage` — the selected muxer
- `ReconnectEngine` — backoff FSM
- `PushMetrics` — counters

### ReconnectEngine

```rust
enum PushState {
    Connecting,
    Connected,
    Backoff { next_attempt: Instant, attempt: u32 },
    Failed,
}
```

State transitions:

```
                    ┌──────────────┐
        start ────►│  Connecting   │
                    └──────┬───────┘
                   success │     │ error
                           ▼     ▼
                    ┌──────────┐  ┌──────────┐
              ┌────►│ Connected │  │ Backoff  │◄────┐
              │     └─────┬────┘  └────┬─────┘     │
              │    send   │    timer   │            │
              │    error  │    fires   │            │
              │           ▼            ▼            │
              │     ┌──────────┐  ┌──────────────┐  │
              │     │ Backoff  │  │  Connecting   ├──┘
              │     └──────────┘  └──────┬───────┘
              │                   success│
              └──────────────────────────┘
                                         │ max_attempts
                                         ▼
                                  ┌──────────┐
                                  │  Failed   │
                                  └──────────┘
```

- **Connecting**: `protocol.connect()` + `protocol.setup(tracks)`. Success →
  Connected. Error → Backoff(attempt=1).
- **Connected**: Mux + send. Send error → close, Backoff(attempt=1).
- **Backoff**: Cursor still drains (samples dropped, counted). Timer fires →
  Connecting. Backoff = `min(initial × 2^attempt, max_backoff)`.
- **Failed**: max_attempts reached. Cursor drains (dropped). Logs every 60s.

On renegotiate (track set changes mid-stream): if connected, call
`protocol.setup(new_tracks)`. If protocol errors (RTSP can't renegotiate
mid-RECORD), disconnect → reconnect with new tracks.

### MuxStage

```rust
pub enum MuxStage {
    Ts(transmux::TsMuxer),
    Flv(transmux::FlvMuxer),
    Rtp(transmux::RtpMuxer),
}
```

Converts `SampleCursorItem` → wire bytes. Selected by `PushFormat` config.
Initialized at connect time with negotiated tracks. The exact transmux muxer
types used depend on what transmux exposes — the enum wraps whatever concrete
TS/FLV/RTP packetizer transmux provides (e.g. the existing TS packetizer from
`ts_hls`, the FLV builder from the RTMP demux path, and the RTP packetizer
from the RTP output path). If a needed muxer doesn't exist in transmux yet,
it is added as part of the relevant phase.

### Config Types (multimux)

```rust
pub enum PushFormat {
    Ts,
    Flv,
    Rtp,
}

pub struct ReconnectPolicy {
    pub initial_backoff: Duration,  // default 1s
    pub max_backoff: Duration,      // default 30s
    pub max_attempts: Option<u32>,  // None = infinite
}

// Added to OutputKind enum:
OutputKind::SrtPush {
    url: String,                        // srt://host:port?options
    format: Option<PushFormat>,         // None = TS
    reconnect: Option<ReconnectPolicy>, // None = default policy
}

OutputKind::RtmpPush {
    url: String,                        // rtmp://host/app/stream_key
    format: Option<PushFormat>,         // None = FLV
    reconnect: Option<ReconnectPolicy>,
}

OutputKind::RtspPush {
    url: String,                        // rtsp://host:port/path
    format: Option<PushFormat>,         // None = RTP
    reconnect: Option<ReconnectPolicy>,
}
```

Invalid format/protocol combos (e.g. RTP over SRT) fail at negotiation time
with a clear error, not at config parse.

### Metrics

```rust
pub struct PushMetrics {
    pub state: &'static str,        // "connected" | "backoff" | "failed"
    pub samples_sent: u64,
    pub samples_dropped: u64,
    pub reconnect_count: u32,
    pub bytes_sent: u64,
    pub last_error: Option<String>,
}
```

Wired into multimux's existing Prometheus `/metrics` endpoint. One metric set
per push output, labeled by URL and protocol.

### Protocol Implementations

#### SRT (`SrtPushProtocol`)

Thin wrapper around `srt-runtime::SrtSocket`. Existing API is complete:
- `connect()`: `SrtSocket::connect(addr, config).await` — dual-stack via `ToSocketAddrs`
- `setup()`: no-op (SRT is format-agnostic)
- `send()`: `socket.send(bytes).await`
- `close()`: drop socket

No changes to `srt-runtime` needed.

#### RTMP (`RtmpPushProtocol`)

Requires new `rtmp-runtime/src/client.rs` module.

```rust
pub struct RtmpClientSession {
    state: ClientState,
    chunk_writer: ChunkWriter,
    chunk_reader: ChunkReader,
    stream_id: u32,
}

enum ClientState {
    Handshake,
    ConnectSent,
    CreateStreamSent,
    PublishSent,
    Publishing,
    Closed,
}
```

Sans-IO state machine mirroring existing server. Reuses existing `chunk`,
`handshake`, `amf0`, `message` modules. New tokio adapter wraps with TCP
connect (dual-stack: `tokio::net::lookup_host` resolves A+AAAA) + read/write
loop.

Key methods:
- `start_handshake() → Vec<u8>` — C0+C1
- `feed(&[u8]) → Result<Vec<ClientOutput>>` — process inbound, emit outbound
- `send_av(&FlvTag) → Result<Vec<u8>>` — audio/video as RTMP chunk message
- `send_metadata(&Metadata) → Result<Vec<u8>>` — onMetaData AMF0

State flow: `Handshake → ConnectSent → CreateStreamSent → PublishSent → Publishing`

#### RTSP (`RtspPushProtocol`)

Two new methods on existing `rtsp-runtime::ClientSession`:
- `announce(uri, sdp) → Result<Request>` — ANNOUNCE request builder
- `record(uri) → Result<Request>` — RECORD request builder

State machine already has `Recording` state and transitions wired (`Ready
→[Record]→ Recording`). Only the request builders are missing.

`RtspPushProtocol`:
- `connect()`: TCP connect (dual-stack) + OPTIONS
- `setup()`: ANNOUNCE with SDP describing tracks + SETUP per track + RECORD
- `send()`: RTP packets over interleaved TCP (existing interleaved framing)
- `close()`: TEARDOWN

## Crate Boundaries + Versioning

| Component | Crate | Change type | Version impact |
|-----------|-------|------------|----------------|
| `PushProtocol` trait | `media-plane` | Additive | Minor bump |
| `PushDriver<P>` | `media-plane` | Additive | Same minor |
| `ReconnectEngine` | `media-plane` | Additive (internal) | Same minor |
| `PushMetrics` | `media-plane` | Additive | Same minor |
| `OutputKind::SrtPush/RtmpPush/RtspPush` | `multimux` | Additive variants | Minor bump |
| `PushFormat`, `ReconnectPolicy` | `multimux` | Additive | Same minor |
| `drive_push` task spawning | `multimux` | Internal wiring | Same minor |
| Protocol impls | `multimux` | Additive | Same minor |
| `RtmpClientSession` + tokio adapter | `rtmp-runtime` | Additive module | Minor bump |
| `ClientSession::announce/record` | `rtsp-runtime` | Additive methods | Minor bump |

No breaking changes. Four crates get minor bumps: `media-plane`, `multimux`,
`rtmp-runtime`, `rtsp-runtime`. `srt-runtime` unchanged.

No new inter-crate dependencies — `multimux` already depends on all of these.

## Implementation Phases

Each phase independently shippable.

### Phase 1: media-plane PushDriver framework
- `PushProtocol` trait
- `PushDriver<P>` implementing `PushEgress`
- `ReconnectEngine` state machine
- `MuxStage` enum
- Tests with `MockPushProtocol` (connect/send/close tracking, injectable errors)

### Phase 2: SRT push (end-to-end)
- `SrtPushProtocol` impl in multimux (wrapper around `SrtSocket`)
- `OutputKind::SrtPush` variant + config parsing
- `drive_push` task spawning in route setup
- `PushFormat` + `ReconnectPolicy` config types
- Integration test: SRT Caller → Listener loopback

### Phase 3: Multimux wiring + metrics
- Prometheus metrics for push outputs
- Config validation (format/protocol combos)
- Fan-out test (multiple SrtPush outputs on one route)
- CLI flags for push outputs in `multimux-cli`

### Phase 4: RTSP push
- `ClientSession::announce()` + `ClientSession::record()` in rtsp-runtime
- `RtspPushProtocol` impl in multimux
- `OutputKind::RtspPush` variant + config
- Integration test: RTSP ANNOUNCE/RECORD loopback

### Phase 5: RTMP push
- `RtmpClientSession` sans-IO state machine in rtmp-runtime
- Tokio adapter with dual-stack TCP connect
- `RtmpPushProtocol` impl in multimux
- `OutputKind::RtmpPush` variant + config
- Integration test: RTMP client publish loopback

## Testing Strategy

- **Unit tests**: `ReconnectEngine` state transitions, `MuxStage` mux output,
  `PushDriver` with mock protocol (connect failures, send failures, backoff
  timing, renegotiation).
- **Sans-IO tests**: `RtmpClientSession` handshake + command flow against
  recorded server responses. `ClientSession::announce/record` request building.
- **Integration tests**: Loopback tests per protocol — spawn a local
  listener/server, connect a push output, verify media arrives. Use existing
  test infrastructure from each runtime crate.
- **Reconnect tests**: Kill listener mid-stream, verify backoff + reconnect +
  resumed sending. Verify samples-dropped counter increments during disconnect.
- **Fan-out tests**: Multiple push outputs from one route, verify all receive
  the same samples independently.
- **Dual-stack tests**: Where feasible, test IPv6 loopback (`[::1]`).
