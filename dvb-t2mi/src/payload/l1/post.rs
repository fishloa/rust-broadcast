//! L1-post signalling — EN 302 755 §7.2.3 (Figures 27 and 28).
//!
//! L1-post consists of:
//! - **Configurable** part (Figure 27, `L1PostConfigurable`) — stable for
//!   one super-frame.
//! - **Dynamic** part (Figure 28, `L1PostDynamic`) — may change each frame.
//! - Optional **Extension** blocks (Table 37, `L1ExtBlock`).
//!
//! As carried in T2-MI (ETSI TS 102 773 §5.2.4 Table 2), each section is
//! prefixed with a 16-bit bit-length and zero-padded to a byte boundary.
//!
//! Parsing uses [`dvb_common::bits::BitReader`] / [`dvb_common::bits::BitWriter`] (MSB-first).

use dvb_common::bits::{BitReader, BitWriter};

use super::enums::{
    AuxStreamType, PlpCodeRate, PlpFecType, PlpMode, PlpModulation, PlpPayloadType, PlpType,
};

// ── field widths: Configurable L1-post (Figure 27) ───────────────────────────

const SUB_SLICES_PER_FRAME_BITS: u32 = 15;
const NUM_PLP_BITS: u32 = 8;
const NUM_AUX_BITS: u32 = 4;
const AUX_CONFIG_RFU_BITS: u32 = 8;
// RF loop per entry
const RF_IDX_BITS: u32 = 3;
const FREQUENCY_BITS: u32 = 32;
// FEF block
const FEF_TYPE_BITS: u32 = 4;
const FEF_LENGTH_BITS: u32 = 22;
const FEF_INTERVAL_BITS: u32 = 8;
// PLP loop per entry
const PLP_ID_BITS: u32 = 8;
const PLP_TYPE_BITS: u32 = 3;
const PLP_PAYLOAD_TYPE_BITS: u32 = 5;
const FF_FLAG_BITS: u32 = 1;
const FIRST_RF_IDX_BITS: u32 = 3;
const FIRST_FRAME_IDX_BITS: u32 = 8;
const PLP_GROUP_ID_BITS: u32 = 8;
const PLP_COD_BITS: u32 = 3;
const PLP_MOD_BITS: u32 = 3;
const PLP_ROTATION_BITS: u32 = 1;
const PLP_FEC_TYPE_BITS: u32 = 2;
const PLP_NUM_BLOCKS_MAX_BITS: u32 = 10;
const FRAME_INTERVAL_BITS: u32 = 8;
const TIME_IL_LENGTH_BITS: u32 = 8;
const TIME_IL_TYPE_BITS: u32 = 1;
const IN_BAND_A_FLAG_BITS: u32 = 1;
const IN_BAND_B_FLAG_BITS: u32 = 1;
const PLP_RESERVED_1_BITS: u32 = 11;
const PLP_MODE_BITS: u32 = 2;
const STATIC_FLAG_BITS: u32 = 1;
const STATIC_PADDING_FLAG_BITS: u32 = 1;
// post-PLP-loop
const FEF_LENGTH_MSB_BITS: u32 = 2;
const CONF_RESERVED_2_BITS: u32 = 30;
// AUX loop per entry
const AUX_STREAM_TYPE_BITS: u32 = 4;
const AUX_PRIVATE_CONF_BITS: u32 = 28;

// ── field widths: Dynamic L1-post (Figure 28) ────────────────────────────────

const FRAME_IDX_BITS: u32 = 8;
const SUB_SLICE_INTERVAL_BITS: u32 = 22;
const TYPE_2_START_BITS: u32 = 22;
const L1_CHANGE_COUNTER_BITS: u32 = 8;
const START_RF_IDX_BITS: u32 = 3;
const DYN_RESERVED_1_BITS: u32 = 8;
// PLP dynamic loop
const DYN_PLP_ID_BITS: u32 = 8;
const PLP_START_BITS: u32 = 22;
const PLP_NUM_BLOCKS_BITS: u32 = 10;
const DYN_PLP_RESERVED_2_BITS: u32 = 8;
// post-PLP-loop
const DYN_RESERVED_3_BITS: u32 = 8;
// AUX dynamic loop
const AUX_PRIVATE_DYN_BITS: u32 = 48;

// ── RF frequency entry ────────────────────────────────────────────────────────

/// One entry in the RF frequency loop (Figure 27, RF loop).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RfFrequency {
    /// RF_IDX — unique index of this frequency (3 bits, 0..NUM_RF−1).
    pub rf_idx: u8,
    /// FREQUENCY — centre frequency in Hz (32 bits; 0 = unknown).
    pub frequency: u32,
}

impl RfFrequency {
    fn parse(r: &mut BitReader<'_>) -> crate::error::Result<Self> {
        let rf_idx = r.read_bits(RF_IDX_BITS)? as u8;
        let frequency = r.read_bits(FREQUENCY_BITS)? as u32;
        Ok(Self { rf_idx, frequency })
    }

    fn write(&self, w: &mut BitWriter<'_>) -> crate::error::Result<()> {
        w.write_bits(u64::from(self.rf_idx), RF_IDX_BITS)?;
        w.write_bits(u64::from(self.frequency), FREQUENCY_BITS)?;
        Ok(())
    }

    const fn bits() -> usize {
        (RF_IDX_BITS + FREQUENCY_BITS) as usize
    }
}

// ── FEF info block ────────────────────────────────────────────────────────────

/// FEF block in the configurable L1-post (Figure 27, FEF conditional).
///
/// Present when `S2 & 1 == 1` (S2 LSB set, i.e. mixed/FEF present).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FefInfo {
    /// FEF_TYPE (4 bits, Table 29 — currently all reserved).
    pub fef_type: u8,
    /// FEF_LENGTH — FEF length in elementary periods T (22 bits).
    pub fef_length: u32,
    /// FEF_INTERVAL — T2-frames between two FEF parts (8 bits).
    pub fef_interval: u8,
}

impl FefInfo {
    fn parse(r: &mut BitReader<'_>) -> crate::error::Result<Self> {
        let fef_type = r.read_bits(FEF_TYPE_BITS)? as u8;
        let fef_length = r.read_bits(FEF_LENGTH_BITS)? as u32;
        let fef_interval = r.read_bits(FEF_INTERVAL_BITS)? as u8;
        Ok(Self {
            fef_type,
            fef_length,
            fef_interval,
        })
    }

    fn write(&self, w: &mut BitWriter<'_>) -> crate::error::Result<()> {
        w.write_bits(u64::from(self.fef_type), FEF_TYPE_BITS)?;
        w.write_bits(u64::from(self.fef_length), FEF_LENGTH_BITS)?;
        w.write_bits(u64::from(self.fef_interval), FEF_INTERVAL_BITS)?;
        Ok(())
    }

    const fn bits() -> usize {
        (FEF_TYPE_BITS + FEF_LENGTH_BITS + FEF_INTERVAL_BITS) as usize
    }
}

// ── PLP configurable entry ────────────────────────────────────────────────────

/// One PLP entry in the configurable L1-post loop (Figure 27, PLP loop).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PlpConfig {
    /// PLP_ID (8 bits).
    pub plp_id: u8,
    /// PLP_TYPE (3 bits, Table 30).
    pub plp_type: PlpType,
    /// PLP_PAYLOAD_TYPE (5 bits, Table 31).
    pub plp_payload_type: PlpPayloadType,
    /// FF_FLAG (1 bit).
    pub ff_flag: bool,
    /// FIRST_RF_IDX (3 bits).
    pub first_rf_idx: u8,
    /// FIRST_FRAME_IDX (8 bits).
    pub first_frame_idx: u8,
    /// PLP_GROUP_ID (8 bits).
    pub plp_group_id: u8,
    /// PLP_COD raw 3-bit value; decode with [`PlpConfig::code_rate`].
    pub plp_cod: u8,
    /// PLP_MOD (3 bits, Table 33).
    pub plp_mod: PlpModulation,
    /// PLP_ROTATION (1 bit).
    pub plp_rotation: bool,
    /// PLP_FEC_TYPE (2 bits, Table 34).
    pub plp_fec_type: PlpFecType,
    /// PLP_NUM_BLOCKS_MAX (10 bits).
    pub plp_num_blocks_max: u16,
    /// FRAME_INTERVAL (8 bits).
    pub frame_interval: u8,
    /// TIME_IL_LENGTH (8 bits).
    pub time_il_length: u8,
    /// TIME_IL_TYPE (1 bit).
    pub time_il_type: bool,
    /// IN_BAND_A_FLAG (1 bit).
    pub in_band_a_flag: bool,
    /// IN_BAND_B_FLAG (1 bit).
    pub in_band_b_flag: bool,
    /// RESERVED_1 (11 bits).
    pub reserved_1: u16,
    /// PLP_MODE (2 bits, Table 35).
    pub plp_mode: PlpMode,
    /// STATIC_FLAG (1 bit).
    pub static_flag: bool,
    /// STATIC_PADDING_FLAG (1 bit).
    pub static_padding_flag: bool,
}

impl PlpConfig {
    /// Decode the PLP code rate using the T2-base profile column of Table 32.
    ///
    /// The T2-Lite column differs at values 110 and 111; this returns the
    /// T2-base interpretation. Callers working with T2-Lite profiles should
    /// inspect `plp_cod` directly.
    #[must_use]
    pub fn code_rate(&self) -> PlpCodeRate {
        PlpCodeRate::from_u8(self.plp_cod)
    }

    fn parse(r: &mut BitReader<'_>) -> crate::error::Result<Self> {
        let plp_id = r.read_bits(PLP_ID_BITS)? as u8;
        let plp_type = PlpType::from_u8(r.read_bits(PLP_TYPE_BITS)? as u8);
        let plp_payload_type = PlpPayloadType::from_u8(r.read_bits(PLP_PAYLOAD_TYPE_BITS)? as u8);
        let ff_flag = r.read_bool()?;
        let first_rf_idx = r.read_bits(FIRST_RF_IDX_BITS)? as u8;
        let first_frame_idx = r.read_bits(FIRST_FRAME_IDX_BITS)? as u8;
        let plp_group_id = r.read_bits(PLP_GROUP_ID_BITS)? as u8;
        let plp_cod = r.read_bits(PLP_COD_BITS)? as u8;
        let plp_mod = PlpModulation::from_u8(r.read_bits(PLP_MOD_BITS)? as u8);
        let plp_rotation = r.read_bool()?;
        let plp_fec_type = PlpFecType::from_u8(r.read_bits(PLP_FEC_TYPE_BITS)? as u8);
        let plp_num_blocks_max = r.read_bits(PLP_NUM_BLOCKS_MAX_BITS)? as u16;
        let frame_interval = r.read_bits(FRAME_INTERVAL_BITS)? as u8;
        let time_il_length = r.read_bits(TIME_IL_LENGTH_BITS)? as u8;
        let time_il_type = r.read_bool()?;
        let in_band_a_flag = r.read_bool()?;
        let in_band_b_flag = r.read_bool()?;
        let reserved_1 = r.read_bits(PLP_RESERVED_1_BITS)? as u16;
        let plp_mode = PlpMode::from_u8(r.read_bits(PLP_MODE_BITS)? as u8);
        let static_flag = r.read_bool()?;
        let static_padding_flag = r.read_bool()?;
        Ok(Self {
            plp_id,
            plp_type,
            plp_payload_type,
            ff_flag,
            first_rf_idx,
            first_frame_idx,
            plp_group_id,
            plp_cod,
            plp_mod,
            plp_rotation,
            plp_fec_type,
            plp_num_blocks_max,
            frame_interval,
            time_il_length,
            time_il_type,
            in_band_a_flag,
            in_band_b_flag,
            reserved_1,
            plp_mode,
            static_flag,
            static_padding_flag,
        })
    }

    fn write(&self, w: &mut BitWriter<'_>) -> crate::error::Result<()> {
        w.write_bits(u64::from(self.plp_id), PLP_ID_BITS)?;
        w.write_bits(u64::from(self.plp_type.to_u8()), PLP_TYPE_BITS)?;
        w.write_bits(
            u64::from(self.plp_payload_type.to_u8()),
            PLP_PAYLOAD_TYPE_BITS,
        )?;
        w.write_bool(self.ff_flag)?;
        w.write_bits(u64::from(self.first_rf_idx), FIRST_RF_IDX_BITS)?;
        w.write_bits(u64::from(self.first_frame_idx), FIRST_FRAME_IDX_BITS)?;
        w.write_bits(u64::from(self.plp_group_id), PLP_GROUP_ID_BITS)?;
        w.write_bits(u64::from(self.plp_cod), PLP_COD_BITS)?;
        w.write_bits(u64::from(self.plp_mod.to_u8()), PLP_MOD_BITS)?;
        w.write_bool(self.plp_rotation)?;
        w.write_bits(u64::from(self.plp_fec_type.to_u8()), PLP_FEC_TYPE_BITS)?;
        w.write_bits(u64::from(self.plp_num_blocks_max), PLP_NUM_BLOCKS_MAX_BITS)?;
        w.write_bits(u64::from(self.frame_interval), FRAME_INTERVAL_BITS)?;
        w.write_bits(u64::from(self.time_il_length), TIME_IL_LENGTH_BITS)?;
        w.write_bool(self.time_il_type)?;
        w.write_bool(self.in_band_a_flag)?;
        w.write_bool(self.in_band_b_flag)?;
        w.write_bits(u64::from(self.reserved_1), PLP_RESERVED_1_BITS)?;
        w.write_bits(u64::from(self.plp_mode.to_u8()), PLP_MODE_BITS)?;
        w.write_bool(self.static_flag)?;
        w.write_bool(self.static_padding_flag)?;
        Ok(())
    }

    const fn bits() -> usize {
        (PLP_ID_BITS
            + PLP_TYPE_BITS
            + PLP_PAYLOAD_TYPE_BITS
            + FF_FLAG_BITS
            + FIRST_RF_IDX_BITS
            + FIRST_FRAME_IDX_BITS
            + PLP_GROUP_ID_BITS
            + PLP_COD_BITS
            + PLP_MOD_BITS
            + PLP_ROTATION_BITS
            + PLP_FEC_TYPE_BITS
            + PLP_NUM_BLOCKS_MAX_BITS
            + FRAME_INTERVAL_BITS
            + TIME_IL_LENGTH_BITS
            + TIME_IL_TYPE_BITS
            + IN_BAND_A_FLAG_BITS
            + IN_BAND_B_FLAG_BITS
            + PLP_RESERVED_1_BITS
            + PLP_MODE_BITS
            + STATIC_FLAG_BITS
            + STATIC_PADDING_FLAG_BITS) as usize
    }
}

// ── AUX configurable entry ────────────────────────────────────────────────────

/// One entry in the configurable AUX loop (Figure 27, AUX loop).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AuxConfig {
    /// AUX_STREAM_TYPE (4 bits, Table 36).
    pub aux_stream_type: AuxStreamType,
    /// AUX_PRIVATE_CONF (28 bits).
    pub aux_private_conf: u32,
}

impl AuxConfig {
    fn parse(r: &mut BitReader<'_>) -> crate::error::Result<Self> {
        let aux_stream_type = AuxStreamType::from_u8(r.read_bits(AUX_STREAM_TYPE_BITS)? as u8);
        let aux_private_conf = r.read_bits(AUX_PRIVATE_CONF_BITS)? as u32;
        Ok(Self {
            aux_stream_type,
            aux_private_conf,
        })
    }

    fn write(&self, w: &mut BitWriter<'_>) -> crate::error::Result<()> {
        w.write_bits(
            u64::from(self.aux_stream_type.to_u8()),
            AUX_STREAM_TYPE_BITS,
        )?;
        w.write_bits(u64::from(self.aux_private_conf), AUX_PRIVATE_CONF_BITS)?;
        Ok(())
    }

    const fn bits() -> usize {
        (AUX_STREAM_TYPE_BITS + AUX_PRIVATE_CONF_BITS) as usize
    }
}

// ── L1PostConfigurable ────────────────────────────────────────────────────────

/// Configurable L1-post signalling — EN 302 755 §7.2.3.1, Figure 27.
///
/// Parsed from the `L1CONF` section of a T2-MI L1-current payload.
/// The caller provides `num_rf` (from L1-pre) and `fef_present` (S2 LSB).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct L1PostConfigurable {
    /// SUB_SLICES_PER_FRAME (15 bits).
    pub sub_slices_per_frame: u16,
    /// NUM_PLP (8 bits) — read from the stream; also determines `plps.len()`.
    pub num_plp: u8,
    /// NUM_AUX (4 bits) — read from the stream; also determines `aux.len()`.
    pub num_aux: u8,
    /// AUX_CONFIG_RFU (8 bits).
    pub aux_config_rfu: u8,
    /// RF frequency loop (length = `num_rf` argument).
    pub rf: Vec<RfFrequency>,
    /// FEF block — present when `fef_present` was `true`.
    pub fef: Option<FefInfo>,
    /// PLP loop (length = `num_plp`).
    pub plps: Vec<PlpConfig>,
    /// FEF_LENGTH_MSB (2 bits, post-PLP-loop).
    pub fef_length_msb: u8,
    /// RESERVED_2 (30 bits, post-PLP-loop).
    pub reserved_2: u32,
    /// AUX loop (length = `num_aux`).
    pub aux: Vec<AuxConfig>,
}

impl L1PostConfigurable {
    /// Parse from `bytes`, using `num_rf` from L1-pre and `fef_present` from S2 LSB.
    ///
    /// # Errors
    /// [`crate::Error::L1Bits`] on bit-stream overrun.
    pub fn parse(bytes: &[u8], num_rf: u8, fef_present: bool) -> crate::error::Result<Self> {
        let mut r = BitReader::new(bytes);

        let sub_slices_per_frame = r.read_bits(SUB_SLICES_PER_FRAME_BITS)? as u16;
        let num_plp = r.read_bits(NUM_PLP_BITS)? as u8;
        let num_aux = r.read_bits(NUM_AUX_BITS)? as u8;
        let aux_config_rfu = r.read_bits(AUX_CONFIG_RFU_BITS)? as u8;

        let mut rf = Vec::with_capacity(num_rf as usize);
        for _ in 0..num_rf {
            rf.push(RfFrequency::parse(&mut r)?);
        }

        let fef = if fef_present {
            Some(FefInfo::parse(&mut r)?)
        } else {
            None
        };

        let mut plps = Vec::with_capacity(num_plp as usize);
        for _ in 0..num_plp {
            plps.push(PlpConfig::parse(&mut r)?);
        }

        let fef_length_msb = r.read_bits(FEF_LENGTH_MSB_BITS)? as u8;
        let reserved_2 = r.read_bits(CONF_RESERVED_2_BITS)? as u32;

        let mut aux = Vec::with_capacity(num_aux as usize);
        for _ in 0..num_aux {
            aux.push(AuxConfig::parse(&mut r)?);
        }

        Ok(Self {
            sub_slices_per_frame,
            num_plp,
            num_aux,
            aux_config_rfu,
            rf,
            fef,
            plps,
            fef_length_msb,
            reserved_2,
            aux,
        })
    }

    /// Compute the total bit-length of this configurable block (without byte padding).
    #[must_use]
    pub fn len_bits(&self) -> usize {
        let header = (SUB_SLICES_PER_FRAME_BITS + NUM_PLP_BITS + NUM_AUX_BITS + AUX_CONFIG_RFU_BITS)
            as usize;
        let rf_bits = self.rf.len() * RfFrequency::bits();
        let fef_bits = if self.fef.is_some() {
            FefInfo::bits()
        } else {
            0
        };
        let plp_bits = self.plps.len() * PlpConfig::bits();
        let post_loop = (FEF_LENGTH_MSB_BITS + CONF_RESERVED_2_BITS) as usize;
        let aux_bits = self.aux.len() * AuxConfig::bits();
        header + rf_bits + fef_bits + plp_bits + post_loop + aux_bits
    }

    /// Serialise into a pre-allocated bit-writer.
    ///
    /// # Errors
    /// [`crate::Error::L1Bits`] if the writer's buffer is too small.
    pub fn serialize_bits(&self, w: &mut BitWriter<'_>) -> crate::error::Result<()> {
        w.write_bits(
            u64::from(self.sub_slices_per_frame),
            SUB_SLICES_PER_FRAME_BITS,
        )?;
        w.write_bits(u64::from(self.num_plp), NUM_PLP_BITS)?;
        w.write_bits(u64::from(self.num_aux), NUM_AUX_BITS)?;
        w.write_bits(u64::from(self.aux_config_rfu), AUX_CONFIG_RFU_BITS)?;
        for rf in &self.rf {
            rf.write(w)?;
        }
        if let Some(fef) = &self.fef {
            fef.write(w)?;
        }
        for plp in &self.plps {
            plp.write(w)?;
        }
        w.write_bits(u64::from(self.fef_length_msb), FEF_LENGTH_MSB_BITS)?;
        w.write_bits(u64::from(self.reserved_2), CONF_RESERVED_2_BITS)?;
        for aux in &self.aux {
            aux.write(w)?;
        }
        Ok(())
    }
}

// ── PLP dynamic entry ─────────────────────────────────────────────────────────

/// One PLP entry in the dynamic L1-post loop (Figure 28, PLP loop).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PlpDynamic {
    /// PLP_ID (8 bits).
    pub plp_id: u8,
    /// PLP_START — start position (cell addressing, 22 bits).
    pub plp_start: u32,
    /// PLP_NUM_BLOCKS (10 bits).
    pub plp_num_blocks: u16,
    /// RESERVED_2 (8 bits).
    pub reserved_2: u8,
}

impl PlpDynamic {
    fn parse(r: &mut BitReader<'_>) -> crate::error::Result<Self> {
        let plp_id = r.read_bits(DYN_PLP_ID_BITS)? as u8;
        let plp_start = r.read_bits(PLP_START_BITS)? as u32;
        let plp_num_blocks = r.read_bits(PLP_NUM_BLOCKS_BITS)? as u16;
        let reserved_2 = r.read_bits(DYN_PLP_RESERVED_2_BITS)? as u8;
        Ok(Self {
            plp_id,
            plp_start,
            plp_num_blocks,
            reserved_2,
        })
    }

    fn write(&self, w: &mut BitWriter<'_>) -> crate::error::Result<()> {
        w.write_bits(u64::from(self.plp_id), DYN_PLP_ID_BITS)?;
        w.write_bits(u64::from(self.plp_start), PLP_START_BITS)?;
        w.write_bits(u64::from(self.plp_num_blocks), PLP_NUM_BLOCKS_BITS)?;
        w.write_bits(u64::from(self.reserved_2), DYN_PLP_RESERVED_2_BITS)?;
        Ok(())
    }

    const fn bits() -> usize {
        (DYN_PLP_ID_BITS + PLP_START_BITS + PLP_NUM_BLOCKS_BITS + DYN_PLP_RESERVED_2_BITS) as usize
    }
}

// ── AUX dynamic entry ─────────────────────────────────────────────────────────

/// One entry in the dynamic AUX loop (Figure 28, AUX loop).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AuxDynamic {
    /// AUX_PRIVATE_DYN (48 bits).
    pub aux_private_dyn: u64,
}

impl AuxDynamic {
    fn parse(r: &mut BitReader<'_>) -> crate::error::Result<Self> {
        let aux_private_dyn = r.read_bits(AUX_PRIVATE_DYN_BITS)?;
        Ok(Self { aux_private_dyn })
    }

    fn write(&self, w: &mut BitWriter<'_>) -> crate::error::Result<()> {
        w.write_bits(self.aux_private_dyn, AUX_PRIVATE_DYN_BITS)?;
        Ok(())
    }

    const fn bits() -> usize {
        AUX_PRIVATE_DYN_BITS as usize
    }
}

// ── L1PostDynamic ─────────────────────────────────────────────────────────────

/// Dynamic L1-post signalling — EN 302 755 §7.2.3.2, Figure 28.
///
/// Parse with `L1PostDynamic::parse(bytes, num_plp, num_aux)`.
/// `num_plp` and `num_aux` come from the configurable block's `num_plp` /
/// `num_aux` fields.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct L1PostDynamic {
    /// FRAME_IDX (8 bits).
    pub frame_idx: u8,
    /// SUB_SLICE_INTERVAL (22 bits; 0 if no type-2 PLPs).
    pub sub_slice_interval: u32,
    /// TYPE_2_START (22 bits; 0 if no type-2 PLPs).
    pub type_2_start: u32,
    /// L1_CHANGE_COUNTER (8 bits; 0 = no scheduled change).
    pub l1_change_counter: u8,
    /// START_RF_IDX (3 bits; 0 if TFS not used).
    pub start_rf_idx: u8,
    /// RESERVED_1 (8 bits).
    pub reserved_1: u8,
    /// PLP dynamic loop (length = `num_plp`).
    pub plps: Vec<PlpDynamic>,
    /// RESERVED_3 (8 bits, post-PLP-loop).
    pub reserved_3: u8,
    /// AUX dynamic loop (length = `num_aux`).
    pub aux: Vec<AuxDynamic>,
}

impl L1PostDynamic {
    /// Parse from `bytes` with `num_plp` and `num_aux` context from the
    /// configurable block.
    ///
    /// # Errors
    /// [`crate::Error::L1Bits`] on bit-stream overrun.
    pub fn parse(bytes: &[u8], num_plp: u8, num_aux: u8) -> crate::error::Result<Self> {
        let mut r = BitReader::new(bytes);

        let frame_idx = r.read_bits(FRAME_IDX_BITS)? as u8;
        let sub_slice_interval = r.read_bits(SUB_SLICE_INTERVAL_BITS)? as u32;
        let type_2_start = r.read_bits(TYPE_2_START_BITS)? as u32;
        let l1_change_counter = r.read_bits(L1_CHANGE_COUNTER_BITS)? as u8;
        let start_rf_idx = r.read_bits(START_RF_IDX_BITS)? as u8;
        let reserved_1 = r.read_bits(DYN_RESERVED_1_BITS)? as u8;

        let mut plps = Vec::with_capacity(num_plp as usize);
        for _ in 0..num_plp {
            plps.push(PlpDynamic::parse(&mut r)?);
        }

        let reserved_3 = r.read_bits(DYN_RESERVED_3_BITS)? as u8;

        let mut aux = Vec::with_capacity(num_aux as usize);
        for _ in 0..num_aux {
            aux.push(AuxDynamic::parse(&mut r)?);
        }

        Ok(Self {
            frame_idx,
            sub_slice_interval,
            type_2_start,
            l1_change_counter,
            start_rf_idx,
            reserved_1,
            plps,
            reserved_3,
            aux,
        })
    }

    /// Compute total bit-length (without byte padding).
    #[must_use]
    pub fn len_bits(&self) -> usize {
        let header = (FRAME_IDX_BITS
            + SUB_SLICE_INTERVAL_BITS
            + TYPE_2_START_BITS
            + L1_CHANGE_COUNTER_BITS
            + START_RF_IDX_BITS
            + DYN_RESERVED_1_BITS) as usize;
        let plp_bits = self.plps.len() * PlpDynamic::bits();
        let post = DYN_RESERVED_3_BITS as usize;
        let aux_bits = self.aux.len() * AuxDynamic::bits();
        header + plp_bits + post + aux_bits
    }

    /// Serialise into a pre-allocated bit-writer.
    ///
    /// # Errors
    /// [`crate::Error::L1Bits`] if the writer's buffer is too small.
    pub fn serialize_bits(&self, w: &mut BitWriter<'_>) -> crate::error::Result<()> {
        w.write_bits(u64::from(self.frame_idx), FRAME_IDX_BITS)?;
        w.write_bits(u64::from(self.sub_slice_interval), SUB_SLICE_INTERVAL_BITS)?;
        w.write_bits(u64::from(self.type_2_start), TYPE_2_START_BITS)?;
        w.write_bits(u64::from(self.l1_change_counter), L1_CHANGE_COUNTER_BITS)?;
        w.write_bits(u64::from(self.start_rf_idx), START_RF_IDX_BITS)?;
        w.write_bits(u64::from(self.reserved_1), DYN_RESERVED_1_BITS)?;
        for plp in &self.plps {
            plp.write(w)?;
        }
        w.write_bits(u64::from(self.reserved_3), DYN_RESERVED_3_BITS)?;
        for aux in &self.aux {
            aux.write(w)?;
        }
        Ok(())
    }
}

// ── L1ExtBlock ────────────────────────────────────────────────────────────────

const L1_EXT_BLOCK_TYPE_BITS: u32 = 8;
const L1_EXT_DATA_LEN_BITS: u32 = 16;

/// One extension block — EN 302 755 §7.2.3.4, Table 37.
///
/// Within the L1EXT region the blocks are bit-contiguous (a block's data is
/// `data_bit_len` bits, not byte-padded); only the whole region is zero-padded
/// to a byte boundary. `data` holds `ceil(data_bit_len/8)` bytes, MSB-first,
/// with the final byte zero-padded in its low bits.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct L1ExtBlock {
    /// L1_EXT_BLOCK_TYPE (8 bits, Table 38).
    pub block_type: u8,
    /// L1_EXT_DATA_LEN — length of `data` in bits (16-bit wire field).
    pub data_bit_len: u16,
    /// L1_EXT_BLOCK_DATA — `ceil(data_bit_len/8)` bytes, MSB-first.
    pub data: Vec<u8>,
}

impl L1ExtBlock {
    /// Bits this block occupies in the L1EXT region (header + data, no padding).
    fn bits(&self) -> usize {
        (L1_EXT_BLOCK_TYPE_BITS + L1_EXT_DATA_LEN_BITS) as usize + self.data_bit_len as usize
    }

    fn parse(r: &mut BitReader<'_>) -> crate::error::Result<Self> {
        let block_type = r.read_bits(L1_EXT_BLOCK_TYPE_BITS)? as u8;
        let data_bit_len = r.read_bits(L1_EXT_DATA_LEN_BITS)? as u16;
        let n = data_bit_len as usize;
        let mut data = vec![0u8; n.div_ceil(8)];
        let mut done = 0usize;
        for byte in data.iter_mut() {
            let take = (n - done).min(8) as u32;
            let v = r.read_bits(take)? as u8;
            // MSB-first: a short final field sits in the high bits.
            *byte = if take < 8 { v << (8 - take) } else { v };
            done += take as usize;
        }
        Ok(Self {
            block_type,
            data_bit_len,
            data,
        })
    }

    fn write(&self, w: &mut BitWriter<'_>) -> crate::error::Result<()> {
        w.write_bits(u64::from(self.block_type), L1_EXT_BLOCK_TYPE_BITS)?;
        w.write_bits(u64::from(self.data_bit_len), L1_EXT_DATA_LEN_BITS)?;
        let n = self.data_bit_len as usize;
        let mut done = 0usize;
        for &byte in &self.data {
            let take = (n - done).min(8) as u32;
            let v = if take < 8 {
                u64::from(byte >> (8 - take))
            } else {
                u64::from(byte)
            };
            w.write_bits(v, take)?;
            done += take as usize;
        }
        Ok(())
    }
}

/// Parse the extension blocks filling the first `total_bits` of `bytes`.
///
/// Blocks are bit-contiguous; the loop stops when fewer than a block header
/// (24 bits) of declared content remains (the trailing zero-pad is < 8 bits).
fn parse_ext_blocks(bytes: &[u8], total_bits: usize) -> crate::error::Result<Vec<L1ExtBlock>> {
    let mut r = BitReader::new(bytes);
    let header_bits = (L1_EXT_BLOCK_TYPE_BITS + L1_EXT_DATA_LEN_BITS) as usize;
    let mut blocks = Vec::new();
    while r.bits_read() + header_bits <= total_bits {
        blocks.push(L1ExtBlock::parse(&mut r)?);
    }
    Ok(blocks)
}

// ── L1Post ────────────────────────────────────────────────────────────────────

/// Parsed L1-post signalling block.
///
/// Produced by [`crate::payload::l1_current::L1CurrentPayload::l1_post`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct L1Post {
    /// Configurable part (Figure 27).
    pub configurable: L1PostConfigurable,
    /// Dynamic part for the current frame (Figure 28).
    pub dynamic_current: L1PostDynamic,
    /// Dynamic part for the next frame — present when L1-pre `L1_REPETITION_FLAG = 1`
    /// (§7.2.3.3). `None` for L1-current payloads (the framing carries only the
    /// current-frame dynamic block).
    pub dynamic_next: Option<L1PostDynamic>,
    /// Extension blocks (Table 37). Empty when `L1_POST_EXTENSION = 0`.
    pub extension: Vec<L1ExtBlock>,
}

impl L1Post {
    /// Serialise back to the framed T2-MI L1-current data region — the bytes
    /// that follow the 21-byte L1PRE inside `L1CurrentPayload::l1_current_data`
    /// (TS 102 773 Table 2). Symmetric with the parse performed by
    /// [`crate::payload::L1CurrentPayload::l1_post`].
    ///
    /// # Errors
    /// [`crate::Error::L1Bits`] on an internal bit-buffer overrun (cannot occur
    /// for a value obtained by parsing valid wire data).
    pub fn to_l1_current_framed(&self) -> crate::error::Result<Vec<u8>> {
        serialize_l1_post_framed(self)
    }
}

/// Parse the framed T2-MI L1-current data region (everything after the 21-byte L1PRE).
///
/// Layout per TS 102 773 Table 2:
/// ```text
/// L1CONF_LEN (16 bits)
/// L1CONF     (ceil(L1CONF_LEN/8) bytes, zero-padded)
/// L1DYN_CURR_LEN (16 bits)
/// L1DYN_CURR     (ceil(L1DYN_CURR_LEN/8) bytes, zero-padded)
/// L1EXT_LEN  (16 bits)
/// L1EXT      (ceil(L1EXT_LEN/8) bytes, zero-padded)
/// ```
///
/// `num_rf` and `fef_present` come from the L1-pre block.
pub(crate) fn parse_l1_post_from_framed(
    framed: &[u8],
    num_rf: u8,
    fef_present: bool,
) -> crate::error::Result<L1Post> {
    let mut pos = 0usize;

    // ── L1CONF ────────────────────────────────────────────────────────────────
    if framed.len() < pos + 2 {
        return Err(crate::Error::BufferTooShort {
            need: pos + 2,
            have: framed.len(),
            what: "L1CONF_LEN",
        });
    }
    let l1conf_len_bits = u16::from_be_bytes([framed[pos], framed[pos + 1]]) as usize;
    pos += 2;
    let l1conf_bytes = l1conf_len_bits.div_ceil(8);
    if framed.len() < pos + l1conf_bytes {
        return Err(crate::Error::BufferTooShort {
            need: pos + l1conf_bytes,
            have: framed.len(),
            what: "L1CONF",
        });
    }
    let configurable =
        L1PostConfigurable::parse(&framed[pos..pos + l1conf_bytes], num_rf, fef_present)?;
    pos += l1conf_bytes;

    let num_plp = configurable.num_plp;
    let num_aux = configurable.num_aux;

    // ── L1DYN_CURR ────────────────────────────────────────────────────────────
    if framed.len() < pos + 2 {
        return Err(crate::Error::BufferTooShort {
            need: pos + 2,
            have: framed.len(),
            what: "L1DYN_CURR_LEN",
        });
    }
    let l1dyn_len_bits = u16::from_be_bytes([framed[pos], framed[pos + 1]]) as usize;
    pos += 2;
    let l1dyn_bytes = l1dyn_len_bits.div_ceil(8);
    if framed.len() < pos + l1dyn_bytes {
        return Err(crate::Error::BufferTooShort {
            need: pos + l1dyn_bytes,
            have: framed.len(),
            what: "L1DYN_CURR",
        });
    }
    let dynamic_current = L1PostDynamic::parse(&framed[pos..pos + l1dyn_bytes], num_plp, num_aux)?;
    pos += l1dyn_bytes;

    // ── L1EXT ─────────────────────────────────────────────────────────────────
    if framed.len() < pos + 2 {
        return Err(crate::Error::BufferTooShort {
            need: pos + 2,
            have: framed.len(),
            what: "L1EXT_LEN",
        });
    }
    let l1ext_len_bits = u16::from_be_bytes([framed[pos], framed[pos + 1]]) as usize;
    pos += 2;
    let l1ext_bytes = l1ext_len_bits.div_ceil(8);
    if framed.len() < pos + l1ext_bytes {
        return Err(crate::Error::BufferTooShort {
            need: pos + l1ext_bytes,
            have: framed.len(),
            what: "L1EXT",
        });
    }
    let extension = if l1ext_bytes > 0 {
        parse_ext_blocks(&framed[pos..pos + l1ext_bytes], l1ext_len_bits)?
    } else {
        Vec::new()
    };

    Ok(L1Post {
        configurable,
        dynamic_current,
        dynamic_next: None, // L1-current carries only the current-frame dynamic
        extension,
    })
}

/// Append one framed block: a 16-bit bit-length, then the bit-packed content
/// zero-padded up to a byte boundary.
fn push_framed_block<F>(out: &mut Vec<u8>, bit_len: usize, write: F) -> crate::error::Result<()>
where
    F: FnOnce(&mut BitWriter<'_>) -> crate::error::Result<()>,
{
    out.extend_from_slice(&(bit_len as u16).to_be_bytes());
    let start = out.len();
    out.resize(start + bit_len.div_ceil(8), 0);
    let mut w = BitWriter::new(&mut out[start..]);
    write(&mut w)?;
    Ok(())
}

/// Serialise an [`L1Post`] back to the framed T2-MI L1-current data region
/// (everything after the 21-byte L1PRE): `L1CONF_LEN`+`L1CONF`,
/// `L1DYN_CURR_LEN`+`L1DYN_CURR`, `L1EXT_LEN`+`L1EXT` (TS 102 773 Table 2).
/// Each block is zero-padded to a byte boundary. `dynamic_next` is not part of
/// the L1-current framing (it is carried in L1-future) and is ignored here.
pub(crate) fn serialize_l1_post_framed(post: &L1Post) -> crate::error::Result<Vec<u8>> {
    let mut out = Vec::new();
    push_framed_block(&mut out, post.configurable.len_bits(), |w| {
        post.configurable.serialize_bits(w)
    })?;
    push_framed_block(&mut out, post.dynamic_current.len_bits(), |w| {
        post.dynamic_current.serialize_bits(w)
    })?;
    let ext_bits: usize = post.extension.iter().map(L1ExtBlock::bits).sum();
    push_framed_block(&mut out, ext_bits, |w| {
        for block in &post.extension {
            block.write(w)?;
        }
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_conf(
        num_rf: u8,
        fef_present: bool,
        num_plp: u8,
        num_aux: u8,
    ) -> L1PostConfigurable {
        let rf = (0..num_rf)
            .map(|i| RfFrequency {
                rf_idx: i,
                frequency: 666_000_000 + u32::from(i) * 8_000_000,
            })
            .collect();
        let fef = if fef_present {
            Some(FefInfo {
                fef_type: 0,
                fef_length: 100,
                fef_interval: 4,
            })
        } else {
            None
        };
        let plps = (0..num_plp)
            .map(|i| PlpConfig {
                plp_id: i,
                plp_type: PlpType::Common,
                plp_payload_type: PlpPayloadType::Ts,
                ff_flag: false,
                first_rf_idx: 0,
                first_frame_idx: 0,
                plp_group_id: 0,
                plp_cod: 0,
                plp_mod: PlpModulation::Qam16,
                plp_rotation: false,
                plp_fec_type: PlpFecType::Ldpc16K,
                plp_num_blocks_max: 10,
                frame_interval: 1,
                time_il_length: 0,
                time_il_type: false,
                in_band_a_flag: false,
                in_band_b_flag: false,
                reserved_1: 0,
                plp_mode: PlpMode::Normal,
                static_flag: false,
                static_padding_flag: false,
            })
            .collect();
        let aux = (0..num_aux)
            .map(|_| AuxConfig {
                aux_stream_type: AuxStreamType::TxSig,
                aux_private_conf: 0,
            })
            .collect();
        L1PostConfigurable {
            sub_slices_per_frame: 512,
            num_plp,
            num_aux,
            aux_config_rfu: 0,
            rf,
            fef,
            plps,
            fef_length_msb: 0,
            reserved_2: 0,
            aux,
        }
    }

    fn synthetic_dyn(num_plp: u8, num_aux: u8) -> L1PostDynamic {
        let plps = (0..num_plp)
            .map(|i| PlpDynamic {
                plp_id: i,
                plp_start: 0x1234,
                plp_num_blocks: 5,
                reserved_2: 0,
            })
            .collect();
        let aux = (0..num_aux)
            .map(|_| AuxDynamic { aux_private_dyn: 0 })
            .collect();
        L1PostDynamic {
            frame_idx: 0,
            sub_slice_interval: 0,
            type_2_start: 0,
            l1_change_counter: 0,
            start_rf_idx: 0,
            reserved_1: 0,
            plps,
            reserved_3: 0,
            aux,
        }
    }

    #[test]
    fn configurable_round_trip() {
        let conf = synthetic_conf(1, false, 1, 0);
        let bits = conf.len_bits();
        let bytes = bits.div_ceil(8);
        let mut buf = vec![0u8; bytes];
        let mut w = BitWriter::new(&mut buf);
        conf.serialize_bits(&mut w).unwrap();
        let parsed = L1PostConfigurable::parse(&buf, 1, false).unwrap();
        assert_eq!(conf, parsed);
    }

    #[test]
    fn configurable_round_trip_with_fef() {
        let conf = synthetic_conf(1, true, 1, 0);
        let bits = conf.len_bits();
        let bytes = bits.div_ceil(8);
        let mut buf = vec![0u8; bytes];
        let mut w = BitWriter::new(&mut buf);
        conf.serialize_bits(&mut w).unwrap();
        let parsed = L1PostConfigurable::parse(&buf, 1, true).unwrap();
        assert_eq!(conf, parsed);
    }

    #[test]
    fn dynamic_round_trip() {
        let dyn_ = synthetic_dyn(1, 0);
        let bits = dyn_.len_bits();
        let bytes = bits.div_ceil(8);
        let mut buf = vec![0u8; bytes];
        let mut w = BitWriter::new(&mut buf);
        dyn_.serialize_bits(&mut w).unwrap();
        let parsed = L1PostDynamic::parse(&buf, 1, 0).unwrap();
        assert_eq!(dyn_, parsed);
    }

    #[test]
    fn plp_config_bits_constant() {
        // Figure 27 PLP entry: 8+3+5+1+3+8+8+3+3+1+2+10+8+8+1+1+1+11+2+1+1 = 89
        assert_eq!(PlpConfig::bits(), 89);
    }

    #[test]
    fn plp_dynamic_bits_constant() {
        // Figure 28 PLP entry: 8+22+10+8 = 48
        assert_eq!(PlpDynamic::bits(), 48);
    }

    #[test]
    fn plp_code_rate_accessor() {
        let plp = PlpConfig {
            plp_id: 0,
            plp_type: PlpType::Common,
            plp_payload_type: PlpPayloadType::Ts,
            ff_flag: false,
            first_rf_idx: 0,
            first_frame_idx: 0,
            plp_group_id: 0,
            plp_cod: 2, // R2_3
            plp_mod: PlpModulation::Qam64,
            plp_rotation: false,
            plp_fec_type: PlpFecType::Ldpc64K,
            plp_num_blocks_max: 20,
            frame_interval: 1,
            time_il_length: 0,
            time_il_type: false,
            in_band_a_flag: false,
            in_band_b_flag: false,
            reserved_1: 0,
            plp_mode: PlpMode::Normal,
            static_flag: false,
            static_padding_flag: false,
        };
        assert_eq!(plp.code_rate(), PlpCodeRate::R2_3);
    }

    #[test]
    fn configurable_header_bits() {
        // Header: 15+8+4+8 = 35, RF×1: 3+32=35, FEF=0, PLP×1=89, post=2+30=32, AUX×0=0
        // Total = 35+35+89+32 = 191
        let conf = synthetic_conf(1, false, 1, 0);
        assert_eq!(conf.len_bits(), 191);
    }

    #[test]
    fn dynamic_header_bits() {
        // Header: 8+22+22+8+3+8 = 71, PLP×1=48, post=8, AUX×0=0
        // Total = 71+48+8 = 127
        let dyn_ = synthetic_dyn(1, 0);
        assert_eq!(dyn_.len_bits(), 127);
    }

    #[test]
    fn l1post_framed_round_trip() {
        // Full L1-current framing round-trip (no extension blocks).
        let post = L1Post {
            configurable: synthetic_conf(1, false, 1, 0),
            dynamic_current: synthetic_dyn(1, 0),
            dynamic_next: None,
            extension: Vec::new(),
        };
        let framed = post.to_l1_current_framed().unwrap();
        // 2 + ceil(191/8)=24 + 2 + ceil(127/8)=16 + 2 + 0 = 46 bytes
        assert_eq!(framed.len(), 46);
        let parsed = parse_l1_post_from_framed(&framed, 1, false).unwrap();
        assert_eq!(post, parsed);
    }

    #[test]
    fn l1post_framed_round_trip_with_nonaligned_ext() {
        // A 12-bit (non-byte-aligned) extension block exercises the
        // bit-contiguous L1ExtBlock parse/serialize path.
        let post = L1Post {
            configurable: synthetic_conf(2, true, 1, 1),
            dynamic_current: synthetic_dyn(1, 1),
            dynamic_next: None,
            extension: vec![L1ExtBlock {
                block_type: 0xFF,
                data_bit_len: 12,
                data: vec![0xAB, 0xC0], // 12 bits: 0xAB then high nibble 0xC, low 4 bits zero-pad
            }],
        };
        let framed = post.to_l1_current_framed().unwrap();
        let parsed = parse_l1_post_from_framed(&framed, 2, true).unwrap();
        assert_eq!(post, parsed);
    }
}
