# Table 4.11: Delta bit allocation exist states

_Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope._


|  cpldeltbae, deltbae | Code  |
| --- | --- |
|  00 | Reuse previous state  |
|  01 | New info follows  |
|  10 | Perform no delta alloc  |
|  11 | Reserved  |

4.4.3.49 deltbae[ch] - Delta bit allocation exists - 2 bits

This per full bandwidth channel 2-bit code indicates the delta bit allocation strategy for the corresponding channel, as shown in Table 4.11. This parameter shall not be set to "00" in block 0.

4.4.3.50 cpldeltnseg - Coupling delta bit allocation number of segments - 3 bits

This 3-bit code indicates the number of delta bit allocation segments that exist for the coupling channel. The value of this parameter ranges from 1 to 8, and is calculated by adding 1 to the 3-bit binary number represented by the code.

4.4.3.51 cpldeltoffst[seg] - Coupling delta bit allocation offset - 5 bits

The first 5-bit code ([seg] = 0) indicates the number of the first bit allocation band (as specified in clause 6.4.2) of the coupling channel for which delta bit allocation values are provided. Subsequent codes indicate the offset from the previous delta segment end point to the next bit allocation band for which delta bit allocation values are provided.

4.4.3.52 cpldeltlen[seg] - Coupling delta bit allocation length - 4 bits

Each 4-bit code indicates the number of bit allocation bands that the corresponding segment spans.

4.4.3.53 cpldeltba[seg] - Coupling delta bit allocation - 3 bits

This 3-bit value is used in the bit allocation process for the coupling channel.

Each 3-bit code indicates an adjustment to the default masking curve computed in the decoder. The deltas are coded as shown in Table 4.12.
