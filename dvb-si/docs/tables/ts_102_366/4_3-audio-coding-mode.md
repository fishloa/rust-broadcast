# Table 4.3: Audio coding mode

_Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope._


|  acmod | Audio coding mode | Nfchans | Channel array ordering  |
| --- | --- | --- | --- |
|  000 | 1 + 1 | 2 | Ch1, Ch2  |
|  001 | 1/0 | 1 | C  |
|  010 | 2/0 | 2 | L, R  |
|  011 | 3/0 | 3 | L, C, R  |
|  100 | 2/1 | 3 | L, R, S  |
|  101 | 3/1 | 4 | L, C, R, S  |
|  110 | 2/2 | 4 | L, R, Ls, Rs  |
|  111 | 3/2 | 5 | L, C, R, Ls, Rs  |

## 4.4.2.4 cmixlev - Centre mix level - 2 bits

When three front channels are in use, this 2-bit code, shown in Table 4.4, indicates the nominal down mix level of the centre channel with respect to the left and right channels. If cmixlev is set to the reserved code, decoders should still reproduce audio. The intermediate value of cmixlev (-4,5 dB) may be used in this case.
