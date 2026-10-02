//! Streaming RTP depayloader — RFC 6184 (H.264) / RFC 3640 (AAC).
//!
//! Stateful counterpart to [`crate::rtp::RtpDepacketiser`]: fed RTP packets
//! incrementally via [`RtpStreamDepacketiser::push`], it emits fully-timed
//! [`Sample`]s (real per-AU `duration` from RTP-timestamp deltas, `is_sync`
//! from IDR detection) carrying the real [`CodecConfig`] supplied at
//! construction (e.g. from [`crate::rtp_sdp`]).
//!
//! See [`transmux/docs/rtp/rtp-payload-formats.md`](../rtp/rtp-payload-formats.md)
//! for the RFC background and timing-model specification.
//!
//! # Timing model
//!
//! - Per track, the IR timescale is the RTP `clock_rate` (video: 90 kHz;
//!   AAC: the sample rate).
//! - A sample's `duration` is the RTP-timestamp delta to the *next* access
//!   unit's timestamp — so a sample can only be emitted once the following
//!   AU's timestamp is known (one-AU emission latency). `flush`
//!   emits the final pending AU using the last-computed duration (there is
//!   no "next" AU to measure against).
//! - The 32-bit wire RTP timestamp is unwrapped to a monotonic `u64` **once,
//!   here at the demux edge** (RFC 3550 §5.1), and carried into each sample's
//!   **absolute** `dts`/`pts` (media plane step 2c) — nothing downstream
//!   re-derives it.
//! - `is_sync` comes straight from the reassembled access unit
//!   (IDR detection for video; always `true` for audio).
//! - `dts == pts`, i.e. a zero composition offset — **v1 assumes low-delay
//!   H.264 with no B-frame reorder**: RTP carries only a presentation
//!   timestamp on the wire (RFC 6184 §5.1's sampling time), so reconstructing
//!   a separate DTS (and therefore a non-zero composition offset) when
//!   B-frames are present is future work. That assumption is **reported when
//!   it fails** — a wire timestamp earlier than a previous one raises
//!   [`RtpLossEvent::NonMonotonicTimestamp`] (audit r04-W29), because RTP
//!   carries no decode time to fall back on. A consumer needing real
//!   per-frame durations or a decode-ordered timeline should use the
//!   single-shot [`crate::rtp::RtpDepacketiser`] path, which holds the whole
//!   stream and therefore knows the frame period exactly.
//! - Each track independently rebases its first unwrapped timestamp to
//!   `start_decode_time = 0` (via the caller building [`crate::media::Track`]
//!   from `track_specs` + emitted samples) **unless** an RTCP Sender Report
//!   has been fed for at least two tracks (issue #722): RTP timestamps alone
//!   carry no cross-stream relationship (RFC 3550 §5.1 — each SSRC's clock
//!   has an arbitrary random offset), so recovering true A/V sync needs the
//!   NTP-wallclock ↔ RTP-timestamp correlation each Sender Report carries
//!   (RFC 3550 §6.4.1). Feed reports via [`RtpStreamDepacketiser::push_sender_report`]
//!   / [`RtpStreamDepacketiser::push_rtcp`] as they arrive on a track's RTCP
//!   channel, then read [`RtpStreamDepacketiser::sync_start_decode_times`]
//!   once at least two tracks have an anchor: it maps every anchored
//!   track's first sample onto one common wallclock and returns each
//!   track's `start_decode_time` (in that track's own `clock_rate` ticks)
//!   relative to the earliest of them — preserving the real inter-track
//!   offset instead of discarding it. This is strictly additive: with no
//!   Sender Reports fed (or fewer than two anchored tracks), the method
//!   returns an empty `Vec` and callers keep the v1
//!   independent-rebase-to-0 behaviour unchanged.
//! - When an RFC 3640 `AAC-hbr` packet aggregates more than one access unit,
//!   all AUs in that packet share the RTP timestamp, so non-final AUs get
//!   `duration = 0`; v1 assumes one AU per packet, which is what transmux's
//!   own packetiser emits.
//!
//! # Loss and reorder detection (issue #779)
//!
//! RTP runs over UDP: loss, reordering, and duplication are all normal, not
//! exceptional. Before this, [`push`](RtpStreamDepacketiser::push) decided access-unit
//! boundaries purely from the RTP timestamp and the marker bit, and never
//! looked at the sequence number at all — a dropped FU-A fragment was
//! concatenated with its neighbours into a malformed access unit and handed
//! downstream with no diagnostic trail.
//!
//! Each track now keeps RFC 3550 §5.1 sequence-number state, compared with
//! **wrapping** arithmetic (never `>` — the field is 16 bits and wraps every
//! 65536 packets, exactly like this module's own `unwrap_ts` for the
//! 32-bit timestamp). RFC 3550 §A.1's `update_seq` is the standard's
//! validity-check algorithm; the discipline it embodies (wrapping
//! comparison, SSRC-scoped state, treating a new SSRC as a new source) is
//! transcribed and cited in full at
//! `transmux/docs/rtp/rtp-sequence-validation.md`, along with a precise
//! account of where this implementation follows it and where it must
//! diverge (that RFC's algorithm classifies validity for RTCP statistics; it
//! never reorders, because nothing in plain RTCP loss accounting needs
//! packets delivered in order — H.264 FU-A reassembly does).
//!
//! - **Source validation (§A.1 `probation`)**: the first packet of a session
//!   takes the new-source branch and does *not* run `update_seq`, so the
//!   `MIN_SEQUENTIAL` = 2 sequential packets §A.1 requires complete on the
//!   **third** packet seen. No media is delayed for probation (see
//!   `SeqState::admit`), so this only means the source is declared valid one
//!   packet later than §A.1's own count would suggest.
//! - **In order**: reassembled immediately, as before.
//! - **Reordered within a small window**: held in a bounded buffer
//!   (`RtpStreamTrack::with_reorder_depth`, default [`DEFAULT_REORDER_DEPTH`])
//!   keyed by wire sequence number, and replayed in the correct order once
//!   the gap fills — reassembly then sees exactly the in-order byte stream.
//!   The buffer is a **hard bound**: this project has already shipped four
//!   unbounded-allocation vectors from RTP/TS input (`MAX_AU_BUFFER_BYTES`
//!   above is one of the fixes), so the reorder buffer never grows past its
//!   configured depth (momentarily `depth + 1` while a just-arrived packet
//!   is considered for the "closest to the hole" resume choice, then
//!   immediately collapsed back down — see `SeqState::force_resolve`).
//! - **Gap** (the buffer's window is exhausted, or end of stream forces a
//!   decision — [`RtpStreamDepacketiser::flush`]): the access unit under construction, if
//!   any, is dropped rather than reassembled from a run missing a fragment;
//!   [`RtpLossEvent::SequenceGap`] is recorded.
//! - **Duplicate or misordered** (behind what the buffer waits for, or already
//!   sitting in it): discarded silently, exactly as RFC 3550 §A.1's
//!   fall-through "duplicate or reordered packet" arm treats it. Two costs are
//!   inherent to that rule and worth stating:
//!   - a jump **at most `MAX_MISORDER` (100) behind** `max_seq` is *accepted* by
//!     §A.1 as misordered, and only then found to be behind `expected` and
//!     dropped by the reorder buffer — so a backward move of up to 100 sequence
//!     numbers is lost media with no [`RtpLossEvent`]. §A.1 treats it that way
//!     because for loss statistics a small backward step is indistinguishable
//!     from a reorder, and a real seek or restart moves much further;
//!   - the first packet of a post-seek run is the one that *arms* `bad_seq`, so
//!     it is discarded (§A.1 returns 0 for it). A run whose first packet was an
//!     FU-A start therefore loses that fragment, which damages the access unit
//!     it belonged to (`RtpLossEvent::DamagedAccessUnit`); the run itself
//!     continues from its second packet.
//! - **Wild jump needing confirmation** (§A.1: ahead of `max_seq` by
//!   `MAX_DROPOUT` or more, or behind by more than `MAX_MISORDER`): the packet
//!   is **discarded** and `bad_seq = (seq + 1) & (RTP_SEQ_MOD - 1)` is
//!   remembered. Only if the *next* packet carries `bad_seq` — §A.1's "two
//!   sequential packets ... the other side restarted without telling us" — is
//!   the source re-synced. One stray far-behind packet therefore cannot move
//!   the stream's baseline at all (the pre-fix bug), and a real seek or
//!   restart, which always brings a *run* of packets at the new numbering,
//!   is adopted on its second packet.
//! - **Resync** (the confirmed restart above, e.g. an RTSP seek or a source
//!   restart that kept its SSRC): the baseline moves onto the new numbering,
//!   the in-progress access unit is dropped (it straddles the seam) and the
//!   reorder buffer is cleared, and the track's timestamp origin is
//!   re-established — the new run's timeline does not inherit a delta measured
//!   across the discontinuity, because each source instance's RTP clock origin
//!   is random (§5.1). No [`RtpLossEvent`]: the sender renumbered, nothing was
//!   lost. Because §A.1's `init_seq` is all that carries over, the *next* run
//!   is an ordinary source from then on — nothing about the seam bounds it,
//!   so a run of any length (including one that wraps past the old numbering)
//!   is delivered in full, and a second seek resyncs the same way.
//!
//! ## Where the signal surfaces (design decision)
//!
//! Issue #778 (the MPEG-TS sibling of this issue — continuity-counter gaps
//! and `transport_error_indicator`) concluded that TR 101 290-style loss
//! belongs in the media plane's byte-level tap, *not* in the demux family's
//! own [`crate::ir::DemuxEvent`] vocabulary, because detecting a CC gap is a
//! pure re-scan of raw bytes: `media-doctor`, `dvb-conformance`, and
//! `ts-fix` each already implement it independently, entirely outside any
//! demuxer, over the same bytes a demuxer merely happens to also be
//! parsing. Widening `DemuxEvent` there would have duplicated a detector
//! that already exists three times over, in the wrong layer.
//!
//! None of that applies here, and the signal surfaces locally instead — a
//! new [`RtpLossEvent`], polled via [`RtpStreamDepacketiser::poll_loss_event`], scoped to
//! this module rather than folded into `DemuxEvent` (whose own doc comment
//! already draws its boundary at "the demux family's own vocabulary" —
//! `StreamingTsDemux` + `StreamingFlvDemux` — and this depacketiser is a
//! different spoke of the hub, not a member of that family):
//!
//! 1. **No detector to bridge.** Unlike #778, there is no existing RTP loss
//!    monitor anywhere in this workspace to wire in; it has to be written
//!    somewhere, and there is no "elsewhere" that already does this.
//! 2. **The signal and the corrective action share one piece of state.**
//!    A CC gap is diagnostic-only-by-construction: a tap can compute
//!    "sequence N is missing" from the wire bytes alone, without needing to
//!    know anything about framing, because nothing about *acting* on that
//!    gap requires framing state (TS payloads are independently
//!    re-synchronisable). An RTP sequence gap is different: knowing a
//!    packet is missing is only useful once paired with knowing *which*
//!    in-progress NAL it was fragmenting — and that is `TrackState`'s own
//!    `cur_pkts`/`cur_ts`, private reassembly state a separate tap could not
//!    see without independently re-implementing this entire module's FU-A
//!    tracking. The gap decision and the "drop this access unit" decision
//!    are the same decision, made from the same state, at the same moment —
//!    so they belong in the same place.
//!
//! A future consumer wanting this folded into a wider cross-container
//! vocabulary (mirroring `DemuxEvent`) remains free to do so additively —
//! [`RtpLossEvent`] is `#[non_exhaustive]` for exactly that reason — but
//! that is a decision for whoever builds that cross-container layer, not one
//! this module should presume.

use crate::error::{Error, Result};
use crate::pipeline::{CodecConfig, Sample, TrackSpec};
use crate::rtcp::SenderReport;
use crate::rtp::{RtpMediaKind, parse_rtp_header, reassemble_audio, reassemble_video};
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use broadcast_common::{Demand, Parse, Stage, Timestamp};

/// Number of fractional-second units in the 32.32 fixed-point NTP timestamp
/// format an RTCP Sender Report's `ntp_msw`/`ntp_lsw` carry (RFC 3550 §6.4.1,
/// citing RFC 5905 §6 for the NTP timestamp format itself): `2^32`.
const NTP_FRACTION_SCALE: f64 = 4_294_967_296.0;

/// Hard cap, in bytes, on one track's in-progress access-unit buffer
/// (`TrackState::cur_pkts`, the raw RTP packets accumulated since the last
/// completed AU) — the same buffer that ultimately feeds
/// [`crate::rtp::reassemble_video`]'s FU-A `fu_buf`, so bounding it here
/// transitively bounds that too. A real access unit (even a 4K IDR frame)
/// is at most a few hundred KB; this is comfortably above that, but far
/// below what a malformed/hostile stream — a dropped final FU-A fragment
/// (`E=1` never seen) or a marker bit that's never set — would otherwise
/// accumulate for the life of the (indefinitely long, per the P0 reconnect
/// loop) session: an unbounded-memory DoS (audit-ingest #4). On overflow the
/// in-progress AU is dropped and [`RtpStreamDepacketiser::push`] returns a
/// recoverable [`Error::BufferCapExceeded`]; internal state is already reset
/// so the next packet starts a fresh AU (resync on the next timestamp change
/// or marker bit).
const MAX_AU_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// Default depth (in packets) of the bounded out-of-order reorder buffer
/// (see the module docs' "Loss and reorder detection" section and
/// `SeqState`), used unless a track is built with
/// [`RtpStreamTrack::with_reorder_depth`].
///
/// Chosen generously above the reorder distances a real RTSP/RTP session on
/// a managed network produces (typically a handful of packet positions at
/// most) while keeping the worst case tiny: `DEFAULT_REORDER_DEPTH + 1`
/// packets at the network MTU (a few KB) sit in the buffer at any instant,
/// nowhere near `MAX_AU_BUFFER_BYTES`. This is deliberate — this project
/// has already shipped four unbounded-allocation DoS vectors from RTP/TS
/// input (audit-ingest #4; `MAX_AU_BUFFER_BYTES` above is one of the fixes),
/// and RTP is untrusted remote input over UDP, so a fifth was not an option.
pub const DEFAULT_REORDER_DEPTH: usize = 16;

/// The 16-bit sequence-number space, RFC 3550 §A.1's `RTP_SEQ_MOD` (`#define
/// RTP_SEQ_MOD (1<<16)` in the RFC's own sample header).
const RTP_SEQ_MOD: u32 = 1 << 16;

/// `RTP_SEQ_MOD + 1`, the value RFC 3550 §A.1's `init_seq` stores in `bad_seq`
/// "so seq == bad_seq is false": it is one past the largest representable
/// sequence number, so no 16-bit `seq` can ever equal it.
const RTP_SEQ_MOD_PLUS_ONE: u32 = RTP_SEQ_MOD + 1;

/// RFC 3550 §A.1's `MAX_DROPOUT` (3000 there, "based on ... a maximum dropout
/// of 1 minute"). A jump *ahead* of `max_seq` smaller than this is treated as
/// an ordinary loss ("in order, with permissible gap"); at or above it the
/// number is a wild jump needing confirmation.
const MAX_DROPOUT: u16 = 3000;

/// RFC 3550 §A.1's `MAX_MISORDER` (100 there, "based on a maximum misordering
/// time of 2 seconds at 50 packets/second"). A number behind `max_seq` by at
/// most this much is a legal reorder and is accepted; further behind and it is
/// either a duplicate or part of a wild jump.
const MAX_MISORDER: u32 = 100;

/// RFC 3550 §A.1's `MIN_SEQUENTIAL`: how many sequential packets a source must
/// produce before it is declared valid. Two, because the RTCP header check is
/// strong and a larger number would delay validation when packets are lost.
const MIN_SEQUENTIAL: u8 = 2;

/// `RTP_SEQ_MOD - MAX_MISORDER`, narrowed to `u16` for the §A.1 comparison
/// `udelta <= RTP_SEQ_MOD - MAX_MISORDER`. `MAX_MISORDER` (100) is far below
/// `RTP_SEQ_MOD` (65536), so the subtraction cannot underflow (asserted in this
/// module's tests).
const MAX_WILD_JUMP_U16: u16 = (RTP_SEQ_MOD - MAX_MISORDER) as u16;

/// Loss/reorder signal from [`RtpStreamDepacketiser`] (RFC 3550 §5.1
/// sequence-number semantics; see the module docs' "Loss and reorder
/// detection" section and `transmux/docs/rtp/rtp-sequence-validation.md` for
/// the RFC 3550 §A.1 discipline this is adapted from, and why the signal
/// surfaces here rather than in [`crate::ir::DemuxEvent`]).
///
/// `#[non_exhaustive]`: a future variant (e.g. a distinct signal for a
/// changed SSRC) is additive.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtpLossEvent {
    /// The sequence numbers strictly between `expected` and `got` were never
    /// recovered — either genuinely lost in transit, or reordered further
    /// than the bounded reorder window
    /// ([`RtpStreamTrack::with_reorder_depth`]) tolerates; from this
    /// depacketiser's point of view those are indistinguishable, and both
    /// require the same corrective action. Any access unit under
    /// construction at the moment the gap was detected was dropped rather
    /// than reassembled from a run missing a fragment.
    SequenceGap {
        /// The track this gap was observed on.
        track_id: u32,
        /// The SSRC (RFC 3550 §5.1) the gap was observed on.
        ssrc: u32,
        /// The sequence number that was expected next.
        expected: u16,
        /// The sequence number actually adopted as the new baseline.
        got: u16,
    },
    /// An access unit could not be reassembled after a [`Self::SequenceGap`]
    /// resync (e.g. the packet resumed on was itself an FU-A continuation
    /// fragment with no preceding start fragment in this run — RFC 6184
    /// §5.8) and was dropped. Always preceded, in the same
    /// [`RtpStreamDepacketiser::push`] call, by the `SequenceGap` that
    /// triggered the resync.
    DamagedAccessUnit {
        /// The track this occurred on.
        track_id: u32,
    },
    /// An access unit's RTP timestamp was **earlier** than the previous
    /// access unit's, i.e. the stream's presentation order is not wire order
    /// (a B-frame reorder).
    ///
    /// RTP carries exactly one timestamp per access unit and it is the
    /// *sampling/presentation* time (RFC 6184 §5.1 for H.264, RFC 3640 §3.3.1
    /// for AAC), so the wire alone cannot supply an independent decode time:
    /// this depacketiser emits `dts == pts` (the documented low-delay model),
    /// and a reordered source therefore yields `dts` values that step
    /// backward. That is not a defect in the reassembly — the samples are
    /// correct and in presentation order — but a consumer that requires
    /// decode-ordered `dts` (a progressive-file or fragment writer) must know
    /// it cannot use it as one. Reported once per occurrence so a caller can
    /// count or route it.
    NonMonotonicTimestamp {
        /// The track this occurred on.
        track_id: u32,
        /// The previous access unit's unwrapped presentation timestamp, in the
        /// track's clock (the same unit as [`crate::ir::Sample::pts`]).
        previous_pts: i64,
        /// This access unit's, which is earlier.
        pts: i64,
    },
}

// `RtpLossEvent` is a data-carrying ADT (each variant is a distinct
// structured signal, not a flat spec code) — see this crate's
// `tests/label_coverage.rs` SKIP list (same category as `DemuxEvent`), so it
// is intentionally exempt from the #204 `name()`/`impl_spec_display!`
// convention.

/// One track's decode config for [`RtpStreamDepacketiser`].
#[non_exhaustive]
pub struct RtpStreamTrack {
    /// Track ID (matches the IR `track_id` this depayloader emits).
    pub track_id: u32,
    /// The payload format carried on this track's RTP stream.
    pub kind: RtpMediaKind,
    /// Real codec config (e.g. from [`crate::rtp_sdp`]).
    pub config: CodecConfig,
    /// RTP clock rate (Hz) — also used as the IR track timescale.
    pub clock_rate: u32,
    /// Bounded reorder-buffer depth (packets) — see [`DEFAULT_REORDER_DEPTH`].
    reorder_depth: usize,
}

impl RtpStreamTrack {
    /// Build a track config from its fields, with the default reorder-buffer
    /// depth ([`DEFAULT_REORDER_DEPTH`]).
    pub fn new(track_id: u32, kind: RtpMediaKind, config: CodecConfig, clock_rate: u32) -> Self {
        Self {
            track_id,
            kind,
            config,
            clock_rate,
            reorder_depth: DEFAULT_REORDER_DEPTH,
        }
    }

    /// Override the bounded reorder-buffer depth (packets) for this track —
    /// see [`DEFAULT_REORDER_DEPTH`] and the module docs' "Loss and reorder
    /// detection" section. `0` disables reordering entirely (any
    /// out-of-order arrival is immediately treated as a gap).
    pub fn with_reorder_depth(mut self, reorder_depth: usize) -> Self {
        self.reorder_depth = reorder_depth;
        self
    }
}

/// Per-(track, SSRC) RTP sequence-number tracking state.
///
/// Two layers, because RFC 3550 §A.1 and an H.264 FU-A reassembler want
/// different things from the same field:
///
/// 1. **[`SeqValidity`]** — a faithful transcription of §A.1's `update_seq`,
///    including `init_seq`, `probation`/`MIN_SEQUENTIAL`, `MAX_DROPOUT`,
///    `MAX_MISORDER`, `bad_seq` and `cycles`. It classifies each arriving
///    sequence number as in-order-with-tolerable-gap, duplicate/misordered, a
///    wild jump needing confirmation, or a confirmed restart, using only the
///    wrapping arithmetic §A.1 prescribes.
/// 2. **The reorder buffer** (`held`/`expected`) — §A.1 never reorders,
///    because RTCP loss accounting does not need packets *delivered* in order.
///    FU-A reassembly does (RFC 6184 §5.8: fragment order *is* the data), so
///    an accepted packet ahead of the hole is held briefly and replayed in
///    order once the hole fills or the bound is reached (a declared gap).
#[derive(Default)]
struct SeqState {
    /// The SSRC this state was initialised for. `None` until the first
    /// packet is seen. A *changed* SSRC (RFC 3550 §8.2: a new source, e.g. a
    /// stream restart) resets tracking rather than raising a gap.
    ssrc: Option<u32>,
    /// Next sequence number the reorder buffer is waiting for.
    expected: Option<u16>,
    /// Packets received ahead of `expected`, held (raw wire bytes, keyed by
    /// their own sequence number) so they can be replayed in order once the
    /// hole fills. Bounded: never holds more than `reorder_depth` entries at
    /// rest (see [`Self::admit`] / [`Self::force_resolve`]).
    held: Vec<(u16, Vec<u8>)>,
    /// RFC 3550 §A.1's per-source validity state (`max_seq`/`bad_seq`/
    /// `cycles`/`probation`), kept in one field so "re-initialise the source"
    /// is a single call and cannot half-reset.
    validity: SeqValidity,
}

/// RFC 3550 §A.1's per-source validity state — a transcription of the RFC's
/// `source` fields that `update_seq` reads and writes (`base_seq`/`received`/
/// `expected_prior` are RTCP-statistics bookkeeping this depacketiser does not
/// keep, since it emits its own [`RtpLossEvent`] signal instead).
struct SeqValidity {
    /// §A.1 `max_seq`: the highest sequence number accepted so far (or the
    /// most recently seen one while in probation).
    max_seq: u16,
    /// §A.1 `bad_seq`: the number that, if the next packet carries it, means
    /// "two sequential packets" arrived at a wild jump and the source has
    /// restarted. `RTP_SEQ_MOD + 1` (i.e. not representable) after
    /// `init_seq`, "so seq == bad_seq is false".
    bad_seq: u32,
    /// §A.1 `cycles`: `RTP_SEQ_MOD` per wrapped 64k cycle. Counted and
    /// exposed for completeness, though this depacketiser's own ordering comes
    /// from the reorder buffer rather than the cycle count.
    cycles: u32,
    /// §A.1 `probation`: non-zero until `MIN_SEQUENTIAL` consecutive packets
    /// have been seen, so a stray packet cannot declare a source valid.
    probation: u8,
}

impl Default for SeqValidity {
    fn default() -> Self {
        Self {
            max_seq: 0,
            bad_seq: RTP_SEQ_MOD_PLUS_ONE,
            cycles: 0,
            probation: 0,
        }
    }
}

impl SeqValidity {
    /// §A.1's `init_seq(s, seq)`: reset the per-source statistics to a known
    /// starting point. `bad_seq` is left at the "cannot match" sentinel, since
    /// that is what `init_seq` does.
    fn init_seq(&mut self, seq: u16) {
        self.max_seq = seq;
        self.bad_seq = RTP_SEQ_MOD_PLUS_ONE;
        self.cycles = 0;
        self.probation = 0;
    }

    /// §A.1's `update_seq(s, seq)`, transcribed arm for arm. Returns
    /// [`SeqVerdict::Valid`] when §A.1 returns 1 (and a restart when the
    /// "two sequential packets" rule fired), [`SeqVerdict::WildJumpConsumed`]
    /// when §A.1 returns 0 having stored `bad_seq` (the packet is discarded but
    /// arms the confirmation), and [`SeqVerdict::WildJumpIgnored`] when
    /// §A.1 returns 0 from the probation branch.
    fn update_seq(&mut self, seq: u16) -> SeqVerdict {
        // u_int16 udelta = seq - s->max_seq
        let udelta = seq.wrapping_sub(self.max_seq);

        // "Source is not valid until MIN_SEQUENTIAL packets with sequential
        // sequence numbers have been received."
        if self.probation != 0 {
            if seq == self.max_seq.wrapping_add(1) {
                self.probation -= 1;
                self.max_seq = seq;
                if self.probation == 0 {
                    self.init_seq(seq);
                    return SeqVerdict::Valid { restarted: false };
                }
            } else {
                self.probation = MIN_SEQUENTIAL - 1;
                self.max_seq = seq;
            }
            return SeqVerdict::Probation;
        }

        if udelta < MAX_DROPOUT {
            // "in order, with permissible gap"
            if seq < self.max_seq {
                // "Sequence number wrapped - count another 64K cycle."
                self.cycles = self.cycles.wrapping_add(RTP_SEQ_MOD);
            }
            self.max_seq = seq;
            SeqVerdict::Valid { restarted: false }
        } else if udelta <= MAX_WILD_JUMP_U16 {
            // "the sequence number made a very large jump"
            if u32::from(seq) == self.bad_seq {
                // "Two sequential packets -- assume that the other side
                // restarted without telling us so just re-sync (i.e., pretend
                // this was the first packet)."
                self.init_seq(seq);
                SeqVerdict::Valid { restarted: true }
            } else {
                self.bad_seq = u32::from(seq.wrapping_add(1)) & (RTP_SEQ_MOD - 1);
                SeqVerdict::WildJumpArmed
            }
        } else {
            // "duplicate or reordered packet" — accepted, no state change.
            SeqVerdict::Valid { restarted: false }
        }
    }
}

/// The outcome of one RFC 3550 §A.1 `update_seq` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeqVerdict {
    /// §A.1 returned 1: the number is valid. `restarted` is set when the
    /// "two sequential packets at a wild jump" rule re-synced the source.
    Valid { restarted: bool },
    /// §A.1 returned 0 from the probation branch — the source is not yet
    /// declared valid and the packet is not counted.
    Probation,
    /// §A.1 returned 0 having stored `bad_seq` for a wild jump: the packet is
    /// discarded, and the *next* packet must carry `bad_seq` to re-sync.
    WildJumpArmed,
}

/// One [`SeqState::admit`] call's outcome.
struct Admission {
    /// `Some((expected, got))` if a gap was declared as a result — the
    /// caller must drop any access unit under construction.
    gap: Option<(u16, u16)>,
    /// Whether the sequence numbering itself moved (a source restart, RFC 3550
    /// §A.1). There is no missing media to report, but any access unit under
    /// construction is still known-broken across the seam and must be
    /// dropped, and the depacketiser's timeline assumptions must be
    /// re-established.
    resynced: bool,
    /// Packets released for reassembly this call, in sequence order (may be
    /// empty — held for reorder, or a discarded duplicate).
    released: Vec<Vec<u8>>,
}

impl SeqState {
    /// Feed one packet's SSRC + sequence number through the gate.
    ///
    /// Order of decisions:
    ///
    /// 1. A changed/unknown SSRC re-initialises the source (§8.2).
    /// 2. §A.1's `update_seq` classifies the number. A rejected number
    ///    (`WildJumpArmed`) is discarded outright; the *next* wild number may
    ///    re-sync (`Valid { restarted: true }`), which also resets the reorder
    ///    buffer.
    /// 3. A number at or behind `expected` is a duplicate/late arrival for the
    ///    reorder buffer and is dropped silently (it is either already
    ///    delivered or already given up on).
    /// 4. Otherwise the packet is released, or held if it is ahead of a hole
    ///    that the bounded reorder buffer can still wait out.
    fn admit(&mut self, reorder_depth: usize, ssrc: u32, seq: u16, packet: &[u8]) -> Admission {
        // A new source (first packet ever, or a changed SSRC — RFC 3550
        // §8.2) resyncs without a gap: there is nothing to have lost yet.
        if self.ssrc != Some(ssrc) || self.expected.is_none() {
            self.ssrc = Some(ssrc);
            self.expected = Some(seq.wrapping_add(1));
            self.held.clear();
            // §A.1: "s->max_seq = seq - 1; s->probation = MIN_SEQUENTIAL".
            self.validity.max_seq = seq.wrapping_sub(1);
            self.validity.bad_seq = RTP_SEQ_MOD_PLUS_ONE;
            self.validity.cycles = 0;
            self.validity.probation = MIN_SEQUENTIAL;
            return Admission {
                gap: None,
                resynced: false,
                released: alloc::vec![packet.to_vec()],
            };
        }

        match self.validity.update_seq(seq) {
            SeqVerdict::Probation => {
                // §A.1 keeps a source on probation until `MIN_SEQUENTIAL`
                // sequential packets have been seen, and an implementation may
                // discard or *delay* those packets. This depacketiser delays
                // none of them: the caller's RTSP `SETUP`/SDP negotiation has
                // already established the session and its SSRC, so there is no
                // unknown source to screen out, and dropping the first packets
                // of a stream would lose media for no benefit. §A.1's own
                // rejection applies to the *statistics*, not to the media; the
                // reorder buffer still gets its normal treatment below, and
                // probation's `max_seq` bookkeeping is what stops a stray
                // number from being read as a restart.
            }
            SeqVerdict::WildJumpArmed => {
                // §A.1 returned 0 having armed `bad_seq`: the packet is
                // discarded, and the reorder buffer is not touched, so a stray
                // far-behind number cannot drag the live run backwards (the
                // pre-fix bug: one late packet 101 behind resynced
                // immediately).
                return Admission {
                    gap: None,
                    resynced: false,
                    released: Vec::new(),
                };
            }
            SeqVerdict::Valid { restarted } => {
                if restarted {
                    // §A.1's "two sequential packets ... just re-sync". The
                    // new run starts here: adopt it and drop everything the
                    // old one left, including a hole the buffer was waiting
                    // on. Nothing about the seam persists (only `init_seq`'s
                    // state carries over), so the new run is an ordinary
                    // source from here on however far it climbs — a run that
                    // passes the old numbering, or the 16-bit wrap, is
                    // delivered in full, and a later seek resyncs the same
                    // way.
                    self.expected = Some(seq.wrapping_add(1));
                    self.held.clear();
                    return Admission {
                        gap: None,
                        resynced: true,
                        released: alloc::vec![packet.to_vec()],
                    };
                }
            }
        }

        let expected = self.expected.expect("checked above");
        if seq == expected {
            self.expected = Some(seq.wrapping_add(1));
            let mut released = alloc::vec![packet.to_vec()];
            self.drain_contiguous(&mut released);
            return Admission {
                gap: None,
                resynced: false,
                released,
            };
        }
        if (seq.wrapping_sub(expected) as i16) < 0 {
            // Behind what the reorder buffer is waiting for: a duplicate of a
            // packet already delivered, or a very late arrival for one already
            // declared lost. RFC 3550 §A.1's fall-through "duplicate or
            // reordered packet" arm takes no corrective action.
            return Admission {
                gap: None,
                resynced: false,
                released: Vec::new(),
            };
        }
        if self.held.iter().any(|(s, _)| *s == seq) {
            // Duplicate of an already-held future packet.
            return Admission {
                gap: None,
                resynced: false,
                released: Vec::new(),
            };
        }
        self.held.push((seq, packet.to_vec()));
        if self.held.len() <= reorder_depth {
            // Still within the bounded window: wait.
            return Admission {
                gap: None,
                resynced: false,
                released: Vec::new(),
            };
        }
        // The buffer's bound is reached (momentarily `reorder_depth + 1`,
        // collapsed back down by `force_resolve` below, never sustained):
        // the hole cannot be waited out any longer. Force a decision.
        let (expected, got, released) = self
            .force_resolve()
            .expect("held is non-empty: just pushed to it");
        Admission {
            gap: Some((expected, got)),
            resynced: false,
            released,
        }
    }

    /// Force a decision when the buffer can wait no longer (its bound was
    /// reached mid-stream, or end of stream — [`RtpStreamDepacketiser::flush`]
    /// — forces one): pick the held packet with the smallest wrapping
    /// distance from `expected` (RFC 3550 §A.1's wrapping-distance
    /// discipline, applied to choosing a resume point rather than
    /// classifying validity), declare the sequence numbers strictly between
    /// `expected` and it lost, adopt it as the new baseline, and drain
    /// whatever else in `held` is now consecutively reachable from there —
    /// so a run of packets that arrived correctly *after* the one genuinely
    /// lost packet is never discarded along with it. Returns `None` if
    /// `held` is empty (nothing to resolve).
    fn force_resolve(&mut self) -> Option<(u16, u16, Vec<Vec<u8>>)> {
        let expected = self.expected?;
        if self.held.is_empty() {
            return None;
        }
        let idx = self
            .held
            .iter()
            .enumerate()
            .min_by_key(|(_, (s, _))| s.wrapping_sub(expected))
            .map(|(i, _)| i)?;
        let (got, bytes) = self.held.remove(idx);
        self.expected = Some(got.wrapping_add(1));
        let mut released = alloc::vec![bytes];
        self.drain_contiguous(&mut released);
        Some((expected, got, released))
    }

    /// Drain any packets in `held` that are now consecutively next after
    /// `self.expected`, advancing `self.expected` and appending each to
    /// `out` — the mechanism that reassembles a reordered run into the
    /// original wire order once the hole it was waiting on fills.
    fn drain_contiguous(&mut self, out: &mut Vec<Vec<u8>>) {
        // `self.expected` is re-read on every iteration because the body
        // advances it, so `while let` is equivalent to the previous
        // `loop { let … else break }` and satisfies `clippy::while_let_loop`.
        while let Some(expected) = self.expected {
            let Some(pos) = self.held.iter().position(|(s, _)| *s == expected) else {
                break;
            };
            let (_, bytes) = self.held.remove(pos);
            out.push(bytes);
            self.expected = Some(expected.wrapping_add(1));
        }
    }
}

/// Per-track mutable depacketisation state.
struct TrackState {
    kind: RtpMediaKind,
    config: CodecConfig,
    clock_rate: u32,
    /// RTP timestamp of the packets currently buffered in `cur_pkts` (the
    /// access unit still being assembled).
    cur_ts: Option<u32>,
    /// Packets accumulated for the current (not-yet-complete) RTP timestamp.
    cur_pkts: Vec<Vec<u8>>,
    /// Total bytes across `cur_pkts` — tracked incrementally so
    /// [`MAX_AU_BUFFER_BYTES`] can be enforced without re-summing the vec on
    /// every packet.
    cur_bytes: usize,
    /// Unwrapped 64-bit form of the most recently completed AU's timestamp.
    last_unwrapped: Option<u64>,
    /// Unwrapped 64-bit form of this track's very first completed AU's
    /// timestamp — the anchor [`RtpStreamDepacketiser::sync_start_decode_times`]
    /// measures this track's RTCP SR offset against.
    first_unwrapped: Option<u64>,
    /// AU awaiting its duration (filled in once the next AU's timestamp is
    /// known).
    pending: Option<PendingAu>,
    /// Last computed duration, reused for the final AU emitted by `flush`.
    last_duration: u32,
    /// Smallest positive step seen between consecutive wire timestamps — the
    /// frame-period estimate used as the duration fallback when a step goes
    /// backward (a presentation reorder has no forward delta). `None` until
    /// such a step has been seen.
    min_positive_step: Option<i64>,
    /// This track's most recent RTCP Sender Report anchor, if any has been
    /// fed via [`RtpStreamDepacketiser::push_sender_report`].
    sr_anchor: Option<SrAnchor>,
    /// Bounded reorder-buffer depth for this track (packets) — see
    /// [`DEFAULT_REORDER_DEPTH`] / [`RtpStreamTrack::with_reorder_depth`].
    reorder_depth: usize,
    /// RFC 3550 §5.1 sequence-number tracking state (issue #779) — see the
    /// module docs' "Loss and reorder detection" section.
    seq: SeqState,
}

/// One track's RTCP Sender Report wallclock anchor (RFC 3550 §6.4.1): the NTP
/// wallclock instant at which the sender's RTP clock read `raw_rtp_ts`.
///
/// `raw_rtp_ts` is kept in wire (32-bit) form rather than eagerly unwrapped:
/// an SR can be fed before, interleaved with, or after the access units it
/// anchors, so it is unwrapped lazily in [`wall_seconds`] against whichever
/// AU's own unwrapped timestamp needs a wallclock value — always valid
/// because a real SR's RTP timestamp is at most a few RTCP-interval seconds
/// from the AUs it anchors, far under half the 32-bit wrap range.
struct SrAnchor {
    /// NTP wallclock time, in seconds (32.32 fixed-point `ntp_msw`/`ntp_lsw`
    /// converted to `f64`).
    ntp_seconds: f64,
    /// The wire-form (32-bit) RTP timestamp corresponding to `ntp_seconds`.
    raw_rtp_ts: u32,
}

/// Resolve one access unit's NTP wallclock instant from a track's SR anchor:
/// `wall(au) = anchor.ntp_seconds + (unwrapped_au_ts − unwrapped_anchor_ts) /
/// clock_rate` (RFC 3550 §6.4.1's NTP/RTP correlation, generalised from the
/// SR's own instant to any AU on the same RTP clock).
fn wall_seconds(anchor: &SrAnchor, clock_rate: u32, au_unwrapped_ts: u64) -> f64 {
    let anchor_unwrapped = unwrap_ts(Some(au_unwrapped_ts), anchor.raw_rtp_ts);
    let delta_ticks = au_unwrapped_ts as i128 - i128::from(anchor_unwrapped);
    anchor.ntp_seconds + (delta_ticks as f64) / f64::from(clock_rate)
}

/// An access unit whose duration is not yet known (waiting on the next AU's
/// timestamp).
struct PendingAu {
    unwrapped_ts: u64,
    is_sync: bool,
    data: Vec<u8>,
}

/// Stateful, timing- and config-aware RTP depayloader (see module docs).
pub struct RtpStreamDepacketiser {
    tracks: Vec<(u32, TrackState)>,
    /// Output buffered for the [`Stage`] adapter (media plane step 2e), which
    /// — unlike [`push`](Self::push)/[`flush`](Self::flush) — cannot return
    /// samples directly (drained via [`Stage::poll`] instead). Unused by the
    /// inherent API.
    stage_ready: VecDeque<Sample>,
    /// Loss/reorder signals raised by [`Self::push`]/[`Self::flush`],
    /// drained via [`Self::poll_loss_event`] — see the module docs' "Loss
    /// and reorder detection" section.
    loss_events: VecDeque<RtpLossEvent>,
}

/// Unwrap a 32-bit wire RTP timestamp against the last unwrapped value.
///
/// RTP timestamps are a 32-bit field (RFC 3550 §5.1) that wraps every
/// `2^32` ticks. Given the previous unwrapped 64-bit value, this takes its
/// low 32 bits and computes
/// the wire-to-wire delta via a wrapping subtraction reinterpreted as a
/// signed 32-bit integer — the standard idiom for unwrapping any monotonic
/// counter that wraps modulo `2^N` (the same trick used for TCP sequence
/// numbers). That signed delta (forward *or* backward) is then added to the
/// full-width previous value.
///
/// This is exact for any single step whose true magnitude is within half the
/// wrap range (~13.6 hours at a 90 kHz clock) — i.e. any real packet
/// arrival, in order or mildly reordered, between two AUs. It cannot tell a
/// giant forward jump from an equally giant backward jump (an inherent
/// ambiguity of wraparound counters), but that distinction only matters at
/// magnitudes no real RTP stream produces between consecutive AUs.
fn unwrap_ts(prev: Option<u64>, ts: u32) -> u64 {
    let Some(prev) = prev else {
        return u64::from(ts);
    };
    let prev_low = prev as u32;
    let delta = ts.wrapping_sub(prev_low) as i32;
    if delta >= 0 {
        prev + u64::from(delta as u32)
    } else {
        prev.saturating_sub(u64::from(delta.unsigned_abs()))
    }
}

/// Narrow an unwrapped RTP media-clock value into the `i64` range
/// [`Sample::dts`]/[`Sample::pts`] carry (media plane step 2c). The unwrapped
/// clock is a `u64` accumulating whole `2^32` windows, so this only clamps at
/// values no real stream reaches (`i64::MAX` ticks is ~3 million years at
/// 90 kHz) — it makes the narrowing checked rather than a silent truncation.
fn to_ticks(uw: u64) -> i64 {
    uw.min(i64::MAX as u64) as i64
}

impl RtpStreamDepacketiser {
    /// Build a depayloader for the given tracks (kind, config, clock rate).
    pub fn new(tracks: Vec<RtpStreamTrack>) -> Self {
        let tracks = tracks
            .into_iter()
            .map(|t| {
                (
                    t.track_id,
                    TrackState {
                        kind: t.kind,
                        config: t.config,
                        clock_rate: t.clock_rate,
                        cur_ts: None,
                        cur_pkts: Vec::new(),
                        cur_bytes: 0,
                        last_unwrapped: None,
                        first_unwrapped: None,
                        pending: None,
                        last_duration: 0,
                        min_positive_step: None,
                        sr_anchor: None,
                        reorder_depth: t.reorder_depth,
                        seq: SeqState::default(),
                    },
                )
            })
            .collect();
        Self {
            tracks,
            stage_ready: VecDeque::new(),
            loss_events: VecDeque::new(),
        }
    }

    /// Build the [`TrackSpec`]s (timescale = `clock_rate`) for init-segment
    /// construction.
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.tracks
            .iter()
            .map(|(id, st)| TrackSpec::new(*id, st.clock_rate, st.config.clone()))
            .collect()
    }

    fn state(&mut self, track_id: u32) -> Option<&mut TrackState> {
        self.tracks
            .iter_mut()
            .find(|(id, _)| *id == track_id)
            .map(|(_, st)| st)
    }

    /// Drain the next pending loss/reorder signal, if any (FIFO across every
    /// track) — see the module docs' "Loss and reorder detection" section.
    /// A clean stream never produces one.
    pub fn poll_loss_event(&mut self) -> Option<RtpLossEvent> {
        self.loss_events.pop_front()
    }

    /// Feed one RTCP Sender Report (RFC 3550 §6.4.1) for `track_id`, anchoring
    /// this track's wallclock for [`Self::sync_start_decode_times`]. Replaces
    /// any previous anchor for the track (the most recent SR wins); unknown
    /// `track_id`s are silently ignored, matching [`Self::push`].
    pub fn push_sender_report(&mut self, track_id: u32, sr: SenderReport) {
        if let Some(st) = self.state(track_id) {
            let ntp_seconds = f64::from(sr.ntp_msw) + f64::from(sr.ntp_lsw) / NTP_FRACTION_SCALE;
            st.sr_anchor = Some(SrAnchor {
                ntp_seconds,
                raw_rtp_ts: sr.rtp_timestamp,
            });
        }
    }

    /// Parse `bytes` as a single RTCP Sender Report (RFC 3550 §6.4.1) and feed
    /// it to [`Self::push_sender_report`]. Anything that isn't a parseable SR
    /// (a Receiver Report, a different compound-packet member, malformed
    /// bytes) is silently ignored — this is a convenience wrapper for
    /// callers holding raw RTCP bytes off the wire, not a general RTCP
    /// dispatcher.
    pub fn push_rtcp(&mut self, track_id: u32, bytes: &[u8]) {
        if let Ok(sr) = SenderReport::parse(bytes) {
            self.push_sender_report(track_id, sr);
        }
    }

    /// Compute the `start_decode_time` (in each track's own `clock_rate`
    /// ticks) that rebases every SR-anchored track onto one common wallclock,
    /// preserving their real inter-track offset (issue #722; RFC 3550
    /// §6.4.1). The earliest anchored track's first sample becomes the
    /// origin (`start_decode_time = 0`); every other anchored track's
    /// `start_decode_time` is its first sample's wallclock distance from
    /// that origin, converted to its own clock rate.
    ///
    /// Returns an empty `Vec` — the v1 opt-out — unless at least two tracks
    /// have both received a Sender Report ([`Self::push_sender_report`]) and
    /// emitted a first sample. A track absent from the returned `Vec` (no
    /// anchor, or fewer than two anchored tracks overall) keeps the existing
    /// independent-rebase-to-0 behaviour: the caller should only apply the
    /// returned `start_decode_time` to tracks it names.
    pub fn sync_start_decode_times(&self) -> Vec<(u32, u64)> {
        let anchored: Vec<(u32, f64, u32)> = self
            .tracks
            .iter()
            .filter_map(|(id, st)| {
                let anchor = st.sr_anchor.as_ref()?;
                let first_ts = st.first_unwrapped?;
                Some((
                    *id,
                    wall_seconds(anchor, st.clock_rate, first_ts),
                    st.clock_rate,
                ))
            })
            .collect();
        if anchored.len() < 2 {
            return Vec::new();
        }
        let origin = anchored
            .iter()
            .map(|(_, wall, _)| *wall)
            .fold(f64::INFINITY, f64::min);
        anchored
            .into_iter()
            .map(|(id, wall, clock_rate)| {
                let raw_ticks = ((wall - origin) * f64::from(clock_rate)).max(0.0);
                // `f64::round` is a `std`-only inherent method (it needs
                // libm), unavailable in this crate's `no_std` core build —
                // round-half-up via a truncating cast instead, which is
                // exact for `round()`'s behaviour given `raw_ticks >= 0.0`.
                let ticks = (raw_ticks + 0.5) as u64;
                (id, ticks)
            })
            .collect()
    }

    /// Feed one RTP packet for `track_id`. Returns any [`Sample`]s that
    /// became fully timed as a result (zero, one — the AU this packet
    /// completed — the previous AU emitted with its now-known duration; or
    /// several, when a reordered run finishes draining — see the module
    /// docs' "Loss and reorder detection" section). Unknown `track_id`s are
    /// silently ignored (return an empty `Vec`).
    ///
    /// The sequence number is checked before anything else: an in-order or
    /// legally-duplicate/reordered-within-window packet is handled as
    /// before; a genuine gap drops any access unit under construction and
    /// records [`RtpLossEvent::SequenceGap`] (drained via
    /// [`Self::poll_loss_event`]) instead of silently reassembling a run
    /// missing a fragment.
    ///
    /// A **resync** (RFC 3550 §A.1: the source's sequence numbering moved,
    /// e.g. an RTSP seek or a restart that kept its SSRC) drops the
    /// in-progress AU and re-establishes the timestamp origin, because the
    /// RTP timestamp clock is unrelated across the seam (§5.1 — the origin
    /// is random per source instance). No `SequenceGap` is raised: nothing
    /// was lost, the sender simply renumbered.
    pub fn push(&mut self, track_id: u32, rtp_packet: &[u8]) -> Result<Vec<Sample>> {
        let hdr = parse_rtp_header(rtp_packet)?;
        let RtpStreamDepacketiser {
            tracks,
            loss_events,
            ..
        } = self;
        let Some((_, st)) = tracks.iter_mut().find(|(id, _)| *id == track_id) else {
            return Ok(Vec::new());
        };

        let adm = st
            .seq
            .admit(st.reorder_depth, hdr.ssrc, hdr.sequence, rtp_packet);
        let mut out = Vec::new();
        if adm.resynced {
            // The AU under construction straddles the seam (its later
            // fragments belong to a different numbering run), so it is
            // known-broken: drop it rather than hand a merged, malformed
            // sample downstream. The timestamp origin is re-established
            // from this packet, so the new run's samples start a fresh,
            // continuous timeline instead of inheriting a delta computed
            // across the discontinuity (`unwrap_ts` cannot know the two
            // runs' clocks are unrelated — RFC 3550 §5.1).
            st.cur_pkts.clear();
            st.cur_bytes = 0;
            st.cur_ts = None;
            st.last_unwrapped = None;
            st.first_unwrapped = None;
            st.pending = None;
            for pkt in adm.released {
                Self::push_one(st, loss_events, track_id, pkt, &mut out)?;
            }
            return Ok(out);
        }
        if let Some((expected, got)) = adm.gap {
            // The AU under construction, if any, is now known-incomplete:
            // drop it rather than hand a merged, malformed sample
            // downstream (issue #779).
            st.cur_pkts.clear();
            st.cur_bytes = 0;
            st.cur_ts = None;
            loss_events.push_back(RtpLossEvent::SequenceGap {
                track_id,
                ssrc: hdr.ssrc,
                expected,
                got,
            });
        }
        for pkt in adm.released {
            Self::push_one(st, loss_events, track_id, pkt, &mut out)?;
        }
        Ok(out)
    }

    /// The original single-packet accumulation logic (RTP timestamp /
    /// marker-bit AU boundaries, [`MAX_AU_BUFFER_BYTES`]) — factored out so
    /// [`Self::push`] can feed it zero, one, or many packets released by
    /// [`SeqState::admit`] in a single call (a drained reordered run). Only
    /// [`Error::BufferCapExceeded`] escapes as a hard error; a reassembly
    /// failure is caught by [`Self::drain_complete_or_discard`] instead, so
    /// one bad packet in a released batch never aborts the packets after it.
    fn push_one(
        st: &mut TrackState,
        loss_events: &mut VecDeque<RtpLossEvent>,
        track_id: u32,
        rtp_packet: Vec<u8>,
        out: &mut Vec<Sample>,
    ) -> Result<()> {
        let hdr = parse_rtp_header(&rtp_packet)?;
        let ts = hdr.timestamp;
        // `hdr` borrows the packet, which is moved into the AU buffer below.
        let marker = hdr.marker;

        // A timestamp change while packets are buffered means the previous
        // timestamp's packets already form a complete AU (defensive: covers
        // a dropped/missing marker bit).
        if let Some(cur) = st.cur_ts
            && cur != ts
            && !st.cur_pkts.is_empty()
        {
            Self::drain_complete_or_discard(st, loss_events, track_id, out);
        }
        st.cur_ts = Some(ts);
        st.cur_bytes += rtp_packet.len();
        st.cur_pkts.push(rtp_packet);

        // Runaway AU — a dropped final FU-A fragment or a marker bit that
        // never arrives would otherwise grow `cur_pkts` forever (see
        // `MAX_AU_BUFFER_BYTES`). Drop the partial unit and resync: the next
        // packet starts a fresh AU exactly as if this were the first packet
        // ever seen for this track.
        if st.cur_bytes > MAX_AU_BUFFER_BYTES {
            st.cur_pkts.clear();
            st.cur_bytes = 0;
            st.cur_ts = None;
            return Err(Error::BufferCapExceeded {
                what: "RTP access-unit reassembly",
                cap: MAX_AU_BUFFER_BYTES,
            });
        }

        // The video marker bit ends an AU immediately (RFC 6184 §5.1).
        if matches!(st.kind, RtpMediaKind::H264) && marker {
            Self::drain_complete_or_discard(st, loss_events, track_id, out);
            st.cur_ts = None;
        }
        Ok(())
    }

    /// [`Self::drain_complete`], but a reassembly failure — e.g. the packet
    /// resumed on after a [`RtpLossEvent::SequenceGap`] turns out to be an
    /// FU-A continuation fragment with no preceding start fragment in this
    /// run (RFC 6184 §5.8) — is caught and recorded as
    /// [`RtpLossEvent::DamagedAccessUnit`] instead of propagated:
    /// `drain_complete` already resets the accumulator
    /// (`core::mem::take`) before attempting reassembly, so there is no
    /// inconsistent state to clean up, and a stream that is actively
    /// recovering from loss must not have that recovery itself abort the
    /// caller's processing of any packets still to come.
    fn drain_complete_or_discard(
        st: &mut TrackState,
        loss_events: &mut VecDeque<RtpLossEvent>,
        track_id: u32,
        out: &mut Vec<Sample>,
    ) {
        if Self::drain_complete(st, loss_events, track_id, out).is_err() {
            loss_events.push_back(RtpLossEvent::DamagedAccessUnit { track_id });
        }
    }

    /// Flush a track at end-of-stream: reassemble any buffered packets, and
    /// emit the final pending AU using the last-known duration (there is no
    /// following AU to measure a real delta against).
    ///
    /// First forces a decision on anything still sitting in the reorder
    /// buffer awaiting a hole that never filled — end of stream is exactly
    /// as final as the buffer's bound being reached mid-stream (see
    /// `SeqState::force_resolve`); without this, a fragment run missing
    /// its still-buffered continuation would sit invisibly in the reorder
    /// buffer forever while `cur_pkts` (missing that fragment) got wrongly
    /// flushed below as if it were complete.
    pub fn flush(&mut self, track_id: u32) -> Result<Vec<Sample>> {
        let RtpStreamDepacketiser {
            tracks,
            loss_events,
            ..
        } = self;
        let Some((_, st)) = tracks.iter_mut().find(|(id, _)| *id == track_id) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();

        if let Some((expected, got, released)) = st.seq.force_resolve() {
            let ssrc = st.seq.ssrc.unwrap_or(0);
            st.cur_pkts.clear();
            st.cur_bytes = 0;
            st.cur_ts = None;
            loss_events.push_back(RtpLossEvent::SequenceGap {
                track_id,
                ssrc,
                expected,
                got,
            });
            for pkt in released {
                Self::push_one(st, loss_events, track_id, pkt, &mut out)?;
            }
        }

        if !st.cur_pkts.is_empty() {
            Self::drain_complete_or_discard(st, loss_events, track_id, &mut out);
            st.cur_ts = None;
        }
        if let Some(p) = st.pending.take() {
            // Absolute dts/pts (media plane step 2c): `unwrapped_ts` is the
            // already-32-bit-unwrapped RTP media clock this depacketiser
            // maintains (see `unwrap_ts`) — carried into the sample instead of
            // being discarded.
            let ts = to_ticks(p.unwrapped_ts);
            out.push(Sample::new(
                p.data,
                Some(ts),
                Some(ts),
                Some(st.last_duration),
                p.is_sync,
            ));
        }
        Ok(out)
    }

    /// Reassemble the buffered packets into AUs, then for each: unwrap its
    /// timestamp and emit the previously-pending AU with `duration` = the
    /// delta to this AU's timestamp.
    fn drain_complete(
        st: &mut TrackState,
        loss_events: &mut VecDeque<RtpLossEvent>,
        track_id: u32,
        out: &mut Vec<Sample>,
    ) -> Result<()> {
        let pkts = core::mem::take(&mut st.cur_pkts);
        st.cur_bytes = 0;
        let aus = match st.kind {
            RtpMediaKind::H264 => reassemble_video(&pkts)?,
            RtpMediaKind::Aac => reassemble_audio(&pkts)?,
        };
        for au in aus {
            let unwrapped = unwrap_ts(st.last_unwrapped, au.timestamp);
            // The wire timestamp is the *presentation* time (RFC 6184 §5.1 /
            // RFC 3640 §3.3.1), and RTP carries no second timestamp from which
            // a decode instant could be recovered. So a step backward means
            // the stream's presentation order is not its wire (decode) order —
            // a B-frame reorder — and `dts == pts` would be an unstated
            // assumption rather than a fact: report it (the samples themselves
            // are intact). See `RtpLossEvent::NonMonotonicTimestamp`.
            if let Some(prev) = st.last_unwrapped
                && unwrapped < prev
            {
                loss_events.push_back(RtpLossEvent::NonMonotonicTimestamp {
                    track_id,
                    previous_pts: to_ticks(prev),
                    pts: to_ticks(unwrapped),
                });
            }
            // Track the smallest positive step between consecutive wire
            // timestamps: the frame-period estimate used when a step goes
            // backward (see the duration fallback below).
            if let Some(prev) = st.last_unwrapped {
                let step = unwrapped as i64 - prev as i64;
                if step > 0 && st.min_positive_step.is_none_or(|p| step < p) {
                    st.min_positive_step = Some(step);
                }
            }
            st.last_unwrapped = Some(unwrapped);
            if st.first_unwrapped.is_none() {
                st.first_unwrapped = Some(unwrapped);
            }
            if let Some(prev) = st.pending.take() {
                // Duration = the *forward* step to this AU's presentation
                // time. On a stream whose presentation order is its wire order
                // that is exactly the frame period. On a reordered one it is
                // the distance to the next presentation instant, which is not
                // a per-frame duration at all — and no online rule can do
                // better, because RTP carries no decode time and the frame
                // period is only recoverable once a consecutive presentation
                // pair has been seen (which a reorder pattern may not show for
                // a whole GOP). This is reported through
                // [`RtpLossEvent::NonMonotonicTimestamp`]; a consumer that
                // needs exact per-frame durations should take the batch
                // [`crate::rtp::RtpDepacketiser`] path, which sees the whole
                // stream and can derive the period exactly.
                let forward = unwrapped.saturating_sub(prev.unwrapped_ts);
                // A step backward (presentation reorder) has no forward delta
                // at all; falling back to the smallest positive step seen so
                // far keeps the duration a real frame period instead of
                // collapsing to 0, which a container writer would reject as a
                // zero-duration sample. Before any positive step is known there
                // is nothing to fall back to, and 0 is honest.
                // A hostile timestamp series can make the running minimum
                // negative; clamp with a checked conversion rather than an
                // `as` cast, which would wrap a large negative into a huge
                // duration.
                let delta = if forward > 0 {
                    forward
                } else {
                    st.min_positive_step
                        .and_then(|p| u64::try_from(p).ok())
                        .unwrap_or(0)
                };
                let duration = u32::try_from(delta).unwrap_or(u32::MAX);
                st.last_duration = duration;
                let ts = to_ticks(prev.unwrapped_ts);
                out.push(Sample::new(
                    prev.data,
                    Some(ts),
                    Some(ts),
                    Some(duration),
                    prev.is_sync,
                ));
            }
            st.pending = Some(PendingAu {
                unwrapped_ts: unwrapped,
                is_sync: au.is_sync,
                data: au.data,
            });
        }
        Ok(())
    }
}

/// [`Stage`] adoption (media plane step 2e) — with a real restriction, stated
/// plainly rather than papered over: `Stage::feed` carries no track key, but
/// every [`push`](Self::push)/[`flush`](Self::flush) call on this type
/// requires an explicit `track_id`, because `RtpStreamDepacketiser` is
/// inherently multi-track (the whole point of
/// [`sync_start_decode_times`](Self::sync_start_decode_times) is correlating
/// *several* tracks' RTCP Sender Reports against one shared wallclock — that
/// needs their state alive together in one instance, not one instance per
/// track). So this impl only works for a depacketiser constructed with
/// **exactly one** track: `feed`/`finish` target that track directly, and
/// return [`Error::InvalidInput`] otherwise. Multi-track depacketisation
/// keeps driving [`push`](Self::push)/[`flush`](Self::flush) with an explicit
/// `track_id` directly — that inherent API is unchanged and still the right
/// tool for that case.
impl Stage for RtpStreamDepacketiser {
    type In<'a> = &'a [u8];
    type Out = Sample;
    type Error = Error;

    fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<()> {
        let [(track_id, _)] = self.tracks.as_slice() else {
            return Err(Error::InvalidInput(
                "Stage::feed requires RtpStreamDepacketiser to be constructed with exactly one track",
            ));
        };
        let track_id = *track_id;
        let samples = self.push(track_id, input)?;
        self.stage_ready.extend(samples);
        Ok(())
    }

    fn poll(&mut self) -> Option<Sample> {
        self.stage_ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        let [(track_id, _)] = self.tracks.as_slice() else {
            return Err(Error::InvalidInput(
                "Stage::finish requires RtpStreamDepacketiser to be constructed with exactly one track",
            ));
        };
        let track_id = *track_id;
        let samples = self.flush(track_id)?;
        self.stage_ready.extend(samples);
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        // RTP packets only produce output in reaction to feed/finish; no
        // rate-scheduled or timeout work needs a deadline.
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    /// Honest against the one real bound this type enforces:
    /// `MAX_AU_BUFFER_BYTES`, the in-progress access-unit buffer a runaway
    /// (never-terminated) AU would otherwise grow without limit.
    fn demand(&self) -> Demand {
        let saturated = self
            .tracks
            .first()
            .map(|(_, st)| st.cur_bytes >= MAX_AU_BUFFER_BYTES)
            .unwrap_or(false);
        if saturated {
            Demand::saturated()
        } else {
            Demand::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};

    fn dummy_avc() -> CodecConfig {
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
                configuration_version: 1,
                profile_indication: 0x42,
                profile_compatibility: 0,
                level_indication: 0x1E,
                length_size_minus_one: 3,
                sps: alloc::vec![],
                pps: alloc::vec![],
                chroma_format: None,
                bit_depth_luma_minus8: None,
                bit_depth_chroma_minus8: None,
                sps_ext: alloc::vec![],
            }),
            width: 0,
            height: 0,
        }
    }

    fn vpkt(seq: u16, ts: u32, marker: bool, nal: &[u8]) -> Vec<u8> {
        let mut p = alloc::vec![0x80u8, if marker { 0x80 | 96 } else { 96 }];
        p.extend_from_slice(&seq.to_be_bytes());
        p.extend_from_slice(&ts.to_be_bytes());
        p.extend_from_slice(&[0, 0, 0, 0]);
        p.extend_from_slice(nal);
        p
    }

    #[test]
    fn video_stream_recovers_durations_and_sync() {
        let mut d = RtpStreamDepacketiser::new(alloc::vec![RtpStreamTrack::new(
            1,
            RtpMediaKind::H264,
            dummy_avc(),
            90_000,
        )]);

        // AU0 @1000 (IDR), AU1 @4000 (non-IDR), AU2 @7000 (non-IDR). 3000-tick spacing.
        let idr = [0x65u8, 0xAA];
        let non = [0x41u8, 0xBB];
        // AU0: emits nothing yet (duration needs AU1).
        assert!(d.push(1, &vpkt(1, 1000, true, &idr)).unwrap().is_empty());
        // AU1 arrives → AU0 emitted with duration 3000, is_sync=true.
        let s0 = d.push(1, &vpkt(2, 4000, true, &non)).unwrap();
        assert_eq!(s0.len(), 1);
        assert_eq!(s0[0].duration, Some(3000));
        assert!(s0[0].flags.is_sync);
        assert_eq!(s0[0].composition_offset(), 0);
        // AU2 arrives → AU1 emitted, duration 3000, is_sync=false.
        let s1 = d.push(1, &vpkt(3, 7000, true, &non)).unwrap();
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].duration, Some(3000));
        assert!(!s1[0].flags.is_sync);
        // flush → AU2 emitted with the last-known duration (3000).
        let s2 = d.flush(1).unwrap();
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].duration, Some(3000));
    }

    /// [`Stage::feed`]/[`Stage::poll`]/[`Stage::finish`] on a single-track
    /// depacketiser must reproduce exactly what
    /// [`RtpStreamDepacketiser::push`]/[`RtpStreamDepacketiser::flush`]
    /// already return — the trait impl is a delegation, not a second
    /// implementation that could drift from the first.
    #[test]
    fn stage_feed_poll_finish_matches_push_flush() {
        let idr = [0x65u8, 0xAA];
        let non = [0x41u8, 0xBB];
        let packets = [
            vpkt(1, 1000, true, &idr),
            vpkt(2, 4000, true, &non),
            vpkt(3, 7000, true, &non),
        ];

        // Oracle: drive the inherent push/flush API directly.
        let mut oracle = RtpStreamDepacketiser::new(alloc::vec![RtpStreamTrack::new(
            1,
            RtpMediaKind::H264,
            dummy_avc(),
            90_000,
        )]);
        let mut oracle_samples: Vec<Sample> = Vec::new();
        for pkt in &packets {
            oracle_samples.extend(oracle.push(1, pkt).unwrap());
        }
        oracle_samples.extend(oracle.flush(1).unwrap());

        // Same input, driven through Stage instead.
        let mut staged = RtpStreamDepacketiser::new(alloc::vec![RtpStreamTrack::new(
            1,
            RtpMediaKind::H264,
            dummy_avc(),
            90_000,
        )]);
        let mut staged_samples: Vec<Sample> = Vec::new();
        for pkt in &packets {
            Stage::feed(&mut staged, pkt, Timestamp::ZERO).unwrap();
            while let Some(s) = Stage::poll(&mut staged) {
                staged_samples.push(s);
            }
        }
        Stage::finish(&mut staged).unwrap();
        while let Some(s) = Stage::poll(&mut staged) {
            staged_samples.push(s);
        }

        assert_eq!(staged_samples.len(), oracle_samples.len());
        for (s, o) in staged_samples.iter().zip(oracle_samples.iter()) {
            assert_eq!(s.dts, o.dts);
            assert_eq!(s.pts, o.pts);
            assert_eq!(s.duration, o.duration);
            assert_eq!(s.flags.is_sync, o.flags.is_sync);
            assert_eq!(s.data, o.data);
        }
    }

    /// `Stage::feed`/`Stage::finish` on a depacketiser constructed with more
    /// than one track must error cleanly (no track key to target), not
    /// silently pick one — see the `Stage` impl's own doc comment.
    #[test]
    fn stage_feed_errors_on_multi_track_construction() {
        let mut d = RtpStreamDepacketiser::new(alloc::vec![
            RtpStreamTrack::new(1, RtpMediaKind::H264, dummy_avc(), 90_000),
            RtpStreamTrack::new(2, RtpMediaKind::H264, dummy_avc(), 90_000),
        ]);
        let err = Stage::feed(&mut d, &vpkt(1, 1000, true, &[0x65]), Timestamp::ZERO).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        let err2 = Stage::finish(&mut d).unwrap_err();
        assert!(matches!(err2, Error::InvalidInput(_)));
    }

    /// Builds one FU-A (RFC 6184 §5.8) fragment payload: `fu_indicator` +
    /// `fu_header` (start bit only on the first fragment, end bit **never**
    /// set) + `extra_len` bytes of filler — the "dropped final fragment"
    /// scenario audit-ingest #4 flags.
    fn fu_a_fragment(start: bool, extra_len: usize) -> Vec<u8> {
        const NAL_TYPE_FU_A: u8 = 28;
        const FU_START: u8 = 0x80;
        const ORIG_TYPE_IDR: u8 = 5;
        let fu_header = if start {
            FU_START | ORIG_TYPE_IDR
        } else {
            ORIG_TYPE_IDR
        };
        let mut payload = alloc::vec![NAL_TYPE_FU_A, fu_header];
        payload.extend(core::iter::repeat_n(0xABu8, extra_len));
        payload
    }

    /// A never-terminating FU-A run (end bit never set, marker bit never
    /// set, same RTP timestamp throughout — exactly a dropped/corrupted
    /// final fragment or a hostile encoder) must not grow `cur_pkts`
    /// without bound: [`MAX_AU_BUFFER_BYTES`] must trip, dropping the
    /// partial AU, and the depacketiser must keep working normally
    /// afterward (resync proof) rather than being wedged or OOMing.
    #[test]
    fn runaway_fu_a_without_end_bit_is_bounded_not_unbounded() {
        let mut d = RtpStreamDepacketiser::new(alloc::vec![RtpStreamTrack::new(
            1,
            RtpMediaKind::H264,
            dummy_avc(),
            90_000,
        )]);

        // Each fragment carries ~2 KiB of filler; MAX_AU_BUFFER_BYTES (4 MiB)
        // must trip well before we'd reach an unreasonable iteration count —
        // bounding proves the cap, not exhausting real memory.
        const FRAGMENT_FILLER: usize = 2048;
        let mut hit_cap = false;
        let mut resume_seq: u16 = 0;
        for i in 0..4096u16 {
            match d.push(
                1,
                &vpkt(i, 1000, false, &fu_a_fragment(i == 0, FRAGMENT_FILLER)),
            ) {
                Ok(samples) => assert!(
                    samples.is_empty(),
                    "a never-completing AU must not emit a sample"
                ),
                Err(e) => {
                    assert!(
                        matches!(e, crate::error::Error::BufferCapExceeded { .. }),
                        "unexpected error variant: {e:?}"
                    );
                    hit_cap = true;
                    // The sequence gate (issue #779) already accepted packet
                    // `i` as in-order before `push_one` hit the byte cap, so
                    // `i + 1` is the real next-expected sequence number —
                    // resuming from a low/reused one (as this test did
                    // pre-#779, back when sequence numbers were ignored)
                    // would now look like a very late duplicate and be
                    // discarded, not a fresh start.
                    resume_seq = i.wrapping_add(1);
                    break;
                }
            }
        }
        assert!(
            hit_cap,
            "expected MAX_AU_BUFFER_BYTES to trip well within {} fragments \
             (never grow unbounded)",
            4096
        );

        // Resync proof: normal AUs at fresh timestamps process exactly as if
        // nothing had gone wrong — the overflow reset internal state cleanly.
        let idr = [0x65u8, 0xAA];
        let non = [0x41u8, 0xBB];
        assert!(
            d.push(1, &vpkt(resume_seq, 4000, true, &idr))
                .unwrap()
                .is_empty()
        );
        let s = d
            .push(1, &vpkt(resume_seq.wrapping_add(1), 7000, true, &non))
            .unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].duration, Some(3000));
        assert!(s[0].flags.is_sync);
        assert!(
            d.poll_loss_event().is_none(),
            "no loss event expected: the byte-cap overflow is its own \
             recorded corrective action, and the sequence numbers this \
             resync uses are contiguous with the ones already accepted"
        );
    }

    #[test]
    fn track_specs_use_clock_rate_as_timescale() {
        let d = RtpStreamDepacketiser::new(alloc::vec![RtpStreamTrack::new(
            7,
            RtpMediaKind::H264,
            dummy_avc(),
            90_000,
        )]);
        let specs = d.track_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].track_id, 7);
        assert_eq!(specs[0].timescale, 90_000);
    }
}
