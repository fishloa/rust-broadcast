//! Parameter-set activation tracking for the display change event
//! (`docs/display-geometry/DESIGN.md` §6.1, §6.2, §6.5, Phase 2).
//!
//! Answers one question per input access unit: **which SPS do this access
//! unit's slices use?** — and therefore what display geometry is in force for
//! it. The answer comes from the access unit's own slice headers
//! (`pic_parameter_set_id` -> PPS `seq_parameter_set_id`; H.265
//! `slice_pic_parameter_set_id` -> PPS -> SPS), never from "the access unit
//! that carries an SPS": an encoder may send an SPS with a new id long before
//! anything references it, and no change is raised until a picture actually
//! decodes with it.
//!
//! # Spec citations
//!
//! - **H.264** ITU-T H.264 (V15, 08/2024) §7.3.2.1 (`seq_parameter_set_id`),
//!   §7.3.2.2 (`pic_parameter_set_id` / `seq_parameter_set_id` in the PPS),
//!   §7.3.3 (`pic_parameter_set_id` in the slice header), §7.4.1.2.1: an SPS
//!   is activated through a PPS activated by a slice, and an activated SPS
//!   "shall remain active for the entire coded video sequence" — so in a
//!   conforming stream a *content* change can only take effect at the first
//!   picture of a coded video sequence (an IDR access unit).
//! - **H.265** ITU-T H.265 (V11, 01/2026) §7.3.2.2 (`sps_seq_parameter_set_id`,
//!   `pps_seq_parameter_set_id`), §7.3.2.3 (`pps_pic_parameter_set_id`),
//!   §7.3.6.1 (`slice_pic_parameter_set_id`), §7.4.3.2 (a base-layer SPS is
//!   active for the whole CVS; an IRAP access unit with `NoRaslOutputFlag`
//!   equal to 1 begins a new CVS).
//!
//! # Output order, not decode order (§6.5)
//!
//! A changed geometry affects **pictures**, and a picture is displayed when
//! its PTS says, not where its access unit sits in decode order. A conforming
//! switch is an IDR, so the change is simply in force at the IDR's own output
//! position. On the real slate captures, though, the keyframe just before the
//! splice is decoded while the *following* codewords still describe the
//! previous picture, so a change raised at its decode position would label
//! one programme picture with the slate's shape. A resolved change is
//! therefore held ([`PendingDisplayChange`]) until the access unit displayed
//! at or after it is observed; then nothing still to come in decode order can
//! be displayed after it with the old geometry (one unit of lookahead is all
//! that is needed).
//!
//! # Real-data "unseen PPS"
//!
//! On the slate captures in `private/fixtures/ts/` the slate arrives with a
//! **new** SPS/PPS pair, and the first switched access unit's slice can
//! reference a `pic_parameter_set_id` not yet received (ffmpeg logs
//! "non-existing PPS 0 referenced" around the splice). The demuxer never
//! panics, never invents a geometry, and never binds a change to a picture it
//! cannot describe: it keeps the previous state for that access unit and
//! **remembers** the dangling reference; the first access unit whose slices
//! resolve it after the PPS (and its SPS) arrive reports the change. When the
//! parameter sets travel ahead of their use (the normal case) the change is
//! raised at the first access unit that references them.

use crate::annexb::iter_annexb_nals;
use crate::bitreader::BitReader;
use crate::ir::{DisplayChangeSet, DisplayGeometry, DisplayInfo};
use crate::nal::NalCodec;
use crate::nalu_types::HEVC_SPS_NUT;
use crate::sps::decode_avc_slice_sps;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

/// `nal_unit_type` of a non-IDR coded slice (H.264 Table 7-1).
const AVC_SLICE: u8 = 1;
/// `nal_unit_type` of an IDR coded slice (H.264 Table 7-1).
const AVC_IDR: u8 = 5;
/// `nal_unit_type` of an SPS (H.264 Table 7-1).
const AVC_SPS: u8 = 7;
/// `nal_unit_type` of a PPS (H.264 Table 7-1).
const AVC_PPS: u8 = 8;
/// Low five bits of the H.264 NAL header byte (`forbidden_zero_bit |
/// nal_ref_idc | nal_unit_type`).
const AVC_NAL_TYPE_MASK: u8 = 0x1F;
/// `nal_unit_type` of a temporal-scalability prefix NAL, whose body is itself
/// a complete NAL unit (H.264 §7.3.1.1, Table 7-1).
const AVC_PREFIX_NAL: u8 = 14;
/// Bytes of a slice NAL whose RBSP prefix is walked: the slice-header prefix
/// is a handful of Exp-Golomb codes, and a multi-megabyte I slice must not be
/// unescaped and copied per access unit (the bound `slice_header.rs` uses).
const SLICE_PREFIX_BYTES: usize = 64;

/// `nal_unit_type` of a PPS (H.265 Table 7-1, `PPS_NUT`).
const HEVC_PPS: u8 = 34;
/// Last VCL `nal_unit_type` (`RSV_VCL31`, H.265 Table 7-1).
const HEVC_LAST_VCL: u8 = 31;
/// First IRAP `nal_unit_type` (`BLA_W_LP`, H.265 Table 7-1).
const HEVC_FIRST_IRAP: u8 = 16;
/// `nal_unit_type` `BLA_N_LP` (H.265 Table 7-1).
const HEVC_BLA_N_LP: u8 = 18;
/// `nal_unit_type` `IDR_W_RADL` (H.265 Table 7-1).
const HEVC_IDR_W_RADL: u8 = 19;
/// `nal_unit_type` `IDR_N_LP` (H.265 Table 7-1).
const HEVC_IDR_N_LP: u8 = 20;

/// `nuh_layer_id` of the base layer. The parameter-set ids are read only for
/// it: a non-base-layer SPS carries `u(4)` `sps_video_parameter_set_id`,
/// `u(3)` `sps_max_sub_layers_minus1`, the nesting flag and (for a slice) the
/// base-layer 2-D id and reference counts before the id.
const BASE_LAYER: u8 = 0;

/// Largest number of remembered dangling references, so a hostile stream
/// cannot grow the state without bound. Far above the 256 / 64 ids
/// H.264 §7.4.2.2 / H.265 §7.4.3.3 allow.
const MAX_UNRESOLVED_PPS: usize = 512;

/// How a slice's parameter sets resolved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum PpsCoverage {
    /// Every slice's PPS and its SPS are known.
    #[default]
    Known,
    /// At least one slice references a PPS not received yet (ffmpeg's
    /// "non-existing PPS N referenced").
    PpsUnseen,
    /// At least one slice references a known PPS whose SPS has not been
    /// received yet.
    SpsUnseen,
}

/// The parameter sets one input access unit activates (every slice before the
/// next access unit delimiter belongs to one picture in a conforming stream).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Activation {
    /// The SPS ids the access unit's slices resolve to.
    pub(crate) sps_ids: Vec<u8>,
    /// How the resolution went.
    pub(crate) coverage: PpsCoverage,
    /// The access unit is an H.264 IDR, or an H.265 IDR / BLA.
    pub(crate) random_access: bool,
}

/// The outcome of [`SpsTracker::observe`] for one access unit.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationOutcome {
    /// The access unit activates no parameter set the tracker follows (no
    /// coded picture, or no parseable slice): the previous state stays and
    /// nothing is raised.
    Unknown,
    /// Resolved, and the geometry is the one already in force: nothing is
    /// raised.
    Unchanged,
    /// The first resolution, or one to a different geometry.
    Changed {
        /// The new per-access-unit state (shared with the sample).
        display: Arc<DisplayInfo>,
        /// The track's first state (`changed.initial`).
        initial: bool,
        /// The access unit's IDR / BLA bit.
        random_access: bool,
        /// Which parts of the state differ from the previous one.
        changed: DisplayChangeSet,
    },
    /// A slice references a PPS (or its SPS) not received yet: the previous
    /// state stays and the reference is remembered (module docs, "unseen
    /// PPS").
    Unseen,
}

/// `seq_parameter_set_id` of an H.264 SPS; `None` if it does not decode.
fn avc_sps_id(nal: &[u8]) -> Option<u8> {
    decode_avc_slice_sps(nal).ok().map(|s| s.sps_id)
}

/// `(pps_id, sps_id)` of an H.264 PPS; `None` if it does not decode.
fn avc_pps_ids(nal: &[u8]) -> Option<(u8, u8)> {
    crate::slice_header::decode_avc_slice_pps(nal)
        .ok()
        .map(|p| (p.pps_id, p.sps_id))
}

/// `sps_seq_parameter_set_id` (H.265 §7.3.2.2) of a base-layer SPS.
fn hevc_sps_id(nal: &[u8]) -> Option<u8> {
    let mut r = BitReader::with_unescape(nal.get(2..)?, "H.265 SPS").ok()?;
    r.read_bits(4, "sps_video_parameter_set_id").ok()?;
    r.read_bits(3, "sps_max_sub_layers_minus1").ok()?;
    r.read_flag("sps_temporal_id_nesting_flag").ok()?;
    u8::try_from(r.read_ue("sps_seq_parameter_set_id").ok()?).ok()
}

/// `(pps_pic_parameter_set_id, pps_seq_parameter_set_id)` (H.265
/// §7.3.2.2 / §7.3.2.3) of a base-layer PPS.
fn hevc_pps_ids(nal: &[u8]) -> Option<(u8, u8)> {
    let mut r = BitReader::with_unescape(nal.get(2..)?, "H.265 PPS").ok()?;
    let pps_id = u8::try_from(r.read_ue("pps_pic_parameter_set_id").ok()?).ok()?;
    let sps_id = u8::try_from(r.read_ue("pps_seq_parameter_set_id").ok()?).ok()?;
    Some((pps_id, sps_id))
}

/// `slice_pic_parameter_set_id` (H.265 §7.3.6.1) of a base-layer,
/// independent slice segment; `None` for anything else.
fn hevc_slice_pps_id(nal: &[u8]) -> Option<u8> {
    let t = crate::nal::nal_unit_type(NalCodec::Hevc, nal)?;
    if t > HEVC_LAST_VCL || (nal[0] & 0x01) != BASE_LAYER {
        return None;
    }
    let end = nal.len().min(2 + SLICE_PREFIX_BYTES);
    let mut r = BitReader::with_unescape(nal.get(2..end)?, "H.265 slice header").ok()?;
    if !r.read_flag("first_slice_segment_in_pic_flag").ok()? {
        // A dependent segment carries overrides instead of the 2-D id and the
        // reference counts (H.265 §7.3.6.1). The picture's first segment is
        // always present in the access unit and is what selects the PPS.
        return None;
    }
    if (HEVC_FIRST_IRAP..=HEVC_LAST_VCL).contains(&t) {
        r.read_flag("no_output_of_prior_pics_flag").ok()?;
    }
    u8::try_from(r.read_ue("slice_pic_parameter_set_id").ok()?).ok()
}

/// `pic_parameter_set_id` of an H.264 slice header (§7.3.3: the third
/// `ue(v)`, after `first_mb_in_slice` and `slice_type`).
fn avc_slice_pps_id(nal: &[u8]) -> Option<u8> {
    let end = nal.len().min(1 + SLICE_PREFIX_BYTES);
    let mut r = BitReader::with_unescape(nal.get(1..end)?, "H.264 slice header").ok()?;
    r.read_ue("first_mb_in_slice").ok()?;
    r.read_ue("slice_type").ok()?;
    u8::try_from(r.read_ue("pic_parameter_set_id").ok()?).ok()
}

/// Read the SPS geometry of an H.264 SPS NAL.
fn avc_geometry(nal: &[u8]) -> Option<DisplayGeometry> {
    DisplayGeometry::from_avc_sps(nal).ok()
}

/// Read the SPS geometry of an H.265 SPS NAL.
fn hevc_geometry(nal: &[u8]) -> Option<DisplayGeometry> {
    DisplayGeometry::from_hevc_sps(nal).ok()
}

/// Which codec's parameter sets a [`SpsTracker`] follows.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackCodec {
    /// H.264 (ITU-T H.264 §7.3.2).
    Avc,
    /// H.265 (ITU-T H.265 §7.3.2).
    Hevc,
}

/// One base-layer PPS id and the SPS it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PpsBinding {
    sps_id: u8,
}

/// Which half of a slice's reference was missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnseenPart {
    Pps,
    Sps,
}

/// An access unit whose slices referenced an unseen PPS/SPS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Unresolved {
    pps_id: u8,
    part: UnseenPart,
}

/// A geometry change resolved in decode order, waiting for its output-order
/// position (module docs, "Output order").
#[derive(Debug, Clone)]
struct PendingDisplayChange {
    /// The state that replaces [`Self::previous`].
    new: Arc<DisplayInfo>,
    /// The state before the change (for [`DisplayInfo::changes_from`]).
    previous: Option<Arc<DisplayInfo>>,
    /// The PTS of the access unit that resolved the change.
    pts: Option<i64>,
    /// The change is the track's first state.
    initial: bool,
    /// The resolving access unit is an IDR / BLA.
    random_access: bool,
}

/// Per-track activation state: parameter sets **by id**, the state in force,
/// the change waiting for its output position, and the dangling references.
#[derive(Debug, Clone)]
pub struct SpsTracker {
    codec: TrackCodec,
    sps: BTreeMap<u8, DisplayGeometry>,
    pps: BTreeMap<u8, PpsBinding>,
    /// The state every sample carries until a new change is raised.
    state: Option<Arc<DisplayInfo>>,
    pending: Option<PendingDisplayChange>,
    unresolved: Vec<Unresolved>,
}

impl SpsTracker {
    /// A tracker for `codec` with no parameter sets and no state.
    pub fn new(codec: TrackCodec) -> Self {
        Self {
            codec,
            sps: BTreeMap::new(),
            pps: BTreeMap::new(),
            state: None,
            pending: None,
            unresolved: Vec::new(),
        }
    }

    /// The display state in force, shared with every sample that is not the
    /// first picture of a change.
    pub fn state(&self) -> Option<&Arc<DisplayInfo>> {
        self.state.as_ref()
    }

    /// Seed the state in force with a geometry already announced
    /// (`TrackAdded`), so the first access unit that resolves to it emits
    /// nothing.
    pub fn seed_state(&mut self, geometry: Option<DisplayGeometry>) {
        if self.state.is_none() {
            self.state = DisplayInfo::from_geometry(geometry).map(Arc::new);
        }
    }

    /// Fold one input access unit (Annex B display units; a raw MPEG-2 access
    /// unit carries no AVC/HEVC parameter sets and resolves to
    /// [`ActivationOutcome::Unknown`]).
    ///
    /// `pts` is the unit's presentation timestamp (track ticks), used to place
    /// a resolved change in output order.
    pub fn observe(&mut self, au: &[u8], pts: Option<i64>) -> ActivationOutcome {
        let mut activation = Activation::default();
        let mut slice_pps: Vec<Option<u8>> = Vec::new();
        for nal in iter_annexb_nals(au) {
            match self.codec {
                TrackCodec::Avc => self.observe_avc_nal(nal, &mut slice_pps, &mut activation),
                TrackCodec::Hevc => self.observe_hevc_nal(nal, &mut slice_pps, &mut activation),
            }
        }
        if slice_pps.is_empty() {
            let _ = self.settle(pts);
            return ActivationOutcome::Unknown;
        }
        self.resolve_slices(&slice_pps, &mut activation);
        match self.finish(activation, pts) {
            // A resolved picture that changed nothing (the pending change
            // already settled, or none) is an ordinary unchanged sample.
            ActivationOutcome::Unknown => ActivationOutcome::Unchanged,
            other => other,
        }
    }

    fn observe_avc_nal(
        &mut self,
        nal: &[u8],
        slice_pps: &mut Vec<Option<u8>>,
        activation: &mut Activation,
    ) {
        let Some(&header) = nal.first() else {
            return;
        };
        match header & AVC_NAL_TYPE_MASK {
            AVC_SPS => {
                if let (Some(id), Some(g)) = (avc_sps_id(nal), avc_geometry(nal)) {
                    self.sps.insert(id, g);
                }
            }
            AVC_PPS => {
                if let Some((pps_id, sps_id)) = avc_pps_ids(nal) {
                    self.pps.insert(pps_id, PpsBinding { sps_id });
                }
            }
            AVC_SLICE | AVC_IDR => {
                activation.random_access |= header & AVC_NAL_TYPE_MASK == AVC_IDR;
                slice_pps.push(avc_slice_pps_id(nal));
            }
            // A prefix NAL (§7.3.1.1) whose body is itself an SPS/PPS/slice.
            AVC_PREFIX_NAL => {
                if let Some(inner) = nal.get(1..) {
                    self.observe_avc_nal(inner, slice_pps, activation);
                }
            }
            _ => {}
        }
    }

    fn observe_hevc_nal(
        &mut self,
        nal: &[u8],
        slice_pps: &mut Vec<Option<u8>>,
        activation: &mut Activation,
    ) {
        let Some(t) = crate::nal::nal_unit_type(NalCodec::Hevc, nal) else {
            return;
        };
        if (nal[0] & 0x01) != BASE_LAYER {
            return;
        }
        match t {
            HEVC_SPS_NUT => {
                if let (Some(id), Some(g)) = (hevc_sps_id(nal), hevc_geometry(nal)) {
                    self.sps.insert(id, g);
                }
            }
            HEVC_PPS => {
                if let Some((pps_id, sps_id)) = hevc_pps_ids(nal) {
                    self.pps.insert(pps_id, PpsBinding { sps_id });
                }
            }
            0..=HEVC_LAST_VCL => {
                // IDR and BLA always begin a coded video sequence. A CRA does
                // not unless it is the first picture or follows an
                // end-of-sequence NAL, which this tracker does not see, so it
                // is not claimed (`random_access` stays false).
                activation.random_access |=
                    matches!(t, HEVC_IDR_W_RADL | HEVC_IDR_N_LP | HEVC_BLA_N_LP);
                slice_pps.push(hevc_slice_pps_id(nal));
            }
            _ => {}
        }
    }

    /// Resolve `slice_pps` against the sets recorded so far (an SPS/PPS in
    /// the same access unit counts: it is received before the slices, as a
    /// decoder sees it).
    fn resolve_slices(&mut self, slice_pps: &[Option<u8>], activation: &mut Activation) {
        for pps_id in slice_pps.iter().flatten() {
            match self.lookup(*pps_id) {
                Ok(sps_id) => {
                    if !activation.sps_ids.contains(&sps_id) {
                        activation.sps_ids.push(sps_id);
                    }
                }
                Err(part) => {
                    activation.coverage = match (activation.coverage, part) {
                        (PpsCoverage::PpsUnseen, _) => PpsCoverage::PpsUnseen,
                        (_, UnseenPart::Sps) => PpsCoverage::SpsUnseen,
                        (_, UnseenPart::Pps) => PpsCoverage::PpsUnseen,
                    };
                    self.remember(*pps_id, part);
                }
            }
        }
    }

    fn lookup(&self, pps_id: u8) -> Result<u8, UnseenPart> {
        let pps = self.pps.get(&pps_id).ok_or(UnseenPart::Pps)?;
        if !self.sps.contains_key(&pps.sps_id) {
            return Err(UnseenPart::Sps);
        }
        Ok(pps.sps_id)
    }

    fn remember(&mut self, pps_id: u8, part: UnseenPart) {
        let entry = Unresolved { pps_id, part };
        if self.unresolved.contains(&entry) {
            return;
        }
        if self.unresolved.len() >= MAX_UNRESOLVED_PPS {
            self.unresolved.remove(0);
        }
        self.unresolved.push(entry);
    }

    /// Forget every dangling reference that now resolves: the picture it bound
    /// was reported, and a later picture on the same sets must not re-raise
    /// the change.
    fn forget_resolved(&mut self) {
        let unresolved = core::mem::take(&mut self.unresolved);
        self.unresolved = unresolved
            .into_iter()
            .filter(|u| self.lookup(u.pps_id).is_err())
            .collect();
    }

    fn finish(&mut self, activation: Activation, pts: Option<i64>) -> ActivationOutcome {
        if activation.sps_ids.is_empty() {
            let _ = self.settle(pts);
            return if activation.coverage == PpsCoverage::Known {
                ActivationOutcome::Unknown
            } else {
                ActivationOutcome::Unseen
            };
        }
        self.forget_resolved();
        if self.pending.is_some() {
            return self.settle(pts);
        }
        let geometry = self.activated_geometry(&activation.sps_ids);
        let display = Arc::new(DisplayInfo::new(geometry));
        let unchanged = self
            .state
            .as_ref()
            .is_some_and(|state| state.as_ref() == display.as_ref());
        if unchanged {
            return ActivationOutcome::Unchanged;
        }
        let initial = self.state.is_none();
        let previous = self.state.replace(display.clone());
        self.pending = Some(PendingDisplayChange {
            new: display,
            previous,
            pts,
            initial,
            random_access: activation.random_access,
        });
        self.settle(pts)
    }

    /// Raise a pending change at its **output-order** position: once an
    /// access unit with a PTS at or after the one that resolved the change is
    /// observed, nothing earlier in decode order that is still to come can be
    /// displayed after it with the old geometry.
    fn settle(&mut self, pts: Option<i64>) -> ActivationOutcome {
        if pts.is_none() {
            // A unit with no clock cannot be placed: keep waiting for one.
            return ActivationOutcome::Unknown;
        }
        let reached = match self.pending.as_ref() {
            Some(pending) => match (pending.pts, pts) {
                (Some(resolved), Some(current)) => current >= resolved,
                _ => false,
            },
            None => false,
        };
        if !reached {
            return ActivationOutcome::Unknown;
        }
        let Some(pending) = self.pending.take() else {
            return ActivationOutcome::Unknown;
        };
        let changed = if pending.initial {
            DisplayChangeSet::initial()
        } else {
            DisplayInfo::changes_from(&pending.new, pending.previous.as_deref())
        };
        ActivationOutcome::Changed {
            display: pending.new,
            initial: pending.initial,
            random_access: pending.random_access,
            changed,
        }
    }

    /// The geometry of the activated set (the first SPS id's; a non-conforming
    /// access unit whose slices disagree keeps the first id rather than a
    /// fabricated merge).
    fn activated_geometry(&self, ids: &[u8]) -> Option<DisplayGeometry> {
        ids.first().and_then(|id| self.sps.get(id)).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::AspectRatioIdc;
    use alloc::vec;

    /// MSB-first bit writer for spec-shaped test NALs (the same shape
    /// `slice_header.rs` and `sps.rs` build theirs with).
    #[derive(Default)]
    struct W {
        out: Vec<u8>,
        cur: u8,
        n: u8,
    }

    impl W {
        fn u(&mut self, bits: u32, v: u32) {
            for i in (0..bits).rev() {
                self.cur = (self.cur << 1) | (((v >> i) & 1) as u8);
                self.n += 1;
                if self.n == 8 {
                    self.out.push(self.cur);
                    self.cur = 0;
                    self.n = 0;
                }
            }
        }
        fn ue(&mut self, v: u32) {
            let x = v + 1;
            let len = 32 - x.leading_zeros();
            self.u(len - 1, 0);
            self.u(len, x);
        }
        fn se(&mut self, v: i32) {
            self.ue(if v > 0 {
                (2 * v - 1) as u32
            } else {
                (-2 * v) as u32
            });
        }
        fn flag(&mut self, b: bool) {
            self.u(1, u32::from(b));
        }
        fn rbsp_trailing_bits(mut self) -> Vec<u8> {
            self.u(1, 1);
            while self.n != 0 {
                self.u(1, 0);
            }
            self.out
        }
    }

    /// An H.264 SPS NAL: `mb` x `mb` macroblock picture (16 luma samples per
    /// MB), no crop, with a VUI carrying `aspect_ratio_idc` (and, for
    /// Extended_SAR, the two terms). `level` differs only in bytes, never in
    /// geometry.
    fn sps_nal(sps_id: u32, mb: u32, idc: u32, extended: Option<(u32, u32)>, level: u8) -> Vec<u8> {
        let mut w = W::default();
        w.ue(sps_id);
        w.ue(0); // log2_max_frame_num_minus4
        w.ue(2); // pic_order_cnt_type
        w.ue(1); // max_num_ref_frames
        w.flag(false); // gaps_in_frame_num_value_allowed_flag
        w.ue(mb - 1); // pic_width_in_mbs_minus1
        w.ue(mb - 1); // pic_height_in_map_units_minus1
        w.flag(true); // frame_mbs_only_flag
        w.flag(true); // direct_8x8_inference_flag
        w.flag(false); // frame_cropping_flag
        w.flag(true); // vui_parameters_present_flag
        w.flag(true); // aspect_ratio_info_present_flag
        w.u(8, idc);
        if let Some((sw, sh)) = extended {
            w.u(16, sw);
            w.u(16, sh);
        }
        w.flag(false); // overscan_info_present_flag
        w.flag(true); // video_signal_type_present_flag
        w.u(3, 5); // video_format
        w.flag(false); // video_full_range_flag
        w.flag(false); // colour_description_present_flag
        let mut v = vec![0x67u8, 66, 0x00, level];
        v.extend(w.rbsp_trailing_bits());
        v
    }

    fn pps_nal(pps_id: u32, sps_id: u32) -> Vec<u8> {
        let mut w = W::default();
        w.ue(pps_id);
        w.ue(sps_id);
        w.flag(false); // entropy_coding_mode_flag
        w.flag(false); // bottom_field_pic_order_in_frame_present_flag
        w.ue(0); // num_slice_groups_minus1
        w.ue(0); // num_ref_idx_l0_default_active_minus1
        w.ue(0); // num_ref_idx_l1_default_active_minus1
        w.flag(false); // weighted_pred_flag
        w.u(2, 0); // weighted_bipred_idc
        w.se(0); // pic_init_qp_minus26
        w.se(0); // pic_init_qs_minus26
        w.se(0); // chroma_qp_index_offset
        w.flag(false); // deblocking_filter_control_present_flag
        w.flag(false); // constrained_intra_pred_flag
        w.flag(false); // redundant_pic_cnt_present_flag
        let mut v = vec![0x68u8];
        v.extend(w.rbsp_trailing_bits());
        v
    }

    fn idr_slice_nal(pps_id: u32) -> Vec<u8> {
        let mut w = W::default();
        w.ue(0); // first_mb_in_slice
        w.ue(2); // slice_type = I
        w.ue(pps_id);
        w.ue(0); // frame_num
        w.ue(0); // idr_pic_id
        let mut v = vec![0x65u8];
        v.extend(w.rbsp_trailing_bits());
        v
    }

    fn p_slice_nal(pps_id: u32) -> Vec<u8> {
        let mut w = W::default();
        w.ue(0); // first_mb_in_slice
        w.ue(0); // slice_type = P
        w.ue(pps_id);
        w.ue(1); // frame_num
        let mut v = vec![0x41u8]; // nal_ref_idc 2, nal_unit_type 1
        v.extend(w.rbsp_trailing_bits());
        v
    }

    fn annexb(nals: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for n in nals {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(n);
        }
        out
    }

    fn change(outcome: &ActivationOutcome) -> &DisplayChangeSet {
        match outcome {
            ActivationOutcome::Changed { changed, .. } => changed,
            other => panic!("expected a change, got {other:?}"),
        }
    }

    fn sar(outcome: &ActivationOutcome) -> Option<AspectRatioIdc> {
        match outcome {
            ActivationOutcome::Changed { display, .. } => display.geometry.as_ref()?.aspect_ratio,
            _ => None,
        }
    }

    fn tracker() -> SpsTracker {
        SpsTracker::new(TrackCodec::Avc)
    }

    fn base_au() -> Vec<u8> {
        annexb(&[sps_nal(0, 8, 4, None, 30), pps_nal(0, 0), idr_slice_nal(0)])
    }

    #[test]
    fn first_activation_is_the_initial_change() {
        let mut t = tracker();
        let outcome = t.observe(&base_au(), Some(100));
        let ActivationOutcome::Changed {
            initial, display, ..
        } = &outcome
        else {
            panic!("expected the initial change, got {outcome:?}");
        };
        assert!(*initial);
        assert_eq!(
            display.sample_aspect_ratio().map(|s| s.as_pair()),
            Some((16, 11))
        );
        assert_eq!(*change(&outcome), DisplayChangeSet::initial());
        assert_eq!(sar(&outcome), Some(AspectRatioIdc::Sar16x11));
    }

    #[test]
    fn an_identical_repeat_changes_nothing() {
        let mut t = tracker();
        t.observe(&base_au(), Some(0));
        assert!(matches!(
            t.observe(&base_au(), Some(3600)),
            ActivationOutcome::Unchanged
        ));
        assert!(matches!(
            t.observe(&annexb(&[p_slice_nal(0)]), Some(7200)),
            ActivationOutcome::Unchanged
        ));
    }

    #[test]
    fn a_geometry_neutral_sps_change_is_not_a_change() {
        let mut t = tracker();
        t.observe(&base_au(), Some(0));
        // Same SAR and size, different level_idc.
        let b = annexb(&[sps_nal(0, 8, 4, None, 40), pps_nal(0, 0), idr_slice_nal(0)]);
        assert!(matches!(
            t.observe(&b, Some(3600)),
            ActivationOutcome::Unchanged
        ));
    }

    #[test]
    fn a_sar_only_change_with_the_same_dar_sets_aspect_ratio() {
        let mut t = tracker();
        t.observe(&base_au(), Some(0));
        // 16:11 (idc 4) -> 64:45 (Extended_SAR).
        let b = annexb(&[
            sps_nal(0, 8, 255, Some((64, 45)), 30),
            pps_nal(0, 0),
            idr_slice_nal(0),
        ]);
        let outcome = t.observe(&b, Some(3600));
        let c = change(&outcome);
        assert!(c.aspect_ratio && !c.initial, "{c:?}");
        assert!(!c.output_rect && !c.video_signal && !c.overscan, "{c:?}");
        assert_eq!(
            sar(&outcome),
            Some(AspectRatioIdc::ExtendedSar {
                sar_width: 64,
                sar_height: 45
            })
        );
    }

    #[test]
    fn a_new_id_sent_ahead_of_use_fires_only_at_the_referencing_au() {
        let mut t = tracker();
        t.observe(&base_au(), Some(0));
        // SPS id 1 (new geometry) arrives with no slice referencing it.
        assert!(matches!(
            t.observe(&annexb(&[sps_nal(1, 16, 1, None, 30)]), Some(3600)),
            ActivationOutcome::Unknown
        ));
        // A picture still on SPS 0.
        assert!(matches!(
            t.observe(&annexb(&[p_slice_nal(0)]), Some(7200)),
            ActivationOutcome::Unchanged
        ));
        // The first picture that references SPS 1, through a new PPS id.
        let outcome = t.observe(&annexb(&[pps_nal(1, 1), idr_slice_nal(1)]), Some(10800));
        let ActivationOutcome::Changed {
            initial,
            random_access,
            display,
            changed,
        } = &outcome
        else {
            panic!("expected a change, got {outcome:?}");
        };
        assert!(!*initial && *random_access);
        assert!(changed.aspect_ratio && changed.output_rect, "{changed:?}");
        assert_eq!(display.geometry.as_ref().map(|g| g.coded_width), Some(256));
    }

    #[test]
    fn a_mid_cvs_change_is_not_random_access() {
        let mut t = tracker();
        t.observe(&base_au(), Some(0));
        // Non-conforming: the active SPS's content changes and a P picture
        // (not an IDR) first uses it.
        let outcome = t.observe(
            &annexb(&[sps_nal(0, 8, 1, None, 30), p_slice_nal(0)]),
            Some(3600),
        );
        assert!(matches!(
            outcome,
            ActivationOutcome::Changed {
                random_access: false,
                ..
            }
        ));
    }

    #[test]
    fn a_pts_earlier_than_the_resolving_au_settles_on_the_first_au_at_or_after_it() {
        let mut t = tracker();
        assert!(matches!(
            t.observe(&base_au(), Some(360)),
            ActivationOutcome::Changed { .. }
        ));
        // Decoded later, displayed before the keyframe: still the old state.
        assert!(matches!(
            t.observe(&annexb(&[p_slice_nal(0)]), Some(0)),
            ActivationOutcome::Unchanged
        ));
    }

    #[test]
    fn an_unseen_pps_holds_the_state_and_binds_on_a_later_au() {
        let mut t = tracker();
        t.observe(&base_au(), Some(0));
        // A slice referencing PPS 1 before PPS 1 arrived: nothing invented,
        // state held.
        assert!(matches!(
            t.observe(&annexb(&[idr_slice_nal(1)]), Some(3600)),
            ActivationOutcome::Unseen
        ));
        assert_eq!(
            t.state()
                .and_then(|s| s.sample_aspect_ratio())
                .map(|s| s.as_pair()),
            Some((16, 11))
        );
        // The new SPS and PPS arrive (carried with the next access unit): the
        // first picture that resolves them reports the change.
        let outcome = t.observe(
            &annexb(&[sps_nal(1, 8, 1, None, 30), pps_nal(1, 1), idr_slice_nal(1)]),
            Some(7200),
        );
        assert!(
            matches!(outcome, ActivationOutcome::Changed { .. }),
            "{outcome:?}"
        );
        assert_eq!(sar(&outcome), Some(AspectRatioIdc::Square));
        // A later picture on the same sets does not report it again.
        assert!(matches!(
            t.observe(&annexb(&[p_slice_nal(1)]), Some(10800)),
            ActivationOutcome::Unchanged
        ));
    }

    #[test]
    fn a_pps_naming_an_unseen_sps_is_held_too() {
        let mut t = tracker();
        t.observe(&base_au(), Some(0));
        assert!(matches!(
            t.observe(&annexb(&[pps_nal(2, 3), idr_slice_nal(2)]), Some(3600)),
            ActivationOutcome::Unseen
        ));
        let outcome = t.observe(
            &annexb(&[sps_nal(3, 8, 1, None, 30), idr_slice_nal(2)]),
            Some(7200),
        );
        assert!(
            matches!(outcome, ActivationOutcome::Changed { .. }),
            "{outcome:?}"
        );
    }

    #[test]
    fn a_non_video_access_unit_resolves_nothing() {
        let mut t = tracker();
        assert!(matches!(
            t.observe(&[0x00, 0x00, 0x01, 0xb3], Some(0)),
            ActivationOutcome::Unknown
        ));
    }
}
