# VP9 in ISOBMFF — vp09 sample entry + vpcC (#437)

Source: WebM Project "VP Codec ISO Media File Format Binding" (https://www.webmproject.org/vp9/mp4/, free).
Fixture: `fixtures/mp4/vp9.mp4`; oracle vpcC body `010000000014820202020000` (FullBox v1).

```c
class VPCodecConfigurationBox extends FullBox('vpcC', version = 1, 0) {
    VPCodecConfigurationRecord() vpcConfig;
}
aligned(8) class VPCodecConfigurationRecord {
    unsigned int(8)  profile;
    unsigned int(8)  level;
    unsigned int(4)  bitDepth;
    unsigned int(3)  chromaSubsampling;
    unsigned int(1)  videoFullRangeFlag;
    unsigned int(8)  colourPrimaries;
    unsigned int(8)  transferCharacteristics;
    unsigned int(8)  matrixCoefficients;
    unsigned int(16) codecInitializationDataSize;   // MUST be 0 for VP8/VP9
    unsigned int(8)[] codecInitializationData;       // unused for VP8/VP9
}
class VP9SampleEntry extends VisualSampleEntry('vp09') { VPCodecConfigurationBox config; }
```
