# Table 4.9: Master coupling coordinate

_Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope._


|  mstrcplco[ch] | cplco[ch][bnd] gain multiplier  |
| --- | --- |
|  00 | 1  |
|  01 | 2^{-3}  |
|  10 | 2^{-6}  |
|  11 | 2^{-9}  |

## 4.4.3.16 cplcoexp[ch][bnd] - Coupling coordinate exponent - 4 bits

Each coupling coordinate is composed of a 4-bit exponent and a 4-bit mantissa. This element is the value of the coupling coordinate exponent for channel [ch] and band [bnd]. The index [ch] only will exist for those channels which are coupled. The index [bnd] will range from 0 to ncplbnds. See clause 6.4.3 for further information on how to interpret coupling coordinates.

## 4.4.3.17 cplcomant[ch][bnd] - Coupling coordinate mantissa - 4 bits

This element is the 4-bit coupling coordinate mantissa for channel [ch] and band [bnd].

## 4.4.3.18 phsflg[bnd] - Phase flag - 1 bit

This element (only used in the 2/0 mode) indicates whether the decoder should phase invert the coupling channel mantissas when reconstructing the right output channel. The index [bnd] can range from 0 to ncplbnd. Phase flags are described in clause 6.4.

## 4.4.3.19 rematstr - Rematrixing strategy - 1 bit

If this bit is a 1, then new rematrix flags are present in the bit stream. If it is 0, rematrix flags are not present, and the previous values should be reused. The rematstr parameter is present only in the 2/0 audio coding mode. This parameter shall not be set to 0 in block 0.

## 4.4.3.20 rematflg[rbnd] - Rematrix flag - 1 bit

This bit indicates whether the transform coefficients in rematrixing band [rbnd] have been rematrixed. If this bit is a 1, then the transform coefficients in [rbnd] were rematrixed into sum and difference channels. If this bit is a 0, then rematrixing has not been performed in band [rbnd]. The number of rematrixing bands (and the number of values of [rbnd]) depend on coupling parameters as shown in Table 4.10. Rematrixing is described in clause 6.5.
