# Table 4.4: Centre mix level

_Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope._


|  cmixlev | clev  |
| --- | --- |
|  00 | 0,707 (-3,0 dB)  |
|  01 | 0,595 (-4,5 dB)  |
|  10 | 0,500 (-6,0 dB)  |
|  11 | Reserved  |

## 4.4.2.5 surmixlev - Surround mix level - 2 bits

If surround channels are in use, this 2-bit code, shown in Table 4.5, indicates the nominal down mix level of the surround channels. If surmixlev is set to the reserved code, the decoder should still reproduce audio. The intermediate value of surmixlev (-6 dB) may be used in this case.
