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

The binding defines **only version 1**: "version is an integer that specifies the
version of this box; should be 1. Version 0 is deprecated and should not be
used." It publishes no syntax for version 0, so the parser accepts version 1
only and rejects any other version with `Error::InvalidValue` — a guessed layout
would report wrong bit depth / chroma / colour values. Serialization likewise
refuses a version other than 1.

Independent oracle: ffmpeg 8.1, `vpcC` body `01000000000a820202020000`
(profile 0, level 10, bitDepth 8, chromaSubsampling 1, CICP 2/2/2, size 0),
produced by
`ffmpeg -f lavfi -i testsrc2=size=160x120:rate=25 -t 0.4 -c:v libvpx-vp9 -b:v 200k out.mp4`;
the committed `fixtures/mp4/vp9.mp4` body is `010000000014820202020000`.
