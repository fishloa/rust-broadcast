# ISO/IEC 14496-3:2001 — AAC AudioSpecificConfig (`esds` audio config)

The audio decoder-config record carried in an MP4 `mp4a` sample entry's `esds`
(`DecoderSpecificInfo` when `objectTypeIndication == 0x40`), and the source for an ADTS header on
TS→MP4 transmux. Source: `specs/fulltext/iso_iec_14496-3_aac_2001.md` (text-layer pdf2md; AOT table
vision-verified from PDF p11). Cites by spec § + PDF page.

> **HE-AAC (SBR AOT 5 / PS AOT 29) — #393 resolved.** The later **text-layer** edition
> **`specs/iso_iec_14496-3_2009_audio_sbr_ps.pdf`** now grounds the SBR + Parametric-Stereo
> extension signaling (folded in by amendment; SBR/PS/AudioSpecificConfig all present in the
> text layer). Free cross-check: 3GPP TS 26.401. Transmux HE-AAC (#432) grounds against this.

## AudioSpecificConfig — §1.6.2.1, Table 1.8 (PDF p33)

```
AudioSpecificConfig() {
  audioObjectType;                        // 5 bslbf
  samplingFrequencyIndex;                 // 4 bslbf
  if (samplingFrequencyIndex == 0xf)
    samplingFrequency;                    // 24 uimsbf  (explicit rate escape)
  channelConfiguration;                   // 4 bslbf
  // object-specific config follows, selected by audioObjectType:
  if (AOT in {1,2,3,4,6,7})  GASpecificConfig();          // AAC Main/LC/SSR/LTP, AAC Scalable, TwinVQ
  if (AOT == 8)  CelpSpecificConfig();                    // subpart 3
  if (AOT == 9)  HvxcSpecificConfig();                    // subpart 2
  if (AOT == 12) TTSSpecificConfig();                     // subpart 6
  if (AOT in {13,14,15,16}) StructuredAudioSpecificConfig();
  if (AOT in {17,19,20,21,22,23}) GASpecificConfig();     // ER GA types
  if (AOT == 24) ErrorResilientCelpSpecificConfig();
  if (AOT == 25) ErrorResilientHvxcSpecificConfig();
  if (AOT in {26,27}) ParametricSpecificConfig();
  if (AOT in 17..27) { epConfig; /*2 bslbf*/ if (epConfig in {2,3}) ErrorProtectionSpecificConfig();
                       if (epConfig == 3) { directMapping; /*1*/ } }
}
```
- `audioObjectType` 5-bit "master switch" selecting the audio bitstream syntax (§1.6.3.1). *(Note:
  later editions add an escape `if (AOT==31) AOT = 32 + audioObjectTypeExt(6)` — not in this 2001
  base text.)*
- For AAC/transmux the common path is `GASpecificConfig()` (AOT 1/2/3/4) defined in **subpart 4**
  (§4.4.1) — carries `frameLengthFlag`, `dependsOnCoreCoder`(+`coreCoderDelay`), `extensionFlag`.

## audioObjectType (AOT) — §1.5.1.1, Table 1.1 (PDF p11, vision-verified)

`0` Null · `1` AAC Main · `2` AAC LC · `3` AAC SSR · `4` AAC LTP · **`5` Reserved** · `6` AAC Scalable ·
`7` TwinVQ · `8` CELP · `9` HVXC · `10`–`11` Reserved · `12` TTSI · `13` Main synthetic ·
`14` Wavetable synthesis · `15` General MIDI · `16` Algorithmic Synthesis & Audio FX · `17` ER AAC LC ·
`18` Reserved · `19` ER AAC LTP · `20` ER AAC Scalable · `21` ER TwinVQ · `22` ER BSAC · `23` ER AAC LD ·
`24` ER CELP · `25` ER HVXC · `26` ER HILN · `27` ER Parametric · `28`–`31` Reserved.
- AAC Main/LC/SSR/LTP bitstreams are syntax-compatible with the ISO/IEC 13818-7 (MPEG-2 AAC) profiles
  (§1.5.1.2). **`5` (SBR) / `29` (PS) are NOT in this 2001 base edition** — they were added by later
  amendments; needed for HE-AAC/HE-AACv2 transmux, vendor a later 14496-3 to ground them.

## samplingFrequencyIndex — §1.6.3.3, Table 1.10 (PDF p33)

`0x0` 96000 · `0x1` 88200 · `0x2` 64000 · `0x3` 48000 · `0x4` 44100 · `0x5` 32000 · `0x6` 24000 ·
`0x7` 22050 · `0x8` 16000 · `0x9` 12000 · `0xa` 11025 · `0xb` 8000 · `0xc` 7350 · `0xd`–`0xe` Reserved ·
`0xf` **escape** → a 24-bit explicit `samplingFrequency` field follows in the ASC.

## channelConfiguration — §1.6.3.4, Table 1.11 (PDF p34)

`0` config in GASpecificConfig (program_config_element) · `1` mono (1 ch, SCE) · `2` stereo (2, CPE) ·
`3` 3ch (SCE+CPE) · `4` 4ch (SCE+CPE+SCE) · `5` 5ch (SCE+2·CPE) · `6` 5.1 (SCE+2·CPE+LFE) ·
`7` 7.1 (SCE+3·CPE+LFE) · `8`–`15` Reserved.

## Implications for our crates
- The `mp4a` sample entry's `esds` `DecoderSpecificInfo` (OTI `0x40`) **is** an `AudioSpecificConfig`
  (see `mp4-esds-rules.md`). A TS→MP4 transmux of ADTS-framed AAC builds the ASC from the ADTS header:
  ADTS `profile` = `audioObjectType − 1`; ADTS `sampling_frequency_index` = ASC
  `samplingFrequencyIndex`; ADTS `channel_configuration` = ASC `channelConfiguration`. Reverse for
  MP4→TS (ADTS framing).
- Parse the first 2 bytes for AOT(5)+freqIdx(4)+chanCfg(4); handle the `0xf` 24-bit explicit-rate
  escape; for AAC (AOT 1/2/3/4) consume `GASpecificConfig` (subpart 4 §4.4.1). Treat unknown AOT
  payloads as opaque.
