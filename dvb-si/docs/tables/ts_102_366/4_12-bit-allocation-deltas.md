# Table 4.12: Bit allocation deltas

_Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope._


|  cpldeltba, deltba | Adjustment (dB)  |
| --- | --- |
|  000 | -24  |
|  001 | -18  |
|  010 | -12  |
|  011 | -6  |
|  100 | +6  |
|  101 | +12  |
|  110 | +18  |
|  111 | +24  |

## 4.4.3.54 deltnseg[ch] - Channel delta bit allocation number of segments - 3 bits

These per full bandwidth channel elements are 3-bit codes indicating the number of delta bit allocation segments that exist for the corresponding channel. The value of this parameter ranges from 1 to 8, and is calculated by adding 1 to the 3-bit binary code.

## 4.4.3.55 deltoffst[ch][seg] - Channel delta bit allocation offset - 5 bits

The first 5-bit code ([seg] = 0) indicates the number of the first bit allocation band (see clause 6.2.2) of the corresponding channel for which delta bit allocation values are provided. Subsequent codes indicate the offset from the previous delta segment end point to the next bit allocation band for which delta bit allocation values are provided.

## 4.4.3.56 deltlen[ch][seg] - Channel delta bit allocation length - 4 bits

Each 4-bit code indicates the number of bit allocation bands that the corresponding segment spans.

## 4.4.3.57 deltba[ch][seg] - Channel delta bit allocation - 3 bits

This 3-bit value is used in the bit allocation process for the indicated channel. Each 3-bit code indicates an adjustment to the default masking curve computed in the decoder. The deltas are coded as shown in Table 4.13.

## 4.4.3.58 skiple - Skip length exists - 1 bit

If this bit is a 1, then the skipl parameter follows in the bit stream. If this bit is a 0, skipl does not exist.

## 4.4.3.59 skipl - Skip length - 9 bits

This 9-bit code indicates the number of dummy bytes to skip (ignore) before unpacking the mantissas of the current audio block.

## 4.4.3.60 skipfld - Skip field - (skipl x 8) bits

This field contains the bytes of data to be skipped, as indicated by the skipl parameter.

## 4.4.3.61 chmant[ch][bin] - Channel mantissas - 0 bits to 16 bits

The actual quantized mantissa values for the indicated channel. Each value may contain from 0 to as many as 16 bits. The number of mantissas for the indicated channel is equal to nchmant[ch], which may be determined from chbwcod[ch] (see clause 6.1.3) if the channel is not coupled, or from cplbegf (see clause 6.4.2) if the channel is coupled. Detailed information on packed mantissa data is in clause 6.3.



4.4.3.62 cplmant[bin] - Coupling mantissas - 0 bits to 16 bits

The actual quantized mantissa values for the coupling channel. Each value may contain from 0 to as many as 16 bits. The number of mantissas for the coupling channel is equal to ncplmant, which may be determined from:

$$
\mathrm{ncplmant} = 12 \times \mathrm{ncplsubnd}.
$$

4.4.3.63 Ifemant[bin] - Low frequency effects channel mantissas - 0 bits to 16 bits

The actual quantized mantissa values for the life channel. Each value may contain from 0 to as many as 16 bits. The value of nlfemant is 7, so there are 7 mantissa values for the life channel.

4.4.4 auxdata - Auxiliary data field

4.4.4.0 Introduction

Unused data at the end of a syncframe will exist whenever the encoder does not utilize all available data for encoding the audio signal. This may occur if the final bit allocation falls short of using all available bits, or if the input audio signal simply does not require all available bits to be coded transparently. Or, the encoder may be instructed to intentionally leave some bits unused by audio so that they are available for use by auxiliary data. Since the number of bits required for auxiliary data may be smaller than the number of bits available (which will be time varying) in any particular syncframe, a method is provided to signal the number of actual auxiliary data bits in each syncframe.

4.4.4.1 auxbits - Auxiliary data bits - nauxbits bits

This field contains auxiliary data. The total number of bits in this field is:

nauxbits = (bits in syncframe) - (bits used by all bit stream elements except for auxbits).

The number of bits in the syncframe can be determined from the frame size code (frmsizcod) and Table 4.13. The number of bits used includes all bits used by bit stream elements with the exception of auxbits. Any dummy data which has been included with skip fields (skipfld) is included in the used bit count. The length of the auxbits field is adjusted by the encoder such that the crc2 element falls on the last 16-bit word of the syncframe.
