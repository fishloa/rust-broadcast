# MPEG-H 3D Audio in ISOBMFF — mha1/mhm1 + mhaC (#433)

Container-level only. Sources: ISO/IEC 23008-3 §20 (paid, canonical) for the record
syntax; **ATSC A/342-3** (vendored `specs/atsc_a342-3_2025_mpegh_system.pdf`, free) for
the mhaC profile/level constraints + MHAS packetisation (`mpegh-atsc-a342-3.md`).

## Sample entries
- `mha1` / `mha2` — MHASampleEntry: each sample is one `mpegh3daFrame`; config carried
  in a **required** `mhaC` box.
- `mhm1` / `mhm2` — MHASampleEntry with an in-band MHAS bitstream (config in `mdat`);
  `mhaC` optional.

```
class MHAConfigurationBox extends Box('mhaC') { MHADecoderConfigurationRecord(); }
aligned(8) class MHADecoderConfigurationRecord {
    unsigned int(8)  configurationVersion;               // = 1
    unsigned int(8)  mpegh3daProfileLevelIndication;     // CICP profile-level (ATSC §MHADecoderConfigurationRecord)
    unsigned int(8)  referenceChannelLayout;             // CICP ChannelConfiguration
    unsigned int(16) mpegh3daConfigLength;
    unsigned int(8)  mpegh3daConfig[mpegh3daConfigLength];   // opaque mpegh3daConfig() blob
}
class MHASampleEntry(type) extends AudioSampleEntry(type) {  // 'mha1'|'mha2'|'mhm1'|'mhm2'
    MHAConfigurationBox();      // 'mhaC' (mandatory for mha1/mha2)
    // optional: MHAStreamGroupInfoBox, MHAProfileAndLevelCompatibilitySetBox 'mhaP', btrt
}
```
`mpegh3daConfig` is an opaque blob to the container (transmux copies it through);
transmux parses only the record wrapper. rfc6381: `mhm1.0xNN` (profile-level hex).

## Fixture (fetch-recipe — not vendored)
No local MPEG-H encoder; Fraunhofer test content is Git-LFS + license-restricted, so
NOT vendored (like the Dolby AC-4 kit). Fetch a real `mha1`/`mhm1` mp4 to derive the
mhaC oracle from:
- Fraunhofer-IIS/mpegh-test-content (GitHub, Git-LFS): `TRI_Fileset_17_..._audio.mp4`
- DASH-IF MCA: `https://dash.akamaized.net/dash264/TestCasesMCA/fraunhofer/…`
Then extract the `mhaC` body as the gate oracle.
