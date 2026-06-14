# Table 4.8: Time code exists

_Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope._


|  timecod2e, timecod1e | Time code present  |
| --- | --- |
|  0,0 | Not present  |
|  0,1 | First half (14 bits) present  |
|  1,0 | Second half (14 bits) present  |
|  1,1 | Both halves (28 bits) present  |

4.4.2.28 timecod2 - Time code second half - 14 bits

The first 3 bits of this 14-bit field represent the time in seconds, with valid values from 0 to 7 (representing 0 to 7 seconds). The next 5 bits represents the time in frames, with valid values from 0 to 29. The final 6 bits represent fractions of 1/64th of a frame, with valid values from 0 to 63.

4.4.2.29 addbsie - Additional bit stream information exists - 1 bit

If this bit has a value of 1 there is additional bit stream information, the length of which is indicated by the next field. If this bit has a value of 0, there is no additional bit stream information.

4.4.2.30 addbsil - Additional bit stream information length - 6 bits

This 6-bit code, which exists only if addbsie is a 1, indicates the length in bytes of additional bit stream information. The valid range of addbsil is 0 to 63, indicating 1 to 64 additional bytes, respectively. The decoder is not required to interpret this information, and thus shall skip over this number of bytes following in the data stream.

4.4.2.31 addbsi - Additional bit stream information - ((addbsil + 1) x 8) bits

This field contains 1 to 64 bytes of any additional information included with the bit stream information structure.



## 4.4.3 audblk - Audio block

### 4.4.3.1 blksw[ch] - Block switch flag - 1 bit

This flag, for channel [ch], indicates whether the current audio block was split into 2 sub-blocks during the transformation from the time domain into the frequency domain. A value of 0 indicates that the block was not split, and that a single 512 point TDAC transform was performed. A value of 1 indicates that the block was split into 2 sub-blocks of length 256, that the TDAC transform length was switched from a length of 512 points to a length of 256 points, and that 2 transforms were performed on the audio block (one on each sub-block). Transform length switching is described in more detail in clause 6.9.

### 4.4.3.2 dithflag[ch] - Dither flag - 1 bit

This flag, for channel [ch], indicates that the decoder should activate dither during the current block. Dither is described in detail in clause 6.3.4.

### 4.4.3.3 dynrnge - Dynamic range gain word exists - 1 bit

If this bit is a 1, the dynamic range gain word follows in the bit stream. If it is 0, the gain word is not present, and the previous value is reused, except for block 0 of a syncframe where if the control word is not present the current value of dynrng is set to 0.

### 4.4.3.4 dynrng - Dynamic range gain word - 8 bits

This encoder-generated gain word is applied to scale the reproduced audio as described in clause 6.7.

### 4.4.3.5 dynrng2e - Dynamic range gain word exists, Ch2 - 1 bit

If this bit is a 1, the dynamic range gain word for channel 2 follows in the bit stream. If it is 0, the gain word is not present, and the previous value is reused, except for block 0 of a syncframe where if the control word is not present the current value of dynrng2 is set to 0.

### 4.4.3.6 dynrng2 - dynamic range gain word, Ch2 - 8 bits

This encoder-generated gain word is applied to scale the reproduced audio of Ch2, in the same manner as dynrng is applied to Ch1, as described in clause 6.7.

### 4.4.3.7 cplstre - Coupling strategy exists - 1 bit

If this bit is a 1, coupling information follows in the bit stream. If it is 0, new coupling information is not present, and coupling parameters previously sent are reused. This parameter shall not be set to 0 in block 0.

### 4.4.3.8 cplinu - Coupling in use - 1 bit

If this bit is a 1, coupling is currently being utilized, and coupling parameters follow. If it is 0, coupling is not being utilized (all channels are independent) and no coupling parameters follow in the bit stream.

### 4.4.3.9 chincpl[ch] - Channel in coupling - 1 bit

If this bit is a 1, then the channel indicated by the index [ch] is a coupled channel. If the bit is a 0, then this channel is not coupled. Since coupling is not used in the 1/0 mode, if any chincpl[] values exist there will be 2 to 5 values. Of the values present, at least two values will be 1, since coupling requires more than one coupled channel to be coupled.

### 4.4.3.10 phsflginu - Phase flags in use - 1 bit

If this bit (defined for 2/0 mode only) is a 1, phase flags are included with coupling coordinate information. Phase flags are described in clause 6.4.



## 4.4.3.11 cplbegf - Coupling begin frequency code - 4 bits

This 4-bit code is interpreted as the sub-band number (0 to 15) which indicates the lower frequency band edge of the coupling channel (or the first active sub-band) as shown in Table 6.24.

## 4.4.3.12 cplendf - Coupling end frequency code - 4 bits

This 4-bit code indicates the upper band edge of the coupling channel. The upper band edge (or last active sub-band) is cplendf + 2, or a value between 2 and 17 (see Table 6.24).

The number of active coupling sub-bands is equal to ncplsubnd, which is calculated as:

$$
\mathrm{ncplsubnd} = 3 + \mathrm{cplendf} - \mathrm{cplbegf}.
$$

## 4.4.3.13 cplbndstrc[sbnd] - Coupling band structure - 1 bit

There are 18 coupling sub-bands defined in Table 6.24, each containing 12 frequency coefficients. The fixed 12-bin wide coupling sub-bands are converted into coupling bands, each of which may be wider than (a multiple of) 12 frequency bins. Each coupling band may contain one or more coupling sub-bands. Coupling coordinates are transmitted for each coupling band. Each band's coupling coordinate shall be applied to all the coefficients in the coupling band.

The coupling band structure indicates which coupling sub-bands are combined into wider coupling bands. When cplbndstrc[sbnd] is a 0, the sub-band number [sbnd] is not combined into the previous band to form a wider band, but starts a new 12 wide coupling band. When cplbndstrc[sbnd] is a 1, then the sub-band [sbnd] is combined with the previous band, making the previous band 12 bins wider. Each successive value of cplbndstrc which is a 1 will continue to combine sub-bands into the current band. When another cplbndstrc value of 0 is received, then a new band will be formed, beginning with the 12 bins of the current sub-band. The set of cplbndstrc[sbnd] values is typically considered an array.

Each bit in the array corresponds to a specific coupling sub-band in ascending frequency order. The first element of the array corresponds to the sub-band cplbegf, is always 0, and is not transmitted. (There is no reason to send a cplbndstrc bit for the first sub-band at cplbegf, since this bit would always be 0.) Thus, there are ncplsubnd-1 values of cplbndstrc transmitted. If there is only one coupling sub-band, then no cplbndstrc bits are sent.

The number of coupling bands, ncplbnd, may be computed from ncplsubnd and cplbndstrc:

$$
\mathrm{ncplbnd} = (\mathrm{ncplsubnd} - (\mathrm{cplbndstrc}[1] + \dots + \mathrm{cplbndstrc}[\mathrm{ncplsubnd} - 1])).
$$

## 4.4.3.14 cplcoe[ch] - Coupling coordinates exist - 1 bit

Coupling coordinates indicate, for a given channel and within a given coupling band, the fraction of the coupling channel frequency coefficients to use to re-create the individual channel frequency coefficients. Coupling coordinates are conditionally transmitted in the bit stream. If new values are not delivered, the previously sent values remain in effect. See clause 6.4 for further information on coupling.

If cplcoe[ch] is 1, the coupling coordinates for the corresponding channel [ch] exist and follow in the bit stream. If the bit is 0, the previously transmitted coupling coordinates for this channel are reused. This parameter shall not be set to 0 in block 0, or in any block for which the corresponding channel is participating in coupling but was not participating in coupling in the previous block.



## 4.4.3.15 mstrcplco[ch] - Master coupling coordinate - 2 bits

This per channel parameter establishes a per channel gain factor (increasing the dynamic range) for the coupling coordinates as shown in Table 4.9.
