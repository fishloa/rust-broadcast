# FLAC in ISOBMFF — fLaC sample entry + dfLa (#437)

Source: xiph FLAC-in-ISOBMFF (https://github.com/xiph/flac/blob/master/doc/isoflac.txt, free).
Fixture: `fixtures/mp4/flac.mp4`; oracle dfLa body `0000000080000022 1200 1200 0002f2 0004f00a c440f00000ac44…`.

```c
class FLACSampleEntry extends AudioSampleEntry('fLaC') {   // channelcount/samplesize/samplerate
    FLACSpecificBox();                                     // from STREAMINFO; samplerate is 16.16
}
class FLACSpecificBox extends FullBox('dfLa', version=0, 0) {
    for (i=0; ; i++) { FLACMetadataBlock(); }              // ≥1; first is STREAMINFO (type 0)
}
// FLACMetadataBlock:
//   unsigned int(1)  LastMetadataBlockFlag
//   unsigned int(7)  BlockType          // first block MUST be 0 (STREAMINFO)
//   unsigned int(24) Length             // bytes of BlockData
//   unsigned int(8)  BlockData[Length]  // raw FLAC metadata (STREAMINFO = 34 bytes)
```
