# Table 4.1: Sample rate codes

_Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope._


|  fscod | Sample rate (kHz)  |
| --- | --- |
|  00 | 48  |
|  01 | 44,1  |
|  10 | 32  |
|  11 | Reserved  |

## 4.4.1.4 frmsizecod - Frame size code - 6 bits

The frame size code is used along with the sample rate code to determine the number of (2-byte) words before the next syncword (see Table 4.13).

## 4.4.2 bsi - Bit stream information

### 4.4.2.1 bsid - Bit stream identification - 5 bits

This bit field has a value of 01000 (= 8) in this version of the present document. Future modifications of the present document may define other values. Values of bsid smaller than 8 will be used for versions of AC-3 which are backward compatible with version 8 decoders. Decoders which can decode version 8 will thus be able to decode bsid version numbers less than 8. If the present document is extended by the addition of additional elements or features that are not compatible with decoders that follow this bsid version 8 specification, a value of bsid greater than 8 will be used. Decoders built to this version of the standard will not be able to decode versions with bsid greater than 8. Thus, decoders built to the present document shall mute if the value of bsid is greater than 8, and should decode and reproduce audio if the value of bsid is less than or equal to 8.

### 4.4.2.2 bsmod - Bit stream mode - 3 bits

This 3-bit code indicates the type of service that the bit stream conveys as defined in Table 4.2.
