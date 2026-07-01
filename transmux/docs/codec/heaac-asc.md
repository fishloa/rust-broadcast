# HE-AAC AudioSpecificConfig SBR/PS signaling (#432)

Container-level only: transmux must **parse/emit the ASC** carrying explicit SBR/PS
signaling (and set rfc6381 `mp4a.40.5` / `mp4a.40.29`) — it does NOT decode SBR/PS.

Sources: ISO/IEC 14496-3 §1.6.2 (AudioSpecificConfig) + Amd 1 (SBR) / Amd 2 (PS);
free authoritative: **3GPP TS 26.401** (Enhanced aacPlus, 3gpp.org) + wiki.multimedia.cx.
Fixtures: `fixtures/ts/heaac/heaac_v1.mp4` (SBR), `heaac_v2.mp4` (SBR+PS), via macOS
AudioToolbox (`ffmpeg -c:a aac_at -profile:a 4|28`). ASC oracle (DecSpecificInfo):
v1 = `13 90 56 e5 a0`, v2 = `13 88 56 e5 a5 48`.

```
AudioSpecificConfig() {
  audioObjectType;                       // GetAudioObjectType(): 5 bits, or 31 + 6 bits
  samplingFrequencyIndex;                // 4 bits; if ==0xF → 24-bit samplingFrequency
  channelConfiguration;                  // 4 bits
  // --- explicit hierarchical SBR/PS signaling (HE-AAC) ---
  if (audioObjectType == 5 || audioObjectType == 29) {   // 5=SBR, 29=PS
    extensionSamplingFrequencyIndex;     // 4 bits (+24-bit escape)
    if (audioObjectType == 5) sbrPresentFlag = 1;
    audioObjectType = GetAudioObjectType();   // the core AOT (usually 2 = AAC-LC)
  }
  GASpecificConfig();                    // core AAC-LC config
  // --- explicit backward-compatible extension (trailing) ---
  // syncExtensionType (11 bits):
  //   0x2B7 → extensionAudioObjectType = GetAudioObjectType();
  //            if ==5 { sbrPresentFlag(1); if set: extensionSamplingFrequencyIndex(4);
  //                     if bits>=12: syncExtensionType(11)==0x548 → psPresentFlag(1) }
  //   0x548 → psPresentFlag(1)
}
```
AudioObjectType table (subset): 2=AAC-LC, 5=SBR, 29=PS (HE-AAC v2), 22=BSAC, 23=ER-AAC-LD.
rfc6381: `mp4a.40.<AOT>` — AAC-LC=`mp4a.40.2`, HE-AAC v1=`mp4a.40.5`, HE-AAC v2=`mp4a.40.29`.
