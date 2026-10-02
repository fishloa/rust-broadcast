//! [`media_plane::PushEgress`] over a [`PushTransport`] (issue #942).
//!
//! # The finding this closes
//!
//! Issue #942 asked whether `media-plane`'s `PushEgress` and this crate's
//! own [`PushTransport`] are duplicate abstractions for the same job.
//! Reading both before writing this module says no — they are two
//! different layers that compose, not two implementations of one layer:
//!
//! - [`PushEgress`] is the **sample-to-wire-message** layer: negotiate which
//!   tracks a container format can carry, mux one drained sample into that
//!   format's native messages. It is deliberately **synchronous** (`send`
//!   returns `Result<(), Self::Error>` directly, no `Future`) — see its own
//!   module doc, "`poll_transmit` mirrors `IngestSession::poll_transmit`
//!   exactly": outbound bytes are *produced* sans-IO and *drained* by an
//!   external caller, the same split every ingest session in this crate
//!   already draws between parsing (sans-IO) and the real socket read/write
//!   (an async driver loop it does not own).
//! - [`PushTransport`] is that external driver's **byte/socket** layer: real
//!   `connect`/`send`/`close` over a live `TcpStream`. It has to be async —
//!   there is a real socket underneath every implementor (SRT/RTMP/RTSP).
//!
//! [`PushTransportEgress`] is the seam: it owns a `T: PushTransport` and
//! implements `PushEgress` over it by routing every encode through
//! [`PushTransport::encode_media`] (issue #942's own addition — the sans-IO
//! half `send_media` already had in spirit, split out so this type's
//! synchronous `send` can call it) and queuing the resulting wire messages
//! for [`PushEgress::poll_transmit`] rather than writing them itself.
//! [`PushTransportEgress::flush_transmit`] is the async half a driving loop
//! (`crate::push::drive_push`) calls once per iteration to actually push
//! those queued messages out over `T`.
//!
//! # Where the codec-refusal vocabulary moved
//!
//! `push::rtmp::RtmpTransport` used to decide "can I carry this track" via
//! its own private `is_flv_codec` predicate, checked ad-hoc in both `setup`
//! (hard error if *nothing* is carriable) and `send_media` (warn once, then
//! silently exclude, if only *some* tracks are). [`PushTransport::
//! supports_codec`] is now the one predicate (RTMP overrides it; SRT/RTSP
//! keep TS's broad default); [`PushTransportEgress::negotiate`]/
//! [`renegotiate`](PushEgress::renegotiate) are the one structured place
//! that predicate is applied, returning:
//!
//! - [`NegotiationOutcome::Error`] when the proposed set has **no**
//!   carriable track at all — the offer itself is unsatisfiable, matching
//!   what `RtmpTransport::setup` used to return as a hard connect-time
//!   error.
//! - [`NegotiationOutcome::Accepted`] with a [`TrackSelection`] naming
//!   exactly the carriable subset, when at least one track is — the
//!   selection itself *is* the structured "some tracks were excluded"
//!   signal a caller can diff against the proposal and log once, replacing
//!   `RtmpTransport`'s own `warned_refused_tracks` flag.
//! - [`NegotiationOutcome::Refused`] from `renegotiate` only, when the
//!   *proposed* carriable subset differs from what was already accepted —
//!   once a connection has committed to a track selection (RTMP: sequence
//!   headers already sent for it), changing which tracks are carried
//!   requires the caller to notice and truthfully refuse (issue #781) rather
//!   than silently carrying an uninitialised new track or dropping a
//!   previously-carried one.

use std::collections::VecDeque;

use bytes::Bytes;
use media_plane::egress::{NegotiationOutcome, PushEgress, TrackSelection};
use media_plane::trunk::SampleCursorItem;
use transmux::ir::{Media, Track, TrackSpec};

use crate::push::{PushTransport, SendMediaError};

/// [`PushEgress`] over an owned `T: PushTransport` — see the module doc.
pub struct PushTransportEgress<T: PushTransport> {
    transport: T,
    /// The currently-accepted selection (empty until the first successful
    /// `negotiate`).
    selection: TrackSelection,
    /// The full [`TrackSpec`]s backing `selection`, in the same order —
    /// `send` looks a drained item's spec up here to build the single-sample
    /// [`Media`] it hands to [`PushTransport::encode_media`].
    specs: Vec<TrackSpec>,
    /// Set once the first `negotiate` accepts a selection — governs whether
    /// `renegotiate` may still freely adopt a changed proposal (see the
    /// module doc's `Refused` bullet).
    committed: bool,
    /// This transport's static "why nothing here is carriable" message —
    /// e.g. `"no AVC video or AAC audio track to publish over RTMP"`. Reused
    /// verbatim for [`NegotiationOutcome::Error`] from both `negotiate` and
    /// `renegotiate`, so the two call sites can never drift.
    unsatisfiable_reason: &'static str,
    /// Encoded wire messages [`PushEgress::send`] has queued but
    /// [`PushEgress::poll_transmit`] has not yet handed to
    /// [`Self::flush_transmit`].
    outbound: VecDeque<Bytes>,
}

/// How a [`PushTransportEgress::flush_transmit_bounded`] ended.
#[derive(Debug)]
pub(crate) enum FlushOutcome {
    /// Everything queued was written.
    Flushed,
    /// The cancellation token fired first.
    Cancelled,
    /// The write did not finish within the limit (a stalled peer).
    TimedOut,
    /// The transport reported a write failure.
    Failed(Box<dyn std::error::Error + Send + Sync>),
}

impl<T: PushTransport> PushTransportEgress<T> {
    /// Wrap an already-connected `transport`. `unsatisfiable_reason` is the
    /// message [`NegotiationOutcome::Error`] carries when a proposed track
    /// set has no carriable track at all (e.g. `"no AVC video or AAC audio
    /// track to publish over RTMP"`).
    pub fn new(transport: T, unsatisfiable_reason: &'static str) -> Self {
        PushTransportEgress {
            transport,
            selection: TrackSelection::new(Vec::new()),
            specs: Vec::new(),
            committed: false,
            unsatisfiable_reason,
            outbound: VecDeque::new(),
        }
    }

    /// The [`TrackSpec`]s currently selected (the last `Accepted` outcome) —
    /// what a driving loop passes to [`PushTransport::setup`] after a
    /// successful [`PushEgress::negotiate`].
    pub fn selected_tracks(&self) -> &[TrackSpec] {
        &self.specs
    }

    /// Tear the transport down and hand it back — mirrors
    /// [`PushTransport::close`], exposed here since [`PushEgress`] itself
    /// has no teardown method (a driving loop that owns this adapter reaches
    /// through to the transport for lifecycle calls `PushEgress` doesn't
    /// model: `connect`/`setup`/`close`).
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Drains every wire message [`PushEgress::send`] has queued via
    /// [`PushEgress::poll_transmit`] and actually writes each one out over
    /// the real transport, via [`PushTransport::write_message`] (**not**
    /// [`PushTransport::send`] — `send`'s contract is "frame this payload",
    /// which would double-frame what [`PushTransport::encode_media`]
    /// already produced; see that method's own doc) — the async half of
    /// this adapter, called once per driving-loop iteration after a batch
    /// of `send` calls (see the module doc for why `PushEgress::send`
    /// itself cannot do this: it is synchronous).
    pub async fn flush_transmit(&mut self) -> Result<(), SendMediaError> {
        while let Some(message) = PushEgress::poll_transmit(self) {
            self.transport
                .write_message(&message)
                .await
                .map_err(|e| SendMediaError::Transport(Box::new(e)))?;
        }
        Ok(())
    }

    /// [`Self::flush_transmit`], but bounded: ends early when `cancel` fires
    /// and gives up after `limit`. A transport write can block indefinitely
    /// on a live peer that keeps the connection up but stops consuming
    /// (SRT's `send` waits for flow-window room), and a driving loop that
    /// only polled its cancellation token between passes would then never
    /// notice a shutdown.
    pub(crate) async fn flush_transmit_bounded(
        &mut self,
        cancel: &tokio_util::sync::CancellationToken,
        limit: std::time::Duration,
    ) -> FlushOutcome {
        tokio::select! {
            () = cancel.cancelled() => FlushOutcome::Cancelled,
            r = tokio::time::timeout(limit, self.flush_transmit()) => match r {
                Err(_) => FlushOutcome::TimedOut,
                Ok(Ok(())) => FlushOutcome::Flushed,
                Ok(Err(SendMediaError::Transport(err))) => FlushOutcome::Failed(err),
                // Only transport failures end a connection (as before).
                Ok(Err(_)) => FlushOutcome::Flushed,
            },
        }
    }

    /// `tracks` filtered to what `self.transport` can carry — the one
    /// predicate `negotiate`/`renegotiate` both apply (see the module doc's
    /// "Where the codec-refusal vocabulary moved" section).
    fn carriable(&self, tracks: &[TrackSpec]) -> Vec<TrackSpec> {
        tracks
            .iter()
            .filter(|t| self.transport.supports_codec(&t.config))
            .cloned()
            .collect()
    }
}

impl<T: PushTransport> PushEgress for PushTransportEgress<T> {
    type Error = SendMediaError;

    fn negotiate(&mut self, tracks: &[TrackSpec]) -> NegotiationOutcome<Self::Error> {
        let carriable = self.carriable(tracks);
        if carriable.is_empty() {
            return NegotiationOutcome::Error(SendMediaError::Mux(
                self.unsatisfiable_reason.to_string(),
            ));
        }
        self.selection = TrackSelection::new(carriable.iter().map(|t| t.track_id).collect());
        self.specs = carriable;
        self.committed = true;
        NegotiationOutcome::Accepted(self.selection.clone())
    }

    /// See the module doc's `Refused` bullet: once `committed`, a proposed
    /// carriable subset that differs from what is already selected is a
    /// truthful refusal (issue #781), not a silent re-adoption — the
    /// selection this adapter keeps carrying is whichever one was last
    /// `Accepted`. A proposal whose carriable subset is *unchanged* (e.g. an
    /// unrelated track's config was updated, or a non-carriable track came
    /// and went) is accepted trivially, since nothing about what this
    /// output actually carries would change.
    fn renegotiate(&mut self, tracks: &[TrackSpec]) -> NegotiationOutcome<Self::Error> {
        let carriable = self.carriable(tracks);
        if carriable.is_empty() {
            return NegotiationOutcome::Error(SendMediaError::Mux(
                self.unsatisfiable_reason.to_string(),
            ));
        }
        let proposed_ids: Vec<u32> = carriable.iter().map(|t| t.track_id).collect();
        if self.committed && proposed_ids != self.selection.track_ids {
            return NegotiationOutcome::Refused {
                reason: "already publishing; cannot change the carried track set mid-session",
            };
        }
        self.selection = TrackSelection::new(proposed_ids);
        self.specs = carriable;
        self.committed = true;
        NegotiationOutcome::Accepted(self.selection.clone())
    }

    /// Builds a single-sample [`Media`] for `item`'s track and encodes it
    /// via [`PushTransport::encode_media`] (sans-IO), queuing the resulting
    /// wire message(s) for [`Self::flush_transmit`] to actually send. A
    /// track not in the current selection (excluded by negotiation, or
    /// dropped by a since-refused `renegotiate`) is silently skipped —
    /// exclusion was already reported structurally when it was decided, not
    /// new data loss each time a sample for it arrives. A
    /// [`SampleCursorItem::Lagged`]/`Degraded` loss report has no sample to
    /// encode; logging it here (rather than truly ignoring it, per this
    /// method's own trait contract) is this adapter's reaction.
    fn send(&mut self, item: &SampleCursorItem) -> Result<(), Self::Error> {
        let (track_id, sample) = match item {
            SampleCursorItem::Timed { track_id, sample }
            | SampleCursorItem::Sparse { track_id, sample } => (*track_id, sample.clone()),
            SampleCursorItem::Lagged { skipped } => {
                tracing::warn!(skipped, "push egress cursor lagged; samples were dropped");
                return Ok(());
            }
            SampleCursorItem::Degraded { skipped } => {
                tracing::warn!(
                    skipped,
                    "push egress cursor degraded (sparse-ring eviction); consumer state may be stale"
                );
                return Ok(());
            }
            // `#[non_exhaustive]`: a future cursor item this adapter has no
            // reaction to yet is treated as nothing to encode this call,
            // exactly like `crate::push::drive_push`'s own `_ => {}` arm.
            _ => return Ok(()),
        };
        let Some(spec) = self.specs.iter().find(|s| s.track_id == track_id).cloned() else {
            return Ok(());
        };
        let timescale = spec.timescale;
        let media = Media::new(vec![Track::new(spec, vec![sample])], timescale);
        let messages = self.transport.encode_media(&media)?;
        self.outbound.extend(messages);
        Ok(())
    }

    fn poll_transmit(&mut self) -> Option<Bytes> {
        self.outbound.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use transmux::CodecConfig;

    /// A fake transport double: no real socket, just an in-memory record of
    /// what `send`/`encode_media` were asked to do — enough to test
    /// [`PushTransportEgress`]'s negotiation/send logic without a real RTMP
    /// server (that's `multimux/tests/push_rtmp.rs`'s job, for the transport
    /// itself).
    #[derive(Default)]
    struct FakeTransport {
        sent: Vec<Bytes>,
        /// Only [`CodecConfig::Avc`]/[`CodecConfig::Aac`] — mirrors
        /// `push::rtmp::RtmpTransport`'s real restriction, so this double
        /// exercises the exact refusal shape RTMP needs.
        restrict_to_avc_aac: bool,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("fake transport error")]
    struct FakeError;

    #[async_trait]
    impl PushTransport for FakeTransport {
        type Config = ();
        type Error = FakeError;

        async fn connect(_url: &str, _config: &Self::Config) -> Result<Self, Self::Error> {
            Ok(Self::default())
        }

        async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
            self.sent.push(Bytes::copy_from_slice(data));
            Ok(())
        }

        fn supports_codec(&self, config: &CodecConfig) -> bool {
            if self.restrict_to_avc_aac {
                matches!(config, CodecConfig::Avc { .. } | CodecConfig::Aac { .. })
            } else {
                true
            }
        }

        fn close(&mut self) {}
    }

    fn avc_spec(track_id: u32) -> TrackSpec {
        TrackSpec::new(
            track_id,
            90_000,
            CodecConfig::Avc {
                config: transmux::AVCConfigurationBox::new(
                    transmux::AVCDecoderConfigurationRecord {
                        configuration_version: 1,
                        profile_indication: 0x42,
                        profile_compatibility: 0,
                        level_indication: 0x1f,
                        length_size_minus_one: 3,
                        sps: Vec::new(),
                        pps: Vec::new(),
                        chroma_format: None,
                        bit_depth_luma_minus8: None,
                        bit_depth_chroma_minus8: None,
                        sps_ext: Vec::new(),
                    },
                ),
                width: 0,
                height: 0,
            },
        )
    }

    fn opaque_spec(track_id: u32) -> TrackSpec {
        TrackSpec::new(
            track_id,
            90_000,
            CodecConfig::Data {
                stream_type: 0x06,
                descriptors: Vec::new(),
                carriage: transmux::ir::DataCarriage::Pes,
            },
        )
    }

    /// One minimal length-prefixed (AVCC-style) NAL, valid enough for
    /// `TsMux`'s AVC path (the default `encode_media`) to package without
    /// erroring: a 4-byte big-endian length prefix + a one-byte IDR-slice
    /// NAL header (type 5, per ISO/IEC 14496-10 Table 7-1) and no payload.
    fn sample() -> transmux::ir::Sample {
        let nal: &[u8] = &[0, 0, 0, 1, 0x65];
        transmux::ir::Sample::new(
            bytes::Bytes::copy_from_slice(nal),
            Some(0),
            Some(0),
            Some(3_000),
            true,
        )
    }

    /// A transport with no codec restriction (SRT/RTSP's real shape)
    /// accepts every proposed track.
    #[test]
    fn unrestricted_transport_accepts_every_track() {
        let mut egress = PushTransportEgress::new(FakeTransport::default(), "unreachable");
        match egress.negotiate(&[avc_spec(1), opaque_spec(2)]) {
            NegotiationOutcome::Accepted(sel) => {
                assert_eq!(sel.track_ids, vec![1, 2]);
            }
            other => panic!("expected Accepted, got {other:?}"),
        }
    }

    /// MUTATION-CHECKED: this is the test that would have caught the
    /// pre-#942 `RtmpTransport::setup` hard error being silently dropped —
    /// changing `negotiate`'s empty-check from `carriable.is_empty()` to
    /// `false` (never refusing) makes this assert fail (`Accepted` instead
    /// of `Error`), because an opaque-only track set would then be
    /// (wrongly) accepted with an empty `TrackSelection`. Restored
    /// afterward; see the crate's own bite-proof discipline.
    #[test]
    fn restricted_transport_errors_when_nothing_is_carriable() {
        let transport = FakeTransport {
            restrict_to_avc_aac: true,
            ..Default::default()
        };
        let mut egress = PushTransportEgress::new(transport, "no AVC video or AAC audio track");
        match egress.negotiate(&[opaque_spec(9)]) {
            NegotiationOutcome::Error(SendMediaError::Mux(reason)) => {
                assert_eq!(reason, "no AVC video or AAC audio track");
            }
            other => panic!("expected Error(Mux(..)), got {other:?}"),
        }
    }

    /// A restricted transport with a *mixed* track set accepts, but the
    /// selection names only the carriable subset — the structured
    /// replacement for `RtmpTransport`'s old `warned_refused_tracks` flag: a
    /// caller diffing `tracks.len()` against `sel.track_ids.len()` gets the
    /// same "some tracks excluded" fact, without a private flag inside the
    /// transport.
    #[test]
    fn restricted_transport_selects_only_the_carriable_subset() {
        let transport = FakeTransport {
            restrict_to_avc_aac: true,
            ..Default::default()
        };
        let mut egress = PushTransportEgress::new(transport, "no AVC video or AAC audio track");
        match egress.negotiate(&[avc_spec(1), opaque_spec(2)]) {
            NegotiationOutcome::Accepted(sel) => assert_eq!(sel.track_ids, vec![1]),
            other => panic!("expected Accepted([1]), got {other:?}"),
        }
        assert_eq!(egress.selected_tracks().len(), 1);
    }

    /// Issue #781's shape, on the push side: once committed, a `renegotiate`
    /// that would change the carried set is truthfully `Refused`, not
    /// silently adopted.
    ///
    /// MUTATION-CHECKED: replacing the `Refused` branch with
    /// `self.selection = TrackSelection::new(proposed_ids); NegotiationOutcome::Accepted(..)`
    /// (silently adopting the change) makes this test's `match` fall into
    /// the `other => panic!` arm instead of matching `Refused` — exactly
    /// the silent-drop failure mode issue #781 already demonstrated on the
    /// ingest side. Reverted after confirming the failure.
    #[test]
    fn renegotiate_refuses_a_track_set_change_once_committed() {
        let mut egress = PushTransportEgress::new(FakeTransport::default(), "unreachable");
        egress.negotiate(&[avc_spec(1)]);
        match egress.renegotiate(&[avc_spec(1), avc_spec(2)]) {
            NegotiationOutcome::Refused { reason } => assert!(!reason.is_empty()),
            other => panic!("expected Refused, got {other:?}"),
        }
        // Still carrying only the originally-accepted selection.
        assert_eq!(egress.selected_tracks().len(), 1);
    }

    /// `send` for a track outside the negotiated selection is a no-op (not
    /// an error, not a panic) — the exclusion was already reported by
    /// `negotiate`/`renegotiate`.
    #[test]
    fn send_for_an_unselected_track_is_a_silent_no_op() {
        let transport = FakeTransport {
            restrict_to_avc_aac: true,
            ..Default::default()
        };
        let mut egress = PushTransportEgress::new(transport, "no AVC video or AAC audio track");
        egress.negotiate(&[avc_spec(1)]);
        egress
            .send(&SampleCursorItem::Timed {
                track_id: 2, // never selected
                sample: sample(),
            })
            .expect("must not error");
        assert!(
            PushEgress::poll_transmit(&mut egress).is_none(),
            "an unselected track's sample must never reach the transport"
        );
    }

    /// The real bite: `send` for a selected track queues a real encoded
    /// message, drained via `poll_transmit` and written by
    /// `flush_transmit` — proving the sans-IO `send`/async
    /// `flush_transmit` split (the module doc's whole point) actually
    /// carries a byte through both halves.
    #[tokio::test]
    async fn send_then_flush_transmit_writes_a_real_encoded_message() {
        let mut egress = PushTransportEgress::new(FakeTransport::default(), "unreachable");
        egress.negotiate(&[avc_spec(1)]);
        egress
            .send(&SampleCursorItem::Timed {
                track_id: 1,
                sample: sample(),
            })
            .expect("send must not error");
        assert!(
            !egress.outbound.is_empty(),
            "send must have queued at least one encoded message"
        );
        egress.flush_transmit().await.expect("flush must not error");
        assert!(
            egress.outbound.is_empty(),
            "flush_transmit must drain everything it queued"
        );
        assert!(
            !egress.transport.sent.is_empty(),
            "the real transport must have received the flushed bytes"
        );
    }

    /// A transport whose writes never complete (a live peer that stopped
    /// consuming).
    #[derive(Default)]
    struct HangingTransport;

    #[async_trait]
    impl PushTransport for HangingTransport {
        /// Counts connects, so each test sees only its own.
        type Config = std::sync::Arc<std::sync::atomic::AtomicUsize>;
        type Error = FakeError;

        async fn connect(_url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
            config.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Self)
        }

        async fn send(&mut self, _data: &[u8]) -> Result<(), Self::Error> {
            std::future::pending().await
        }

        fn close(&mut self) {}
    }

    fn egress_with_queued_message() -> PushTransportEgress<HangingTransport> {
        let mut egress = PushTransportEgress::new(HangingTransport, "unreachable");
        egress.negotiate(&[avc_spec(1)]);
        egress
            .send(&SampleCursorItem::Timed {
                track_id: 1,
                sample: sample(),
            })
            .expect("send");
        assert!(!egress.outbound.is_empty());
        egress
    }

    /// A stalled write ends promptly on cancellation instead of wedging the
    /// push task (paused clock: no real waiting).
    #[tokio::test(start_paused = true)]
    async fn a_stalled_flush_is_cancellable() {
        let mut egress = egress_with_queued_message();
        let cancel = tokio_util::sync::CancellationToken::new();
        let canceller = cancel.clone();
        tokio::spawn(async move { canceller.cancel() });
        let outcome = egress
            .flush_transmit_bounded(&cancel, std::time::Duration::from_secs(3600))
            .await;
        assert!(matches!(outcome, FlushOutcome::Cancelled), "{outcome:?}");
    }

    /// A stalled write that nobody cancels times out, which `drive_push`
    /// treats as a push failure (reconnect).
    #[tokio::test(start_paused = true)]
    async fn a_stalled_flush_times_out() {
        let mut egress = egress_with_queued_message();
        let cancel = tokio_util::sync::CancellationToken::new();
        let outcome = egress
            .flush_transmit_bounded(&cancel, std::time::Duration::from_secs(5))
            .await;
        assert!(matches!(outcome, FlushOutcome::TimedOut), "{outcome:?}");
    }

    /// A healthy flush is unaffected.
    #[tokio::test]
    async fn a_healthy_flush_completes_within_the_bound() {
        let mut egress = PushTransportEgress::new(FakeTransport::default(), "unreachable");
        egress.negotiate(&[avc_spec(1)]);
        egress
            .send(&SampleCursorItem::Timed {
                track_id: 1,
                sample: sample(),
            })
            .expect("send");
        let cancel = tokio_util::sync::CancellationToken::new();
        let outcome = egress
            .flush_transmit_bounded(&cancel, std::time::Duration::from_secs(5))
            .await;
        assert!(matches!(outcome, FlushOutcome::Flushed), "{outcome:?}");
        assert!(egress.outbound.is_empty());
    }

    /// A `drive_push` whose transport write never completes (a live peer that
    /// stopped consuming) is wired through the bounded flush: cancellation
    /// ends the task promptly, and without cancellation the stall times out
    /// and the task reconnects. Both used to wedge the task (cancellation was
    /// only checked between passes, and the write had no timeout).
    async fn stalled_push_trunk() -> (
        std::sync::Arc<media_plane::trunk::Trunk>,
        media_plane::trunk::TrunkWriter,
    ) {
        use std::num::NonZeroUsize;
        let nz = |n| NonZeroUsize::new(n).unwrap();
        let trunk = media_plane::trunk::Trunk::new(media_plane::trunk::TrunkConfig::new(
            nz(64),
            nz(64),
            nz(64),
            nz(64),
            nz(64),
        ));
        let writer = trunk.writer().expect("the writer is free");
        writer.set_tracks(vec![avc_spec(1)]);
        (trunk, writer)
    }

    fn zero_backoff() -> crate::config::ReconnectPolicy {
        crate::config::ReconnectPolicy {
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            max_attempts: None,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_push_stalled_on_a_live_peer_ends_promptly_on_cancel() {
        let (trunk, writer) = stalled_push_trunk().await;
        let cancel = tokio_util::sync::CancellationToken::new();
        let task = tokio::spawn(crate::push::drive_push::<HangingTransport>(
            trunk,
            "push://stalled".to_string(),
            std::sync::Arc::default(),
            crate::config::PushFormat::Ts,
            zero_backoff(),
            cancel.clone(),
        ));
        // Let it connect (its cursor starts at the live edge), then publish a
        // sample: it is queued and the write blocks.
        for _ in 0..20 {
            tokio::time::advance(std::time::Duration::from_millis(100)).await;
            tokio::task::yield_now().await;
        }
        writer.publish(1, media_plane::trunk::RetentionClass::Timed, sample());
        for _ in 0..20 {
            tokio::time::advance(std::time::Duration::from_millis(100)).await;
            tokio::task::yield_now().await;
        }
        assert!(!task.is_finished(), "still stuck in the stalled write");
        cancel.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("cancel must end a push stalled in a write")
            .expect("join");
    }

    #[tokio::test(start_paused = true)]
    async fn a_push_stalled_on_a_live_peer_times_out_and_reconnects() {
        let (trunk, writer) = stalled_push_trunk().await;
        let cancel = tokio_util::sync::CancellationToken::new();
        let connects = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let task = tokio::spawn(crate::push::drive_push::<HangingTransport>(
            trunk,
            "push://stalled".to_string(),
            std::sync::Arc::clone(&connects),
            crate::config::PushFormat::Ts,
            zero_backoff(),
            cancel.clone(),
        ));
        // Well past the flush timeout, feeding one sample per step so every
        // fresh connection has something to block on.
        for _ in 0..300 {
            writer.publish(1, media_plane::trunk::RetentionClass::Timed, sample());
            tokio::time::advance(std::time::Duration::from_millis(100)).await;
            tokio::task::yield_now().await;
        }
        cancel.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), task).await;
        assert!(
            connects.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "a stalled write must be treated as a push failure and reconnected"
        );
    }
}
