//! Issue #3681 (audit 23, finding 204) end-to-end half: the [`PushMetrics`]
//! `drive_push` returns must report what the scripted run actually did, and a
//! bounded-flush failure must tear the connection down and reconnect.
//!
//! The decision rows themselves are `media_plane::push_session`'s unit tests;
//! this drives the production loop over a real `Trunk` with a scripted
//! transport, so a metrics-wiring regression or a flush outcome that never
//! reconnects fails here.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use crate::support::shared_it::{HANG_GUARD, nz};
use broadcast_common::timescale::Timescale;
use bytes::Bytes;
use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig};
use multimux::config::{PushFormat, ReconnectPolicy};
use multimux::push::{MuxError, PushTransport, SendMediaError, drive_push};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use transmux::TrackId;
use transmux::ir::{CodecConfig, DataCarriage, Media, Sample, TrackSpec};

const TRACK: u32 = 1;
const MESSAGE_LEN: usize = 188;
const POISON: [u8; 4] = [0xEE, 0xEE, 0xEE, 0xEE];
static MESSAGE: [u8; MESSAGE_LEN] = [0x47; MESSAGE_LEN];

#[derive(Default)]
struct ScriptedState {
    connects: AtomicU32,
    written: AtomicU32,
    write_failures: AtomicU32,
    mux_failures: AtomicU32,
    fail_writes: AtomicU32,
    notify: Notify,
}

impl ScriptedState {
    fn announce(&self) {
        self.notify.notify_waiters();
    }
}

#[derive(Clone)]
struct ScriptedConfig {
    state: Arc<ScriptedState>,
}

struct ScriptedTransport {
    state: Arc<ScriptedState>,
}

#[derive(Debug, thiserror::Error)]
#[error("scripted write failed")]
struct ScriptedError;

#[async_trait::async_trait]
impl PushTransport for ScriptedTransport {
    type Config = ScriptedConfig;
    type Error = ScriptedError;

    async fn connect(_url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        config.state.connects.fetch_add(1, Ordering::Relaxed);
        config.state.announce();
        Ok(ScriptedTransport {
            state: Arc::clone(&config.state),
        })
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        if self.state.fail_writes.load(Ordering::Relaxed) > 0 {
            self.state.fail_writes.fetch_sub(1, Ordering::Relaxed);
            self.state.write_failures.fetch_add(1, Ordering::Relaxed);
            self.state.announce();
            return Err(ScriptedError);
        }
        assert_eq!(data.len(), MESSAGE_LEN);
        self.state.written.fetch_add(1, Ordering::Relaxed);
        self.state.announce();
        Ok(())
    }

    fn encode_media(&mut self, media: &Media) -> Result<Vec<Bytes>, SendMediaError> {
        let poisoned = media
            .tracks
            .iter()
            .flat_map(|track| track.samples.iter())
            .any(|sample| sample.data.as_ref() == POISON);
        if poisoned {
            self.state.mux_failures.fetch_add(1, Ordering::Relaxed);
            self.state.announce();
            return Err(SendMediaError::Mux(MuxError::NoCarriableTrack));
        }
        Ok(vec![Bytes::from_static(&MESSAGE)])
    }

    fn close(&mut self) {}
}

fn timescale() -> Timescale {
    Timescale::try_new(90_000).expect("a non-zero timescale")
}

fn data_spec() -> TrackSpec {
    TrackSpec::new(
        TrackId::new(TRACK),
        timescale(),
        CodecConfig::Data {
            stream_type: 0x06,
            descriptors: Vec::new(),
            carriage: DataCarriage::Pes,
        },
    )
}

fn scripted_trunk() -> Arc<Trunk> {
    let trunk = Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz(64))
            .with_sparse_capacity(nz(16))
            .with_segment_capacity(nz(8))
            .with_event_capacity(nz(16))
            .with_part_capacity(nz(16)),
    );
    let writer = trunk.writer().expect("the trunk has a writer");
    writer.set_tracks(vec![data_spec()]);
    trunk
}

fn sample(marker: u8) -> Sample {
    Sample::new(Bytes::from(vec![marker; 4]), Some(0), Some(0), None, true)
        .expect("a valid sample timing")
}

async fn wait_until(state: &ScriptedState, ready: impl Fn() -> bool) {
    let reached = async {
        loop {
            let announced = state.notify.notified();
            if ready() {
                return;
            }
            announced.await;
        }
    };
    tokio::time::timeout(HANG_GUARD, reached)
        .await
        .expect("the scripted run reaches the expected state");
}

fn start_push(
    trunk: &Arc<Trunk>,
    state: &Arc<ScriptedState>,
    cancel: &CancellationToken,
) -> tokio::task::JoinHandle<multimux::push::PushMetrics> {
    tokio::spawn(drive_push::<ScriptedTransport>(
        Arc::clone(trunk),
        "scripted://push/drive".to_string(),
        ScriptedConfig {
            state: Arc::clone(state),
        },
        PushFormat::Ts,
        ReconnectPolicy::doubling(Duration::ZERO, Duration::ZERO),
        cancel.clone(),
    ))
}

async fn stop_push(
    cancel: CancellationToken,
    push: tokio::task::JoinHandle<multimux::push::PushMetrics>,
) -> multimux::push::PushMetrics {
    cancel.cancel();
    tokio::time::timeout(HANG_GUARD, push)
        .await
        .expect("drive_push stops on cancel")
        .expect("the push task must not panic")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn push_metrics_report_a_scripted_run() {
    const SENT: u32 = 5;
    let state = Arc::new(ScriptedState::default());
    let trunk = scripted_trunk();
    let writer = trunk.writer().expect("the trunk has a writer");
    let cancel = CancellationToken::new();
    let push = start_push(&trunk, &state, &cancel);

    wait_until(&state, || state.connects.load(Ordering::Relaxed) == 1).await;
    for marker in 0..SENT {
        writer.publish(
            TrackId::new(TRACK),
            RetentionClass::Timed,
            sample(marker as u8),
        );
    }
    writer.publish(
        TrackId::new(TRACK),
        RetentionClass::Timed,
        Sample::new(Bytes::from_static(&POISON), Some(0), Some(0), None, true)
            .expect("a valid sample timing"),
    );

    wait_until(&state, || state.written.load(Ordering::Relaxed) == SENT).await;
    wait_until(&state, || state.mux_failures.load(Ordering::Relaxed) == 1).await;

    let metrics = stop_push(cancel, push).await;
    assert_eq!(metrics.samples_sent(), u64::from(SENT));
    assert_eq!(metrics.samples_dropped(), 1);
    assert_eq!(metrics.reconnect_count(), 1);
    assert_eq!(metrics.bytes_sent(), u64::from(SENT) * MESSAGE_LEN as u64);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_flush_failure_reconnects() {
    let state = Arc::new(ScriptedState::default());
    state.fail_writes.store(1, Ordering::Relaxed);
    let trunk = scripted_trunk();
    let writer = trunk.writer().expect("the trunk has a writer");
    let cancel = CancellationToken::new();
    let push = start_push(&trunk, &state, &cancel);

    wait_until(&state, || state.connects.load(Ordering::Relaxed) == 1).await;
    writer.publish(TrackId::new(TRACK), RetentionClass::Timed, sample(1));
    wait_until(&state, || state.write_failures.load(Ordering::Relaxed) == 1).await;
    wait_until(&state, || state.connects.load(Ordering::Relaxed) == 2).await;

    let metrics = stop_push(cancel, push).await;
    assert_eq!(metrics.reconnect_count(), 2);
    assert_eq!(metrics.samples_sent(), 1);
}
