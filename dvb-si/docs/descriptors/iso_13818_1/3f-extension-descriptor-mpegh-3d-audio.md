## Extension descriptor (tag 0x3F) and the MPEG-H 3D audio descriptors
_Rec. ITU-T H.222.0 (06/2021, 08/2023 text) | ISO/IEC 13818-1 clause 2.6.90 (Table 2-109, Table 2-110) and clauses 2.6.106-2.6.118 (Tables 2-121…2-127). Transcribed from `private/specs-md/itu_t_h222_0_202308_mpeg2_systems.md`. Tag 63 (Table 2-45), TS and PS. Carried in the PMT ES_info loop of an MPEG-H 3D audio stream (`stream_type` 0x2D main / 0x2E auxiliary; ETSI TS 101 154 V2.10.1 §6.8, §4.1.8.31)._

```
Extension_descriptor() {
  descriptor_tag(8)=0x3F  descriptor_length(8)  extension_descriptor_tag(8)
  if (extension_descriptor_tag == 0x08) MPEG-H_3dAudio_descriptor()
  else if (== 0x09) MPEG-H_3dAudio_config_descriptor()
  else if (== 0x0A) MPEG-H_3dAudio_scene_descriptor()
  else if (== 0x0B) MPEG-H_3dAudio_text_label_descriptor()
  else if (== 0x0C) MPEG-H_3dAudio_multi-stream_descriptor()
  else if (== 0x0D) MPEG-H_3dAudio_drc_loudness_descriptor()
  else if (== 0x0E) MPEG-H_3dAudio_command_descriptor()
  ...
}
```

Table 2-110 `extension_descriptor_tag`: 0 reserved; 1 forbidden (PS); 2 ODUpdate; 3 HEVC_timing_and_HRD; 4 af_extensions; 5 HEVC_operation_point; 6 HEVC_hierarchy_extension; 7 Green_extension; 8 MPEG-H_3dAudio; 9 _config; 0x0A _scene; 0x0B _text_label; 0x0C _multi-stream; 0x0D _drc_loudness; 0x0E _command; 0x0F Quality_extension; 0x10 Virtual_segmentation; 0x11 timed_metadata_extension; 0x12 HEVC_tile_substream; 0x13 HEVC_subregion; 0x14 JXS_video; 0x15 VVC_timing_and_HRD; 0x16 EVC_timing_and_HRD; 0x17 LCEVC_video; 0x18 LCEVC_linkage; 0x19 Media_service_kind; 0x1A..0xFF reserved.

### MPEG-H_3dAudio_descriptor (Table 2-121)

| Syntax | Bits | Mnemonic |
|---|---|---|
| mpegh3daProfileLevelIndication | 8 | uimsbf |
| interactivityEnabled | 1 | bslbf |
| compatibleProfileSetsPresent | 1 | bslbf |
| reserved | 8 | bslbf |
| referenceChannelLayout | 6 | uimsbf |
| if (compatibleProfileSetsPresent == '0') { numCompatibleSets 8; for (n<numCompatibleSets) CompatibleSetIndication 8 } | | |
| for (i<N) reserved | 8 | bslbf |

`compatibleProfileSetsPresent` "when set to '0' indicates the presence of compatible profile sets" (2.6.107). The printed 8-bit `reserved` straddles the byte boundary (6 + 2 bits). **Verified on real data** (Fraunhofer-IIS mpegh-test-content PMTs): `3F 04 08 10 7F C1` (baseline, flag = 1, no sets), `3F 06 08 0B 3F C1 01 10` (LC profile, flag = 0, one compatible set 0x10); reserved bits are all ones.

### MPEG-H_3dAudio_config_descriptor (Table 2-122)
`mpegh3daConfig()` (ISO/IEC 23008-3 5.2.2.1), nothing else. Carried opaquely.

### MPEG-H_3dAudio_scene_descriptor (Table 2-123)
Flags `groupDefinitionPresent`, `switchGroupDefinitionPresent`, `groupPresetDefinitionPresent` (1 bit each), `reserved(5)`, `3dAudioSceneInfoID(8)`; then, if present: groups (`reserved(1) numGroups(7)`; per group `reserved(1) mae_groupID(7)`, `reserved(3) mae_allowOnOff mae_defaultOnOff mae_allowPositionInteractivity mae_allowGainInteractivity mae_hasContentLanguage`, `reserved(4) mae_contentKind(4)`, optional position block (`reserved(1) MinAz(7)`, `reserved(1) MaxAz(7)`, `reserved(3) MinEl(5)`, `reserved(3) MaxEl(5)`, `MinDist(4) MaxDist(4)`), optional gain block (`reserved(2) MinGain(6)`, `reserved(3) MaxGain(5)`), optional `mae_contentLanguage(24)`); switch groups (`reserved(3) numSwitchGroups(5)`; per group `reserved(1) mae_switchGroupID(5) AllowOnOff DefaultOnOff`, `reserved(3) mae_bsSwitchGroupNumMembers(5)`, (members+1) × `reserved(1) memberID(7)`, `reserved(1) mae_switchGroupDefaultGroupID(7)`); presets (`reserved(3) mae_numGroupPresets(5)`; per preset `reserved(3) ID(5)`, `reserved(3) Kind(5)`, `reserved(4) mae_numGroupPresetConditions(4)`, (n+1) conditions of `mae_groupPresetGroupID(7) ConditionOnOff(1)` and, when on, `reserved(4) DisableGainInteractivity GainFlag DisablePositionInteractivity PositionFlag`, optional `Gain(8)`, optional `AzOffset(8) reserved(2) ElOffset(6) reserved(4) DistFactor(4)`); trailing `reserved(8)`. The semantics say `numSwitchGroups` is "the number of switch groups minus one" while the loop is `i < numSwitchGroups`; the loop is followed.

### MPEG-H_3dAudio_text_label_descriptor (Table 2-124)
`3dAudioSceneInfoID(8) reserved(4) numDescLanguages(4)`; per language `descriptionLanguage(24)`, `reserved(1) numGroupDescriptions(7)`, entries `reserved(1) mae_descriptionGroupID(7) groupDescriptionDataLength(8) data`, `reserved(3) numSwitchGroupDescriptions(5)`, entries `reserved(3) ID(5) length(8) data`, `reserved(3) numGroupPresetsDescriptions(5)`, entries `reserved(3) ID(5) length(8) data`; trailing `reserved(8)`. Description data is UTF-8 (ISO/IEC 10646).

### MPEG-H_3dAudio_multi-stream_descriptor (Table 2-125)
`thisIsMainStream(1) thisStreamID(7)`; if main: `reserved(1) numAuxiliaryStreams(7)`, `reserved(1) mae_numGroups(7)`, per group `mae_groupID(7) isInMainStream(1)` and, **when `isInMainStream == 0`** (the printed condition `thisIsMainStream == '0'` is unsatisfiable inside `if (thisIsMainStream)`), `isInTS(1) auxiliaryStreamID(7)`; trailing `reserved(8)`. TSDuck `tstabcomp` binaries agree: `3F 09 0C 8F 8B 83 25 26 DA 28 5B`.

### MPEG-H_3dAudio_drc_loudness_descriptor (Table 2-126)
`reserved(7) mpegh3daDrcAndLoudnessInfoPresent(1)`; if present: `reserved(2) drcInstructionsUniDrcCount(6)`, `reserved(2) loudnessInfoCount(6)`, `reserved(3) downmixIdCount(5)`; per DRC instruction `reserved(6) drcInstructionsType(2)` (+ `reserved(1) mae_groupID(7)` for type 2, `reserved(3) mae_groupPresetID(5)` for type 3), `reserved(2) drcSetId(6)`, `reserved(1) downmixId(7)`, `reserved(3) additionalDownmixIdCount(3) limiterPeakTargetPresent drcSetTargetLoudnessPresent`, additional ids (`reserved(1) id(7)`), `drcSetEffect(16)`, optional `bsLimiterPeakTarget(8)`, optional target-loudness block (`reserved(1) Upper(6) LowerPresent(1)`, optional `reserved(2) Lower(6)`, `reserved(1) dependsOnDrcSet(6) noIndependentUse/reserved(1)`); per loudness info `reserved(6) loudnessInfoType(2)` (+ group id for types 1/2, preset id for type 3), `loudnessInfo_size(8)`, `loudnessInfo()`; per downmix `reserved(1) downmixId(7) downmixType(2) CICPspeakerLayoutIdx(6)`; trailing `reserved(8)`.

### MPEG-H_3dAudio_command_descriptor (Table 2-127)
One complete MHAS packet (type `PACTYP_USERINTERACTION` or `PACTYP_AUDIOSCENEINFO`), opaque.
