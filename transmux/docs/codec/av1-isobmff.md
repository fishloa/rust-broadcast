# AV1 in ISOBMFF — av01 sample entry + av1C (#436)

Source: AOMedia "AV1 Codec ISO Media File Format Binding" (https://aomediacodec.github.io/av1-isobmff/, free).
Fixture: `fixtures/mp4/av1.mp4`; oracle av1C body `81000c000a0b000000043cffbc02f80040`.

```c
class AV1SampleEntry extends VisualSampleEntry('av01') { AV1CodecConfigurationBox config; }

class AV1CodecConfigurationBox extends Box('av1C') { AV1CodecConfigurationRecord av1Config; }

aligned(8) class AV1CodecConfigurationRecord {
  unsigned int(1) marker = 1;
  unsigned int(7) version = 1;
  unsigned int(3) seq_profile;
  unsigned int(5) seq_level_idx_0;
  unsigned int(1) seq_tier_0;
  unsigned int(1) high_bitdepth;
  unsigned int(1) twelve_bit;
  unsigned int(1) monochrome;
  unsigned int(1) chroma_subsampling_x;
  unsigned int(1) chroma_subsampling_y;
  unsigned int(2) chroma_sample_position;
  unsigned int(3) reserved = 0;
  unsigned int(1) initial_presentation_delay_present;
  if (initial_presentation_delay_present)
       unsigned int(4) initial_presentation_delay_minus_one;
  else  unsigned int(4) reserved = 0;
  unsigned int(8) configOBUs[];   // ≤1 Sequence Header OBU, must be first if present
}
```
Fields SHALL equal the Sequence Header OBU values (seq_profile, seq_level_idx[0],
seq_tier[0], high_bitdepth, twelve_bit, mono_chrome, subsampling_x/y, chroma_sample_position).

RFC 6381: `av01.P.LLT.DD.M.CSP.CP.TC.MC.VRF` — P=profile(1 digit), LL=level(2),
T=tier(M/H), DD=bitdepth(2), M=mono, CSP=subx·suby·samplepos(3), CP/TC/MC=colour(2 each),
VRF=full-range(1). Trailing defaults `.110.01.01.01.0` omittable. e.g. `av01.0.04M.10.0.112.09.16.09.0`.
