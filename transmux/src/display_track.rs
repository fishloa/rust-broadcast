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
//! # Decode order (§6.2, §6.5)
//!
//! A change is resolved at the access unit whose slices first activate the
//! changed SPS and is reported for **that** access unit: the demuxer parks it
//! on the access unit's own one-behind slot and releases it immediately
//! before that access unit's `Sample`, with the sample's `dts`/`pts` and the
//! access unit's own IDR / BLA bit. A consumer applying it on receipt labels
//! exactly the first picture that carries the new geometry.
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

use crate::ir::{
    ActiveFormat, AfdPersistence, DisplayChangeSet, DisplayGeometry, DisplayInfo, UserDataCarriage,
};
use broadcast_bitstream::facts::{ActiveFormatDescription, BarData};

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use broadcast_bitstream::annexb::iter_annexb_nals;
use broadcast_bitstream::bitreader::BitReader;
use broadcast_bitstream::nal::{HEVC_NAL_HEADER_LEN, NAL_TYPE_MASK, NalCodec};
use broadcast_bitstream::sps::decode_avc_slice_sps;
use broadcast_bitstream::user_data;
use isobmff::nalu_types::HEVC_SPS_NUT;

/// `nal_unit_type` of a non-IDR coded slice (H.264 Table 7-1).
const AVC_SLICE: u8 = 1;
/// `nal_unit_type` of an IDR coded slice (H.264 Table 7-1).
const AVC_IDR: u8 = 5;
/// `nal_unit_type` of an SPS (H.264 Table 7-1).
const AVC_SPS: u8 = 7;
/// `nal_unit_type` of a PPS (H.264 Table 7-1).
const AVC_PPS: u8 = 8;
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

/// Bit of the H.265 NAL header byte 0 carrying the low bit of `nuh_layer_id`
/// (ITU-T H.265 §7.3.1.2).
const HEVC_NAL_LAYER_ID_BIT_MASK: u8 = 0x01;
/// Width of `sps_video_parameter_set_id` (H.265 §7.3.2.2).
const SPS_VIDEO_PARAMETER_SET_ID_BITS: usize = 4;
/// Width of `sps_max_sub_layers_minus1` (H.265 §7.3.2.2).
const SPS_MAX_SUB_LAYERS_MINUS1_BITS: usize = 3;

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
    /// AFD signal present in this AU.
    pub(crate) afd: Option<broadcast_bitstream::AfdSignal>,
    /// Parsed bar data.
    pub(crate) bar_data: Option<BarData>,
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
    decode_avc_slice_sps(nal).ok().map(|s| s.sps_id())
}

/// `(pps_id, sps_id)` of an H.264 PPS; `None` if it does not decode.
fn avc_pps_ids(nal: &[u8]) -> Option<(u8, u8)> {
    broadcast_bitstream::slice_header::decode_avc_slice_pps(nal)
        .ok()
        .map(|p| (p.pps_id(), p.sps_id()))
}

/// `sps_seq_parameter_set_id` (H.265 §7.3.2.2) of a base-layer SPS.
fn hevc_sps_id(nal: &[u8]) -> Option<u8> {
    let mut r = BitReader::with_unescape(nal.get(HEVC_NAL_HEADER_LEN..)?, "H.265 SPS").ok()?;
    r.read_bits(
        SPS_VIDEO_PARAMETER_SET_ID_BITS,
        "sps_video_parameter_set_id",
    )
    .ok()?;
    let max_sub = r
        .read_bits(SPS_MAX_SUB_LAYERS_MINUS1_BITS, "sps_max_sub_layers_minus1")
        .ok()? as u8;
    r.read_flag("sps_temporal_id_nesting_flag").ok()?;
    let _ = broadcast_bitstream::sps::decode_ptl(&mut r, true, max_sub).ok()?;
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
    let t = broadcast_bitstream::nal::nal_unit_type(NalCodec::Hevc, nal)?;
    if t > HEVC_LAST_VCL || (nal[0] & HEVC_NAL_LAYER_ID_BIT_MASK) != BASE_LAYER {
        return None;
    }
    let end = nal.len().min(HEVC_NAL_HEADER_LEN + SLICE_PREFIX_BYTES);
    let mut r =
        BitReader::with_unescape(nal.get(HEVC_NAL_HEADER_LEN..end)?, "H.265 slice header").ok()?;
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

/// Per-track activation state: parameter sets **by id**, the state in force
/// and the change waiting for its output position.
#[derive(Debug, Clone)]
pub struct SpsTracker {
    codec: TrackCodec,
    afd_persistence: AfdPersistence,
    sps: BTreeMap<u8, DisplayGeometry>,
    pps: BTreeMap<u8, PpsBinding>,
    /// The state every sample carries until a new change is raised.
    state: Option<Arc<DisplayInfo>>,
    /// SEI structures the walk could not parse, for the caller to report.
    findings: broadcast_bitstream::Findings,
}

impl SpsTracker {
    /// A tracker for `codec` with no parameter sets and no state.
    pub fn new(codec: TrackCodec) -> Self {
        Self {
            codec,
            afd_persistence: AfdPersistence::UntilReplaced,
            sps: BTreeMap::new(),
            pps: BTreeMap::new(),
            state: None,
            findings: broadcast_bitstream::Findings::new(),
        }
    }

    pub fn with_afd_persistence(mut self, policy: AfdPersistence) -> Self {
        self.afd_persistence = policy;
        self
    }

    /// Seed the state with a full DisplayInfo (geometry + active format).
    pub fn seed_full(&mut self, display: Option<Arc<DisplayInfo>>) {
        if self.state.is_none() {
            self.state = display;
        }
    }

    pub fn set_state(&mut self, display: Option<Arc<DisplayInfo>>) {
        self.state = display;
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
    pub fn observe(&mut self, au: &[u8]) -> ActivationOutcome {
        let mut activation = Activation::default();
        let mut slice_pps: Vec<Option<u8>> = Vec::new();
        for nal in iter_annexb_nals(au) {
            match self.codec {
                TrackCodec::Avc => self.observe_avc_nal(nal, &mut slice_pps, &mut activation),
                TrackCodec::Hevc => self.observe_hevc_nal(nal, &mut slice_pps, &mut activation),
            }
        }
        if slice_pps.is_empty() {
            return ActivationOutcome::Unknown;
        }
        self.resolve_slices(&slice_pps, &mut activation);
        self.finish(activation)
    }

    /// The SEI structures the walk could not parse since the last call.
    pub fn take_findings(&mut self) -> broadcast_bitstream::Findings {
        core::mem::take(&mut self.findings)
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
        match header & NAL_TYPE_MASK {
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
                activation.random_access |= header & NAL_TYPE_MASK == AVC_IDR;
                slice_pps.push(avc_slice_pps_id(nal));
            }
            // SEI: collect AFD / bar data (first wins).
            6 => {
                self.parse_sei_afd(NalCodec::Avc, nal, activation);
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
        let Some(t) = broadcast_bitstream::nal::nal_unit_type(NalCodec::Hevc, nal) else {
            return;
        };
        if (nal[0] & HEVC_NAL_LAYER_ID_BIT_MASK) != BASE_LAYER {
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
            39 => {
                self.parse_sei_afd(NalCodec::Hevc, nal, activation);
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

    /// Parse AFD / bar data from a single SEI NAL into `activation` (first wins).
    fn parse_sei_afd(&mut self, codec: NalCodec, nal: &[u8], activation: &mut Activation) {
        let body = match codec {
            NalCodec::Avc => &nal[1..],
            NalCodec::Hevc => &nal[2..],
            _ => return,
        };
        let msgs = broadcast_bitstream::nal::sei_messages(body, &mut self.findings);
        for (payload_type, payload) in msgs {
            if payload_type != user_data::SEI_PAYLOAD_TYPE_USER_DATA_REGISTERED_ITU_T_T35 {
                continue;
            }
            if activation.afd.is_none()
                && let Some(signal) = user_data::afd_from_t35_payload(&payload)
            {
                activation.afd = Some(signal);
            }
            if activation.bar_data.is_none()
                && let Some(bd) = user_data::bar_data_from_t35_payload(&payload)
            {
                activation.bar_data = Some(bd);
            }
        }
    }

    fn finish(&mut self, activation: Activation) -> ActivationOutcome {
        if activation.sps_ids.is_empty() {
            return if activation.coverage == PpsCoverage::Known {
                ActivationOutcome::Unknown
            } else {
                // The dangling reference is remembered; the first access unit
                // that resolves it (once its PPS and SPS have arrived)
                // reports the change.
                ActivationOutcome::Unseen
            };
        }
        let geometry = self.activated_geometry(&activation.sps_ids);
        let carriage = match self.codec {
            TrackCodec::Avc => UserDataCarriage::AvcSei,
            TrackCodec::Hevc => UserDataCarriage::HevcPrefixSei,
        };
        let previous_active = self.state.as_ref().and_then(|s| s.active_format.clone());
        let mut new_active = previous_active.clone();
        if let Some(signal) = &activation.afd {
            let afd_val = match signal {
                broadcast_bitstream::AfdSignal::Format(a) => Some(*a),
                _ => None,
            };
            new_active = Some(ActiveFormat {
                afd: afd_val,
                bar_data: activation.bar_data.clone(),
                carriage,
            });
        } else if activation.bar_data.is_some() {
            match &mut new_active {
                Some(af) => af.bar_data = activation.bar_data.clone(),
                None => {
                    new_active = Some(ActiveFormat {
                        afd: None,
                        bar_data: activation.bar_data.clone(),
                        carriage,
                    })
                }
            }
        }
        if activation.afd.is_none()
            && self.afd_persistence == AfdPersistence::ResetAtRandomAccess
            && activation.random_access
        {
            new_active = Some(ActiveFormat {
                afd: Some(ActiveFormatDescription::SameAsCodedFrame),
                bar_data: None,
                carriage,
            });
        }
        let display = Arc::new(DisplayInfo::new(geometry).with_active_format(new_active));
        let unchanged = self
            .state
            .as_ref()
            .is_some_and(|state| state.as_ref() == display.as_ref());
        if unchanged {
            return ActivationOutcome::Unchanged;
        }
        let initial = self.state.is_none();
        let previous = self.state.replace(display.clone());
        let changed = if initial {
            DisplayChangeSet::initial()
        } else {
            DisplayInfo::changes_from(&display, previous.as_deref())
        };
        // `random_access` is this access unit's own IDR / BLA bit, not the
        // sample's sync flag: an H.264 open-GOP recovery-point picture is a
        // sync sample but not an IDR, and cannot carry a conforming SPS
        // content change (design §6.2).
        ActivationOutcome::Changed {
            display,
            initial,
            random_access: activation.random_access,
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
    use alloc::vec;
    use broadcast_bitstream::facts::AspectRatioIdc;
    use test_fixtures::bits::BitWriter;

    /// An H.264 SPS NAL: `mb` x `mb` macroblock picture (16 luma samples per
    /// MB), no crop, with a VUI carrying `aspect_ratio_idc` (and, for
    /// Extended_SAR, the two terms). `level` differs only in bytes, never in
    /// geometry.
    fn sps_nal(sps_id: u32, mb: u32, idc: u32, extended: Option<(u32, u32)>, level: u8) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.ue(u64::from(sps_id));
        w.ue(0); // log2_max_frame_num_minus4
        w.ue(2); // pic_order_cnt_type
        w.ue(1); // max_num_ref_frames
        w.bit(false); // gaps_in_frame_num_value_allowed_flag
        w.ue(u64::from(mb - 1)); // pic_width_in_mbs_minus1
        w.ue(u64::from(mb - 1)); // pic_height_in_map_units_minus1
        w.bit(true); // frame_mbs_only_flag
        w.bit(true); // direct_8x8_inference_flag
        w.bit(false); // frame_cropping_flag
        w.bit(true); // vui_parameters_present_flag
        w.bit(true); // aspect_ratio_info_present_flag
        w.u(8, u64::from(idc));
        if let Some((sw, sh)) = extended {
            w.u(16, u64::from(sw));
            w.u(16, u64::from(sh));
        }
        w.bit(false); // overscan_info_present_flag
        w.bit(true); // video_signal_type_present_flag
        w.u(3, 5); // video_format
        w.bit(false); // video_full_range_flag
        w.bit(false); // colour_description_present_flag
        let mut v = vec![0x67u8, 66, 0x00, level];
        v.extend(w.trailing());
        v
    }

    fn pps_nal(pps_id: u32, sps_id: u32) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.ue(u64::from(pps_id));
        w.ue(u64::from(sps_id));
        w.bit(false); // entropy_coding_mode_flag
        w.bit(false); // bottom_field_pic_order_in_frame_present_flag
        w.ue(0); // num_slice_groups_minus1
        w.ue(0); // num_ref_idx_l0_default_active_minus1
        w.ue(0); // num_ref_idx_l1_default_active_minus1
        w.bit(false); // weighted_pred_flag
        w.u(2, 0); // weighted_bipred_idc
        w.se(0); // pic_init_qp_minus26
        w.se(0); // pic_init_qs_minus26
        w.se(0); // chroma_qp_index_offset
        w.bit(false); // deblocking_filter_control_present_flag
        w.bit(false); // constrained_intra_pred_flag
        w.bit(false); // redundant_pic_cnt_present_flag
        let mut v = vec![0x68u8];
        v.extend(w.trailing());
        v
    }

    fn idr_slice_nal(pps_id: u32) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.ue(0); // first_mb_in_slice
        w.ue(2); // slice_type = I
        w.ue(u64::from(pps_id));
        w.ue(0); // frame_num
        w.ue(0); // idr_pic_id
        let mut v = vec![0x65u8];
        v.extend(w.trailing());
        v
    }

    fn p_slice_nal(pps_id: u32) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.ue(0); // first_mb_in_slice
        w.ue(0); // slice_type = P
        w.ue(u64::from(pps_id));
        w.ue(1); // frame_num
        let mut v = vec![0x41u8]; // nal_ref_idc 2, nal_unit_type 1
        v.extend(w.trailing());
        v
    }

    /// An H.265 base-layer SPS carrying only the fields the geometry needs
    /// (`sps_seq_parameter_set_id`, chroma format, picture size); the walk past
    /// `bit_depth_chroma_minus8` is error-swallowing, so the RBSP ends there.
    fn hevc_sps_nal(sps_id: u32, width: u32, height: u32) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.u(4, 0); // sps_video_parameter_set_id
        w.u(3, 0); // sps_max_sub_layers_minus1
        w.bit(true); // sps_temporal_id_nesting_flag
        w.u(2, 0); // general_profile_space
        w.bit(false); // general_tier_flag
        w.u(5, 1); // general_profile_idc
        w.u(32, 0); // general_profile_compatibility_flags
        w.u(48, 0); // general_constraint_indicator_flags
        w.u(8, 93); // general_level_idc
        w.ue(u64::from(sps_id)); // sps_seq_parameter_set_id
        w.ue(1); // chroma_format_idc
        w.ue(u64::from(width)); // pic_width_in_luma_samples
        w.ue(u64::from(height)); // pic_height_in_luma_samples
        w.bit(false); // conformance_window_flag
        w.ue(0); // bit_depth_luma_minus8
        w.ue(0); // bit_depth_chroma_minus8
        let mut v = vec![0x42u8, 0x01];
        v.extend(w.trailing());
        v
    }

    fn hevc_pps_nal(pps_id: u32, sps_id: u32) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.ue(u64::from(pps_id)); // pps_pic_parameter_set_id
        w.ue(u64::from(sps_id)); // pps_seq_parameter_set_id
        let mut v = vec![0x44u8, 0x01];
        v.extend(w.trailing());
        v
    }

    fn hevc_slice_nal(nal_type: u8, pps_id: u32) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.bit(true); // first_slice_segment_in_pic_flag
        if (HEVC_FIRST_IRAP..=HEVC_LAST_VCL).contains(&nal_type) {
            w.bit(false); // no_output_of_prior_pics_flag
        }
        w.ue(u64::from(pps_id)); // slice_pic_parameter_set_id
        let mut v = vec![nal_type << 1, 0x01];
        v.extend(w.trailing());
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
        let outcome = t.observe(&base_au());
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
        t.observe(&base_au());
        assert!(matches!(
            t.observe(&base_au()),
            ActivationOutcome::Unchanged
        ));
        assert!(matches!(
            t.observe(&annexb(&[p_slice_nal(0)])),
            ActivationOutcome::Unchanged
        ));
    }

    /// An H.264 SVC/MVC prefix NAL (`nal_unit_type` 14, H.264 Table 7-1): its
    /// payload is a 3-byte SVC extension followed by `prefix_nal_unit_rbsp()`
    /// (§7.3.2.12.2), never a nested SPS, PPS or slice.
    fn prefix_nal(payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0x6Eu8];
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn a_prefix_nal_carries_no_parameter_set() {
        let plain = base_au();
        let mut t = tracker();
        let without = t.observe(&plain);

        // The prefix NAL's bytes after its header happen to spell a complete
        // SPS with a different geometry and the same id; reading them as one
        // would replace the geometry the slices activate.
        let mut nested = tracker();
        let with_nested = nested.observe(&annexb(&[
            sps_nal(0, 8, 4, None, 30),
            prefix_nal(&sps_nal(0, 16, 1, None, 30)),
            pps_nal(0, 0),
            idr_slice_nal(0),
        ]));
        assert_eq!(with_nested, without);

        let mut leading = tracker();
        let with_leading = leading.observe(&annexb(&[
            prefix_nal(&sps_nal(1, 16, 1, None, 30)),
            sps_nal(0, 8, 4, None, 30),
            pps_nal(0, 0),
            idr_slice_nal(0),
        ]));
        assert_eq!(with_leading, without);
    }

    #[test]
    fn a_geometry_neutral_sps_change_is_not_a_change() {
        let mut t = tracker();
        t.observe(&base_au());
        // Same SAR and size, different level_idc.
        let b = annexb(&[sps_nal(0, 8, 4, None, 40), pps_nal(0, 0), idr_slice_nal(0)]);
        assert!(matches!(t.observe(&b), ActivationOutcome::Unchanged));
    }

    #[test]
    fn a_sar_only_change_with_the_same_dar_sets_aspect_ratio() {
        let mut t = tracker();
        t.observe(&base_au());
        // 16:11 (idc 4) -> 64:45 (Extended_SAR).
        let b = annexb(&[
            sps_nal(0, 8, 255, Some((64, 45)), 30),
            pps_nal(0, 0),
            idr_slice_nal(0),
        ]);
        let outcome = t.observe(&b);
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
        t.observe(&base_au());
        // SPS id 1 (new geometry) arrives with no slice referencing it.
        assert!(matches!(
            t.observe(&annexb(&[sps_nal(1, 16, 1, None, 30)])),
            ActivationOutcome::Unknown
        ));
        // A picture still on SPS 0.
        assert!(matches!(
            t.observe(&annexb(&[p_slice_nal(0)])),
            ActivationOutcome::Unchanged
        ));
        // The first picture that references SPS 1, through a new PPS id.
        let outcome = t.observe(&annexb(&[pps_nal(1, 1), idr_slice_nal(1)]));
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
        t.observe(&base_au());
        // Non-conforming: the active SPS's content changes and a P picture
        // (not an IDR) first uses it.
        let outcome = t.observe(&annexb(&[sps_nal(0, 8, 1, None, 30), p_slice_nal(0)]));
        assert!(matches!(
            outcome,
            ActivationOutcome::Changed {
                random_access: false,
                ..
            }
        ));
    }

    #[test]
    fn an_unseen_pps_holds_the_state_and_binds_on_a_later_au() {
        let mut t = tracker();
        t.observe(&base_au());
        // A slice referencing PPS 1 before PPS 1 arrived: nothing invented,
        // state held.
        assert!(matches!(
            t.observe(&annexb(&[idr_slice_nal(1)])),
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
        let outcome = t.observe(&annexb(&[
            sps_nal(1, 8, 1, None, 30),
            pps_nal(1, 1),
            idr_slice_nal(1),
        ]));
        assert!(
            matches!(outcome, ActivationOutcome::Changed { .. }),
            "{outcome:?}"
        );
        assert_eq!(sar(&outcome), Some(AspectRatioIdc::Square));
        // A later picture on the same sets does not report it again.
        assert!(matches!(
            t.observe(&annexb(&[p_slice_nal(1)])),
            ActivationOutcome::Unchanged
        ));
    }

    #[test]
    fn a_pps_naming_an_unseen_sps_is_held_too() {
        let mut t = tracker();
        t.observe(&base_au());
        assert!(matches!(
            t.observe(&annexb(&[pps_nal(2, 3), idr_slice_nal(2)])),
            ActivationOutcome::Unseen
        ));
        let outcome = t.observe(&annexb(&[sps_nal(3, 8, 1, None, 30), idr_slice_nal(2)]));
        assert!(
            matches!(outcome, ActivationOutcome::Changed { .. }),
            "{outcome:?}"
        );
    }

    fn geometry_state(width: u32) -> Option<Arc<DisplayInfo>> {
        DisplayInfo::from_geometry(Some(DisplayGeometry::new(width, width, None))).map(Arc::new)
    }

    fn state_width(t: &SpsTracker) -> Option<u32> {
        t.state()
            .and_then(|s| s.geometry.as_ref())
            .map(|g| g.coded_width)
    }

    #[test]
    fn seed_full_only_fills_an_empty_state() {
        let mut t = tracker();
        t.seed_full(geometry_state(8));
        assert_eq!(state_width(&t), Some(8));
        t.seed_full(geometry_state(16));
        assert_eq!(state_width(&t), Some(8), "the first state is kept");
    }

    #[test]
    fn set_state_replaces_the_state_unconditionally() {
        let mut t = tracker();
        t.set_state(geometry_state(8));
        assert_eq!(state_width(&t), Some(8));
        t.set_state(geometry_state(16));
        assert_eq!(state_width(&t), Some(16));
        t.set_state(None);
        assert_eq!(state_width(&t), None);
    }

    #[test]
    fn a_non_video_access_unit_resolves_nothing() {
        let mut t = tracker();
        assert!(matches!(
            t.observe(&[0x00, 0x00, 0x01, 0xb3]),
            ActivationOutcome::Unknown
        ));
    }

    #[test]
    fn hevc_ids_beyond_zero_resolve_to_their_own_sps() {
        let mut t = SpsTracker::new(TrackCodec::Hevc);
        let first = t.observe(&annexb(&[
            hevc_sps_nal(0, 64, 64),
            hevc_pps_nal(0, 0),
            hevc_slice_nal(HEVC_IDR_W_RADL, 0),
        ]));
        let ActivationOutcome::Changed {
            display,
            random_access,
            ..
        } = &first
        else {
            panic!("expected the initial change, got {first:?}");
        };
        assert!(*random_access, "IDR_W_RADL is a random-access picture");
        let g = display.geometry.as_ref().expect("geometry");
        assert_eq!((g.coded_width, g.coded_height), (64, 64));

        // SPS 1 (a different picture size) reached through PPS 2.
        let second = t.observe(&annexb(&[
            hevc_sps_nal(1, 128, 96),
            hevc_pps_nal(2, 1),
            hevc_slice_nal(HEVC_IDR_W_RADL, 2),
        ]));
        let ActivationOutcome::Changed {
            display,
            random_access,
            ..
        } = &second
        else {
            panic!("expected the change to SPS 1, got {second:?}");
        };
        assert!(*random_access);
        let g = display.geometry.as_ref().expect("geometry");
        assert_eq!((g.coded_width, g.coded_height), (128, 96));

        // A CRA picture (H.265 Table 7-1, `CRA_NUT` 21) is not claimed as a
        // coded-video-sequence start by this tracker.
        const HEVC_CRA_NUT: u8 = 21;
        let third = t.observe(&annexb(&[
            hevc_sps_nal(3, 192, 108),
            hevc_pps_nal(4, 3),
            hevc_slice_nal(HEVC_CRA_NUT, 4),
        ]));
        let ActivationOutcome::Changed {
            display,
            random_access,
            ..
        } = &third
        else {
            panic!("expected the change to SPS 3, got {third:?}");
        };
        assert!(!*random_access, "a CRA does not begin a CVS here");
        let g = display.geometry.as_ref().expect("geometry");
        assert_eq!((g.coded_width, g.coded_height), (192, 108));
    }
}
