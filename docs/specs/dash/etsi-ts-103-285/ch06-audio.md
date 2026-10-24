---
source: private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
document: ETSI TS 103 285 V1.4.1 (2023-09)
clauses: 6 DASH specific aspects for audio (6.1-6.8)
pdf_pages: 40-57 (PDF page = printed page)
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch06-audio.md
verified_against_render: true
notes_original_wording: true
---

# TS 103 285 - clause 6 (notes): audio

Original-wording notes. Base semantics of Role/Accessibility/Preselection: ISO/IEC 23009-1
(`transmux/docs/codec/dash-23009-1-2022-ids.md`). Box-level codec notes: `transmux/docs/codec/` (`ac4-dac4.md`,
`mpegh-mhaC.md`, `dts-isobmff-etsi102114.md`, `ac3-eac3-mp4.md`).

## 6.1 Common rules
### 6.1.1 AdaptationSets / Representations (pp. 40-41)
- A player may skip an AdaptationSet whose Representations' four-character codec prefixes differ from each other or
  from the set's own `@codecs`.
- Every audio Representation (except NGA auxiliary streams, see 6.7.3) shall define or inherit (Table 4):

| Attribute | Rule across the Representations of a set |
|---|---|
| `@mimeType` | shall be identical (e.g. `audio/mp4`) |
| `@codecs` | should be identical for seamless switching (clause 10.4) |
| `@audioSamplingRate` | should be identical |
| `AudioChannelConfiguration` | should be identical |

### 6.1.2 Role scheme for AdaptationSets (pp. 41-43)
Applies when Preselections are not used (otherwise 6.7).
- Each audio AdaptationSet shall have at least one Role with scheme `urn:mpeg:dash:role:2011`; `main` marks the
  provider's preferred set. With more than one audio set, at least one shall be `main`; several `main` sets shall be
  told apart by `@lang`, `@codecs` or AudioChannelConfiguration; a lone `main` set may omit Role. The player picks among
  several `main` sets and uses non-`main` only if all are unsuitable. Several roles need several Role elements.
- Same codec, translated soundtrack: only the original language is `main`, translations `dub`. Same codec, different
  original soundtracks (e.g. multilingual commentary): all `main`. Several codecs, same sound and language, no codec
  preference: several `main` sets let the player choose. Evaluate `@lang` to match the user's language.
- Audio description (AD) and clean audio need both Role and Accessibility (scheme
  `urn:tva:metadata:cs:AudioPurposeCS:2007`, defined in TV-Anytime):

| Kind | Role value | Accessibility value |
|---|---|---|
| Broadcast-mix AD | `alternate` | 1 (visually impaired) |
| Receiver-mix AD | `commentary` | 1 |
| Clean audio | `alternate` | 2 (hard of hearing) |

  Receiver-mix AD shall use `@dependencyId` on the related set's Representations (so it is never offered standalone).
  Players should ignore audio with Role/Accessibility values they do not understand.
- Table 6 (examples) shows: multilingual set with English original = en `main`, fr/de `dub`; channel layouts of one
  language = 5.1 `main`, 2.0 and 7.1 `alternate`; multi-codec offers (`ec-3`, `dtsh`, `mp4a.40.5`/`.40.2`) with
  preferred codec(s) `main`; AD/clean-audio pairs; a French-original programme for UK viewers where English is both
  `main` and `dub` (two Role elements). Rates written like "44,1 kHz" use a decimal comma.

## 6.2 HE-AACv2 (pp. 43-44)
Basic stereo profile is MPEG-4 HE-AAC v2 level 2 (so AAC-LC and HE-AAC decode too); SBR/PS shall be signalled
explicitly (backward-compatible signalling); every (sub)segment starts with SAP 1 (per TR 23009-3);
AudioChannelConfiguration required (either scheme of 23009-1 5.8.4.7); loudness/DRC info should be embedded and
decoders shall parse `dynamic_range_info()` and `MPEG4_ancillary_data()`. Codec strings (Table 7, MIME `audio/mp4`,
14496-14 encapsulation, SAP 1): AAC `mp4a.40.2`, HE-AAC `mp4a.40.5`, HE-AACv2 `mp4a.40.29`; the two HE strings
imply AAC-LC (and for .29 also SBR) are present.

## 6.3 Dolby (pp. 44-47)
- E-AC-3 / AC-4 part 1 (Table 8): `audio/mp4`, SAP 1; E-AC-3 `@codecs`=`ec-3` (TS 102 366 annex F); AC-4 =
  `ac-4.aa.bb.cc` (three two-digit hex fields; TS 103 190-1 annex E encapsulation, annex F signalling - the spec cites
  both). AC-4 fragments shall begin with a sync frame (TS 103 190-1 E.5). For E-AC-3/AC-4 part 1,
  AudioChannelConfiguration uses scheme `tag:dolby.com,2014:dash:audio_channel_configuration:2011`, `@value` = 4 hex
  digits of the 16-bit channel map (TS 102 366 Table E.5, left channel = MSB; L,C,R,Ls,Rs,LFE = `F801`).
- AC-4 part 2 (6.3.2): configuration (DSI) fields drive manifest values; `bitstream_version` 0 = TS 103 190-1,
  2 = TS 103 190-2; new content should use version 2. Follow TS 101 154 6.7.2/6.7.4.1 (PES parts apply to DASH
  Representations; the AD_descriptor rule is ignored); an AdaptationSet shall not carry audio above 60 Hz frame rate
  unless all video sets in the Period are above 60 Hz. SRSP/SRMP: TS 101 154 6.7.4.2; MRMP: 6.7.4.3; DRC/loudness 6.7.5;
  dialogue enhancement 6.7.6.
- MRMP packaging (6.3.2.5): corresponding segments/subsegments across the contributing sets shall have equal durations
  and aligned timestamps; same `frame_rate_index`; synchronized sequence numbers; `b_multi_pid` = 1 in every contributing
  `ac4_presentation_v1_info`; presentation elements identical apart from substream-group data (`presentation_id` match).
- Table 9, AC-4 attribute mapping (usable on AdaptationSet and Preselection):

| Item | Rule |
|---|---|
| `@codecs` | `ac-4.<bitstream_version>.<presentation_version>.<mdcompat>` from the DSI (example `ac-4.02.01.03`) |
| `Preselection@tag` | the DSI `presentation_group_index` of the presentation |
| AudioChannelConfiguration | version 2: either `urn:mpeg:mpegB:cicp:ChannelConfiguration` (recommended, for layouts mappable per TS 103 190-2 G.3.2) or `tag:dolby.com,2015:dash:audio_channel_configuration:2015`; version 0: the 2014 Dolby scheme with the hex map as above |
| `@audioSamplingRate` | derived from `fs_index` and `dsi_sf_multiplier` |
| `@mimeType` / `@startWithSAP` | `audio/mp4` / 1 |
| `@lang` | should follow the `language_tag_bytes` of the "dialog" or "complete main" content classifier |
| Role | author's choice (the content classifier is insufficient for whole-experience Preselections) |
| Accessibility | shall flag AD for the visually impaired (AudioPurposeCS) when a component's classifier is 2; may flag "hard of hearing" (dialogue-level adjustable) when a component is not music-and-effects (classifier != 1) |
| SupplementalProperty | headphone-tailored content should use the Immersive Audio for Headphones descriptor (TS 103 190-2 G.2.12.1); an audio frame-rate descriptor should be used (G.2.12.2) |
| Label | author's choice |

- 6.3.3: within one Representation `frame_rate_index`, `presentation_config`, `channel_mode` and `content_classifier`
  shall stay constant; each segment and subsegment starts with an I-frame.

## 6.4 DTS (pp. 47-48)
Table 10 (`audio/mp4`, SAP 1): `dtsc` core, `dtsh` DTS-HD with core, `dtse` DTS-HD LBR, `dtsl` DTS-HD lossless
(all TS 102 114 annex E encapsulation, DASH rules annex G); `dtsx` DTS-UHD Profile 2, `dtsy` Profile 3 (TS 103 491
annex B, DASH rules annex D). All streams follow TS 101 154. DTS-UHD: Profile 2 = channel-based, up to 9 full-band + 1
LFE, always SRSP; Profile 3 adds SRMP/MRMP, objects, up to 4th-order Ambisonics (a player renders >= 16 waveforms and
2 LFE). Presentations are chosen by presentation ID; the TS 101 154 `BroadcastChunk` is unnecessary in DASH and a player
ignores it if present. Accessibility roles come from `ucAssociatedAssetType` (should be set). Decoding starts only at a
sync frame.

## 6.5 MPEG Surround (p. 48)
Down-mix coded as HE-AAC v2 (6.2); Baseline profile level 4; each (sub)segment starts at SAP 1; `audio/mp4`,
`@codecs` `mp4a.40.30` (implies AOT 2 + 5 + 30).

## 6.6 Service continuity (pp. 48-49)
Under tight bandwidth (~500 kbit/s total) audio should scale too (64-96 kbit/s suggested). The main audio set should
reach down to the lowest needed rate; below 96 kbit/s support for HE-AACv2, MPEG-H, AC-4 or DTS-UHD is advised. If the
main codec cannot scale down, add a fallback audio AdaptationSet marked with SupplementalProperty
`urn:dvb:dash:fallback_adaptation_set:2014`, `@value` = the `@id` of the set it backs up, carrying the same Role; options:
same codec at lower layout or another codec (e.g. HE-AACv2 mono/stereo). Players should pick it under low bandwidth, but
seamless switching between differently configured sets is not expected.

## 6.7 NGA Preselections (pp. 49-54)
- Scope: AC-4, DTS-UHD, MPEG-H. Component-based audio means a stream (or several) carries parts of a Preselection, so
  per-stream properties are not meaningful. Pre-mixed single-experience streams (SRSP, "full-main") shall use plain
  AdaptationSet signalling (6.1), not Preselections.
- 6.7.3 Players supporting SRMP/MRMP shall handle `Preselection` elements; each Preselection in a bundle should have one.
  Role/Accessibility/`@codecs` on a Preselection describe that Preselection only. AdaptationSets referenced by
  Preselections: auxiliary-stream sets shall carry an EssentialProperty Preselection descriptor and need not (the spec
  says "may not") carry `@lang`, Role, Accessibility or AudioChannelConfiguration; the main-stream set may carry a
  SupplementalProperty Preselection descriptor. Clause 6.1.1's attribute table does not apply to auxiliary streams.
- 6.7.4 Each Preselection shall have a Role (`urn:mpeg:dash:role:2011`); with several in a bundle at least one is `main`,
  several `main` ones must differ (`@lang`; the spec writes `@codec`). Original language `main`, translations `dub`;
  differing original languages all `main`. `@selectionPriority` (23009-1) is available.
- 6.7.5 Accessibility Preselections use the Table-12 values (broadcast-mix AD: `alternate` + 1; clean audio:
  `alternate` + 2; no receiver-mix row). Ignore unknown Role/Accessibility. Dependencies shall use the Preselection's
  own mechanism, never `@dependencyId`.
- 6.7.6 Examples (Tables 13-16), each for AC-4 / MPEG-H / DTS-UHD with five Preselections (primary 5.1.2 `main`,
  AD `commentary` + Accessibility 1, clean audio `alternate` + 2, French `dub` 2.0, stereo `main` 2.0):
  - SRMP (one AdaptationSet, id 1, Supplemental Preselection descriptor): all Preselection `@preselectionComponents`
    = 1; codecs `ac-4.02.01.02/.03/.03/.01/.01`, `mhm1.0x0C`, `dtsy`; `@tag` 101-105 (AC-4, MPEG-H) or P1-P5 (DTS-UHD).
  - MRMP (four AdaptationSets, set 1 main with Supplemental descriptor, sets 2-4 Essential descriptor, no
    lang/role/accessibility): AD uses components "1 2", clean audio "1 3", French "1 4"; MPEG-H uses `mhm2.0x0C`; DTS-UHD
    `@tag` values are P1, P1, P2, P3, P4 as printed (the AD and primary rows share P1); one row label is misspelt in the spec.

## 6.8 MPEG-H Audio (pp. 54-57)
- Elementary stream, MHAS encapsulation, multi-stream, preselection and loudness rules are by reference to TS 101 154
  6.8.2, 6.8.3, 6.8.7, 6.8.6, 6.8.8 (config change: 6.8.5). `PACTYP_SYNC` and `PACTYP_SYNCGAP` packets may appear and
  need not be parsed.
- Sample entry: `mhm1` for SRSP/SRMP (ISO/IEC 23008-3 20.6); `mhm2` for MRMP (14.6). If `MHAConfigurationBox` is
  present, `mpegh3daProfileLevelIndication` is 0x0B / 0x0C / 0x0D for LC profile level 1 / 2 / 3 and the record must
  match the bitstream.
- Sync samples (random access): no `PACTYP_SYNC` packet; packets in order `PACTYP_MPEGH3DACFG`, `PACTYP_AUDIOSCENEINFO`
  (if present), `PACTYP_BUFFERINFO`, `PACTYP_MPEGH3DAFRAME`; the frame carries `AudioPreRoll()` (IPF) with `configLen` 0
  (config is in the CFG packet); 23008-3 20.6.1 rules apply; first sample of each segment/subsegment is a sync sample;
  `trun` flag `sample_is_non_sync_sample` = 0 for sync samples, 1 otherwise.
- Config change: at a segment start or mid-segment, in the latter case on a sync sample; the changing sample and the
  one before it may carry `PACTYP_AUDIOTRUNCATION`. `MHASPacketLabel` values must differ across all Representations of a
  programme; in MRMP each stream uses its own range (main 1-16, first auxiliary 17-32, ...).
- Table 17 mapping: `@codecs` = `mhm1.` + `0x`profile-level-id (example `mhm1.0x0D`; Tables 13-16 use `0x0C`, and
  MRMP uses `mhm2` - spec inconsistency in the example); `Preselection@tag` = `mae_groupPresetID`;
  AudioChannelConfiguration scheme `urn:mpeg:mpegB:cicp:ChannelConfiguration` with values 1-7, 9-12, 14-17 or 19
  (ISO/IEC 23091-3 Table 3); `@audioSamplingRate` from the USAC sampling-frequency fields; `@startWithSAP`=1;
  `@mimeType`=`audio/mp4`; `@lang` from `mae_contentLanguage` of the default dialogue group; Role/Label author's choice;
  Accessibility: shall flag AD (visually impaired) when any component's `mae_contentKind` is 9; may flag hard-of-hearing
  when the dialogue (kind 2) components allow gain interactivity with non-zero maximum gain.
