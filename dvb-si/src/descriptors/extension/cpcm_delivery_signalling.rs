//! CPCM Delivery Signalling Descriptor — ETSI TS 102 825-9 §4.1.5, Table 2
//! (tag_extension 0x01); typed CPCM USI decode per ETSI TS 102 825-4 §5.4 Table 8.
use super::*;
use alloc::vec::Vec;

impl<'a> ExtensionBodyDef<'a> for CpcmDeliverySignalling<'a> {
    const TAG_EXTENSION: u8 = 0x01;
    const NAME: &'static str = "CPCM_DELIVERY_SIGNALLING";
}

/// cpcm_delivery_signalling body (Table 2, §4.1.5): an encoding version plus the
/// version-dependent CPCM USI `selector_byte`s. For `cpcm_version == 1` the
/// selector is the CPCM delivery signalling (USI) of ETSI TS 102 825-4; at the
/// descriptor level it is a version-tagged opaque payload.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct CpcmDeliverySignalling<'a> {
    /// `cpcm_version` — encoding version of the USI structure in the selector bytes.
    pub cpcm_version: u8,
    /// The `selector_byte`s (version-dependent CPCM USI payload; see TS 102 825-4).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub selector_bytes: &'a [u8],
}

impl CpcmDeliverySignalling<'_> {
    /// Attempt to decode the selector bytes as a typed [`CpcmUsi`].
    ///
    /// Returns `Some(Ok(_))` when `cpcm_version == 1` and the selector parses
    /// successfully; `Some(Err(_))` when version is 1 but the bytes are malformed;
    /// `None` when `cpcm_version != 1`.
    #[must_use]
    pub fn usi(&self) -> Option<Result<CpcmUsi>> {
        if self.cpcm_version == 1 {
            Some(CpcmUsi::parse(self.selector_bytes))
        } else {
            None
        }
    }
}

impl<'a> Parse<'a> for CpcmDeliverySignalling<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        let (cpcm_version, selector_bytes) = sel.split_first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "cpcm_delivery_signalling body",
        })?;
        Ok(CpcmDeliverySignalling {
            cpcm_version: *cpcm_version,
            selector_bytes,
        })
    }
}

impl Serialize for CpcmDeliverySignalling<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        1 + self.selector_bytes.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = self.cpcm_version;
        buf[1..len].copy_from_slice(self.selector_bytes);
        Ok(len)
    }
}

// ── CpcmUsi ──────────────────────────────────────────────────────────────────

/// `copy_control` (cci_and_zero_retention) — ETSI TS 102 825-4 Table 9.
///
/// Coded as a 3-bit `uimsbf` in byte 1 `[7:5]` of the USI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
#[repr(u8)]
pub enum CopyControl {
    /// 0 — Copy Control Not Asserted.
    CopyControlNotAsserted = 0,
    /// 1 — Copy Once.
    CopyOnce = 1,
    /// 2 — Copy No More.
    CopyNoMore = 2,
    /// 3 — Copy Never — Zero Retention Not Asserted.
    CopyNeverZeroRetentionNotAsserted = 3,
    /// 4 — Copy Never — Zero Retention Asserted.
    CopyNeverZeroRetentionAsserted = 4,
    /// 5–7 — Reserved for future use.
    Reserved(u8) = 5,
}

impl CopyControl {
    /// Construct from the raw 3-bit value; values 5–7 map to `Reserved(v)`.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x07 {
            0 => CopyControl::CopyControlNotAsserted,
            1 => CopyControl::CopyOnce,
            2 => CopyControl::CopyNoMore,
            3 => CopyControl::CopyNeverZeroRetentionNotAsserted,
            4 => CopyControl::CopyNeverZeroRetentionAsserted,
            r => CopyControl::Reserved(r),
        }
    }

    /// Return the raw 3-bit value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            CopyControl::CopyControlNotAsserted => 0,
            CopyControl::CopyOnce => 1,
            CopyControl::CopyNoMore => 2,
            CopyControl::CopyNeverZeroRetentionNotAsserted => 3,
            CopyControl::CopyNeverZeroRetentionAsserted => 4,
            CopyControl::Reserved(r) => r,
        }
    }

    /// Spec label per Table 9.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            CopyControl::CopyControlNotAsserted => "Copy Control Not Asserted",
            CopyControl::CopyOnce => "Copy Once",
            CopyControl::CopyNoMore => "Copy No More",
            CopyControl::CopyNeverZeroRetentionNotAsserted => {
                "Copy Never - Zero Retention Not Asserted"
            }
            CopyControl::CopyNeverZeroRetentionAsserted => "Copy Never - Zero Retention Asserted",
            CopyControl::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(CopyControl, Reserved);

/// `move_and_copy_propagation_information` — ETSI TS 102 825-4 Table 10.
///
/// Coded as a 2-bit `uimsbf` in byte 2 `[5:4]` of the USI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
#[repr(u8)]
pub enum MoveCopyPropagation {
    /// 0 — MLAD: copying/movement within the same Localized AD is allowed.
    Mlad = 0,
    /// 1 — MGAD: copying/movement within the same Geographically-constrained AD is allowed.
    Mgad = 1,
    /// 2 — MAD: copying/movement within the same Authorized Domain is allowed.
    Mad = 2,
    /// 3 — MCPCM: copying/movement to any CPCM-compliant Storage Entity is allowed.
    Mcpcm = 3,
}

impl MoveCopyPropagation {
    /// Construct from the raw 2-bit value; the field is total (values 0–3 exhaustive).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0 => MoveCopyPropagation::Mlad,
            1 => MoveCopyPropagation::Mgad,
            2 => MoveCopyPropagation::Mad,
            _ => MoveCopyPropagation::Mcpcm,
        }
    }

    /// Return the raw 2-bit value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Spec label per Table 10.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            MoveCopyPropagation::Mlad => "MLAD",
            MoveCopyPropagation::Mgad => "MGAD",
            MoveCopyPropagation::Mad => "MAD",
            MoveCopyPropagation::Mcpcm => "MCPCM",
        }
    }
}
broadcast_common::impl_spec_display!(MoveCopyPropagation);

/// `view_propagation_information` — ETSI TS 102 825-4 Table 11.
///
/// Coded as a 2-bit `uimsbf` in byte 2 `[3:2]` of the USI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
#[repr(u8)]
pub enum ViewPropagation {
    /// 0 — VLAD: consumption within the same Localized AD is allowed.
    Vlad = 0,
    /// 1 — VGAD: consumption within the same Geographically-constrained AD is allowed.
    Vgad = 1,
    /// 2 — VAD: consumption within the same Authorized Domain is allowed.
    Vad = 2,
    /// 3 — VCPCM: consumption using any CPCM-compliant Consumption Point is allowed.
    Vcpcm = 3,
}

impl ViewPropagation {
    /// Construct from the raw 2-bit value; the field is total (values 0–3 exhaustive).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0 => ViewPropagation::Vlad,
            1 => ViewPropagation::Vgad,
            2 => ViewPropagation::Vad,
            _ => ViewPropagation::Vcpcm,
        }
    }

    /// Return the raw 2-bit value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Spec label per Table 11.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            ViewPropagation::Vlad => "VLAD",
            ViewPropagation::Vgad => "VGAD",
            ViewPropagation::Vad => "VAD",
            ViewPropagation::Vcpcm => "VCPCM",
        }
    }
}
broadcast_common::impl_spec_display!(ViewPropagation);

/// One entry in the `export_controlled_cps` CPS vector (Table 8, §5.4).
///
/// Layout per entry: `C_and_R_regime_mask`(8) || `cps_vector_length`(16) ||
/// `cps_vector_byte`\[cps_vector_length\].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CpsVectorEntry {
    /// `C_and_R_regime_mask` — identifies which C&R regimes this vector applies to.
    pub c_and_r_regime_mask: u8,
    /// `cps_vector_byte` payload — length encoded as `cps_vector_length`(16).
    pub cps_vector: Vec<u8>,
}

// Fixed-byte widths within USI (ETSI TS 102 825-4 Table 8).
/// Byte width of the 3 flag bytes (bytes 1–3 after `length`).
const USI_FLAGS_LEN: usize = 3;
/// Byte width of `CPCM_date_time` (40-bit opaque timestamp, Table 8).
const CPCM_DATE_TIME_LEN: usize = 5;
/// Byte width of `CPCM_playback_period` (16-bit opaque value, Table 8).
const CPCM_PLAYBACK_PERIOD_LEN: usize = 2;
/// Byte width of `simultaneous_view_count` (8-bit uimsbf).
const SIMULTANEOUS_VIEW_COUNT_LEN: usize = 1;
/// Byte width of the per-CPS-entry header before the variable payload
/// (`C_and_R_regime_mask`(1) + `cps_vector_length`(2)).
const CPS_ENTRY_HDR_LEN: usize = 3;

/// Typed decode of `CPCM_usage_state_information()` — ETSI TS 102 825-4 §5.4 Table 8.
///
/// This struct mirrors the wire layout exactly:
///
/// ```text
/// byte 0     length (bytes following this field)
/// byte 1     copy_control[7:5] | do_not_cpcm_scramble[4] | viewable[3]
///            | view_window_activated[2] | view_period_activated[1]
///            | simultaneous_view_count_activated[0]
/// byte 2     move_local[7] | view_local[6]
///            | move_and_copy_propagation_information[5:4]
///            | view_propagation_information[3:2]
///            | remote_access_date_moving_window_flag[1]
///            | remote_access_date_immediate_flag[0]
/// byte 3     remote_access_record_flag[7] | export_controlled_cps[6]
///            | export_beyond_trust[5] | disable_analogue_sd_export[4]
///            | disable_analogue_sd_consumption[3] | disable_analogue_hd_export[2]
///            | disable_analogue_hd_consumption[1] | image_constraint[0]
/// then conditional fields in declaration order (§5.4)
/// ```
///
/// Obtain via [`CpcmDeliverySignalling::usi()`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CpcmUsi {
    // ── byte 1 ───────────────────────────────────────────────────────────────
    /// `copy_control`(3) — cci_and_zero_retention per Table 9.
    pub copy_control: CopyControl,
    /// `do_not_cpcm_scramble`(1) — 1 = do not apply CPCM scrambling.
    pub do_not_cpcm_scramble: bool,
    /// `viewable`(1) — 1 = consumption and export enabled.
    pub viewable: bool,
    /// `view_window_activated`(1) — gates `view_window_start`/`view_window_end`.
    pub view_window_activated: bool,
    /// `view_period_activated`(1) — gates `view_period_from_first_playback`.
    pub view_period_activated: bool,
    /// `simultaneous_view_count_activated`(1) — gates `simultaneous_view_count`.
    pub simultaneous_view_count_activated: bool,
    // ── byte 2 ───────────────────────────────────────────────────────────────
    /// `move_local`(1) — copying/movement allowed if destination is local.
    pub move_local: bool,
    /// `view_local`(1) — consumption allowed if consumption point is local.
    pub view_local: bool,
    /// `move_and_copy_propagation_information`(2) — per Table 10.
    pub move_and_copy_propagation_information: MoveCopyPropagation,
    /// `view_propagation_information`(2) — per Table 11.
    pub view_propagation_information: ViewPropagation,
    /// `remote_access_date_moving_window_flag`(1) — gates `remote_access_date`.
    pub remote_access_date_moving_window_flag: bool,
    /// `remote_access_date_immediate_flag`(1) — gates `remote_access_date`.
    pub remote_access_date_immediate_flag: bool,
    // ── byte 3 ───────────────────────────────────────────────────────────────
    /// `remote_access_record_flag`(1).
    pub remote_access_record_flag: bool,
    /// `export_controlled_cps`(1) — gates the CPS vector.
    pub export_controlled_cps: bool,
    /// `export_beyond_trust`(1) — content may be exported to an untrusted space.
    pub export_beyond_trust: bool,
    /// `disable_analogue_sd_export`(1).
    pub disable_analogue_sd_export: bool,
    /// `disable_analogue_sd_consumption`(1).
    pub disable_analogue_sd_consumption: bool,
    /// `disable_analogue_hd_export`(1).
    pub disable_analogue_hd_export: bool,
    /// `disable_analogue_hd_consumption`(1).
    pub disable_analogue_hd_consumption: bool,
    /// `image_constraint`(1) — HD images shall be rendered at lower resolutions.
    pub image_constraint: bool,
    // ── conditional fields (in declaration order, §5.4) ──────────────────────
    /// `view_window_start` (40-bit `CPCM_date_time`) — present iff `view_window_activated`.
    pub view_window_start: Option<[u8; CPCM_DATE_TIME_LEN]>,
    /// `view_window_end` (40-bit `CPCM_date_time`) — present iff `view_window_activated`.
    pub view_window_end: Option<[u8; CPCM_DATE_TIME_LEN]>,
    /// `view_period_from_first_playback` (16-bit `CPCM_playback_period`) — present iff
    /// `view_period_activated`.
    pub view_period_from_first_playback: Option<[u8; CPCM_PLAYBACK_PERIOD_LEN]>,
    /// `simultaneous_view_count` (8-bit uimsbf) — present iff
    /// `simultaneous_view_count_activated`.
    pub simultaneous_view_count: Option<u8>,
    /// `remote_access_date` (40-bit `CPCM_date_time`) — present iff
    /// `remote_access_date_immediate_flag || remote_access_date_moving_window_flag`.
    pub remote_access_date: Option<[u8; CPCM_DATE_TIME_LEN]>,
    /// CPS vector entries — present (non-empty) iff `export_controlled_cps`.
    pub cps_vectors: Vec<CpsVectorEntry>,
}

impl<'a> Parse<'a> for CpcmUsi {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // byte 0: length (counts bytes after itself)
        if bytes.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "CpcmUsi length",
            });
        }
        let length = bytes[0] as usize;
        // Total bytes available: 1 (length byte) + length.
        // We need at least 1 + USI_FLAGS_LEN (3) = 4 bytes.
        if bytes.len() < 1 + USI_FLAGS_LEN {
            return Err(Error::BufferTooShort {
                need: 1 + USI_FLAGS_LEN,
                have: bytes.len(),
                what: "CpcmUsi fixed flags",
            });
        }
        // The `length` field describes how many bytes follow it.
        // We validate that the slice is exactly that long.
        if bytes.len() < 1 + length {
            return Err(Error::BufferTooShort {
                need: 1 + length,
                have: bytes.len(),
                what: "CpcmUsi body",
            });
        }
        // Work within the declared extent: bytes[1 .. 1+length].
        let body = &bytes[1..1 + length];
        if body.len() < USI_FLAGS_LEN {
            return Err(Error::BufferTooShort {
                need: USI_FLAGS_LEN,
                have: body.len(),
                what: "CpcmUsi flag bytes",
            });
        }

        // ── byte 1 ─────────────────────────────────────────────────────────
        let b1 = body[0];
        let copy_control = CopyControl::from_u8(b1 >> 5);
        let do_not_cpcm_scramble = (b1 >> 4) & 1 != 0;
        let viewable = (b1 >> 3) & 1 != 0;
        let view_window_activated = (b1 >> 2) & 1 != 0;
        let view_period_activated = (b1 >> 1) & 1 != 0;
        let simultaneous_view_count_activated = b1 & 1 != 0;

        // ── byte 2 ─────────────────────────────────────────────────────────
        let b2 = body[1];
        let move_local = (b2 >> 7) & 1 != 0;
        let view_local = (b2 >> 6) & 1 != 0;
        let move_and_copy_propagation_information = MoveCopyPropagation::from_u8(b2 >> 4);
        let view_propagation_information = ViewPropagation::from_u8(b2 >> 2);
        let remote_access_date_moving_window_flag = (b2 >> 1) & 1 != 0;
        let remote_access_date_immediate_flag = b2 & 1 != 0;

        // ── byte 3 ─────────────────────────────────────────────────────────
        let b3 = body[2];
        let remote_access_record_flag = (b3 >> 7) & 1 != 0;
        let export_controlled_cps = (b3 >> 6) & 1 != 0;
        let export_beyond_trust = (b3 >> 5) & 1 != 0;
        let disable_analogue_sd_export = (b3 >> 4) & 1 != 0;
        let disable_analogue_sd_consumption = (b3 >> 3) & 1 != 0;
        let disable_analogue_hd_export = (b3 >> 2) & 1 != 0;
        let disable_analogue_hd_consumption = (b3 >> 1) & 1 != 0;
        let image_constraint = b3 & 1 != 0;

        // ── conditional fields ──────────────────────────────────────────────
        let mut pos = USI_FLAGS_LEN; // position within `body`

        let view_window_start;
        let view_window_end;
        if view_window_activated {
            if body.len() < pos + CPCM_DATE_TIME_LEN * 2 {
                return Err(Error::BufferTooShort {
                    need: pos + CPCM_DATE_TIME_LEN * 2,
                    have: body.len(),
                    what: "CpcmUsi view_window_start/end",
                });
            }
            let mut start = [0u8; CPCM_DATE_TIME_LEN];
            start.copy_from_slice(&body[pos..pos + CPCM_DATE_TIME_LEN]);
            pos += CPCM_DATE_TIME_LEN;
            let mut end = [0u8; CPCM_DATE_TIME_LEN];
            end.copy_from_slice(&body[pos..pos + CPCM_DATE_TIME_LEN]);
            pos += CPCM_DATE_TIME_LEN;
            view_window_start = Some(start);
            view_window_end = Some(end);
        } else {
            view_window_start = None;
            view_window_end = None;
        }

        let view_period_from_first_playback = if view_period_activated {
            if body.len() < pos + CPCM_PLAYBACK_PERIOD_LEN {
                return Err(Error::BufferTooShort {
                    need: pos + CPCM_PLAYBACK_PERIOD_LEN,
                    have: body.len(),
                    what: "CpcmUsi view_period_from_first_playback",
                });
            }
            let mut vp = [0u8; CPCM_PLAYBACK_PERIOD_LEN];
            vp.copy_from_slice(&body[pos..pos + CPCM_PLAYBACK_PERIOD_LEN]);
            pos += CPCM_PLAYBACK_PERIOD_LEN;
            Some(vp)
        } else {
            None
        };

        let simultaneous_view_count;
        if simultaneous_view_count_activated {
            if body.len() < pos + SIMULTANEOUS_VIEW_COUNT_LEN {
                return Err(Error::BufferTooShort {
                    need: pos + SIMULTANEOUS_VIEW_COUNT_LEN,
                    have: body.len(),
                    what: "CpcmUsi simultaneous_view_count",
                });
            }
            simultaneous_view_count = Some(body[pos]);
            pos += SIMULTANEOUS_VIEW_COUNT_LEN;
        } else {
            simultaneous_view_count = None;
        }

        let remote_access_date =
            if remote_access_date_immediate_flag || remote_access_date_moving_window_flag {
                if body.len() < pos + CPCM_DATE_TIME_LEN {
                    return Err(Error::BufferTooShort {
                        need: pos + CPCM_DATE_TIME_LEN,
                        have: body.len(),
                        what: "CpcmUsi remote_access_date",
                    });
                }
                let mut rad = [0u8; CPCM_DATE_TIME_LEN];
                rad.copy_from_slice(&body[pos..pos + CPCM_DATE_TIME_LEN]);
                pos += CPCM_DATE_TIME_LEN;
                Some(rad)
            } else {
                None
            };

        let mut cps_vectors = Vec::new();
        if export_controlled_cps {
            if body.len() < pos + 1 {
                return Err(Error::BufferTooShort {
                    need: pos + 1,
                    have: body.len(),
                    what: "CpcmUsi cps_vector_count",
                });
            }
            let cps_vector_count = body[pos] as usize;
            pos += 1;
            for _ in 0..cps_vector_count {
                if body.len() < pos + CPS_ENTRY_HDR_LEN {
                    return Err(Error::BufferTooShort {
                        need: pos + CPS_ENTRY_HDR_LEN,
                        have: body.len(),
                        what: "CpcmUsi CPS entry header",
                    });
                }
                let c_and_r_regime_mask = body[pos];
                let cps_vector_length = u16::from_be_bytes([body[pos + 1], body[pos + 2]]) as usize;
                pos += CPS_ENTRY_HDR_LEN;
                if body.len() < pos + cps_vector_length {
                    return Err(Error::BufferTooShort {
                        need: pos + cps_vector_length,
                        have: body.len(),
                        what: "CpcmUsi cps_vector_byte",
                    });
                }
                let cps_vector = body[pos..pos + cps_vector_length].to_vec();
                pos += cps_vector_length;
                cps_vectors.push(CpsVectorEntry {
                    c_and_r_regime_mask,
                    cps_vector,
                });
            }
        }

        Ok(CpcmUsi {
            copy_control,
            do_not_cpcm_scramble,
            viewable,
            view_window_activated,
            view_period_activated,
            simultaneous_view_count_activated,
            move_local,
            view_local,
            move_and_copy_propagation_information,
            view_propagation_information,
            remote_access_date_moving_window_flag,
            remote_access_date_immediate_flag,
            remote_access_record_flag,
            export_controlled_cps,
            export_beyond_trust,
            disable_analogue_sd_export,
            disable_analogue_sd_consumption,
            disable_analogue_hd_export,
            disable_analogue_hd_consumption,
            image_constraint,
            view_window_start,
            view_window_end,
            view_period_from_first_playback,
            simultaneous_view_count,
            remote_access_date,
            cps_vectors,
        })
    }
}

impl CpcmUsi {
    /// Compute the byte length of the body after the `length` field.
    fn body_len(&self) -> usize {
        let mut n = USI_FLAGS_LEN;
        if self.view_window_activated {
            n += CPCM_DATE_TIME_LEN * 2;
        }
        if self.view_period_activated {
            n += CPCM_PLAYBACK_PERIOD_LEN;
        }
        if self.simultaneous_view_count_activated {
            n += SIMULTANEOUS_VIEW_COUNT_LEN;
        }
        if self.remote_access_date_immediate_flag || self.remote_access_date_moving_window_flag {
            n += CPCM_DATE_TIME_LEN;
        }
        if self.export_controlled_cps {
            n += 1; // cps_vector_count
            for entry in &self.cps_vectors {
                n += CPS_ENTRY_HDR_LEN + entry.cps_vector.len();
            }
        }
        n
    }
}

impl Serialize for CpcmUsi {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        1 + self.body_len() // length byte + body
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        let body_len = self.body_len();
        buf[0] = body_len as u8;

        // ── byte 1 ─────────────────────────────────────────────────────────
        buf[1] = (self.copy_control.to_u8() << 5)
            | ((u8::from(self.do_not_cpcm_scramble)) << 4)
            | ((u8::from(self.viewable)) << 3)
            | ((u8::from(self.view_window_activated)) << 2)
            | ((u8::from(self.view_period_activated)) << 1)
            | u8::from(self.simultaneous_view_count_activated);

        // ── byte 2 ─────────────────────────────────────────────────────────
        buf[2] = ((u8::from(self.move_local)) << 7)
            | ((u8::from(self.view_local)) << 6)
            | ((self.move_and_copy_propagation_information.to_u8() & 0x03) << 4)
            | ((self.view_propagation_information.to_u8() & 0x03) << 2)
            | ((u8::from(self.remote_access_date_moving_window_flag)) << 1)
            | u8::from(self.remote_access_date_immediate_flag);

        // ── byte 3 ─────────────────────────────────────────────────────────
        buf[3] = ((u8::from(self.remote_access_record_flag)) << 7)
            | ((u8::from(self.export_controlled_cps)) << 6)
            | ((u8::from(self.export_beyond_trust)) << 5)
            | ((u8::from(self.disable_analogue_sd_export)) << 4)
            | ((u8::from(self.disable_analogue_sd_consumption)) << 3)
            | ((u8::from(self.disable_analogue_hd_export)) << 2)
            | ((u8::from(self.disable_analogue_hd_consumption)) << 1)
            | u8::from(self.image_constraint);

        let mut pos = 1 + USI_FLAGS_LEN; // 4

        // ── conditional fields ──────────────────────────────────────────────
        if self.view_window_activated {
            if let (Some(start), Some(end)) = (self.view_window_start, self.view_window_end) {
                buf[pos..pos + CPCM_DATE_TIME_LEN].copy_from_slice(&start);
                pos += CPCM_DATE_TIME_LEN;
                buf[pos..pos + CPCM_DATE_TIME_LEN].copy_from_slice(&end);
                pos += CPCM_DATE_TIME_LEN;
            }
        }
        if self.view_period_activated {
            if let Some(vp) = self.view_period_from_first_playback {
                buf[pos..pos + CPCM_PLAYBACK_PERIOD_LEN].copy_from_slice(&vp);
                pos += CPCM_PLAYBACK_PERIOD_LEN;
            }
        }
        if self.simultaneous_view_count_activated {
            if let Some(svc) = self.simultaneous_view_count {
                buf[pos] = svc;
                pos += SIMULTANEOUS_VIEW_COUNT_LEN;
            }
        }
        if self.remote_access_date_immediate_flag || self.remote_access_date_moving_window_flag {
            if let Some(rad) = self.remote_access_date {
                buf[pos..pos + CPCM_DATE_TIME_LEN].copy_from_slice(&rad);
                pos += CPCM_DATE_TIME_LEN;
            }
        }
        if self.export_controlled_cps {
            buf[pos] = self.cps_vectors.len() as u8;
            pos += 1;
            for entry in &self.cps_vectors {
                buf[pos] = entry.c_and_r_regime_mask;
                let vlen = entry.cps_vector.len() as u16;
                buf[pos + 1..pos + 3].copy_from_slice(&vlen.to_be_bytes());
                pos += CPS_ENTRY_HDR_LEN;
                buf[pos..pos + entry.cps_vector.len()].copy_from_slice(&entry.cps_vector);
                pos += entry.cps_vector.len();
            }
        }
        Ok(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};
    use broadcast_common::Serialize;

    // ── helper: construct a minimal USI (no conditional fields) ──────────────
    fn minimal_usi() -> CpcmUsi {
        CpcmUsi {
            copy_control: CopyControl::CopyOnce,
            do_not_cpcm_scramble: false,
            viewable: true,
            view_window_activated: false,
            view_period_activated: false,
            simultaneous_view_count_activated: false,
            move_local: false,
            view_local: false,
            move_and_copy_propagation_information: MoveCopyPropagation::Mlad,
            view_propagation_information: ViewPropagation::Vlad,
            remote_access_date_moving_window_flag: false,
            remote_access_date_immediate_flag: false,
            remote_access_record_flag: false,
            export_controlled_cps: false,
            export_beyond_trust: false,
            disable_analogue_sd_export: false,
            disable_analogue_sd_consumption: false,
            disable_analogue_hd_export: false,
            disable_analogue_hd_consumption: false,
            image_constraint: false,
            view_window_start: None,
            view_window_end: None,
            view_period_from_first_playback: None,
            simultaneous_view_count: None,
            remote_access_date: None,
            cps_vectors: Vec::new(),
        }
    }

    fn round_trip_usi(usi: &CpcmUsi) {
        let mut buf = vec![0u8; usi.serialized_len()];
        usi.serialize_into(&mut buf).unwrap();
        let parsed = CpcmUsi::parse(&buf).unwrap();
        assert_eq!(usi, &parsed, "USI round-trip mismatch");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Byte-anchored minimal USI test.
    //
    // For `copy_control=CopyOnce(1)`, `viewable=true`, all else 0:
    //   byte 0: length = 3 (3 flag bytes follow, no conditional fields)
    //   byte 1: copy_control[7:5]=001 | do_not=0 | viewable=1 | vwa=0 | vpa=0 | svca=0
    //           = 0b0010_1000 = 0x28
    //   byte 2: all zero = 0x00
    //   byte 3: all zero = 0x00
    // ─────────────────────────────────────────────────────────────────────────
    #[test]
    fn minimal_usi_byte_anchor() {
        let usi = minimal_usi();
        let expected = [
            0x03, // length=3
            0x28, // copy_control=1<<5=0x20 | viewable=1<<3=0x08 => 0x28
            0x00, 0x00,
        ];
        let mut buf = vec![0u8; usi.serialized_len()];
        usi.serialize_into(&mut buf).unwrap();
        assert_eq!(
            buf.as_slice(),
            &expected,
            "byte-anchor mismatch for minimal USI"
        );
        let back = CpcmUsi::parse(&expected).unwrap();
        assert_eq!(usi, back);
    }

    // ── Conditional: view_window (two 40-bit dates) ───────────────────────────
    #[test]
    fn usi_view_window_round_trip() {
        let mut usi = minimal_usi();
        usi.view_window_activated = true;
        usi.view_window_start = Some([0x01, 0x02, 0x03, 0x04, 0x05]);
        usi.view_window_end = Some([0x06, 0x07, 0x08, 0x09, 0x0A]);
        round_trip_usi(&usi);
        // Verify length field: 3 flags + 5 + 5 = 13
        let mut buf = vec![0u8; usi.serialized_len()];
        usi.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[0], 13, "length byte for view_window USI");
        // view_window_activated bit set in byte 1 bit 2
        assert_ne!(buf[1] & 0x04, 0, "view_window_activated bit must be set");
        // view_window_start starts at buf[4]
        assert_eq!(&buf[4..9], &[0x01, 0x02, 0x03, 0x04, 0x05]);
        assert_eq!(&buf[9..14], &[0x06, 0x07, 0x08, 0x09, 0x0A]);
    }

    // ── Conditional: view_period (16-bit) ────────────────────────────────────
    #[test]
    fn usi_view_period_round_trip() {
        let mut usi = minimal_usi();
        usi.view_period_activated = true;
        usi.view_period_from_first_playback = Some([0xAB, 0xCD]);
        round_trip_usi(&usi);
        let mut buf = vec![0u8; usi.serialized_len()];
        usi.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[0], 5, "length byte for view_period USI (3+2)");
        assert_eq!(&buf[4..6], &[0xAB, 0xCD]);
    }

    // ── Conditional: simultaneous_view_count (8-bit) ─────────────────────────
    #[test]
    fn usi_simultaneous_view_count_round_trip() {
        let mut usi = minimal_usi();
        usi.simultaneous_view_count_activated = true;
        usi.simultaneous_view_count = Some(4);
        round_trip_usi(&usi);
        let mut buf = vec![0u8; usi.serialized_len()];
        usi.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[0], 4, "length byte for svc USI (3+1)");
        assert_eq!(buf[4], 4u8);
    }

    // ── Conditional: remote_access_date (40-bit) ─────────────────────────────
    #[test]
    fn usi_remote_access_date_round_trip() {
        let mut usi = minimal_usi();
        usi.remote_access_date_immediate_flag = true;
        usi.remote_access_date = Some([0x11, 0x22, 0x33, 0x44, 0x55]);
        round_trip_usi(&usi);
        let mut buf = vec![0u8; usi.serialized_len()];
        usi.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[0], 8, "length byte for rad USI (3+5)");
        assert_eq!(&buf[4..9], &[0x11, 0x22, 0x33, 0x44, 0x55]);
    }

    // ── Conditional: export_controlled_cps (CPS vector) ─────────────────────
    #[test]
    fn usi_export_controlled_cps_round_trip() {
        // Two CPS entries: first has 2-byte payload, second has 3-byte payload.
        // Total body: 3 flags + 1 (count) + (1+2+2) + (1+2+3) = 3+1+5+6 = 15
        let mut usi = minimal_usi();
        usi.export_controlled_cps = true;
        usi.cps_vectors = vec![
            CpsVectorEntry {
                c_and_r_regime_mask: 0xA5,
                cps_vector: vec![0x10, 0x20],
            },
            CpsVectorEntry {
                c_and_r_regime_mask: 0x3C,
                cps_vector: vec![0xDE, 0xAD, 0xBE],
            },
        ];
        round_trip_usi(&usi);
        let mut buf = vec![0u8; usi.serialized_len()];
        usi.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[0], 15, "length byte for cps USI");
        // cps_vector_count at byte 4
        assert_eq!(buf[4], 2);
        // First entry: C_and_R_regime_mask=0xA5, length=0x0002, payload=0x10,0x20
        assert_eq!(buf[5], 0xA5);
        assert_eq!(&buf[6..8], &[0x00, 0x02]);
        assert_eq!(&buf[8..10], &[0x10, 0x20]);
        // Second entry
        assert_eq!(buf[10], 0x3C);
        assert_eq!(&buf[11..13], &[0x00, 0x03]);
        assert_eq!(&buf[13..16], &[0xDE, 0xAD, 0xBE]);
    }

    // ── Mutation test ─────────────────────────────────────────────────────────
    #[test]
    fn usi_mutation_changes_expected_bytes() {
        let usi1 = minimal_usi();
        let mut usi2 = minimal_usi();
        usi2.copy_control = CopyControl::CopyNeverZeroRetentionAsserted; // 4 = 0b100
        usi2.image_constraint = true;

        let mut buf1 = vec![0u8; usi1.serialized_len()];
        let mut buf2 = vec![0u8; usi2.serialized_len()];
        usi1.serialize_into(&mut buf1).unwrap();
        usi2.serialize_into(&mut buf2).unwrap();

        // byte 1: copy_control at [7:5]
        // usi1: 0x28 (CopyOnce=1 → 0x20 | viewable=0x08)
        // usi2: CopyNeverZeroRetentionAsserted=4 → 0x80 | viewable=0x08 = 0x88
        assert_eq!(buf1[1], 0x28);
        assert_eq!(buf2[1], 0x88);
        // byte 3: image_constraint at bit 0
        assert_eq!(buf1[3], 0x00);
        assert_eq!(buf2[3], 0x01);
    }

    // ── CopyControl value mapping per Table 9 ────────────────────────────────
    #[test]
    fn copy_control_table9_mapping() {
        assert_eq!(CopyControl::from_u8(0), CopyControl::CopyControlNotAsserted);
        assert_eq!(CopyControl::from_u8(1), CopyControl::CopyOnce);
        assert_eq!(CopyControl::from_u8(2), CopyControl::CopyNoMore);
        assert_eq!(
            CopyControl::from_u8(3),
            CopyControl::CopyNeverZeroRetentionNotAsserted
        );
        assert_eq!(
            CopyControl::from_u8(4),
            CopyControl::CopyNeverZeroRetentionAsserted
        );
        // 5-7 reserved
        assert_eq!(CopyControl::from_u8(5), CopyControl::Reserved(5));
        assert_eq!(CopyControl::from_u8(6), CopyControl::Reserved(6));
        assert_eq!(CopyControl::from_u8(7), CopyControl::Reserved(7));
        // round-trip to_u8
        for v in 0u8..=7 {
            assert_eq!(CopyControl::from_u8(v).to_u8(), v);
        }
        // name() on reserved
        assert_eq!(CopyControl::Reserved(5).name(), "reserved");
    }

    // ── usi() returns None for cpcm_version != 1 ─────────────────────────────
    #[test]
    fn usi_returns_none_for_non_v1() {
        let desc = CpcmDeliverySignalling {
            cpcm_version: 2,
            selector_bytes: &[0x03, 0x28, 0x00, 0x00],
        };
        assert!(desc.usi().is_none());

        let desc0 = CpcmDeliverySignalling {
            cpcm_version: 0,
            selector_bytes: &[],
        };
        assert!(desc0.usi().is_none());
    }

    // ── usi() returns Some for cpcm_version == 1 ─────────────────────────────
    #[test]
    fn usi_returns_some_for_v1() {
        let selector = [0x03u8, 0x28, 0x00, 0x00];
        let desc = CpcmDeliverySignalling {
            cpcm_version: 1,
            selector_bytes: &selector,
        };
        let usi = desc
            .usi()
            .expect("should be Some")
            .expect("should parse OK");
        assert_eq!(usi.copy_control, CopyControl::CopyOnce);
        assert!(usi.viewable);
    }

    // ── Existing TSDuck round-trip tests (kept passing unchanged) ────────────
    #[test]
    fn parse_cpcm_delivery_signalling_structured() {
        // cpcm_version=1, then 4 selector bytes
        let sel = [0x01, 0x39, 0x24, 0x45, 0x03];
        let bytes = wrap(0x01, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::CpcmDeliverySignalling(b) => {
                assert_eq!(b.cpcm_version, 1);
                assert_eq!(b.selector_bytes, &[0x39, 0x24, 0x45, 0x03]);
            }
            other => panic!("expected CpcmDeliverySignalling, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_cpcm_delivery_signalling_version_only() {
        let sel = [0x01]; // cpcm_version, empty selector
        let bytes = wrap(0x01, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::CpcmDeliverySignalling(b) => {
                assert_eq!(b.cpcm_version, 1);
                assert!(b.selector_bytes.is_empty());
            }
            other => panic!("expected CpcmDeliverySignalling, got {other:?}"),
        }
        round_trip(&d);
    }
}
