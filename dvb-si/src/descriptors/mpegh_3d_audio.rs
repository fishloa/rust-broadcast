//! MPEG-H 3D audio descriptors — Rec. ITU-T H.222.0 (06/2021 | 08/2023) |
//! ISO/IEC 13818-1 clauses 2.6.106–2.6.118, Tables 2-121…2-127. They are the
//! bodies of the MPEG-2 `Extension_descriptor` (tag 0x3F, clause 2.6.90, see
//! [`super::mpeg2_extension`]) selected by `extension_descriptor_tag`
//! 0x08…0x0E (Table 2-110), and are carried in the PMT ES_info loop of an
//! MPEG-H 3D audio stream (`stream_type` 0x2D main / 0x2E auxiliary, see
//! [`crate::tables::pmt::StreamType`]; ETSI TS 101 154 V2.10.1 §6.8, §4.1.8.31).
//!
//! | `extension_descriptor_tag` | Descriptor | Type |
//! |---|---|---|
//! | 0x08 | `MPEG-H_3dAudio_descriptor` | [`MpegH3dAudio`] |
//! | 0x09 | `MPEG-H_3dAudio_config_descriptor` | [`MpegH3dAudioConfig`] (opaque `mpegh3daConfig()`) |
//! | 0x0A | `MPEG-H_3dAudio_scene_descriptor` | [`MpegH3dAudioScene`] |
//! | 0x0B | `MPEG-H_3dAudio_text_label_descriptor` | [`MpegH3dAudioTextLabel`] |
//! | 0x0C | `MPEG-H_3dAudio_multi-stream_descriptor` | [`MpegH3dAudioMultiStream`] |
//! | 0x0D | `MPEG-H_3dAudio_drc_loudness_descriptor` | [`MpegH3dAudioDrcLoudness`] |
//! | 0x0E | `MPEG-H_3dAudio_command_descriptor` | [`MpegH3dAudioCommand`] (opaque MHAS packet) |
//!
//! # Reserved bits
//!
//! Every `reserved` run in the syntax tables is captured on parse into a
//! `reserved*` field and re-emitted verbatim (right-aligned in the field's
//! own width). Hand-built values default to all ones: H.222.0 clause 2.1
//! ("reserved ... shall be set to '1'") and the real Fraunhofer MPEG-H
//! transport-layer streams (PMT bytes `08 10 7F C1`, `08 0B 3F C1 01 10`) carry
//! all-ones reserved bits. The trailing `for (i=0; i<N; i++) reserved(8)` run of
//! every descriptor is kept verbatim in a `trailing` byte vector.
//!
//! # Source inconsistencies (documented, not guessed)
//!
//! * **Table 2-121**: the printed `reserved` run is 8 bits between
//!   `compatibleProfileSetsPresent` and `referenceChannelLayout`: it spans the
//!   low 6 bits of the second byte and the top 2 bits of the third (real data:
//!   `0x7F`/`0x3F` then `0xC1`). `compatibleProfileSetsPresent == '0'` means
//!   the compatible-sets list **is present** (confirmed by real data:
//!   `3F C1 01 10` carries `numCompatibleSets = 1`, `CompatibleSetIndication = 0x10`).
//! * **Table 2-125**: the printed condition `if (thisIsMainStream == '0')` inside
//!   the `mae_numGroups` loop (itself inside `if (thisIsMainStream)`) can never be
//!   true; the auxiliary-stream fields (`isInTS`, `auxiliaryStreamID`) are
//!   decoded when `isInMainStream == '0'`, which is what the semantics of
//!   `isInMainStream`/`isInTS`/`auxiliaryStreamID` describe and what TSDuck
//!   (secondary cross-check) implements.
//! * **Table 2-123 / 2-124 / 2-126**: loop bounds are used exactly as printed
//!   (`numSwitchGroups`, `mae_numGroupPresets`, `numDescLanguages` are counts;
//!   the semantics sentence "number of switch groups minus one" disagrees with the
//!   loop `i < numSwitchGroups` and the loop is followed). There are no real
//!   captures of these descriptors in the real-capture corpus: they are verified
//!   against the printed syntax only (round-trip + hand-derived vectors).

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::len::{fit_bits, fit_u8};
use broadcast_common::reserved::ReservedOnes;

/// Checked bit-field value: `v` must fit `bits` bits.
fn f(v: u8, bits: u32, name: &'static str) -> Result<u8> {
    Ok(fit_bits(u64::from(v), bits, name)? as u8)
}

/// Checked list length that must fit `bits` bits.
fn n(len: usize, bits: u32, name: &'static str) -> Result<u8> {
    Ok(fit_bits(len as u64, bits, name)? as u8)
}

/// Byte reader over a descriptor body.
pub(crate) struct Reader<'a> {
    body: &'a [u8],
    pos: usize,
    what: &'static str,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(body: &'a [u8], what: &'static str) -> Self {
        Self { body, pos: 0, what }
    }
    fn u8(&mut self) -> Result<u8> {
        let b = *self.body.get(self.pos).ok_or(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: self.pos + 1,
                have: self.body.len(),
                what: self.what,
            },
        ))?;
        self.pos += 1;
        Ok(b)
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.pos + len;
        let s = self.body.get(self.pos..end).ok_or(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: end,
                have: self.body.len(),
                what: self.what,
            },
        ))?;
        self.pos = end;
        Ok(s)
    }
    fn rest(&mut self) -> &'a [u8] {
        let s = &self.body[self.pos..];
        self.pos = self.body.len();
        s
    }
}

// ───────────────────────── 2.6.106  MPEG-H_3dAudio_descriptor ──────────────

/// `MPEG-H_3dAudio_descriptor()` (Table 2-121), `extension_descriptor_tag` 0x08.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MpegH3dAudio {
    /// `mpegh3daProfileLevelIndication` (8 bits, ISO/IEC 23008-3 5.3.2).
    pub profile_level_indication: u8,
    /// `interactivityEnabled` (1 bit).
    pub interactivity_enabled: bool,
    /// `CompatibleSetIndication` list: `Some` iff `compatibleProfileSetsPresent == '0'`
    /// (the flag is inverted, see the module docs); the list length is `numCompatibleSets`.
    pub compatible_sets: Option<Vec<u8>>,
    /// First 6 bits of the printed 8-bit `reserved` run (right-aligned), default `0x3F`.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_flags: ReservedOnes<6>,
    /// Last 2 bits of the printed 8-bit `reserved` run (right-aligned), default `0x3`.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_layout: ReservedOnes<2>,
    /// `referenceChannelLayout` (6 bits, ISO/IEC 23001-8 `ChannelConfiguration`).
    pub reference_channel_layout: u8,
    /// Trailing `reserved(8)` bytes, verbatim.
    pub trailing: Vec<u8>,
}

impl MpegH3dAudio {
    /// Build with all-ones reserved bits and no trailing bytes.
    #[must_use]
    pub fn new(
        profile_level_indication: u8,
        interactivity_enabled: bool,
        compatible_sets: Option<Vec<u8>>,
        reference_channel_layout: u8,
    ) -> Self {
        Self {
            profile_level_indication,
            interactivity_enabled,
            compatible_sets,
            reserved_flags: ReservedOnes::FILL,
            reserved_layout: ReservedOnes::FILL,
            reference_channel_layout,
            trailing: Vec::new(),
        }
    }

    pub(crate) fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let profile_level_indication = r.u8()?;
        let b1 = r.u8()?;
        let b2 = r.u8()?;
        let present = b1 & 0x40 == 0;
        let compatible_sets = if present {
            let cnt = r.u8()? as usize;
            Some(r.take(cnt)?.to_vec())
        } else {
            None
        };
        Ok(Self {
            profile_level_indication,
            interactivity_enabled: b1 & 0x80 != 0,
            compatible_sets,
            reserved_flags: ReservedOnes::from_bits(u64::from(b1 & 0x3F)),
            reserved_layout: ReservedOnes::from_bits(u64::from(b2 >> 6)),
            reference_channel_layout: b2 & 0x3F,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        out.push(self.profile_level_indication);
        out.push(
            (u8::from(self.interactivity_enabled) << 7)
                | (u8::from(self.compatible_sets.is_none()) << 6)
                | (self.reserved_flags.get() as u8),
        );
        out.push(
            ((self.reserved_layout.get() as u8) << 6)
                | f(self.reference_channel_layout, 6, "referenceChannelLayout")?,
        );
        if let Some(sets) = &self.compatible_sets {
            out.push(fit_u8(sets.len(), "numCompatibleSets")?);
            out.extend_from_slice(sets);
        }
        out.extend_from_slice(&self.trailing);
        Ok(())
    }
}

// ───────────────────────── 2.6.108  config descriptor ──────────────────────

/// `MPEG-H_3dAudio_config_descriptor()` (Table 2-122), tag 0x09: exactly one
/// `mpegh3daConfig()` (ISO/IEC 23008-3 5.2.2.1). The configuration bitstream is
/// carried opaquely (its grammar is in the paid ISO/IEC 23008-3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MpegH3dAudioConfig {
    /// The `mpegh3daConfig()` bytes.
    pub mpegh3da_config: Vec<u8>,
}

// ───────────────────────── 2.6.110  scene descriptor ───────────────────────

/// `mae_interactivity*Offset` block of a group (position interactivity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PositionInteractivity {
    /// `reserved(1)`, default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_min_az: ReservedOnes<1>,
    /// `mae_interactivityMinAzOffset` (7).
    pub min_az_offset: u8,
    /// `reserved(1)`, default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_max_az: ReservedOnes<1>,
    /// `mae_interactivityMaxAzOffset` (7).
    pub max_az_offset: u8,
    /// `reserved(3)`, default 7.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_min_el: ReservedOnes<3>,
    /// `mae_interactivityMinElOffset` (5).
    pub min_el_offset: u8,
    /// `reserved(3)`, default 7.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_max_el: ReservedOnes<3>,
    /// `mae_interactivityMaxElOffset` (5).
    pub max_el_offset: u8,
    /// `mae_interactivityMinDistOffset` (4).
    pub min_dist_offset: u8,
    /// `mae_interactivityMaxDistOffset` (4).
    pub max_dist_offset: u8,
}

impl PositionInteractivity {
    /// Build with all-ones reserved bits.
    #[must_use]
    pub const fn new(
        min_az_offset: u8,
        max_az_offset: u8,
        min_el_offset: u8,
        max_el_offset: u8,
        min_dist_offset: u8,
        max_dist_offset: u8,
    ) -> Self {
        Self {
            reserved_min_az: ReservedOnes::FILL,
            min_az_offset,
            reserved_max_az: ReservedOnes::FILL,
            max_az_offset,
            reserved_min_el: ReservedOnes::FILL,
            min_el_offset,
            reserved_max_el: ReservedOnes::FILL,
            max_el_offset,
            min_dist_offset,
            max_dist_offset,
        }
    }
}

/// `mae_interactivityMinGain` / `MaxGain` block of a group (gain interactivity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct GainInteractivity {
    /// `reserved(2)`, default 3.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_min: ReservedOnes<2>,
    /// `mae_interactivityMinGain` (6).
    pub min_gain: u8,
    /// `reserved(3)`, default 7.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_max: ReservedOnes<3>,
    /// `mae_interactivityMaxGain` (5).
    pub max_gain: u8,
}

impl GainInteractivity {
    /// Build with all-ones reserved bits.
    #[must_use]
    pub const fn new(min_gain: u8, max_gain: u8) -> Self {
        Self {
            reserved_min: ReservedOnes::FILL,
            min_gain,
            reserved_max: ReservedOnes::FILL,
            max_gain,
        }
    }
}

/// One `mae_groupID` entry of the scene descriptor's group loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SceneGroup {
    /// `reserved(1)`, default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_id: ReservedOnes<1>,
    /// `mae_groupID` (7).
    pub group_id: u8,
    /// `reserved(3)`, default 7.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_flags: ReservedOnes<3>,
    /// `mae_allowOnOff`.
    pub allow_on_off: bool,
    /// `mae_defaultOnOff`.
    pub default_on_off: bool,
    /// `reserved(4)`, default 15.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_kind: ReservedOnes<4>,
    /// `mae_contentKind` (4).
    pub content_kind: u8,
    /// Present iff `mae_allowPositionInteractivity`.
    pub position: Option<PositionInteractivity>,
    /// Present iff `mae_allowGainInteractivity`.
    pub gain: Option<GainInteractivity>,
    /// `mae_contentLanguage` (24 bits, ISO 639-2 chars), present iff `mae_hasContentLanguage`.
    pub content_language: Option<[u8; 3]>,
}

impl SceneGroup {
    /// Build with all-ones reserved bits.
    #[must_use]
    pub const fn new(
        group_id: u8,
        allow_on_off: bool,
        default_on_off: bool,
        content_kind: u8,
    ) -> Self {
        Self {
            reserved_id: ReservedOnes::FILL,
            group_id,
            reserved_flags: ReservedOnes::FILL,
            allow_on_off,
            default_on_off,
            reserved_kind: ReservedOnes::FILL,
            content_kind,
            position: None,
            gain: None,
            content_language: None,
        }
    }
}

/// One member of a switch group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SwitchGroupMember {
    /// `reserved(1)`, default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedOnes<1>,
    /// `mae_switchGroupMemberID` (7).
    pub member_id: u8,
}

impl SwitchGroupMember {
    /// Build with the all-ones reserved bit.
    #[must_use]
    pub const fn new(member_id: u8) -> Self {
        Self {
            reserved: ReservedOnes::FILL,
            member_id,
        }
    }
}

/// One switch-group entry (`mae_switchGroupID` …).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SwitchGroup {
    /// `reserved(1)`, default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_id: ReservedOnes<1>,
    /// `mae_switchGroupID` (5).
    pub switch_group_id: u8,
    /// `mae_switchGroupAllowOnOff`.
    pub allow_on_off: bool,
    /// `mae_switchGroupDefaultOnOff` (shall be 0 when `allow_on_off` is 0).
    pub default_on_off: bool,
    /// `reserved(3)`, default 7.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_members: ReservedOnes<3>,
    /// Members; `mae_bsSwitchGroupNumMembers` = len − 1 (so 1..=32 entries).
    pub members: Vec<SwitchGroupMember>,
    /// `reserved(1)`, default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_default: ReservedOnes<1>,
    /// `mae_switchGroupDefaultGroupID` (7).
    pub default_group_id: u8,
}

/// Detail block of a preset condition with `mae_groupPresetConditionOnOff == 1`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PresetConditionDetail {
    /// `reserved(4)`, default 15.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedOnes<4>,
    /// `mae_groupPresetDisableGainInteractivity`.
    pub disable_gain_interactivity: bool,
    /// `mae_groupPresetDisablePositionInteractivity`.
    pub disable_position_interactivity: bool,
    /// `mae_groupPresetGain` (8), present iff `mae_groupPresetGainFlag`.
    pub gain: Option<u8>,
    /// Position offsets, present iff `mae_groupPresetPositionFlag`.
    pub position: Option<PresetPosition>,
}

/// `mae_groupPresetAzOffset` / `ElOffset` / `DistFactor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PresetPosition {
    /// `mae_groupPresetAzOffset` (8).
    pub az_offset: u8,
    /// `reserved(2)`, default 3.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_el: ReservedOnes<2>,
    /// `mae_groupPresetElOffset` (6).
    pub el_offset: u8,
    /// `reserved(4)`, default 15.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_dist: ReservedOnes<4>,
    /// `mae_groupPresetDistFactor` (4).
    pub dist_factor: u8,
}

impl PresetPosition {
    /// Build with all-ones reserved bits.
    #[must_use]
    pub const fn new(az_offset: u8, el_offset: u8, dist_factor: u8) -> Self {
        Self {
            az_offset,
            reserved_el: ReservedOnes::FILL,
            el_offset,
            reserved_dist: ReservedOnes::FILL,
            dist_factor,
        }
    }
}

/// One condition of a group preset.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PresetCondition {
    /// `mae_groupPresetGroupID` (7).
    pub group_id: u8,
    /// Present iff `mae_groupPresetConditionOnOff == 1`.
    pub detail: Option<PresetConditionDetail>,
}

/// One group preset (`mae_groupPresetID` …).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct GroupPreset {
    /// `reserved(3)`, default 7.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_id: ReservedOnes<3>,
    /// `mae_groupPresetID` (5).
    pub preset_id: u8,
    /// `reserved(3)`, default 7.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_kind: ReservedOnes<3>,
    /// `mae_groupPresetKind` (5).
    pub preset_kind: u8,
    /// `reserved(4)`, default 15.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_conditions: ReservedOnes<4>,
    /// Conditions; `mae_numGroupPresetConditions` = len − 1 (so 1..=16 entries).
    pub conditions: Vec<PresetCondition>,
}

/// `MPEG-H_3dAudio_scene_descriptor()` (Table 2-123), tag 0x0A.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MpegH3dAudioScene {
    /// `reserved(5)` after the three presence flags, default 31.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_flags: ReservedOnes<5>,
    /// `3dAudioSceneInfoID` (8, `mae_audioSceneInfoID`).
    pub scene_info_id: u8,
    /// Groups, present iff `groupDefinitionPresent`.
    pub groups: Option<Vec<SceneGroup>>,
    /// `reserved(3)` before `numSwitchGroups` (kept only when switch groups are present), default 7.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_switch_groups: ReservedOnes<3>,
    /// Switch groups, present iff `switchGroupDefinitionPresent`.
    pub switch_groups: Option<Vec<SwitchGroup>>,
    /// `reserved(3)` before `mae_numGroupPresets` (kept only when presets are present), default 7.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_presets: ReservedOnes<3>,
    /// Group presets, present iff `groupPresetDefinitionPresent`.
    pub presets: Option<Vec<GroupPreset>>,
    /// `reserved(1)` before `numGroups` (kept only when groups are present), default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_groups: ReservedOnes<1>,
    /// Trailing `reserved(8)` bytes, verbatim.
    pub trailing: Vec<u8>,
}

impl MpegH3dAudioScene {
    /// Build with all-ones reserved bits and no trailing bytes.
    #[must_use]
    pub fn new(
        scene_info_id: u8,
        groups: Option<Vec<SceneGroup>>,
        switch_groups: Option<Vec<SwitchGroup>>,
        presets: Option<Vec<GroupPreset>>,
    ) -> Self {
        Self {
            reserved_flags: ReservedOnes::FILL,
            scene_info_id,
            groups,
            reserved_switch_groups: ReservedOnes::FILL,
            switch_groups,
            reserved_presets: ReservedOnes::FILL,
            presets,
            reserved_groups: ReservedOnes::FILL,
            trailing: Vec::new(),
        }
    }

    pub(crate) fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let flags = r.u8()?;
        let scene_info_id = r.u8()?;
        let (group_present, switch_present, preset_present) =
            (flags & 0x80 != 0, flags & 0x40 != 0, flags & 0x20 != 0);
        let mut reserved_groups = ReservedOnes::FILL;
        let groups = if group_present {
            let b = r.u8()?;
            reserved_groups = ReservedOnes::from_bits(u64::from(b >> 7));
            let count = usize::from(b & 0x7F);
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push(parse_group(r)?);
            }
            Some(v)
        } else {
            None
        };
        let mut reserved_switch_groups = ReservedOnes::FILL;
        let switch_groups = if switch_present {
            let b = r.u8()?;
            reserved_switch_groups = ReservedOnes::from_bits(u64::from(b >> 5));
            let count = usize::from(b & 0x1F);
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push(parse_switch_group(r)?);
            }
            Some(v)
        } else {
            None
        };
        let mut reserved_presets = ReservedOnes::FILL;
        let presets = if preset_present {
            let b = r.u8()?;
            reserved_presets = ReservedOnes::from_bits(u64::from(b >> 5));
            let count = usize::from(b & 0x1F);
            let mut v = Vec::with_capacity(count);
            for _ in 0..count {
                v.push(parse_preset(r)?);
            }
            Some(v)
        } else {
            None
        };
        Ok(Self {
            reserved_flags: ReservedOnes::from_bits(u64::from(flags & 0x1F)),
            scene_info_id,
            groups,
            reserved_switch_groups,
            switch_groups,
            reserved_presets,
            presets,
            reserved_groups,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        out.push(
            (u8::from(self.groups.is_some()) << 7)
                | (u8::from(self.switch_groups.is_some()) << 6)
                | (u8::from(self.presets.is_some()) << 5)
                | (self.reserved_flags.get() as u8),
        );
        out.push(self.scene_info_id);
        if let Some(groups) = &self.groups {
            out.push(((self.reserved_groups.get() as u8) << 7) | n(groups.len(), 7, "numGroups")?);
            for g in groups {
                write_group(g, out)?;
            }
        }
        if let Some(sgs) = &self.switch_groups {
            out.push(
                ((self.reserved_switch_groups.get() as u8) << 5)
                    | n(sgs.len(), 5, "numSwitchGroups")?,
            );
            for sg in sgs {
                write_switch_group(sg, out)?;
            }
        }
        if let Some(ps) = &self.presets {
            out.push(
                ((self.reserved_presets.get() as u8) << 5) | n(ps.len(), 5, "mae_numGroupPresets")?,
            );
            for p in ps {
                write_preset(p, out)?;
            }
        }
        out.extend_from_slice(&self.trailing);
        Ok(())
    }
}

fn parse_group(r: &mut Reader<'_>) -> Result<SceneGroup> {
    let b0 = r.u8()?;
    let b1 = r.u8()?;
    let b2 = r.u8()?;
    let (allow_pos, allow_gain, has_lang) = (b1 & 0x04 != 0, b1 & 0x02 != 0, b1 & 0x01 != 0);
    let position = if allow_pos {
        let a = r.u8()?;
        let b = r.u8()?;
        let c = r.u8()?;
        let d = r.u8()?;
        let e = r.u8()?;
        Some(PositionInteractivity {
            reserved_min_az: ReservedOnes::from_bits(u64::from(a >> 7)),
            min_az_offset: a & 0x7F,
            reserved_max_az: ReservedOnes::from_bits(u64::from(b >> 7)),
            max_az_offset: b & 0x7F,
            reserved_min_el: ReservedOnes::from_bits(u64::from(c >> 5)),
            min_el_offset: c & 0x1F,
            reserved_max_el: ReservedOnes::from_bits(u64::from(d >> 5)),
            max_el_offset: d & 0x1F,
            min_dist_offset: e >> 4,
            max_dist_offset: e & 0x0F,
        })
    } else {
        None
    };
    let gain = if allow_gain {
        let a = r.u8()?;
        let b = r.u8()?;
        Some(GainInteractivity {
            reserved_min: ReservedOnes::from_bits(u64::from(a >> 6)),
            min_gain: a & 0x3F,
            reserved_max: ReservedOnes::from_bits(u64::from(b >> 5)),
            max_gain: b & 0x1F,
        })
    } else {
        None
    };
    let content_language = if has_lang {
        let l = r.take(3)?;
        Some([l[0], l[1], l[2]])
    } else {
        None
    };
    Ok(SceneGroup {
        reserved_id: ReservedOnes::from_bits(u64::from(b0 >> 7)),
        group_id: b0 & 0x7F,
        reserved_flags: ReservedOnes::from_bits(u64::from(b1 >> 5)),
        allow_on_off: b1 & 0x10 != 0,
        default_on_off: b1 & 0x08 != 0,
        reserved_kind: ReservedOnes::from_bits(u64::from(b2 >> 4)),
        content_kind: b2 & 0x0F,
        position,
        gain,
        content_language,
    })
}

fn write_group(g: &SceneGroup, out: &mut Vec<u8>) -> Result<()> {
    out.push(((g.reserved_id.get() as u8) << 7) | f(g.group_id, 7, "mae_groupID")?);
    out.push(
        ((g.reserved_flags.get() as u8) << 5)
            | (u8::from(g.allow_on_off) << 4)
            | (u8::from(g.default_on_off) << 3)
            | (u8::from(g.position.is_some()) << 2)
            | (u8::from(g.gain.is_some()) << 1)
            | u8::from(g.content_language.is_some()),
    );
    out.push(((g.reserved_kind.get() as u8) << 4) | f(g.content_kind, 4, "mae_contentKind")?);
    if let Some(p) = &g.position {
        out.push(
            ((p.reserved_min_az.get() as u8) << 7)
                | f(p.min_az_offset, 7, "mae_interactivityMinAzOffset")?,
        );
        out.push(
            ((p.reserved_max_az.get() as u8) << 7)
                | f(p.max_az_offset, 7, "mae_interactivityMaxAzOffset")?,
        );
        out.push(
            ((p.reserved_min_el.get() as u8) << 5)
                | f(p.min_el_offset, 5, "mae_interactivityMinElOffset")?,
        );
        out.push(
            ((p.reserved_max_el.get() as u8) << 5)
                | f(p.max_el_offset, 5, "mae_interactivityMaxElOffset")?,
        );
        out.push(
            (f(p.min_dist_offset, 4, "mae_interactivityMinDistOffset")? << 4)
                | f(p.max_dist_offset, 4, "mae_interactivityMaxDistOffset")?,
        );
    }
    if let Some(gn) = &g.gain {
        out.push(
            ((gn.reserved_min.get() as u8) << 6) | f(gn.min_gain, 6, "mae_interactivityMinGain")?,
        );
        out.push(
            ((gn.reserved_max.get() as u8) << 5) | f(gn.max_gain, 5, "mae_interactivityMaxGain")?,
        );
    }
    if let Some(l) = &g.content_language {
        out.extend_from_slice(l);
    }
    Ok(())
}

fn parse_switch_group(r: &mut Reader<'_>) -> Result<SwitchGroup> {
    let b0 = r.u8()?;
    let b1 = r.u8()?;
    let members_n = usize::from(b1 & 0x1F) + 1;
    let mut members = Vec::with_capacity(members_n);
    for _ in 0..members_n {
        let m = r.u8()?;
        members.push(SwitchGroupMember {
            reserved: ReservedOnes::from_bits(u64::from(m >> 7)),
            member_id: m & 0x7F,
        });
    }
    let d = r.u8()?;
    Ok(SwitchGroup {
        reserved_id: ReservedOnes::from_bits(u64::from(b0 >> 7)),
        switch_group_id: (b0 >> 2) & 0x1F,
        allow_on_off: b0 & 0x02 != 0,
        default_on_off: b0 & 0x01 != 0,
        reserved_members: ReservedOnes::from_bits(u64::from(b1 >> 5)),
        members,
        reserved_default: ReservedOnes::from_bits(u64::from(d >> 7)),
        default_group_id: d & 0x7F,
    })
}

fn write_switch_group(sg: &SwitchGroup, out: &mut Vec<u8>) -> Result<()> {
    if sg.members.is_empty() {
        return Err(Error::InvalidDescriptor {
            tag: super::mpeg2_extension::TAG,
            reason: "switch group needs at least one member (mae_bsSwitchGroupNumMembers + 1)",
        });
    }
    out.push(
        ((sg.reserved_id.get() as u8) << 7)
            | (f(sg.switch_group_id, 5, "mae_switchGroupID")? << 2)
            | (u8::from(sg.allow_on_off) << 1)
            | u8::from(sg.default_on_off),
    );
    out.push(
        ((sg.reserved_members.get() as u8) << 5)
            | n(sg.members.len() - 1, 5, "mae_bsSwitchGroupNumMembers")?,
    );
    for m in &sg.members {
        out.push(((m.reserved.get() as u8) << 7) | f(m.member_id, 7, "mae_switchGroupMemberID")?);
    }
    out.push(
        ((sg.reserved_default.get() as u8) << 7)
            | f(sg.default_group_id, 7, "mae_switchGroupDefaultGroupID")?,
    );
    Ok(())
}

fn parse_preset(r: &mut Reader<'_>) -> Result<GroupPreset> {
    let b0 = r.u8()?;
    let b1 = r.u8()?;
    let b2 = r.u8()?;
    let cond_n = usize::from(b2 & 0x0F) + 1;
    let mut conditions = Vec::with_capacity(cond_n);
    for _ in 0..cond_n {
        let c = r.u8()?;
        let detail = if c & 1 != 0 {
            let d = r.u8()?;
            let (gain_flag, pos_flag) = (d & 0x04 != 0, d & 0x01 != 0);
            let gain = if gain_flag { Some(r.u8()?) } else { None };
            let position = if pos_flag {
                let az = r.u8()?;
                let el = r.u8()?;
                let dist = r.u8()?;
                Some(PresetPosition {
                    az_offset: az,
                    reserved_el: ReservedOnes::from_bits(u64::from(el >> 6)),
                    el_offset: el & 0x3F,
                    reserved_dist: ReservedOnes::from_bits(u64::from(dist >> 4)),
                    dist_factor: dist & 0x0F,
                })
            } else {
                None
            };
            Some(PresetConditionDetail {
                reserved: ReservedOnes::from_bits(u64::from(d >> 4)),
                disable_gain_interactivity: d & 0x08 != 0,
                disable_position_interactivity: d & 0x02 != 0,
                gain,
                position,
            })
        } else {
            None
        };
        conditions.push(PresetCondition {
            group_id: c >> 1,
            detail,
        });
    }
    Ok(GroupPreset {
        reserved_id: ReservedOnes::from_bits(u64::from(b0 >> 5)),
        preset_id: b0 & 0x1F,
        reserved_kind: ReservedOnes::from_bits(u64::from(b1 >> 5)),
        preset_kind: b1 & 0x1F,
        reserved_conditions: ReservedOnes::from_bits(u64::from(b2 >> 4)),
        conditions,
    })
}

fn write_preset(p: &GroupPreset, out: &mut Vec<u8>) -> Result<()> {
    if p.conditions.is_empty() {
        return Err(Error::InvalidDescriptor {
            tag: super::mpeg2_extension::TAG,
            reason: "group preset needs at least one condition (mae_numGroupPresetConditions + 1)",
        });
    }
    out.push(((p.reserved_id.get() as u8) << 5) | f(p.preset_id, 5, "mae_groupPresetID")?);
    out.push(((p.reserved_kind.get() as u8) << 5) | f(p.preset_kind, 5, "mae_groupPresetKind")?);
    out.push(
        ((p.reserved_conditions.get() as u8) << 4)
            | n(p.conditions.len() - 1, 4, "mae_numGroupPresetConditions")?,
    );
    for c in &p.conditions {
        out.push((f(c.group_id, 7, "mae_groupPresetGroupID")? << 1) | u8::from(c.detail.is_some()));
        if let Some(d) = &c.detail {
            out.push(
                ((d.reserved.get() as u8) << 4)
                    | (u8::from(d.disable_gain_interactivity) << 3)
                    | (u8::from(d.gain.is_some()) << 2)
                    | (u8::from(d.disable_position_interactivity) << 1)
                    | u8::from(d.position.is_some()),
            );
            if let Some(g) = d.gain {
                out.push(g);
            }
            if let Some(pos) = &d.position {
                out.push(pos.az_offset);
                out.push(
                    ((pos.reserved_el.get() as u8) << 6)
                        | f(pos.el_offset, 6, "mae_groupPresetElOffset")?,
                );
                out.push(
                    ((pos.reserved_dist.get() as u8) << 4)
                        | f(pos.dist_factor, 4, "mae_groupPresetDistFactor")?,
                );
            }
        }
    }
    Ok(())
}

// ───────────────────────── 2.6.112  text label descriptor ──────────────────

/// A `(id, description)` pair: group, switch-group or preset description
/// (`groupDescriptionData` etc., UTF-8 per ISO/IEC 10646, carried verbatim).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TextLabelEntry {
    /// `reserved` ahead of the ID (1 bit for group IDs, 3 bits for switch group / preset IDs).
    pub reserved: u8,
    /// `mae_descriptionGroupID` (7) / `mae_descriptionSwitchGroupID` (5) / `mae_descriptionGroupPresetID` (5).
    pub id: u8,
    /// Description bytes (`…DescriptionDataLength` bytes).
    pub data: Vec<u8>,
}

/// One language block of the text label descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TextLabelLanguage {
    /// `descriptionLanguage` (24 bits, ISO 639-2 characters).
    pub language: [u8; 3],
    /// `reserved(1)` before `numGroupDescriptions`, default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_groups: ReservedOnes<1>,
    /// Group descriptions (`numGroupDescriptions`, 7 bits).
    pub groups: Vec<TextLabelEntry>,
    /// `reserved(3)` before `numSwitchGroupDescriptions`, default 7.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_switch_groups: ReservedOnes<3>,
    /// Switch-group descriptions (`numSwitchGroupDescriptions`, 5 bits).
    pub switch_groups: Vec<TextLabelEntry>,
    /// `reserved(3)` before `numGroupPresetsDescriptions`, default 7.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_presets: ReservedOnes<3>,
    /// Group-preset descriptions (`numGroupPresetsDescriptions`, 5 bits).
    pub presets: Vec<TextLabelEntry>,
}

/// `MPEG-H_3dAudio_text_label_descriptor()` (Table 2-124), tag 0x0B.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MpegH3dAudioTextLabel {
    /// `3dAudioSceneInfoID` (8).
    pub scene_info_id: u8,
    /// `reserved(4)` before `numDescLanguages`, default 15.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedOnes<4>,
    /// Languages (`numDescLanguages`, 4 bits).
    pub languages: Vec<TextLabelLanguage>,
    /// Trailing `reserved(8)` bytes, verbatim.
    pub trailing: Vec<u8>,
}

fn parse_entries(r: &mut Reader<'_>, count: usize, id_bits: u32) -> Result<Vec<TextLabelEntry>> {
    let mut v = Vec::with_capacity(count);
    for _ in 0..count {
        let b = r.u8()?;
        let len = usize::from(r.u8()?);
        v.push(TextLabelEntry {
            reserved: b >> id_bits,
            id: b & ((1u8 << id_bits) - 1),
            data: r.take(len)?.to_vec(),
        });
    }
    Ok(v)
}

fn write_entries(
    entries: &[TextLabelEntry],
    id_bits: u32,
    id_name: &'static str,
    len_name: &'static str,
    out: &mut Vec<u8>,
) -> Result<()> {
    for e in entries {
        out.push((f(e.reserved, 8 - id_bits, "reserved")? << id_bits) | f(e.id, id_bits, id_name)?);
        out.push(fit_u8(e.data.len(), len_name)?);
        out.extend_from_slice(&e.data);
    }
    Ok(())
}

impl MpegH3dAudioTextLabel {
    pub(crate) fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let scene_info_id = r.u8()?;
        let b = r.u8()?;
        let langs = usize::from(b & 0x0F);
        let mut languages = Vec::with_capacity(langs);
        for _ in 0..langs {
            let l = r.take(3)?;
            let language = [l[0], l[1], l[2]];
            let g = r.u8()?;
            let groups = parse_entries(r, usize::from(g & 0x7F), 7)?;
            let s = r.u8()?;
            let switch_groups = parse_entries(r, usize::from(s & 0x1F), 5)?;
            let p = r.u8()?;
            let presets = parse_entries(r, usize::from(p & 0x1F), 5)?;
            languages.push(TextLabelLanguage {
                language,
                reserved_groups: ReservedOnes::from_bits(u64::from(g >> 7)),
                groups,
                reserved_switch_groups: ReservedOnes::from_bits(u64::from(s >> 5)),
                switch_groups,
                reserved_presets: ReservedOnes::from_bits(u64::from(p >> 5)),
                presets,
            });
        }
        Ok(Self {
            scene_info_id,
            reserved: ReservedOnes::from_bits(u64::from(b >> 4)),
            languages,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        out.push(self.scene_info_id);
        out.push(
            ((self.reserved.get() as u8) << 4) | n(self.languages.len(), 4, "numDescLanguages")?,
        );
        for l in &self.languages {
            out.extend_from_slice(&l.language);
            out.push(
                ((l.reserved_groups.get() as u8) << 7)
                    | n(l.groups.len(), 7, "numGroupDescriptions")?,
            );
            write_entries(
                &l.groups,
                7,
                "mae_descriptionGroupID",
                "groupDescriptionDataLength",
                out,
            )?;
            out.push(
                ((l.reserved_switch_groups.get() as u8) << 5)
                    | n(l.switch_groups.len(), 5, "numSwitchGroupDescriptions")?,
            );
            write_entries(
                &l.switch_groups,
                5,
                "mae_descriptionSwitchGroupID",
                "switchGroupDescriptionDataLength",
                out,
            )?;
            out.push(
                ((l.reserved_presets.get() as u8) << 5)
                    | n(l.presets.len(), 5, "numGroupPresetsDescriptions")?,
            );
            write_entries(
                &l.presets,
                5,
                "mae_descriptionGroupPresetID",
                "groupPresetDescriptionDataLength",
                out,
            )?;
        }
        out.extend_from_slice(&self.trailing);
        Ok(())
    }
}

// ───────────────────────── 2.6.114  multi-stream descriptor ────────────────

/// One `mae_groupID` entry of the multi-stream descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MultiStreamGroup {
    /// `mae_groupID` (7).
    pub group_id: u8,
    /// `isInMainStream`.
    pub is_in_main_stream: bool,
    /// Where the group lives when `isInMainStream == 0` (see the module docs on the printed condition).
    pub auxiliary: Option<AuxiliaryLocation>,
}

/// `isInTS` + `auxiliaryStreamID` of a group carried in an auxiliary stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AuxiliaryLocation {
    /// `isInTS`: 1 = same transport stream, 0 = external source.
    pub is_in_ts: bool,
    /// `auxiliaryStreamID` (7).
    pub auxiliary_stream_id: u8,
}

/// Main-stream part of the multi-stream descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MultiStreamMain {
    /// `reserved(1)` before `numAuxiliaryStreams`, default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_aux: ReservedOnes<1>,
    /// `numAuxiliaryStreams` (7).
    pub num_auxiliary_streams: u8,
    /// `reserved(1)` before `mae_numGroups`, default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_groups: ReservedOnes<1>,
    /// Groups (`mae_numGroups`, 7 bits).
    pub groups: Vec<MultiStreamGroup>,
}

/// `MPEG-H_3dAudio_multi-stream_descriptor()` (Table 2-125), tag 0x0C.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MpegH3dAudioMultiStream {
    /// `thisStreamID` (7).
    pub this_stream_id: u8,
    /// `Some` iff `thisIsMainStream == 1`.
    pub main: Option<MultiStreamMain>,
    /// Trailing `reserved(8)` bytes, verbatim.
    pub trailing: Vec<u8>,
}

impl MpegH3dAudioMultiStream {
    pub(crate) fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let b0 = r.u8()?;
        let main = if b0 & 0x80 != 0 {
            let a = r.u8()?;
            let g = r.u8()?;
            let count = usize::from(g & 0x7F);
            let mut groups = Vec::with_capacity(count);
            for _ in 0..count {
                let e = r.u8()?;
                let in_main = e & 1 != 0;
                let auxiliary = if in_main {
                    None
                } else {
                    let x = r.u8()?;
                    Some(AuxiliaryLocation {
                        is_in_ts: x & 0x80 != 0,
                        auxiliary_stream_id: x & 0x7F,
                    })
                };
                groups.push(MultiStreamGroup {
                    group_id: e >> 1,
                    is_in_main_stream: in_main,
                    auxiliary,
                });
            }
            Some(MultiStreamMain {
                reserved_aux: ReservedOnes::from_bits(u64::from(a >> 7)),
                num_auxiliary_streams: a & 0x7F,
                reserved_groups: ReservedOnes::from_bits(u64::from(g >> 7)),
                groups,
            })
        } else {
            None
        };
        Ok(Self {
            this_stream_id: b0 & 0x7F,
            main,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        out.push((u8::from(self.main.is_some()) << 7) | f(self.this_stream_id, 7, "thisStreamID")?);
        if let Some(m) = &self.main {
            out.push(
                ((m.reserved_aux.get() as u8) << 7)
                    | f(m.num_auxiliary_streams, 7, "numAuxiliaryStreams")?,
            );
            out.push(
                ((m.reserved_groups.get() as u8) << 7) | n(m.groups.len(), 7, "mae_numGroups")?,
            );
            for g in &m.groups {
                if g.is_in_main_stream != g.auxiliary.is_none() {
                    return Err(Error::InvalidDescriptor {
                        tag: super::mpeg2_extension::TAG,
                        reason: "multi-stream group: auxiliary location must be present iff isInMainStream == 0",
                    });
                }
                out.push((f(g.group_id, 7, "mae_groupID")? << 1) | u8::from(g.is_in_main_stream));
                if let Some(a) = &g.auxiliary {
                    out.push(
                        (u8::from(a.is_in_ts) << 7)
                            | f(a.auxiliary_stream_id, 7, "auxiliaryStreamID")?,
                    );
                }
            }
        }
        out.extend_from_slice(&self.trailing);
        Ok(())
    }
}

// ───────────────────────── 2.6.116  DRC and loudness descriptor ────────────

/// Scope selected by `drcInstructionsType` / `loudnessInfoType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DrcScope {
    /// Type without an extra ID field (`drcInstructionsType` 0/1, `loudnessInfoType` 0);
    /// carries the raw 2-bit type.
    Plain(u8),
    /// An `mae_groupID` follows (`drcInstructionsType` 2, `loudnessInfoType` 1 or 2);
    /// carries the 2-bit type, the `reserved(1)` bit and the 7-bit ID.
    Group {
        /// The 2-bit type value.
        kind: u8,
        /// `reserved(1)`, default 1.
        reserved: ReservedOnes<1>,
        /// `mae_groupID` (7).
        group_id: u8,
    },
    /// An `mae_groupPresetID` follows (type 3); carries the `reserved(3)` bits and the 5-bit ID.
    Preset {
        /// `reserved(3)`, default 7.
        reserved: ReservedOnes<3>,
        /// `mae_groupPresetID` (5).
        preset_id: u8,
    },
}

impl DrcScope {
    /// Label of the scope kind.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Plain(_) => "no ID",
            Self::Group { .. } => "mae_groupID",
            Self::Preset { .. } => "mae_groupPresetID",
        }
    }
}
broadcast_common::impl_spec_display!(DrcScope);

/// One `drcInstructionsUniDrc` entry of the DRC and loudness descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DrcInstructions {
    /// `reserved(6)` before `drcInstructionsType`, default 63.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_type: ReservedOnes<6>,
    /// Type and its dependent ID (`drcInstructionsType` 0/1 plain, 2 group, 3 preset).
    pub scope: DrcScope,
    /// `reserved(2)` before `drcSetId`, default 3.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_set: ReservedOnes<2>,
    /// `drcSetId` (6).
    pub drc_set_id: u8,
    /// `reserved(1)` before `downmixId`, default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_downmix: ReservedOnes<1>,
    /// `downmixId` (7).
    pub downmix_id: u8,
    /// `reserved(3)` before `additionalDownmixIdCount`, default 7.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_additional: ReservedOnes<3>,
    /// `additionalDownmixId` list (count: 3 bits); each `(reserved(1), id(7))`.
    pub additional_downmix_ids: Vec<(u8, u8)>,
    /// `drcSetEffect` (16).
    pub drc_set_effect: u16,
    /// `bsLimiterPeakTarget` (8), present iff `limiterPeakTargetPresent`.
    pub limiter_peak_target: Option<u8>,
    /// Target-loudness block, present iff `drcSetTargetLoudnessPresent`.
    pub target_loudness: Option<DrcTargetLoudness>,
}

/// Target loudness block of a DRC set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DrcTargetLoudness {
    /// `reserved(1)` before `bsDrcSetTargetLoudnessValueUpper`, default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_upper: ReservedOnes<1>,
    /// `bsDrcSetTargetLoudnessValueUpper` (6).
    pub upper: u8,
    /// `(reserved(2), bsDrcSetTargetLoudnessValueLower(6))` iff `drcSetTargetLoudnessValueLowerPresent`.
    pub lower: Option<(u8, u8)>,
    /// `reserved(1)` before `dependsOnDrcSet`, default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_depends: ReservedOnes<1>,
    /// `dependsOnDrcSet` (6).
    pub depends_on_drc_set: u8,
    /// `noIndependentUse` when `dependsOnDrcSet == 0`; otherwise the `reserved(1)` bit.
    pub no_independent_use_or_reserved: u8,
}

/// One `loudnessInfo` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LoudnessInfoEntry {
    /// `reserved(6)` before `loudnessInfoType`, default 63.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_type: ReservedOnes<6>,
    /// Type and its dependent ID (`loudnessInfoType` 0 plain, 1/2 group, 3 preset).
    pub scope: DrcScope,
    /// The `loudnessInfo()` structure (ISO/IEC 23003-4), `loudnessInfo_size` bytes, opaque.
    pub loudness_info: Vec<u8>,
}

/// One `downmixId` definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DownmixDefinition {
    /// `reserved(1)`, default 1.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedOnes<1>,
    /// `downmixId` (7).
    pub downmix_id: u8,
    /// `downmixType` (2).
    pub downmix_type: u8,
    /// `CICPspeakerLayoutIdx` (6).
    pub cicp_speaker_layout_idx: u8,
}

/// The `mpegh3daDrcAndLoudnessInfoPresent` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DrcLoudnessInfo {
    /// `reserved(2)` before `drcInstructionsUniDrcCount`, default 3.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_drc: ReservedOnes<2>,
    /// `reserved(2)` before `loudnessInfoCount`, default 3.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_loudness: ReservedOnes<2>,
    /// `reserved(3)` before `downmixIdCount`, default 7.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_downmix: ReservedOnes<3>,
    /// DRC instructions (`drcInstructionsUniDrcCount`, 6 bits).
    pub drc_instructions: Vec<DrcInstructions>,
    /// Loudness info blocks (`loudnessInfoCount`, 6 bits).
    pub loudness_infos: Vec<LoudnessInfoEntry>,
    /// Downmix definitions (`downmixIdCount`, 5 bits).
    pub downmix_definitions: Vec<DownmixDefinition>,
}

/// `MPEG-H_3dAudio_drc_loudness_descriptor()` (Table 2-126), tag 0x0D.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MpegH3dAudioDrcLoudness {
    /// `reserved(7)` before `mpegh3daDrcAndLoudnessInfoPresent`, default 127.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedOnes<7>,
    /// Present iff `mpegh3daDrcAndLoudnessInfoPresent == 1`.
    pub info: Option<DrcLoudnessInfo>,
    /// Trailing `reserved(8)` bytes, verbatim.
    pub trailing: Vec<u8>,
}

/// Parse the scope header byte(s) following `reserved(6) type(2)`.
/// `group_types` are the type values followed by `reserved(1) mae_groupID(7)`;
/// type 3 is followed by `reserved(3) mae_groupPresetID(5)`.
fn parse_scope(r: &mut Reader<'_>, kind: u8, group_types: &[u8]) -> Result<DrcScope> {
    if group_types.contains(&kind) {
        let b = r.u8()?;
        Ok(DrcScope::Group {
            kind,
            reserved: ReservedOnes::from_bits(u64::from(b >> 7)),
            group_id: b & 0x7F,
        })
    } else if kind == 3 {
        let b = r.u8()?;
        Ok(DrcScope::Preset {
            reserved: ReservedOnes::from_bits(u64::from(b >> 5)),
            preset_id: b & 0x1F,
        })
    } else {
        Ok(DrcScope::Plain(kind))
    }
}

/// `(2-bit type, bytes after the type byte)` of a scope; validates the type is
/// legal for this list (`group_types`).
fn write_scope(s: &DrcScope, group_types: &[u8], out_tail: &mut Vec<u8>) -> Result<u8> {
    match *s {
        DrcScope::Plain(k) => {
            f(k, 2, "type")?;
            if group_types.contains(&k) || k == 3 {
                return Err(Error::InvalidDescriptor {
                    tag: super::mpeg2_extension::TAG,
                    reason: "DrcScope::Plain used with a type that requires an ID",
                });
            }
            Ok(k)
        }
        DrcScope::Group {
            kind,
            reserved,
            group_id,
        } => {
            if !group_types.contains(&kind) {
                return Err(Error::InvalidDescriptor {
                    tag: super::mpeg2_extension::TAG,
                    reason: "DrcScope::Group type does not carry an mae_groupID here",
                });
            }
            out_tail.push(((reserved.get() as u8) << 7) | f(group_id, 7, "mae_groupID")?);
            Ok(kind)
        }
        DrcScope::Preset {
            reserved,
            preset_id,
        } => {
            out_tail.push(((reserved.get() as u8) << 5) | f(preset_id, 5, "mae_groupPresetID")?);
            Ok(3)
        }
    }
}

const DRC_GROUP_TYPES: &[u8] = &[2];
const LOUDNESS_GROUP_TYPES: &[u8] = &[1, 2];

impl MpegH3dAudioDrcLoudness {
    pub(crate) fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let b0 = r.u8()?;
        let info = if b0 & 1 != 0 {
            let a = r.u8()?;
            let l = r.u8()?;
            let d = r.u8()?;
            let mut drc_instructions = Vec::new();
            for _ in 0..(a & 0x3F) {
                drc_instructions.push(parse_drc_instructions(r)?);
            }
            let mut loudness_infos = Vec::new();
            for _ in 0..(l & 0x3F) {
                let t = r.u8()?;
                let scope = parse_scope(r, t & 0x03, LOUDNESS_GROUP_TYPES)?;
                let size = usize::from(r.u8()?);
                loudness_infos.push(LoudnessInfoEntry {
                    reserved_type: ReservedOnes::from_bits(u64::from(t >> 2)),
                    scope,
                    loudness_info: r.take(size)?.to_vec(),
                });
            }
            let mut downmix_definitions = Vec::new();
            for _ in 0..(d & 0x1F) {
                let x = r.u8()?;
                let y = r.u8()?;
                downmix_definitions.push(DownmixDefinition {
                    reserved: ReservedOnes::from_bits(u64::from(x >> 7)),
                    downmix_id: x & 0x7F,
                    downmix_type: y >> 6,
                    cicp_speaker_layout_idx: y & 0x3F,
                });
            }
            Some(DrcLoudnessInfo {
                reserved_drc: ReservedOnes::from_bits(u64::from(a >> 6)),
                reserved_loudness: ReservedOnes::from_bits(u64::from(l >> 6)),
                reserved_downmix: ReservedOnes::from_bits(u64::from(d >> 5)),
                drc_instructions,
                loudness_infos,
                downmix_definitions,
            })
        } else {
            None
        };
        Ok(Self {
            reserved: ReservedOnes::from_bits(u64::from(b0 >> 1)),
            info,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        out.push(((self.reserved.get() as u8) << 1) | u8::from(self.info.is_some()));
        if let Some(i) = &self.info {
            out.push(
                ((i.reserved_drc.get() as u8) << 6)
                    | n(i.drc_instructions.len(), 6, "drcInstructionsUniDrcCount")?,
            );
            out.push(
                ((i.reserved_loudness.get() as u8) << 6)
                    | n(i.loudness_infos.len(), 6, "loudnessInfoCount")?,
            );
            out.push(
                ((i.reserved_downmix.get() as u8) << 5)
                    | n(i.downmix_definitions.len(), 5, "downmixIdCount")?,
            );
            for d in &i.drc_instructions {
                write_drc_instructions(d, out)?;
            }
            for e in &i.loudness_infos {
                let mut tail = Vec::new();
                let kind = write_scope(&e.scope, LOUDNESS_GROUP_TYPES, &mut tail)?;
                out.push(((e.reserved_type.get() as u8) << 2) | kind);
                out.extend_from_slice(&tail);
                out.push(fit_u8(e.loudness_info.len(), "loudnessInfo_size")?);
                out.extend_from_slice(&e.loudness_info);
            }
            for m in &i.downmix_definitions {
                out.push(((m.reserved.get() as u8) << 7) | f(m.downmix_id, 7, "downmixId")?);
                out.push(
                    (f(m.downmix_type, 2, "downmixType")? << 6)
                        | f(m.cicp_speaker_layout_idx, 6, "CICPspeakerLayoutIdx")?,
                );
            }
        }
        out.extend_from_slice(&self.trailing);
        Ok(())
    }
}

fn parse_drc_instructions(r: &mut Reader<'_>) -> Result<DrcInstructions> {
    let t = r.u8()?;
    let scope = parse_scope(r, t & 0x03, DRC_GROUP_TYPES)?;
    let s = r.u8()?;
    let dm = r.u8()?;
    let c = r.u8()?;
    let (limiter_present, loud_present) = (c & 0x02 != 0, c & 0x01 != 0);
    let mut additional_downmix_ids = Vec::new();
    for _ in 0..((c >> 2) & 0x07) {
        let x = r.u8()?;
        additional_downmix_ids.push((x >> 7, x & 0x7F));
    }
    let hi = r.u8()?;
    let lo = r.u8()?;
    let limiter_peak_target = if limiter_present { Some(r.u8()?) } else { None };
    let target_loudness = if loud_present {
        let u = r.u8()?;
        let lower = if u & 1 != 0 {
            let x = r.u8()?;
            Some((x >> 6, x & 0x3F))
        } else {
            None
        };
        let dep = r.u8()?;
        Some(DrcTargetLoudness {
            reserved_upper: ReservedOnes::from_bits(u64::from(u >> 7)),
            upper: (u >> 1) & 0x3F,
            lower,
            reserved_depends: ReservedOnes::from_bits(u64::from(dep >> 7)),
            depends_on_drc_set: (dep >> 1) & 0x3F,
            no_independent_use_or_reserved: dep & 1,
        })
    } else {
        None
    };
    Ok(DrcInstructions {
        reserved_type: ReservedOnes::from_bits(u64::from(t >> 2)),
        scope,
        reserved_set: ReservedOnes::from_bits(u64::from(s >> 6)),
        drc_set_id: s & 0x3F,
        reserved_downmix: ReservedOnes::from_bits(u64::from(dm >> 7)),
        downmix_id: dm & 0x7F,
        reserved_additional: ReservedOnes::from_bits(u64::from(c >> 5)),
        additional_downmix_ids,
        drc_set_effect: u16::from_be_bytes([hi, lo]),
        limiter_peak_target,
        target_loudness,
    })
}

fn write_drc_instructions(d: &DrcInstructions, out: &mut Vec<u8>) -> Result<()> {
    let mut tail = Vec::new();
    let kind = write_scope(&d.scope, DRC_GROUP_TYPES, &mut tail)?;
    out.push(((d.reserved_type.get() as u8) << 2) | kind);
    out.extend_from_slice(&tail);
    out.push(((d.reserved_set.get() as u8) << 6) | f(d.drc_set_id, 6, "drcSetId")?);
    out.push(((d.reserved_downmix.get() as u8) << 7) | f(d.downmix_id, 7, "downmixId")?);
    out.push(
        ((d.reserved_additional.get() as u8) << 5)
            | (n(
                d.additional_downmix_ids.len(),
                3,
                "additionalDownmixIdCount",
            )? << 2)
            | (u8::from(d.limiter_peak_target.is_some()) << 1)
            | u8::from(d.target_loudness.is_some()),
    );
    for &(res, id) in &d.additional_downmix_ids {
        out.push((f(res, 1, "reserved")? << 7) | f(id, 7, "additionalDownmixId")?);
    }
    out.extend_from_slice(&d.drc_set_effect.to_be_bytes());
    if let Some(p) = d.limiter_peak_target {
        out.push(p);
    }
    if let Some(t) = &d.target_loudness {
        out.push(
            ((t.reserved_upper.get() as u8) << 7)
                | (f(t.upper, 6, "bsDrcSetTargetLoudnessValueUpper")? << 1)
                | u8::from(t.lower.is_some()),
        );
        if let Some((res, lower)) = t.lower {
            out.push(
                (f(res, 2, "reserved")? << 6) | f(lower, 6, "bsDrcSetTargetLoudnessValueLower")?,
            );
        }
        out.push(
            ((t.reserved_depends.get() as u8) << 7)
                | (f(t.depends_on_drc_set, 6, "dependsOnDrcSet")? << 1)
                | f(t.no_independent_use_or_reserved, 1, "noIndependentUse")?,
        );
    }
    Ok(())
}

// ───────────────────────── 2.6.118  command descriptor ─────────────────────

/// `MPEG-H_3dAudio_command_descriptor()` (Table 2-127), tag 0x0E: one complete
/// MHAS packet (type `PACTYP_USERINTERACTION` or `PACTYP_AUDIOSCENEINFO`,
/// header included), carried opaquely.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MpegH3dAudioCommand {
    /// The MHAS packet bytes.
    pub mhas_packet: Vec<u8>,
}

// ───────────────────────── public constructors ─────────────────────────────
//
// Every struct above is `#[non_exhaustive]`, so code outside this crate cannot
// build one with a struct literal. These constructors take the spec fields and
// default every `reserved` run to its all-ones value (H.222.0 clause 2.1:
// "reserved" bits are 1), every optional part to absent and every trailing run
// to empty; assign the public fields afterwards to override a reserved value or
// to set an optional part.

impl MpegH3dAudioConfig {
    /// Wrap an opaque `mpegh3daConfig()` bitstream.
    #[must_use]
    pub fn new(mpegh3da_config: Vec<u8>) -> Self {
        Self { mpegh3da_config }
    }
}

impl SwitchGroup {
    /// Build with all-ones reserved bits.
    #[must_use]
    pub fn new(
        switch_group_id: u8,
        allow_on_off: bool,
        default_on_off: bool,
        members: Vec<SwitchGroupMember>,
        default_group_id: u8,
    ) -> Self {
        Self {
            reserved_id: ReservedOnes::FILL,
            switch_group_id,
            allow_on_off,
            default_on_off,
            reserved_members: ReservedOnes::FILL,
            members,
            reserved_default: ReservedOnes::FILL,
            default_group_id,
        }
    }
}

impl PresetConditionDetail {
    /// Build with all-ones reserved bits.
    #[must_use]
    pub const fn new(
        disable_gain_interactivity: bool,
        disable_position_interactivity: bool,
        gain: Option<u8>,
        position: Option<PresetPosition>,
    ) -> Self {
        Self {
            reserved: ReservedOnes::FILL,
            disable_gain_interactivity,
            disable_position_interactivity,
            gain,
            position,
        }
    }
}

impl PresetCondition {
    /// Build a condition; `detail` is `Some` iff `mae_groupPresetConditionOnOff == 1`.
    #[must_use]
    pub const fn new(group_id: u8, detail: Option<PresetConditionDetail>) -> Self {
        Self { group_id, detail }
    }
}

impl GroupPreset {
    /// Build with all-ones reserved bits.
    #[must_use]
    pub fn new(preset_id: u8, preset_kind: u8, conditions: Vec<PresetCondition>) -> Self {
        Self {
            reserved_id: ReservedOnes::FILL,
            preset_id,
            reserved_kind: ReservedOnes::FILL,
            preset_kind,
            reserved_conditions: ReservedOnes::FILL,
            conditions,
        }
    }
}

impl TextLabelEntry {
    /// A group description (`mae_descriptionGroupID`, 7-bit ID, 1 reserved bit set).
    #[must_use]
    pub fn group(id: u8, data: Vec<u8>) -> Self {
        Self {
            reserved: 0x01,
            id,
            data,
        }
    }

    /// A switch-group or group-preset description (5-bit ID, 3 reserved bits set).
    #[must_use]
    pub fn switch_group_or_preset(id: u8, data: Vec<u8>) -> Self {
        Self {
            reserved: 0x07,
            id,
            data,
        }
    }
}

impl TextLabelLanguage {
    /// Build with all-ones reserved bits.
    #[must_use]
    pub fn new(
        language: [u8; 3],
        groups: Vec<TextLabelEntry>,
        switch_groups: Vec<TextLabelEntry>,
        presets: Vec<TextLabelEntry>,
    ) -> Self {
        Self {
            language,
            reserved_groups: ReservedOnes::FILL,
            groups,
            reserved_switch_groups: ReservedOnes::FILL,
            switch_groups,
            reserved_presets: ReservedOnes::FILL,
            presets,
        }
    }
}

impl MpegH3dAudioTextLabel {
    /// Build with all-ones reserved bits and no trailing bytes.
    #[must_use]
    pub fn new(scene_info_id: u8, languages: Vec<TextLabelLanguage>) -> Self {
        Self {
            scene_info_id,
            reserved: ReservedOnes::FILL,
            languages,
            trailing: Vec::new(),
        }
    }
}

impl AuxiliaryLocation {
    /// `isInTS` + `auxiliaryStreamID`.
    #[must_use]
    pub const fn new(is_in_ts: bool, auxiliary_stream_id: u8) -> Self {
        Self {
            is_in_ts,
            auxiliary_stream_id,
        }
    }
}

impl MultiStreamGroup {
    /// A group carried in the main stream (no auxiliary location).
    #[must_use]
    pub const fn in_main_stream(group_id: u8) -> Self {
        Self {
            group_id,
            is_in_main_stream: true,
            auxiliary: None,
        }
    }

    /// A group carried in an auxiliary stream.
    #[must_use]
    pub const fn in_auxiliary_stream(group_id: u8, auxiliary: AuxiliaryLocation) -> Self {
        Self {
            group_id,
            is_in_main_stream: false,
            auxiliary: Some(auxiliary),
        }
    }
}

impl MultiStreamMain {
    /// Build with all-ones reserved bits.
    #[must_use]
    pub fn new(num_auxiliary_streams: u8, groups: Vec<MultiStreamGroup>) -> Self {
        Self {
            reserved_aux: ReservedOnes::FILL,
            num_auxiliary_streams,
            reserved_groups: ReservedOnes::FILL,
            groups,
        }
    }
}

impl MpegH3dAudioMultiStream {
    /// Build with no trailing bytes; `main` is `Some` iff `thisIsMainStream == 1`.
    #[must_use]
    pub fn new(this_stream_id: u8, main: Option<MultiStreamMain>) -> Self {
        Self {
            this_stream_id,
            main,
            trailing: Vec::new(),
        }
    }
}

impl DrcScope {
    /// A type that carries an `mae_groupID` (reserved bit set).
    #[must_use]
    pub const fn group(kind: u8, group_id: u8) -> Self {
        Self::Group {
            kind,
            reserved: ReservedOnes::FILL,
            group_id,
        }
    }

    /// Type 3: an `mae_groupPresetID` (reserved bits set).
    #[must_use]
    pub const fn preset(preset_id: u8) -> Self {
        Self::Preset {
            reserved: ReservedOnes::FILL,
            preset_id,
        }
    }
}

impl DrcInstructions {
    /// Build with all-ones reserved bits, no additional downmix IDs, no limiter
    /// peak target and no target-loudness block.
    #[must_use]
    pub fn new(scope: DrcScope, drc_set_id: u8, downmix_id: u8, drc_set_effect: u16) -> Self {
        Self {
            reserved_type: ReservedOnes::FILL,
            scope,
            reserved_set: ReservedOnes::FILL,
            drc_set_id,
            reserved_downmix: ReservedOnes::FILL,
            downmix_id,
            reserved_additional: ReservedOnes::FILL,
            additional_downmix_ids: Vec::new(),
            drc_set_effect,
            limiter_peak_target: None,
            target_loudness: None,
        }
    }
}

impl DrcTargetLoudness {
    /// Build with all-ones reserved bits and no lower value.
    #[must_use]
    pub const fn new(
        upper: u8,
        depends_on_drc_set: u8,
        no_independent_use_or_reserved: u8,
    ) -> Self {
        Self {
            reserved_upper: ReservedOnes::FILL,
            upper,
            lower: None,
            reserved_depends: ReservedOnes::FILL,
            depends_on_drc_set,
            no_independent_use_or_reserved,
        }
    }
}

impl LoudnessInfoEntry {
    /// Build with the all-ones reserved bits; `loudness_info` is the opaque
    /// ISO/IEC 23003-4 `loudnessInfo()` structure.
    #[must_use]
    pub fn new(scope: DrcScope, loudness_info: Vec<u8>) -> Self {
        Self {
            reserved_type: ReservedOnes::FILL,
            scope,
            loudness_info,
        }
    }
}

impl DownmixDefinition {
    /// Build with the all-ones reserved bit.
    #[must_use]
    pub const fn new(downmix_id: u8, downmix_type: u8, cicp_speaker_layout_idx: u8) -> Self {
        Self {
            reserved: ReservedOnes::FILL,
            downmix_id,
            downmix_type,
            cicp_speaker_layout_idx,
        }
    }
}

impl DrcLoudnessInfo {
    /// Build with all-ones reserved bits.
    #[must_use]
    pub fn new(
        drc_instructions: Vec<DrcInstructions>,
        loudness_infos: Vec<LoudnessInfoEntry>,
        downmix_definitions: Vec<DownmixDefinition>,
    ) -> Self {
        Self {
            reserved_drc: ReservedOnes::FILL,
            reserved_loudness: ReservedOnes::FILL,
            reserved_downmix: ReservedOnes::FILL,
            drc_instructions,
            loudness_infos,
            downmix_definitions,
        }
    }
}

impl MpegH3dAudioDrcLoudness {
    /// Build with all-ones reserved bits and no trailing bytes; `info` is
    /// `Some` iff `mpegh3daDrcAndLoudnessInfoPresent == 1`.
    #[must_use]
    pub fn new(info: Option<DrcLoudnessInfo>) -> Self {
        Self {
            reserved: ReservedOnes::FILL,
            info,
            trailing: Vec::new(),
        }
    }
}

impl MpegH3dAudioCommand {
    /// Wrap one complete MHAS packet (header included).
    #[must_use]
    pub fn new(mhas_packet: Vec<u8>) -> Self {
        Self { mhas_packet }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each reserved field is width-limited by its type, so a wider value is
    /// unrepresentable and cannot spill into the adjacent syntax bits.
    #[test]
    fn reserved_fields_cannot_hold_more_than_their_width() {
        let pos_wide = PositionInteractivity {
            reserved_min_az: ReservedOnes::from_bits(0xFF),
            min_az_offset: 0,
            reserved_max_az: ReservedOnes::from_bits(0xFF),
            max_az_offset: 0,
            reserved_min_el: ReservedOnes::from_bits(0xFF),
            min_el_offset: 0,
            reserved_max_el: ReservedOnes::from_bits(0xFF),
            max_el_offset: 0,
            min_dist_offset: 0,
            max_dist_offset: 0,
        };
        assert_eq!(pos_wide.reserved_min_az.get(), 0x01);
        assert_eq!(pos_wide.reserved_max_az.get(), 0x01);
        assert_eq!(pos_wide.reserved_min_el.get(), 0x07);
        assert_eq!(pos_wide.reserved_max_el.get(), 0x07);

        let gain_wide = GainInteractivity {
            reserved_min: ReservedOnes::from_bits(0xFF),
            min_gain: 0,
            reserved_max: ReservedOnes::from_bits(0xFF),
            max_gain: 0,
        };
        assert_eq!(gain_wide.reserved_min.get(), 0x03);
        assert_eq!(gain_wide.reserved_max.get(), 0x07);

        let group_wide = SceneGroup {
            reserved_id: ReservedOnes::from_bits(0xFF),
            group_id: 0,
            reserved_flags: ReservedOnes::from_bits(0xFF),
            allow_on_off: false,
            default_on_off: false,
            reserved_kind: ReservedOnes::from_bits(0xFF),
            content_kind: 0,
            position: None,
            gain: None,
            content_language: None,
        };
        assert_eq!(group_wide.reserved_id.get(), 0x01);
        assert_eq!(group_wide.reserved_flags.get(), 0x07);
        assert_eq!(group_wide.reserved_kind.get(), 0x0F);
    }

    #[test]
    fn drc_scope_labels_name_the_id_carried() {
        assert_eq!(DrcScope::Plain(0).name(), "no ID");
        assert_eq!(DrcScope::Plain(0).to_string(), "no ID");
        assert_eq!(DrcScope::group(2, 5).name(), "mae_groupID");
        assert_eq!(DrcScope::group(2, 5).to_string(), "mae_groupID");
        assert_eq!(DrcScope::preset(4).name(), "mae_groupPresetID");
        assert_eq!(DrcScope::preset(4).to_string(), "mae_groupPresetID");
    }

    /// A truncated text-label body reports the byte the reader wanted, so a
    /// caller sizing a refill gets the right amount.
    #[test]
    fn text_label_truncation_reports_the_next_byte_needed() {
        use crate::descriptors::mpeg2_extension::Mpeg2ExtensionDescriptor;
        use broadcast_common::Parse;

        // scene_info_id 5, one language, "eng", then the group-count byte is
        // missing: the reader stops at pos 5 of a 5-byte body.
        let bytes = [0x3F, 6, 0x0B, 0x05, 0x01, b'e', b'n', b'g'];
        match Mpeg2ExtensionDescriptor::parse(&bytes) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!(b.need, 6);
                assert_eq!(b.have, 5);
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
    }

    /// A group-description entry has 1 reserved bit ahead of its 7-bit ID; a
    /// wider reserved value must be rejected instead of shifted out silently.
    #[test]
    fn text_label_group_reserved_wider_than_one_bit_is_rejected() {
        use crate::descriptors::mpeg2_extension::{Mpeg2ExtensionBody, Mpeg2ExtensionDescriptor};
        use broadcast_common::Serialize;

        let mut group = TextLabelEntry::group(1, Vec::new());
        group.reserved = 2;
        let label = MpegH3dAudioTextLabel::new(
            5,
            vec![TextLabelLanguage::new(*b"eng", vec![group], Vec::new(), Vec::new())],
        );
        let d = Mpeg2ExtensionDescriptor::new(Mpeg2ExtensionBody::MpegH3dAudioTextLabel(label));
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf),
            Err(Error::FieldOverflow(_))
        ));

        let label = MpegH3dAudioTextLabel::new(
            5,
            vec![TextLabelLanguage::new(
                *b"eng",
                vec![TextLabelEntry::group(1, Vec::new())],
                Vec::new(),
                Vec::new(),
            )],
        );
        let d = Mpeg2ExtensionDescriptor::new(Mpeg2ExtensionBody::MpegH3dAudioTextLabel(label));
        let mut buf = vec![0u8; d.serialized_len()];
        assert_eq!(d.serialize_into(&mut buf).unwrap(), buf.len());
    }

    /// A plain scope may only name a type that carries no following ID; type 2
    /// needs an `mae_groupID` and type 3 is the preset encoding.
    #[test]
    fn drc_plain_scope_rejects_types_that_carry_an_id() {
        let mut out = Vec::new();
        assert!(matches!(
            write_scope(&DrcScope::Plain(2), DRC_GROUP_TYPES, &mut out),
            Err(Error::InvalidDescriptor { .. })
        ));
        assert!(matches!(
            write_scope(&DrcScope::Plain(3), DRC_GROUP_TYPES, &mut out),
            Err(Error::InvalidDescriptor { .. })
        ));
        out.clear();
        assert_eq!(
            write_scope(&DrcScope::Plain(0), DRC_GROUP_TYPES, &mut out).unwrap(),
            0
        );
        assert_eq!(
            write_scope(&DrcScope::Plain(1), DRC_GROUP_TYPES, &mut out).unwrap(),
            1
        );
    }

    /// The 6-bit `dependsOnDrcSet` value is shifted into its own bits and comes
    /// back unchanged.
    #[test]
    fn drc_target_loudness_packs_depends_on_drc_set() {
        use crate::descriptors::mpeg2_extension::{Mpeg2ExtensionBody, Mpeg2ExtensionDescriptor};
        use broadcast_common::{Parse, Serialize};

        let mut drc = DrcInstructions::new(DrcScope::Plain(0), 1, 2, 0);
        drc.target_loudness = Some(DrcTargetLoudness::new(10, 0x2A, 0));
        let info = DrcLoudnessInfo::new(vec![drc], Vec::new(), Vec::new());
        let d = Mpeg2ExtensionDescriptor::new(Mpeg2ExtensionBody::MpegH3dAudioDrcLoudness(
            MpegH3dAudioDrcLoudness::new(Some(info)),
        ));
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[14], (1 << 7) | (0x2A << 1));
        let Mpeg2ExtensionBody::MpegH3dAudioDrcLoudness(back) =
            &Mpeg2ExtensionDescriptor::parse(&buf).unwrap().body
        else {
            panic!("expected DRC/loudness body");
        };
        let target = back
            .info
            .as_ref()
            .unwrap()
            .drc_instructions
            .first()
            .unwrap()
            .target_loudness
            .unwrap();
        assert_eq!(target.depends_on_drc_set, 0x2A);
    }
}
