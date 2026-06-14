# Table 4.10: Number of rematrixing bands

_Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope._


|  Condition | No. of rematrixing bands  |
| --- | --- |
|  cplinu == 0 | 4  |
|  (cplinu == 1) && (cplbegf > 2) | 4  |
|  (cplinu == 1) && (2 ≥ cplbegf > 0) | 3  |
|  (cplinu == 1) && (cplbegf == 0) | 2  |

## 4.4.3.21 cplexpstr - Coupling exponent strategy - 2 bits

This element indicates the method of exponent coding that is used for the coupling channel as shown in Table 6.4. See clause 6.1 for explanation of each exponent strategy. This parameter shall not be set to 0 in block 0, or in any block for which coupling is enabled but was disabled in the previous block.



4.4.3.22 chexpstr[ch] - Channel exponent strategy - 2 bits

This element indicates the method of exponent coding that is used for channel [ch], as shown in Table 6.4. This element exists for each full bandwidth channel. This parameter shall not be set to 0 in block 0.

4.4.3.23 lfeexpstr - Low frequency effects channel exponent strategy - 1 bit

This element indicates the method of exponent coding that is used for the lfe channel, as shown in Table 6.5. This parameter shall not be set to 0 in block 0.

4.4.3.24 chbwcod[ch] - Channel bandwidth code - 6 bits

The chbwcod[ch] element is an unsigned integer which defines the upper band edge for full-bandwidth channel [ch]. This parameter is only included for fbw channels which are not coupled. (See clause 6.1.3 on exponents for the definition of this parameter.) Valid values are in the range of 0 - 60. If a value greater than 60 is received, the bit stream is invalid and the decoder shall cease decoding audio and mute.

4.4.3.25 cplabsexp - Coupling absolute exponent - 4 bits

This is an absolute exponent, which is used as a reference when decoding the differential exponents for the coupling channel.

4.4.3.26 cplexps[grp] - Coupling exponents - 7 bits

Each value of cplexps indicates the value of 3, 6, or 12 differentially-coded coupling channel exponents for the coupling exponent group [grp] for the case of d15, d25, or d45 coding, respectively. The number of cplexps values transmitted equals ncplgrps, which may be determined from cplbegf, cplendf, and cplexpstr. Refer to clause 6.1.3 for further information.

4.4.3.27 exps[ch][grp] - Channel exponents - 4 bits or 7 bits

These elements represent the encoded exponents for channel [ch]. The first element ([grp] = 0) is a 4-bit absolute exponent for the first (DC term) transform coefficient. The subsequent elements ([grp] &gt; 0) are 7-bit representations of a group of 3, 6, or 12 differentially coded exponents (corresponding to d15, d25, d45 exponent strategies respectively). The number of groups for each channel, nchgrps[ch], is determined from cplbegf if the channel is coupled, or chbwcod[ch] if the channel is not coupled. Refer to clause 6.1.3 for further information.

4.4.3.28 gainrng[ch] - Channel gain range code - 2 bits

This per channel 2-bit element may be used to determine a block floating-point shift value for the inverse TDAC transform filter bank. Use of this code allows increased dynamic range to be obtained from a limited word length transform computation. For further information see clause 6.9.5.

4.4.3.29 lfeexps[grp] - Low frequency effects channel exponents - 4 bits or 7 bits

These elements represent the encoded exponents for the lfe channel. The first element ([grp] = 0) is a 4-bit absolute exponent for the first (DC term) transform coefficient. There are two additional elements (nlfegps = 2) which are 7-bit representations of a group of 3 differentially coded exponents. The total number of lfe channel exponents (nlfemant) is 7.

4.4.3.30 baie - Bit allocation information exists - 1 bit

If this bit is a 1, then five separate fields (totalling 11 bits) follow in the bit stream. Each field indicates parameter values for the bit allocation process. If this bit is a 0, these fields do not exist. Further details on these fields may be found in clause 6.2. This parameter shall not be set to 0 in block 0.

4.4.3.31 sdcycod - Slow decay code - 2 bits

This 2-bit code specifies the slow decay parameter in the bit allocation process.



4.4.3.32 fdcycod - Fast decay code - 2 bits

This 2-bit code specifies the fast decay parameter in the decode bit allocation process.

4.4.3.33 sgaincod - Slow gain code - 2 bits

This 2-bit code specifies the slow gain parameter in the decode bit allocation process.

4.4.3.34 dbpbcod - dB per bit code - 2 bits

This 2-bit code specifies the dB per bit parameter in the bit allocation process.

4.4.3.35 floorcod - Masking floor code - 3 bits

This 3-bit code specifies the floor code parameter in the bit allocation process.

4.4.3.36 snroffste - SNR offset exists - 1 bit

If this bit has a value of 1, a number of bit allocation parameters follow in the bit stream. If this bit has a value of 0, SNR offset information does not follow, and the previously transmitted values should be used for this block. The bit allocation process and these parameters are described in clause 6.2. This parameter shall not be set to 0 in block 0.

4.4.3.37 csnroffst - Coarse SNR offset - 6 bits

This 6-bit code specifies the coarse SNR offset parameter in the bit allocation process.

4.4.3.38 cplfsnroffst - Coupling fine SNR offset - 4 bits

This 4-bit code specifies the coupling channel fine SNR offset in the bit allocation process.

4.4.3.39 cplfgaincod - Coupling fast gain code - 3 bits

This 3-bit code specifies the coupling channel fast gain code used in the bit allocation process.

4.4.3.40 fsnroffst[ch] - Channel fine SNR offset - 4 bits

This 4-bit code specifies the fine SNR offset used in the bit allocation process for channel [ch].

4.4.3.41 fgaincod[ch] - Channel fast gain code - 3 bits

This 3-bit code specifies the fast gain parameter used in the bit allocation process for channel [ch].

4.4.3.42 lfefsnroffst - Low frequency effects channel fine SNR offset - 4 bits

This 4-bit code specifies the fine SNR offset parameter used in the bit allocation process for the lfe channel.

4.4.3.43 lfefgaincod - Low frequency effects channel fast gain code - 3 bits

This 3-bit code specifies the fast gain parameter used in the bit allocation process for the lfe channel.

4.4.3.44 cplleake - Coupling leak initialization exists - 1 bit

If this bit is a 1, leak initialization parameters follow in the bit stream. If this bit is a 0, the previously transmitted values still apply. This parameter shall not be set to 0 in block 0, or in any block for which coupling is enabled but was disabled in the previous block.



4.4.3.45 cplfleak - Coupling fast leak initialization - 3 bits

This 3-bit code specifies the fast leak initialization value for the coupling channel's excitation function calculation in the bit allocation process.

4.4.3.46 cplsleak - Coupling slow leak initialization - 3 bits

This 3-bit code specifies the slow leak initialization value for the coupling channel's excitation function calculation in the bit allocation process.

4.4.3.47 deltbaie - Delta bit allocation information exists - 1 bit

If this bit is a 1, some delta bit allocation information follows in the bit stream. If this bit is a 0, the previously transmitted delta bit allocation information still applies, except for block 0. If deltbaie is 0 in block 0, then cpldeltbae and deltbae[ch] are set to the binary value "10", and no delta bit allocation is applied. Delta bit allocation is described in clause 6.2.2.

4.4.3.48 cpldeltbae - Coupling delta bit allocation exists - 2 bits

This 2-bit code indicates the delta bit allocation strategy for the coupling channel, as shown in Table 4.11. If the reserved state is received, the decoder should not decode audio, and should mute. This parameter shall not be set to "00" in block 0, or in any block for which coupling is enabled but was disabled in the previous block.
