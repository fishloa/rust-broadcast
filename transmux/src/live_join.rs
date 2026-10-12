//! Live-join helper: find the first usable random-access point of a video
//! stream joined mid-flight and flag the pictures that cannot be decoded
//! (issue #1162, R4).
//!
//! A player that tunes into a broadcast starts at an arbitrary sample.
//! [`LiveJoin`] is fed the track's samples in **decode order** and answers one
//! [`JoinDecision`] per sample: keep it, or drop it and why. It is sans-IO and
//! allocation-free after construction.
//!
//! # Random-access points ([`RapKind`])
//!
//! - **H.264**: an IDR picture ([`RapKind::Idr`], §7.4.1.2.4) is
//!   self-contained. A `recovery_point` SEI (§D.1.8/D.2.8,
//!   [`RapKind::RecoveryPoint`]) marks the open-GOP random access point broadcast
//!   streams use instead; decoding from it "may contain references to pictures
//!   not available in the decoded picture buffer", the output is "correct or
//!   approximately correct" only from the recovery point in output order, and
//!   `exact_match_flag`/`broken_link_flag` qualify that — so a recovery point is
//!   **not necessarily self-contained**. An SPS plus an I slice without either
//!   signal is the pragmatic fallback the rest of this crate already uses
//!   ([`access_unit_is_rap`]): [`RapKind::SpsIntra`]. Transcription:
//!   `docs/codec/h264-vui-pic-timing.md` (recovery_point section).
//! - **H.265**: IDR (types 19, 20), CRA (21) and BLA (16..=18) pictures
//!   (Table 7-1). The first picture of a join has `NoRaslOutputFlag` = 1, so the
//!   RASL pictures (types 8, 9) that follow a CRA/BLA "can be
//!   non-correctly-decodable, as the RASL picture can contain references to
//!   pictures that are not present in the bitstream" (§3.126) and are dropped;
//!   RADL pictures reference only the IRAP and are kept. Reserved IRAP types 22
//!   and 23 are not random-access points here (decoders ignore reserved types).
//! - **H.266**: IDR (7, 8) and CRA (9); leading pictures are detected by
//!   presentation time only (no VVC spec transcription for RASL here).
//!
//! # Leading pictures
//!
//! After the join point a run of *leading* pictures can follow in decode order
//! that precede the random-access point in output order (open GOP B pictures).
//! They reference pictures from before the join. They are dropped:
//! H.265 by NAL type (RASL), every other codec by `pts < pts of the join
//! point`. The run ends at the first sample that is not leading, after which
//! every sample is kept.
//!
//! # Asking for the next random-access point
//!
//! Joining at a non-self-contained point can still produce garbage (a
//! recovery point whose recovery distance has not elapsed, a CRA whose RASL the
//! decoder mishandles). Two controls:
//!
//! - [`JoinPolicy::SelfContainedOnly`] — accept only IDR/IRAP points; a recovery
//!   point or SPS+I fallback is reported as [`DropReason::NotSelfContained`].
//!   Set it up front, or switch with [`LiveJoin::set_policy`].
//! - [`LiveJoin::rejoin`] — forget the current join and wait for the next
//!   acceptable point. Call it when the decoder reports a failure after the join
//!   or on [`DemuxEvent::Discontinuity`](crate::DemuxEvent::Discontinuity)
//!   (timestamps and references no longer continue), typically together with
//!   `set_policy(JoinPolicy::SelfContainedOnly)` to escalate.

use alloc::vec::Vec;

use crate::annexb::{iter_length_prefixed_nals_with, normalise_nal_length_size};
use crate::bitreader::BitReader;
use crate::ir::{CodecConfig, Sample};
use crate::nal::{NalCodec, access_unit_is_rap, nal_unit_type, sei_messages};

/// H.264 `nal_unit_type` of an IDR slice (Table 7-1).
const AVC_NAL_IDR: u8 = 5;
/// H.264 `nal_unit_type` of an SEI NAL.
const AVC_NAL_SEI: u8 = 6;
/// `payloadType` of the `recovery_point` SEI (H.264 Table D-1, §D.1.8).
const AVC_SEI_RECOVERY_POINT: u32 = 6;
/// Length of the H.264 NAL header.
const AVC_NAL_HEADER_LEN: usize = 1;

/// H.265 RASL_N / RASL_R (Table 7-1).
const HEVC_RASL_N: u8 = 8;
const HEVC_RASL_R: u8 = 9;
/// H.265 BLA_W_LP .. BLA_N_LP.
const HEVC_BLA_FIRST: u8 = 16;
const HEVC_BLA_LAST: u8 = 18;
/// H.265 IDR_W_RADL, IDR_N_LP.
const HEVC_IDR_W_RADL: u8 = 19;
const HEVC_IDR_N_LP: u8 = 20;
/// H.265 CRA_NUT.
const HEVC_CRA: u8 = 21;
/// First H.265 non-VCL `nal_unit_type`.
const HEVC_FIRST_NON_VCL: u8 = 32;
/// H.266 IDR_W_RADL, IDR_N_LP, CRA_NUT (Table 5).
const VVC_IDR_W_RADL: u8 = 7;
const VVC_IDR_N_LP: u8 = 8;
const VVC_CRA: u8 = 9;

/// The canonical NAL length-prefix width the NAL helpers read.
const CANONICAL_LENGTH_SIZE: usize = 4;

/// What kind of random-access point a join landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum RapKind {
    /// An IDR picture (H.264/H.265/H.266): self-contained.
    Idr,
    /// H.265 BLA picture (its RASL pictures are dropped): self-contained.
    Bla,
    /// H.265/H.266 CRA picture (its RASL pictures are dropped): self-contained.
    Cra,
    /// H.264 access unit announced by a `recovery_point` SEI (§D.2.8). Decoding
    /// from here may reference unavailable pictures; correct output is only
    /// guaranteed from the recovery point in output order.
    RecoveryPoint {
        /// `recovery_frame_cnt`: frame_num increments until the recovery point.
        recovery_frame_cnt: u32,
        /// `exact_match_flag`.
        exact_match: bool,
        /// `broken_link_flag`.
        broken_link: bool,
    },
    /// H.264 access unit with an SPS and an I slice but neither IDR nor
    /// recovery-point SEI (the open-GOP fallback of [`access_unit_is_rap`]).
    SpsIntra,
}

impl RapKind {
    /// Whether decoding from this point needs no earlier picture: IDR, BLA and
    /// CRA (once RASL pictures are dropped). Recovery points and the SPS+I
    /// fallback are not.
    pub fn is_self_contained(&self) -> bool {
        matches!(self, RapKind::Idr | RapKind::Bla | RapKind::Cra)
    }

    /// The short spec-token label.
    pub fn name(&self) -> &'static str {
        match self {
            RapKind::Idr => "IDR",
            RapKind::Bla => "BLA",
            RapKind::Cra => "CRA",
            RapKind::RecoveryPoint { .. } => "recovery_point",
            RapKind::SpsIntra => "SPS+I",
        }
    }
}

broadcast_common::impl_spec_display!(RapKind);

/// Which random-access points [`LiveJoin`] accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum JoinPolicy {
    /// Join at the first random-access point of any kind (fastest start).
    #[default]
    AnyRap,
    /// Join only at a self-contained point ([`RapKind::is_self_contained`]).
    SelfContainedOnly,
}

impl JoinPolicy {
    /// The short label.
    pub fn name(&self) -> &'static str {
        match self {
            JoinPolicy::AnyRap => "any RAP",
            JoinPolicy::SelfContainedOnly => "self-contained only",
        }
    }
}

broadcast_common::impl_spec_display!(JoinPolicy);

/// Why [`LiveJoin`] dropped a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropReason {
    /// Decoded before any acceptable random-access point.
    BeforeRandomAccessPoint,
    /// A random-access point that [`JoinPolicy::SelfContainedOnly`] rejected.
    NotSelfContained,
    /// A leading picture of the join point: it precedes the join point in
    /// output order and references pictures from before the join.
    LeadingPicture,
}

impl DropReason {
    /// The short label.
    pub fn name(&self) -> &'static str {
        match self {
            DropReason::BeforeRandomAccessPoint => "before random-access point",
            DropReason::NotSelfContained => "not self-contained",
            DropReason::LeadingPicture => "leading picture",
        }
    }
}

broadcast_common::impl_spec_display!(DropReason);

/// What to do with one sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum JoinDecision {
    /// Drop the sample: it cannot be decoded (or was rejected by the policy).
    Drop(DropReason),
    /// This sample is the join point; keep it and start decoding here.
    JoinPoint(RapKind),
    /// Keep the sample (it follows the join point).
    Keep,
}

impl JoinDecision {
    /// Whether the sample should be passed on to the decoder.
    pub fn is_keep(&self) -> bool {
        !matches!(self, JoinDecision::Drop(_))
    }
}

/// Where the join state machine is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum JoinState {
    /// Waiting for an acceptable random-access point.
    Waiting,
    /// Joined at a random-access point.
    Joined {
        /// The kind of the join point.
        kind: RapKind,
        /// Presentation timestamp of the join point (track timescale), if it
        /// had one.
        pts: Option<i64>,
        /// Whether the leading-picture run after the join point is still open.
        in_leading_run: bool,
    },
}

/// Live-join state machine for one video track; see the [module docs](self).
#[derive(Debug, Clone)]
pub struct LiveJoin {
    codec: NalCodec,
    length_size: usize,
    policy: JoinPolicy,
    state: JoinState,
}

impl LiveJoin {
    /// A join helper for `codec` samples whose NAL length prefix is
    /// `length_size` bytes (1, 2 or 4; the demuxers emit 4).
    pub fn new(codec: NalCodec, length_size: usize) -> Self {
        Self {
            codec,
            length_size,
            policy: JoinPolicy::default(),
            state: JoinState::Waiting,
        }
    }

    /// A join helper for a track's config; `None` unless it is AVC, HEVC or VVC.
    pub fn for_config(config: &CodecConfig) -> Option<Self> {
        let (codec, minus_one) = match config {
            CodecConfig::Avc { config, .. } => (NalCodec::Avc, config.config.length_size_minus_one),
            CodecConfig::Hevc { config, .. } => {
                (NalCodec::Hevc, config.config.length_size_minus_one)
            }
            CodecConfig::Vvc { config, .. } => (NalCodec::Vvc, config.config.length_size_minus_one),
            _ => return None,
        };
        Some(Self::new(codec, usize::from(minus_one) + 1))
    }

    /// Builder form of [`LiveJoin::set_policy`].
    pub fn with_policy(mut self, policy: JoinPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Change which random-access points are accepted from the next sample on.
    /// Does not undo a join already made; combine with [`LiveJoin::rejoin`].
    pub fn set_policy(&mut self, policy: JoinPolicy) {
        self.policy = policy;
    }

    /// The current policy.
    pub fn policy(&self) -> JoinPolicy {
        self.policy
    }

    /// The current state.
    pub fn state(&self) -> JoinState {
        self.state
    }

    /// Forget the current join and wait for the next acceptable
    /// random-access point (the "ask for the next RAP" request).
    pub fn rejoin(&mut self) {
        self.state = JoinState::Waiting;
    }

    /// Decide one sample. Samples must be fed in decode order.
    pub fn push(&mut self, sample: &Sample) -> JoinDecision {
        let Some(au) = self.canonical(&sample.data) else {
            // A malformed access unit is never a random-access point and
            // cannot be proven decodable.
            return match self.state {
                JoinState::Waiting => JoinDecision::Drop(DropReason::BeforeRandomAccessPoint),
                JoinState::Joined { .. } => JoinDecision::Keep,
            };
        };
        match self.state {
            JoinState::Waiting => match rap_kind(self.codec, &au) {
                None => JoinDecision::Drop(DropReason::BeforeRandomAccessPoint),
                Some(kind)
                    if self.policy == JoinPolicy::SelfContainedOnly
                        && !kind.is_self_contained() =>
                {
                    JoinDecision::Drop(DropReason::NotSelfContained)
                }
                Some(kind) => {
                    self.state = JoinState::Joined {
                        kind,
                        pts: sample.pts,
                        in_leading_run: true,
                    };
                    JoinDecision::JoinPoint(kind)
                }
            },
            JoinState::Joined {
                kind,
                pts,
                in_leading_run: true,
            } => {
                if is_leading(self.codec, &au, sample.pts, pts) {
                    JoinDecision::Drop(DropReason::LeadingPicture)
                } else {
                    self.state = JoinState::Joined {
                        kind,
                        pts,
                        in_leading_run: false,
                    };
                    JoinDecision::Keep
                }
            }
            JoinState::Joined { .. } => JoinDecision::Keep,
        }
    }

    /// The sample in canonical 4-byte-prefixed form, or `None` if malformed.
    fn canonical<'a>(&self, data: &'a [u8]) -> Option<alloc::borrow::Cow<'a, [u8]>> {
        if self.length_size == CANONICAL_LENGTH_SIZE {
            iter_length_prefixed_nals_with(data, CANONICAL_LENGTH_SIZE).ok()?;
            Some(alloc::borrow::Cow::Borrowed(data))
        } else {
            normalise_nal_length_size(data, self.length_size)
                .ok()
                .map(alloc::borrow::Cow::Owned)
        }
    }
}

/// The random-access-point kind of one canonical (4-byte-prefixed) access unit.
fn rap_kind(codec: NalCodec, au: &[u8]) -> Option<RapKind> {
    let nals = iter_length_prefixed_nals_with(au, CANONICAL_LENGTH_SIZE).ok()?;
    match codec {
        NalCodec::Avc => {
            if nals
                .iter()
                .any(|n| nal_unit_type(NalCodec::Avc, n) == Some(AVC_NAL_IDR))
            {
                return Some(RapKind::Idr);
            }
            if let Some(kind) = nals.iter().find_map(|n| avc_recovery_point(n)) {
                return Some(kind);
            }
            access_unit_is_rap(NalCodec::Avc, au, true).then_some(RapKind::SpsIntra)
        }
        NalCodec::Hevc => nals
            .iter()
            .filter_map(|n| nal_unit_type(NalCodec::Hevc, n))
            .find_map(|t| match t {
                HEVC_IDR_W_RADL | HEVC_IDR_N_LP => Some(RapKind::Idr),
                HEVC_CRA => Some(RapKind::Cra),
                HEVC_BLA_FIRST..=HEVC_BLA_LAST => Some(RapKind::Bla),
                _ => None,
            }),
        NalCodec::Vvc => nals
            .iter()
            .filter_map(|n| nal_unit_type(NalCodec::Vvc, n))
            .find_map(|t| match t {
                VVC_IDR_W_RADL | VVC_IDR_N_LP => Some(RapKind::Idr),
                VVC_CRA => Some(RapKind::Cra),
                _ => None,
            }),
    }
}

/// The `recovery_point` SEI of an H.264 SEI NAL, if it carries one (§D.1.8).
fn avc_recovery_point(nal: &[u8]) -> Option<RapKind> {
    if nal_unit_type(NalCodec::Avc, nal) != Some(AVC_NAL_SEI) {
        return None;
    }
    let (_, payload) = sei_messages(&nal[AVC_NAL_HEADER_LEN..])
        .into_iter()
        .find(|(t, _)| *t == AVC_SEI_RECOVERY_POINT)?;
    let mut r = BitReader::from_rbsp(&payload, "recovery_point").ok()?;
    let recovery_frame_cnt = u32::try_from(r.read_ue("recovery_frame_cnt").ok()?).ok()?;
    let exact_match = r.read_flag("exact_match_flag").ok()?;
    let broken_link = r.read_flag("broken_link_flag").ok()?;
    Some(RapKind::RecoveryPoint {
        recovery_frame_cnt,
        exact_match,
        broken_link,
    })
}

/// Whether the access unit is a leading picture of the join point.
fn is_leading(codec: NalCodec, au: &[u8], pts: Option<i64>, rap_pts: Option<i64>) -> bool {
    match codec {
        NalCodec::Hevc => {
            // RASL pictures (types 8, 9) — and only a sample whose VCL NALs
            // are all RASL (a sample is one picture).
            let Ok(nals) = iter_length_prefixed_nals_with(au, CANONICAL_LENGTH_SIZE) else {
                return false;
            };
            let vcl: Vec<u8> = nals
                .iter()
                .filter_map(|n| nal_unit_type(NalCodec::Hevc, n))
                .filter(|&t| t < HEVC_FIRST_NON_VCL)
                .collect();
            !vcl.is_empty() && vcl.iter().all(|&t| t == HEVC_RASL_N || t == HEVC_RASL_R)
        }
        NalCodec::Avc | NalCodec::Vvc => match (pts, rap_pts) {
            (Some(p), Some(r)) => p < r,
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// Length-prefix (`size` bytes) a list of NAL units into one sample.
    fn au(size: usize, nals: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for n in nals {
            let len = (n.len() as u32).to_be_bytes();
            out.extend_from_slice(&len[4 - size..]);
            out.extend_from_slice(n);
        }
        out
    }

    fn sample(data: Vec<u8>, pts: i64) -> Sample {
        Sample::new(data, Some(pts), Some(pts), None, false)
    }

    // ── H.264 ──────────────────────────────────────────────────────────
    const SPS: [u8; 4] = [0x67, 66, 0, 30];
    /// I slice: first_mb ue(0)=1, slice_type ue(7)=0001000, then filler.
    const I_SLICE: [u8; 3] = [0x61, 0b1000_1000, 0x80];
    /// P slice: first_mb ue(0)=1, slice_type ue(5)=00110.
    const P_SLICE: [u8; 3] = [0x41, 0b1001_1000, 0x80];
    const IDR_SLICE: [u8; 3] = [0x65, 0b1000_1000, 0x80];

    /// SEI NAL with a recovery_point: ue(cnt), exact_match, broken_link, 2 bits.
    fn recovery_sei(cnt_ue_bits: &[bool], exact: bool, broken: bool) -> Vec<u8> {
        let mut bits: Vec<bool> = cnt_ue_bits.to_vec();
        bits.push(exact);
        bits.push(broken);
        bits.extend([false, false]); // changing_slice_group_idc
        let mut payload = vec![0u8; bits.len().div_ceil(8)];
        for (i, b) in bits.iter().enumerate() {
            if *b {
                payload[i / 8] |= 0x80 >> (i % 8);
            }
        }
        let mut nal = vec![0x06, 6, payload.len() as u8];
        nal.extend(payload);
        nal.push(0x80);
        nal
    }

    #[test]
    fn avc_drops_until_idr_and_keeps_everything_after() {
        let mut j = LiveJoin::new(NalCodec::Avc, 4);
        let p = sample(au(4, &[P_SLICE.to_vec()]), 10);
        assert_eq!(
            j.push(&p),
            JoinDecision::Drop(DropReason::BeforeRandomAccessPoint)
        );
        // An SPS alone (no I slice) is not a random-access point.
        let sps_p = sample(au(4, &[SPS.to_vec(), P_SLICE.to_vec()]), 11);
        assert_eq!(
            j.push(&sps_p),
            JoinDecision::Drop(DropReason::BeforeRandomAccessPoint)
        );
        let idr = sample(au(4, &[SPS.to_vec(), IDR_SLICE.to_vec()]), 12);
        assert_eq!(j.push(&idr), JoinDecision::JoinPoint(RapKind::Idr));
        // `p` (pts 10) precedes the IDR (pts 12) in output order: leading.
        assert_eq!(j.push(&p), JoinDecision::Drop(DropReason::LeadingPicture));
        let later = sample(au(4, &[P_SLICE.to_vec()]), 13);
        assert_eq!(j.push(&later), JoinDecision::Keep);
        assert_eq!(
            j.state(),
            JoinState::Joined {
                kind: RapKind::Idr,
                pts: Some(12),
                in_leading_run: false
            }
        );
    }

    #[test]
    fn avc_recovery_point_fields_and_leading_pictures() {
        // ue(3) = 00100, exact_match 1, broken_link 0.
        let sei = recovery_sei(&[false, false, true, false, false], true, false);
        let rp = sample(au(4, &[sei, I_SLICE.to_vec()]), 100);
        let mut j = LiveJoin::new(NalCodec::Avc, 4);
        let kind = RapKind::RecoveryPoint {
            recovery_frame_cnt: 3,
            exact_match: true,
            broken_link: false,
        };
        assert_eq!(j.push(&rp), JoinDecision::JoinPoint(kind));
        assert!(!kind.is_self_contained());
        // Two B pictures that precede the join point in output order.
        for pts in [96, 98] {
            assert_eq!(
                j.push(&sample(au(4, &[P_SLICE.to_vec()]), pts)),
                JoinDecision::Drop(DropReason::LeadingPicture)
            );
        }
        assert_eq!(
            j.push(&sample(au(4, &[P_SLICE.to_vec()]), 104)),
            JoinDecision::Keep
        );
        // The run is closed: a later lower pts (e.g. a B picture of the next
        // GOP's reordering) is kept, never mistaken for a leading picture.
        assert_eq!(
            j.push(&sample(au(4, &[P_SLICE.to_vec()]), 50)),
            JoinDecision::Keep
        );
    }

    #[test]
    fn sps_plus_intra_without_sei_is_the_fallback_kind() {
        let mut j = LiveJoin::new(NalCodec::Avc, 4);
        let s = sample(au(4, &[SPS.to_vec(), I_SLICE.to_vec()]), 5);
        assert_eq!(j.push(&s), JoinDecision::JoinPoint(RapKind::SpsIntra));
    }

    #[test]
    fn self_contained_policy_waits_for_idr_and_rejoin_asks_for_the_next_rap() {
        let sei = recovery_sei(&[true], true, false); // recovery_frame_cnt 0
        let rp = |pts| sample(au(4, &[sei.clone(), I_SLICE.to_vec()]), pts);
        let idr = |pts| sample(au(4, &[IDR_SLICE.to_vec()]), pts);
        let p = |pts| sample(au(4, &[P_SLICE.to_vec()]), pts);

        let mut j = LiveJoin::new(NalCodec::Avc, 4).with_policy(JoinPolicy::SelfContainedOnly);
        assert_eq!(
            j.push(&rp(1)),
            JoinDecision::Drop(DropReason::NotSelfContained)
        );
        assert_eq!(
            j.push(&p(2)),
            JoinDecision::Drop(DropReason::BeforeRandomAccessPoint)
        );
        assert_eq!(j.push(&idr(3)), JoinDecision::JoinPoint(RapKind::Idr));

        // The consumer asks for the next RAP: state resets, everything up to
        // the next acceptable point is dropped again.
        j.rejoin();
        assert_eq!(j.state(), JoinState::Waiting);
        assert_eq!(
            j.push(&p(4)),
            JoinDecision::Drop(DropReason::BeforeRandomAccessPoint)
        );
        // Relax the policy and the recovery point is acceptable.
        j.set_policy(JoinPolicy::AnyRap);
        assert!(matches!(
            j.push(&rp(5)),
            JoinDecision::JoinPoint(RapKind::RecoveryPoint { .. })
        ));
    }

    #[test]
    fn two_byte_length_prefix_and_malformed_samples() {
        let mut j = LiveJoin::new(NalCodec::Avc, 2);
        let idr = sample(au(2, &[SPS.to_vec(), IDR_SLICE.to_vec()]), 1);
        assert_eq!(j.push(&idr), JoinDecision::JoinPoint(RapKind::Idr));
        // A length running past the end is never a RAP while waiting.
        let mut j = LiveJoin::new(NalCodec::Avc, 4);
        let bad = sample(vec![0, 0, 0xFF, 0xFF, 0x65, 1], 1);
        assert_eq!(
            j.push(&bad),
            JoinDecision::Drop(DropReason::BeforeRandomAccessPoint)
        );
        assert_eq!(j.state(), JoinState::Waiting);
    }

    // ── H.265 / H.266 ──────────────────────────────────────────────────
    fn hevc(nal_type: u8) -> Vec<u8> {
        vec![nal_type << 1, 0x01, 0xAF]
    }

    #[test]
    fn hevc_cra_join_drops_rasl_keeps_radl_and_trailing() {
        let mut j = LiveJoin::new(NalCodec::Hevc, 4);
        // Leading RASL before any RAP: dropped as before-RAP.
        assert_eq!(
            j.push(&sample(au(4, &[hevc(9)]), 90)),
            JoinDecision::Drop(DropReason::BeforeRandomAccessPoint)
        );
        assert_eq!(
            j.push(&sample(au(4, &[hevc(21)]), 100)),
            JoinDecision::JoinPoint(RapKind::Cra)
        );
        // RASL_N, RASL_R dropped (even though pts alone would not say so for
        // the second); RADL_R kept — it references only the IRAP.
        assert_eq!(
            j.push(&sample(au(4, &[hevc(8)]), 96)),
            JoinDecision::Drop(DropReason::LeadingPicture)
        );
        assert_eq!(
            j.push(&sample(au(4, &[hevc(9)]), 140)),
            JoinDecision::Drop(DropReason::LeadingPicture)
        );
        assert_eq!(j.push(&sample(au(4, &[hevc(7)]), 98)), JoinDecision::Keep);
        // First trailing picture ends the leading run: a later CRA's RASL
        // (NoRaslOutputFlag 0) is decodable and kept.
        assert_eq!(j.push(&sample(au(4, &[hevc(1)]), 104)), JoinDecision::Keep);
        assert_eq!(j.push(&sample(au(4, &[hevc(9)]), 99)), JoinDecision::Keep);
    }

    #[test]
    fn hevc_irap_kinds_and_reserved_types() {
        for (t, want) in [
            (19, Some(RapKind::Idr)),
            (20, Some(RapKind::Idr)),
            (16, Some(RapKind::Bla)),
            (17, Some(RapKind::Bla)),
            (18, Some(RapKind::Bla)),
            (21, Some(RapKind::Cra)),
            (22, None),
            (23, None),
            (1, None),
            (6, None),
        ] {
            let a = au(4, &[hevc(t)]);
            assert_eq!(rap_kind(NalCodec::Hevc, &a), want, "type {t}");
        }
    }

    #[test]
    fn vvc_uses_pts_for_leading_pictures() {
        let vvc = |t: u8| vec![0x00, t << 3, 0x01];
        let mut j = LiveJoin::new(NalCodec::Vvc, 4);
        assert_eq!(
            j.push(&sample(au(4, &[vvc(9)]), 50)),
            JoinDecision::JoinPoint(RapKind::Cra)
        );
        assert_eq!(
            j.push(&sample(au(4, &[vvc(1)]), 40)),
            JoinDecision::Drop(DropReason::LeadingPicture)
        );
        assert_eq!(j.push(&sample(au(4, &[vvc(1)]), 60)), JoinDecision::Keep);
    }

    #[test]
    fn labels() {
        assert_eq!(RapKind::Idr.to_string(), "IDR");
        assert_eq!(DropReason::LeadingPicture.to_string(), "leading picture");
        assert_eq!(
            JoinPolicy::SelfContainedOnly.to_string(),
            "self-contained only"
        );
    }
}
