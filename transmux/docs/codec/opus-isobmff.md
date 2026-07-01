# Opus in ISOBMFF — Opus sample entry + dOps (#437)

Source: "Encapsulation of Opus in ISO Base Media File Format" (https://opus-codec.org/docs/opus_in_isobmff.html, free).
NOTE: unlike the Ogg ID header (RFC 7845), dOps fields are **big-endian** and there is **no** "OpusHead" magic.
Fixture: `fixtures/mp4/opus.mp4`; oracle dOps body `00 01 0138 0000bb80 0000 00` (v1, 1ch, preskip 0x0138, 48000, gain 0, family 0).

```c
class OpusSampleEntry() extends AudioSampleEntry('Opus') {
    OpusSpecificBox();     // channelcount = M+N; samplesize = 16; samplerate = 48000<<16
}
aligned(8) class OpusSpecificBox extends Box('dOps') {
    unsigned int(8)  Version;               // 0
    unsigned int(8)  OutputChannelCount;
    unsigned int(16) PreSkip;
    unsigned int(32) InputSampleRate;
    signed   int(16) OutputGain;            // 8.8 fixed-point
    unsigned int(8)  ChannelMappingFamily;
    if (ChannelMappingFamily != 0) {
        unsigned int(8) StreamCount;
        unsigned int(8) CoupledCount;
        unsigned int(8) ChannelMapping[OutputChannelCount];
    }
}
```
