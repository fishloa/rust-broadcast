# 4.3 Syntax specification

# 4.3.0 AC-3\_bit\_stream and syncframe

A continuous audio bit stream consists of a sequence of synchronization frames:

```
Syntax
AC-3_bit_stream() 
{ 
 while(true) 
 { 
 syncframe() ; 
 } 
} /* end of AC-3 bit stream */
```

The syncframe consists of the **syncinfo** and **bsi** fields, the 6 coded **audblk** fields, the **auxdata** field, and the **errorcheck** field.

```
Syntax
syncframe() 
{ 
 syncinfo() ; 
 bsi() ; 
  for(blk = 0; blk < 6; blk++) 
  { 
 audblk() ; 
  } 
 auxdata() ; 
 errorcheck() ; 
} /* end of syncframe */
```

Each of the bit stream elements, and their length, are itemized in the following pseudo code. Note that all bit stream elements arrive most significant bit first, or left bit first, in time.

# 4.3.1 syncinfo - Synchronization information

```
Syntax Word size
syncinfo() 
{ 
 syncword ..................................................................................16 
 crc1 ......................................................................................16 
 fscod ......................................................................................2 
 frmsizecod .................................................................................6 
} /* end of syncinfo */
```

# 4.3.2 bsi - Bit stream information

```
Syntax Word size
bsi() 
{ 
 bsid .......................................................................................5 
 bsmod ......................................................................................3 
 acmod ......................................................................................3 
  if((acmod & 0x1) && (acmod != 0x1)) /* if 3 front channels */ {cmixlev} ....................2 
  if(acmod & 0x4) /* if a surround channel exists */ {surmixlev} .............................2 
  if(acmod == 0x2) /* if in 2/0 mode */ {dsurmod} ............................................2 
 lfeon ......................................................................................1 
 dialnorm ...................................................................................5 
 compre .....................................................................................1 
  if(compre) {compr} .........................................................................8 
 langcode ...................................................................................1 
  if(langcode) {langcod} .....................................................................8 
 audprodie ..................................................................................1 
  if(audprodie) 
  { 
 mixlevel ................................................................................5 
 roomtyp .................................................................................2
```

```
Word size
Syntax
 if (acmod == 0) /* if 1+1 mode (dual mono, so some items need a second value) */
  dialnorm2 ...... 5
  compr2e ....................................
  if(compr2e) {compr2} ......8
  if(langcod2e) {langcod2} ......8
  audprodi2e 1
  if(audprodi2e)
   mixlevel2 ....
   roomtvp2 . . . . . . . . . . . . . . . . . . .
 copyrightb .....
 origbs ....
 timecod1e . . . . . . . . . . . . . . . . . . .
 if(timecodle) {timecodl} . . . . . . . . . . . . . . . . . . .
 \verb|timecod2e| ....................................
 if(timecod2e) {timecod2} ....................................
 addbsie ....................................
 if (addbsie)
  addbsil .....6
  addbsi ... (addbsil+1) x 8
  end of bsi */
```

### 4.3.3 audblk - Audio block

```
Syntax
                                                          Word size
audhlk()
^{\prime} these fields for block switch and dither flags */
  for(ch = 0; ch < nfchans; ch++) {blksw[ch]}....................................
  for(ch = 0; ch < nfchans; ch++) {dithflag[ch]}....................................
 these fields for dynamic range control */
  dynrnge....
  if(dynrnge) {dynrng}......8
  if(acmod == 0) /* if 1+1 mode */
    dynrng2e . . . . . . . . . . . . . . . . . . .
    if(dynrng2e) {dynrng2}.....8
/* these fields for coupling strategy information */
  cplstre.....1
  if(cplstre)
    cplinu ....................................
    if(cplinu)
      for(ch = 0; ch < nfchans; ch++) {chincpl[ch]} ....................................
      cplbegf ...... 4
      cplendf ..... 4
      /* ncplsubnd = 3 + cplendf - cplbeaf */
      for (bnd = 1; bnd < ncplsubnd; bnd++) {cplbndstrc[bnd]}....................................
 these fields for coupling coordinates, phase flags */
  if (cplinu)
    for(ch = 0; ch < nfchans; ch++)
      if(chincpl[ch])
        cplcoe[ch] ....
                  .......................................
        if(cplcoe[ch])
         mstrcplco[ch] ....................................
          /* ncplbnd derived from ncplsubnd, and cplbndstrc */
          for(bnd = 0; bnd < ncplbnd; bnd++)</pre>
```

```
Syntax Word size
 cplcoexp[ch][bnd] ........................................................... 4 
 cplcomant[ch][bnd] .......................................................... 4 
 } 
 } 
 } 
 } 
 if((acmod == 0x2) && phsflginu && (cplcoe[0] || cplcoe[1])) 
 { 
 for(bnd = 0; bnd < ncplbnd; bnd++) {phsflg[bnd]} ..................................... 1 
 } 
  } 
/* these fields for rematrixing operation in the 2/0 mode */ 
  if(acmod == 0x2) /* if in 2/0 mode */ 
  { 
 rematstr ................................................................................ 1 
 if(rematstr) 
 { 
 if((cplbegf > 2) || (cplinu == 0)) 
 { 
 for(rbnd = 0; rbnd < 4; rbnd++) {rematflg[rbnd]} .................................. 1 
 } 
 if((2 >= cplbegf > 0) && cplinu) 
 { 
 for(rbnd = 0; rbnd < 3; rbnd++) {rematflg[rbnd]} .................................. 1 
 } 
 if((cplbegf == 0) && cplinu) 
 { 
 for(rbnd = 0; rbnd < 2; rbnd++) {rematflg[rbnd]} .................................. 1 
 } 
 } 
  } 
/* these fields for exponent strategy */ 
  if(cplinu) {cplexpstr} ..................................................................... 2 
  for(ch = 0; ch < nfchans; ch++) {chexpstr[ch]} ............................................. 2 
  if(lfeon) {lfeexpstr} ...................................................................... 1 
  for(ch = 0; ch < nfchans; ch++) 
  { 
 if(chexpstr[ch] != reuse) 
 { 
 if(!chincpl[ch]) {chbwcod[ch]} ....................................................... 6 
 } 
  } 
/* these fields for exponents */ 
  if(cplinu) /* exponents for the coupling channel */ 
  { 
 if(cplexpstr != reuse) 
 { 
 cplabsexp ............................................................................ 4 
 /* ncplgrps derived from ncplsubnd, cplexpstr */ 
 for(grp = 0; grp < ncplgrps; grp++) {cplexps[grp]} ................................... 7 
 } 
  } 
  for(ch = 0; ch < nfchans; ch++) /* exponents for full bandwidth channels */ 
  { 
 if(chexpstr[ch] != reuse) 
 { 
 exps[ch][0] .......................................................................... 4 
 /* nchgrps derived from chexpstr[ch], and cplbegf or chbwcod[ch] */ 
 for(grp = 1; grp <= nchgrps[ch]; grp++) {exps[ch][grp]} .............................. 7 
 gainrng[ch] .......................................................................... 2 
 } 
  } 
  if(lfeon) /* exponents for the low frequency effects channel */ 
  { 
 if(lfeexpstr != reuse) 
 { 
 lfeexps[0] ........................................................................... 4 
 /* nlfegrps = 2 */ 
 for(grp = 1; grp <= nlfegrps; grp++) {lfeexps[grp]} .................................. 7 
 } 
  } 
/* these fields for bit-allocation parametric information */ 
 baie ....................................................................................... 1 
  if(baie) 
  { 
 sdcycod ................................................................................. 2 
 fdcycod ................................................................................. 2
```

```
Syntax
                                             Word size
   sgaincod _______ 2
   dbpbcod ....
   floorcod ....................................
 if(snroffste)
   csnroffst
  if(cplinu)
    cplfqaincod.....3
   for (ch = 0; ch < nfchans; ch++)
    fsnroffst[ch] ..... 4
    fqaincod[ch] ....................................
   if(lfeon)
    lfefsnroffst 4
    lfefgaincod ....................................
 if(cplinu)
   cplleake
   if(cplleake)
    cplfleak ....................................
    cp|s|eak 3
 these fields for delta bit allocation information */
 deltbaie ....................................
 if (delthaie)
   if(cplinu) {cpldeltbae} .....
   for (ch = 0; ch < nfchans; ch++) {deltbae[ch]}....................................
   if(cplinu)
    if(cpldeltbae==new info follows)
      cpldeltnseg .....
      for(seg = 0; seg <= cpldeltnseg; seg++)</pre>
       cpldeltoffst[seg] ....................................
       cpldeltlen[seg] ....................................
       cpldeltba[seg] ....................................
   for(ch = 0; ch < nfchans; ch++)
    if(deltbae[ch] == new info follows)
      deltnseg[ch] ....................................
      for(seg = 0; seg <= deltnseg[ch]; seg++)</pre>
       deltoffst[ch][seq] ....................................
       deltlen[ch][seq] ....................................
       deltba[ch][seg] ....................................
/* these fields for inclusion of unused dummy data */
 skiple .....
 if(skiple)
   skipl .....
   skipfld ...... skipl x 8
/* These fields for quantized mantissa values */......
 got cplchan = 0 ..................................
 for (ch = 0; ch < nfchans; ch++)......
```

```
Syntax Word size
 for (bin = 0; bin < nchmant[ch]; bin++) {chmant[ch][bin]} .......................... (0-16) 
 if (cplinu && chincpl[ch] && !got_cplchan) ............................................... 
 { 
 for (bin = 0; bin < ncplmant; bin++) {cplmant[bin]} ............................. (0-16) 
 got_cplchan = 1 ....................................................................... 
 } 
 } 
 if(lfeon) /* mantissas of low frequency effects channel */ .................................. 
 { 
 for (bin = 0; bin < nlfemant; bin++) {lfemant[bin]} ................................ (0-16) 
 } 
} /* end of audblk */ ..........................................................................
```

# 4.3.4 auxdata - Auxiliary data

```
Syntax Word size
auxdata() 
{ 
 auxbits ............................................................................. nauxbits 
  if(auxdatae) 
  { 
 auxdatal ............................................................................... 14 
  } 
 auxdatae ................................................................................... 1 
} /* end of auxdata */
```

# 4.3.5 errorcheck - Error detection code

| Syntax                    | Word size |
|---------------------------|-----------|
| errorcheck()              |           |
| {                         |           |
| crcrsv 1                  |           |
| crc2 16                   |           |
| } /* end of errorcheck */ |           |

# 4.4 Description of bit stream elements

# 4.4.0 Introduction

A number of bit stream elements have values which may be transmitted, but whose meaning has been reserved. If a decoder receives a bit stream which contains reserved values, the decoder may or may not be able to decode and produce audio. In the description of bit stream elements which have reserved codes, there is an indication of what the decoder can do if the reserved code is received. In some cases, the decoder can not decode audio. In other cases, the decoder can still decode audio by using a default value for a parameter which was indicated by a reserved code.

# 4.4.1 syncinfo - Synchronization information

# 4.4.1.1 syncword - Synchronization word - 16 bits

The syncword is always 0x0B77, or 0000 1011 0111 0111. Transmission of the syncword, like other bit field elements, is left bit first.

#### 4.4.1.2 crc1 - Cyclic redundancy check 1 bit to 16 bits

This 16 bit-CRC applies to the first 5/8 of the syncframe. Transmission of the CRC, like other numerical values, is most significant bit first.

#### 4.4.1.3 fscod - Sample rate code - 2 bits

This is a 2-bit code indicating sample rate according to Table 4.1. If the reserved code is indicated, the decoder should not attempt to decode audio and should mute.

**Table 4.1: Sample rate codes** 

| fscod | Sample rate (kHz) |
|-------|-------------------|
| 00    | 48                |
| 01    | 44,1              |
| 10    | 32                |
| 11    | Reserved          |

#### 4.4.1.4 frmsizecod - Frame size code - 6 bits

The frame size code is used along with the sample rate code to determine the number of (2-byte) words before the next syncword (see Table 4.13).

# 4.4.2 bsi - Bit stream information

#### 4.4.2.1 bsid - Bit stream identification - 5 bits

This bit field has a value of 01000 (= 8) in this version of the present document. Future modifications of the present document may define other values. Values of bsid smaller than 8 will be used for versions of AC-3 which are backward compatible with version 8 decoders. Decoders which can decode version 8 will thus be able to decode bsid version numbers less than 8. If the present document is extended by the addition of additional elements or features that are not compatible with decoders that follow this bsid version 8 specification, a value of bsid greater than 8 will be used. Decoders built to this version of the standard will not be able to decode versions with bsid greater than 8. Thus, decoders built to the present document shall mute if the value of bsid is greater than 8, and should decode and reproduce audio if the value of bsid is less than or equal to 8.

#### 4.4.2.2 bsmod - Bit stream mode - 3 bits

This 3-bit code indicates the type of service that the bit stream conveys as defined in Table 4.2.

**Table 4.2: Bit stream mode** 

| bsmod | acmod      | Type of service                            |
|-------|------------|--------------------------------------------|
| 000   | Any        | Main audio service: complete main (CM)     |
| 001   | Any        | Main audio service: music and effects (ME) |
| 010   | Any        | Associated service: visually impaired (VI) |
| 011   | Any        | Associated service: hearing impaired (HI)  |
| 100   | Any        | Associated service: dialogue (D)           |
| 101   | Any        | Associated service: commentary (C)         |
| 110   | Any        | Associated service: emergency (E)          |
| 111   | 001        | Associated service: voice over (VO)        |
| 111   | 010 to 111 | Main audio service: karaoke                |

#### 4.4.2.3 acmod - Audio coding mode - 3 bits

This 3-bit code, shown in Table 4.3, indicates which of the main service channels are in use, ranging from 3/2 to 1/0. If the MSB of acmod is a 1, surround channels are in use and surmixlev follows in the bit stream. If the MSB of acmod is a 0, the surround channels are not in use and surmixlev does not follow in the bit stream. If the LSB of acmod is a 0, the centre channel is not in use. If the LSB of acmod is a 1, the centre channel is in use.

NOTE: The state of acmod sets the number of full-bandwidth channels parameter, nfchans, (e.g. for 3/2 mode, nfchans = 5; for 2/1 mode, nfchans = 3; etc.). The total number of channels, nchans, is equal to nfchans if the lfe channel is off, and is equal to 1 + nfchans if the lfe channel is on. If acmod is 0, then two completely independent programme channels (dual mono) are encoded into the bit stream, and are referenced as Ch1, Ch2. In this case, a number of additional items are present in BSI or audblk to fully describe Ch2. Table 4.3 also indicates the channel ordering (the order in which the channels are processed) for each of the modes.

**acmod Audio coding mode Nfchans Channel array ordering** 000 1 + 1 2 Ch1, Ch2 001 1/0 1 C 010 2/0 2 L, R 011 3/0 3 L, C, R 100 2/1 3 L, R, S 101 3/1 4 L, C, R, S

110 2/2 4 L, R, Ls, Rs 111 3/2 5 L, C, R, Ls, Rs

**Table 4.3: Audio coding mode** 

# 4.4.2.4 cmixlev - Centre mix level - 2 bits

When three front channels are in use, this 2-bit code, shown in Table 4.4, indicates the nominal down mix level of the centre channel with respect to the left and right channels. If cmixlev is set to the reserved code, decoders should still reproduce audio. The intermediate value of cmixlev (-4,5 dB) may be used in this case.

**cmixlev clev** 00 0,707 (-3,0 dB) 01 0,595 (-4,5 dB) 10 0,500 (-6,0 dB) 11 Reserved

**Table 4.4: Centre mix level** 

# 4.4.2.5 surmixlev - Surround mix level - 2 bits

If surround channels are in use, this 2-bit code, shown in Table 4.5, indicates the nominal down mix level of the surround channels. If surmixlev is set to the reserved code, the decoder should still reproduce audio. The intermediate value of surmixlev (-6 dB) may be used in this case.

**Table 4.5: Surround mix level** 

| surmixlev | slev          |
|-----------|---------------|
| 00        | 0,707 (-3 dB) |
| 01        | 0,500 (-6 dB) |
| 10        | 0             |
| 11        | Reserved      |

#### 4.4.2.6 dsurmod - Dolby® Surround mode - 2 bits

When operating in the two channel mode, this 2-bit code, as shown in Table 4.6, indicates whether or not the programme has been encoded in Dolby® Surround. This information is not used by the AC-3 decoder, but may be used by other portions of the audio reproduction equipment. If dsurmod is set to the reserved code, the decoder should still reproduce audio. The reserved code may be interpreted as "not indicated".

<span id="page-7-0"></span>NOTE: "Dolby®", "Pro Logic®", "Surround EXTM" and the double -D symbol are trademarks of Dolby® Laboratories. This information is given for the convenience of users of the present document and does not constitute an endorsement by ETSI of the product named. Equivalent products may be used if they can be shown to lead to the same results."

**Table 4.6: Dolby® Surround mode** 

| dsurmod | Indication                  |
|---------|-----------------------------|
| 00      | Not indicated               |
| 01      | NOT Dolby® Surround encoded |
| 10      | Dolby® Surround encoded     |
| 11      | Reserved                    |

# 4.4.2.7 lfeon - Low frequency effects channel on - 1 bit

This bit has a value of 1 if the lfe (sub woofer) channel is on, and a value of 0 if the lfe channel is off.

#### 4.4.2.8 dialnorm - Dialogue normalization - 5 bits

This 5-bit code indicates how far the average dialogue level is below digital 100 percent. Valid values are 1 to 31. The value of 0 is reserved. The values of 1 to 31 are interpreted as -1 dB to -31 dB with respect to digital 100 %. If the reserved value of 0 is received, the decoder shall use -31 dB. The value of dialnorm shall affect the sound reproduction level. If the value is not used by the AC-3 decoder itself, the value shall be used by other parts of the audio reproduction equipment. Dialogue normalization is further explained in clause 6.6.

#### 4.4.2.9 compre - Compression gain word exists - 1 bit

If this bit is a 1, the following 8 bits represent a compression control word.

#### 4.4.2.10 compr - Compression gain word - 8 bits

This encoder generated gain word may be present in the bit stream. If so, it may be used to scale the reproduced audio level in order to reproduce a very narrow dynamic range, with an assured upper limit of instantaneous peak reproduced signal level in the monophonic downmix. The meaning and use of compr is described further in clause 6.7.3.

#### 4.4.2.11 langcode - Language code exists - 1 bit

If this bit is a 1, the following 8 bits (i.e. the element langcod) shall be reserved. If this bit is a 0, the element langcod does not exist in the bit stream.

### 4.4.2.12 langcod - Language code - 8 bits

This is an 8 bit reserved value. (This element was originally intended to carry an 8-bit value that would, via a table lookup, indicate the language of the audio programme. Because modern delivery systems provide the ISO 639-2 [i.2] language code in the multiplexing layer, indication of language within the AC-3 bit stream was unnecessary, and so was removed from the AC-3 syntax.)

#### 4.4.2.13 audprodie - Audio production information exists - 1 bit

If this bit is a 1, the mixlevel and roomtyp fields exist, indicating information about the audio production environment (mixing room).

#### 4.4.2.14 mixlevel - Mixing level - 5 bits

This 5-bit code indicates the absolute acoustic sound pressure level of an individual channel during the final audio mixing session. The 5-bit code represents a value in the range 0 to 31. The peak mixing level is 80 plus the value of mixlevel dB SPL, or 80 dB to 111 dB SPL. The peak mixing level is the acoustic level of a sine wave in a single channel whose peaks reach 100 percent in the PCM representation. The absolute SPL value is typically measured by means of pink noise with an RMS value of -20 dB or -30 dB with respect to the peak RMS sine wave level. The value of mixlevel is not typically used within the AC-3 decoder, but may be used by other parts of the audio reproduction equipment.

#### 4.4.2.15 roomtyp - Room type - 2 bits

This 2-bit code, shown in Table 4.7, indicates the type and calibration of the mixing room used for the final audio mixing session. The value of roomtyp is not typically used by the AC-3 decoder, but may be used by other parts of the audio reproduction equipment. If roomtyp is set to the reserved code, the decoder should still reproduce audio. The reserved code may be interpreted as "not indicated".

**roomtyp Type of mixing room** 00 Not indicated 01 Large room, X curve monitor 10 Small room, flat monitor 11 Reserved

**Table 4.7: Room type** 

# 4.4.2.16 dialnorm2 - Dialogue normalization, Ch2 - 5 bits

This 5-bit code has the same meaning as dialnorm, except that it applies to the second audio channel when acmod indicates two independent channels (dual mono 1 + 1 mode).

#### 4.4.2.17 compr2e - Compression gain word exists, Ch2 - 1 bit

If this bit is a 1, the following 8 bits represent a compression gain word for Ch2.

#### 4.4.2.18 compr2 - Compression gain word, Ch2 - 8 bits

This 8-bit word has the same meaning as compr, except that it applies to the second audio channel when acmod indicates two independent channels (dual mono 1 + 1 mode).

#### 4.4.2.19 langcod2e - Language code exists, Ch2 - 1 bit

If this bit is a 1, the following 8 bits (i.e. the element langcod2) shall be reserved. If this bit is a 0, the element langcod2 does not exist in the bit stream.

#### 4.4.2.20 langcod2 - Language code, Ch2 - 8 bits

This is an 8 bit reserved value. See langcod, clause [4.4.2.12](#page-7-0).

#### 4.4.2.21 audprodi2e - Audio production information exists, Ch2 - 1 bit

If this bit is a 1, the following two data fields exist indicating information about the audio production for Ch2.

#### 4.4.2.22 mixlevel2 - Mixing level, Ch2 - 5 bits

This 5-bit code has the same meaning as mixlevel, except that it applies to the second audio channel when acmod indicates two independent channels (dual mono 1 + 1 mode).

#### 4.4.2.23 roomtyp2 - Room type, Ch2 - 2 bits

This 2-bit code has the same meaning as roomtyp, except that it applies to the second audio channel when acmod indicates two independent channels (dual mono 1 + 1 mode).

#### 4.4.2.24 copyrightb - Copyright bit - 1 bit

If this bit has a value of 1, the information in the bit stream is indicated as protected by copyright. It has a value of 0 if the information is not indicated as protected.

### 4.4.2.25 origbs - Original bit stream - 1 bit

This bit has a value of 1 if this is an original bit stream. This bit has a value of 0 if this is a copy of another bit stream.

### 4.4.2.26 timecod1e, timecod2e - Time code (first and second) halves exists - 2 bits

These values indicate, as shown in Table 4.8, whether time codes follow in the bit stream. The time code can have a resolution of 1/64th of a frame (one frame = 1/30th of a second). Since only the high resolution portion of the time code is needed for fine synchronization, the 28 bit time code is broken into two 14 bit halves. The low resolution first half represents the code in 8 second increments up to 24 hours. The high resolution second half represents the code in 1/64th frame increments up to 8 seconds.

#### 4.4.2.27 timecod1 - Time code first half - 14 bits

The first 5 bits of this 14 bit field represent the time in hours, with valid values of 0 to 23. The next 6 bits represent the time in minutes, with valid values of 0 to 59. The final 3 bits represents the time in 8 second increments, with valid values of 0 - 7 (representing 0, 8, 16, ..., 56 seconds).

| timecod2e, timecod1e | Time code present             |
|----------------------|-------------------------------|
| 0,0                  | Not present                   |
| 0,1                  | First half (14 bits) present  |
| 1,0                  | Second half (14 bits) present |
| 1,1                  | Both halves (28 bits) present |

**Table 4.8: Time code exists** 

#### 4.4.2.28 timecod2 - Time code second half - 14 bits

The first 3 bits of this 14-bit field represent the time in seconds, with valid values from 0 to 7 (representing 0 to 7 seconds). The next 5 bits represents the time in frames, with valid values from 0 to 29. The final 6 bits represents fractions of 1/64th of a frame, with valid values from 0 to 63.

### 4.4.2.29 addbsie - Additional bit stream information exists - 1 bit

If this bit has a value of 1 there is additional bit stream information, the length of which is indicated by the next field. If this bit has a value of 0, there is no additional bit stream information.

#### 4.4.2.30 addbsil - Additional bit stream information length - 6 bits

This 6-bit code, which exists only if addbsie is a 1, indicates the length in bytes of additional bit stream information. The valid range of addbsil is 0 to 63, indicating 1 to 64 additional bytes, respectively. The decoder is not required to interpret this information, and thus shall skip over this number of bytes following in the data stream.

#### 4.4.2.31 addbsi - Additional bit stream information - ((addbsil + 1) x 8) bits

This field contains 1 to 64 bytes of any additional information included with the bit stream information structure.

# 4.4.3 audblk - Audio block

#### 4.4.3.1 blksw[ch] - Block switch flag - 1 bit

This flag, for channel [ch], indicates whether the current audio block was split into 2 sub-blocks during the transformation from the time domain into the frequency domain. A value of 0 indicates that the block was not split, and that a single 512 point TDAC transform was performed. A value of 1 indicates that the block was split into 2 sub-blocks of length 256, that the TDAC transform length was switched from a length of 512 points to a length of 256 points, and that 2 transforms were performed on the audio block (one on each sub-block). Transform length switching is described in more detail in clause 6.9.

# 4.4.3.2 dithflag[ch] - Dither flag - 1 bit

This flag, for channel [ch], indicates that the decoder should activate dither during the current block. Dither is described in detail in clause 6.3.4.

# 4.4.3.3 dynrnge - Dynamic range gain word exists - 1 bit

If this bit is a 1, the dynamic range gain word follows in the bit stream. If it is 0, the gain word is not present, and the previous value is reused, except for block 0 of a syncframe where if the control word is not present the current value of dynrng is set to 0.

# 4.4.3.4 dynrng - Dynamic range gain word - 8 bits

This encoder-generated gain word is applied to scale the reproduced audio as described in clause 6.7.

# 4.4.3.5 dynrng2e - Dynamic range gain word exists, Ch2 - 1 bit

If this bit is a 1, the dynamic range gain word for channel 2 follows in the bit stream. If it is 0, the gain word is not present, and the previous value is reused, except for block 0 of a syncframe where if the control word is not present the current value of dynrng2 is set to 0.

#### 4.4.3.6 dynrng2 - dynamic range gain word, Ch2 - 8 bits

This encoder-generated gain word is applied to scale the reproduced audio of Ch2, in the same manner as dynrng is applied to Ch1, as described in clause 6.7.

# 4.4.3.7 cplstre - Coupling strategy exists - 1 bit

If this bit is a 1, coupling information follows in the bit stream. If it is 0, new coupling information is not present, and coupling parameters previously sent are reused. This parameter shall not be set to 0 in block 0.

#### 4.4.3.8 cplinu - Coupling in use - 1 bit

If this bit is a 1, coupling is currently being utilized, and coupling parameters follow. If it is 0, coupling is not being utilized (all channels are independent) and no coupling parameters follow in the bit stream.

#### 4.4.3.9 chincpl[ch] - Channel in coupling - 1 bit

If this bit is a 1, then the channel indicated by the index [ch] is a coupled channel. If the bit is a 0, then this channel is not coupled. Since coupling is not used in the 1/0 mode, if any chincpl[] values exist there will be 2 to 5 values. Of the values present, at least two values will be 1, since coupling requires more than one coupled channel to be coupled.

# 4.4.3.10 phsflginu - Phase flags in use - 1 bit

If this bit (defined for 2/0 mode only) is a 1, phase flags are included with coupling coordinate information. Phase flags are described in clause 6.4.

#### 4.4.3.11 cplbegf - Coupling begin frequency code - 4 bits

This 4-bit code is interpreted as the sub-band number (0 to 15) which indicates the lower frequency band edge of the coupling channel (or the first active sub-band) as shown in Table 6.24.

### 4.4.3.12 cplendf - Coupling end frequency code - 4 bits

This 4-bit code indicates the upper band edge of the coupling channel. The upper band edge (or last active sub-band) is cplendf + 2, or a value between 2 and 17 (see Table 6.24).

The number of active coupling sub-bands is equal to ncplsubnd, which is calculated as:

ncplsubnd = 3 + cplendf - cplbegf.

#### 4.4.3.13 cplbndstrc[sbnd] - Coupling band structure - 1 bit

There are 18 coupling sub-bands defined in Table 6.24, each containing 12 frequency coefficients. The fixed 12-bin wide coupling sub-bands are converted into coupling bands, each of which may be wider than (a multiple of) 12 frequency bins. Each coupling band may contain one or more coupling sub-bands. Coupling coordinates are transmitted for each coupling band. Each band's coupling coordinate shall be applied to all the coefficients in the coupling band.

The coupling band structure indicates which coupling sub-bands are combined into wider coupling bands. When cplbndstrc[sbnd] is a 0, the sub-band number [sbnd] is not combined into the previous band to form a wider band, but starts a new 12 wide coupling band. When cplbndstrc[sbnd] is a 1, then the sub-band [sbnd] is combined with the previous band, making the previous band 12 bins wider. Each successive value of cplbndstrc which is a 1 will continue to combine sub-bands into the current band. When another cplbndstrc value of 0 is received, then a new band will be formed, beginning with the 12 bins of the current sub-band. The set of cplbndstrc[sbnd] values is typically considered an array.

Each bit in the array corresponds to a specific coupling sub-band in ascending frequency order. The first element of the array corresponds to the sub-band cplbegf, is always 0, and is not transmitted. (There is no reason to send a cplbndstrc bit for the first sub-band at cplbegf, since this bit would always be 0.) Thus, there are ncplsubnd-1 values of cplbndstrc transmitted. If there is only one coupling sub-band, then no cplbndstrc bits are sent.

The number of coupling bands, ncplbnd, may be computed from ncplsubnd and cplbndstrc:

ncplbnd = (ncplsubnd - (cplbndstrc[1] + ... + cplbndstrc[ncplsubnd - 1])).

#### 4.4.3.14 cplcoe[ch] - Coupling coordinates exist - 1 bit

Coupling coordinates indicate, for a given channel and within a given coupling band, the fraction of the coupling channel frequency coefficients to use to re-create the individual channel frequency coefficients. Coupling coordinates are conditionally transmitted in the bit stream. If new values are not delivered, the previously sent values remain in effect. See clause 6.4 for further information on coupling.

If cplcoe[ch] is 1, the coupling coordinates for the corresponding channel [ch] exist and follow in the bit stream. If the bit is 0, the previously transmitted coupling coordinates for this channel are reused. This parameter shall not be set to 0 in block 0, or in any block for which the corresponding channel is participating in coupling but was not participating in coupling in the previous block.

#### 4.4.3.15 mstrcplco[ch] - Master coupling coordinate - 2 bits

This per channel parameter establishes a per channel gain factor (increasing the dynamic range) for the coupling coordinates as shown in Table 4.9.

**Table 4.9: Master coupling coordinate** 

| mstrcplco[ch] | cplco[ch][bnd] gain multiplier |
|---------------|--------------------------------|
| 00            | 1                              |
| 01            | 2-3                            |
| 10            | 2-6                            |
| 11            | 2-9                            |

# 4.4.3.16 cplcoexp[ch][bnd] - Coupling coordinate exponent - 4 bits

Each coupling coordinate is composed of a 4-bit exponent and a 4-bit mantissa. This element is the value of the coupling coordinate exponent for channel [ch] and band [bnd]. The index [ch] only will exist for those channels which are coupled. The index [bnd] will range from 0 to ncplbnds. See clause 6.4.3 for further information on how to interpret coupling coordinates.

# 4.4.3.17 cplcomant[ch][bnd] - Coupling coordinate mantissa - 4 bits

This element is the 4-bit coupling coordinate mantissa for channel [ch] and band [bnd].

# 4.4.3.18 phsflg[bnd] - Phase flag - 1 bit

This element (only used in the 2/0 mode) indicates whether the decoder should phase invert the coupling channel mantissas when reconstructing the right output channel. The index [bnd] can range from 0 to ncplbnd. Phase flags are described in clause 6.4.

#### 4.4.3.19 rematstr - Rematrixing strategy - 1 bit

If this bit is a 1, then new rematrix flags are present in the bit stream. If it is 0, rematrix flags are not present, and the previous values should be reused. The rematstr parameter is present only in the 2/0 audio coding mode. This parameter shall not be set to 0 in block 0.

#### 4.4.3.20 rematflg[rbnd] - Rematrix flag - 1 bit

This bit indicates whether the transform coefficients in rematrixing band [rbnd] have been rematrixed. If this bit is a 1, then the transform coefficients in [rbnd] were rematrixed into sum and difference channels. If this bit is a 0, then rematrixing has not been performed in band [rbnd]. The number of rematrixing bands (and the number of values of [rbnd]) depend on coupling parameters as shown in Table 4.10. Rematrixing is described in clause 6.5.

**Table 4.10: Number of rematrixing bands** 

| Condition                          | No. of rematrixing bands |  |
|------------------------------------|--------------------------|--|
| cplinu == 0                        | 4                        |  |
| (cplinu == 1) && (cplbegf > 2)     | 4                        |  |
| (cplinu == 1) && (2 ≥ cplbegf > 0) | 3                        |  |
| (cplinu == 1) && (cplbegf == 0)    | 2                        |  |

#### 4.4.3.21 cplexpstr - Coupling exponent strategy - 2 bits

This element indicates the method of exponent coding that is used for the coupling channel as shown in Table 6.4. See clause 6.1 for explanation of each exponent strategy. This parameter shall not be set to 0 in block 0, or in any block for which coupling is enabled but was disabled in the previous block.

#### 4.4.3.22 chexpstr[ch] - Channel exponent strategy - 2 bits

This element indicates the method of exponent coding that is used for channel [ch], as shown in Table 6.4. This element exists for each full bandwidth channel. This parameter shall not be set to 0 in block 0.

# 4.4.3.23 lfeexpstr - Low frequency effects channel exponent strategy - 1 bit

This element indicates the method of exponent coding that is used for the lfe channel, as shown in Table 6.5. This parameter shall not be set to 0 in block 0.

### 4.4.3.24 chbwcod[ch] - Channel bandwidth code - 6 bits

The chbwcod[ch] element is an unsigned integer which defines the upper band edge for full-bandwidth channel [ch]. This parameter is only included for fbw channels which are not coupled. (See clause 6.1.3 on exponents for the definition of this parameter.) Valid values are in the range of 0 - 60. If a value greater than 60 is received, the bit stream is invalid and the decoder shall cease decoding audio and mute.

#### 4.4.3.25 cplabsexp - Coupling absolute exponent - 4 bits

This is an absolute exponent, which is used as a reference when decoding the differential exponents for the coupling channel.

### 4.4.3.26 cplexps[grp] - Coupling exponents - 7 bits

Each value of cplexps indicates the value of 3, 6, or 12 differentially-coded coupling channel exponents for the coupling exponent group [grp] for the case of d15, d25, or d45 coding, respectively. The number of cplexps values transmitted equals ncplgrps, which may be determined from cplbegf, cplendf, and cplexpstr. Refer to clause 6.1.3 for further information.

### 4.4.3.27 exps[ch][grp] - Channel exponents - 4 bits or 7 bits

These elements represent the encoded exponents for channel [ch]. The first element ([grp] = 0) is a 4-bit absolute exponent for the first (DC term) transform coefficient. The subsequent elements ([grp] > 0) are 7-bit representations of a group of 3, 6, or 12 differentially coded exponents (corresponding to d15, d25, d45 exponent strategies respectively). The number of groups for each channel, nchgrps[ch], is determined from cplbegf if the channel is coupled, or chbwcod[ch] if the channel is not coupled. Refer to clause 6.1.3 for further information.

### 4.4.3.28 gainrng[ch] - Channel gain range code - 2 bits

This per channel 2-bit element may be used to determine a block floating-point shift value for the inverse TDAC transform filter bank. Use of this code allows increased dynamic range to be obtained from a limited word length transform computation. For further information see clause 6.9.5.

#### 4.4.3.29 lfeexps[grp] - Low frequency effects channel exponents - 4 bits or 7 bits

These elements represent the encoded exponents for the lfe channel. The first element ([grp] = 0) is a 4-bit absolute exponent for the first (DC term) transform coefficient. There are two additional elements (nlfegrps = 2) which are 7-bit representations of a group of 3 differentially coded exponents. The total number of lfe channel exponents (nlfemant) is 7.

#### 4.4.3.30 baie - Bit allocation information exists - 1 bit

If this bit is a 1, then five separate fields (totalling 11 bits) follow in the bit stream. Each field indicates parameter values for the bit allocation process. If this bit is a 0, these fields do not exist. Further details on these fields may be found in clause 6.2. This parameter shall not be set to 0 in block 0.

#### 4.4.3.31 sdcycod - Slow decay code - 2 bits

This 2-bit code specifies the slow decay parameter in the bit allocation process.

# 4.4.3.32 fdcycod - Fast decay code - 2 bits

This 2-bit code specifies the fast decay parameter in the decode bit allocation process.

#### 4.4.3.33 sgaincod - Slow gain code - 2 bits

This 2-bit code specifies the slow gain parameter in the decode bit allocation process.

### 4.4.3.34 dbpbcod - dB per bit code - 2 bits

This 2-bit code specifies the dB per bit parameter in the bit allocation process.

### 4.4.3.35 floorcod - Masking floor code - 3 bits

This 3-bit code specifies the floor code parameter in the bit allocation process.

# 4.4.3.36 snroffste - SNR offset exists - 1 bit

If this bit has a value of 1, a number of bit allocation parameters follow in the bit stream. If this bit has a value of 0, SNR offset information does not follow, and the previously transmitted values should be used for this block. The bit allocation process and these parameters are described in clause 6.2. This parameter shall not be set to 0 in block 0.

#### 4.4.3.37 csnroffst - Coarse SNR offset - 6 bits

This 6-bit code specifies the coarse SNR offset parameter in the bit allocation process.

#### 4.4.3.38 cplfsnroffst - Coupling fine SNR offset - 4 bits

This 4-bit code specifies the coupling channel fine SNR offset in the bit allocation process.

# 4.4.3.39 cplfgaincod - Coupling fast gain code - 3 bits

This 3-bit code specifies the coupling channel fast gain code used in the bit allocation process.

# 4.4.3.40 fsnroffst[ch] - Channel fine SNR offset - 4 bits

This 4-bit code specifies the fine SNR offset used in the bit allocation process for channel [ch].

#### 4.4.3.41 fgaincod[ch] - Channel fast gain code - 3 bits

This 3-bit code specifies the fast gain parameter used in the bit allocation process for channel [ch].

#### 4.4.3.42 lfefsnroffst - Low frequency effects channel fine SNR offset - 4 bits

This 4-bit code specifies the fine SNR offset parameter used in the bit allocation process for the lfe channel.

#### 4.4.3.43 lfefgaincod - Low frequency effects channel fast gain code - 3 bits

This 3-bit code specifies the fast gain parameter used in the bit allocation process for the lfe channel.

# 4.4.3.44 cplleake - Coupling leak initialization exists - 1 bit

If this bit is a 1, leak initialization parameters follow in the bit stream. If this bit is a 0, the previously transmitted values still apply. This parameter shall not be set to 0 in block 0, or in any block for which coupling is enabled but was disabled in the previous block.

#### 4.4.3.45 cplfleak - Coupling fast leak initialization - 3 bits

This 3-bit code specifies the fast leak initialization value for the coupling channel's excitation function calculation in the bit allocation process.

# 4.4.3.46 cplsleak - Coupling slow leak initialization - 3 bits

This 3-bit code specifies the slow leak initialization value for the coupling channel's excitation function calculation in the bit allocation process.

#### 4.4.3.47 deltbaie - Delta bit allocation information exists - 1 bit

If this bit is a 1, some delta bit allocation information follows in the bit stream. If this bit is a 0, the previously transmitted delta bit allocation information still applies, except for block 0. If deltbaie is 0 in block 0, then cpldeltbae and deltbae[ch] are set to the binary value "10", and no delta bit allocation is applied. Delta bit allocation is described in clause 6.2.2.

# 4.4.3.48 cpldeltbae - Coupling delta bit allocation exists - 2 bits

This 2-bit code indicates the delta bit allocation strategy for the coupling channel, as shown in Table 4.11. If the reserved state is received, the decoder should not decode audio, and should mute. This parameter shall not be set to "00" in block 0, or in any block for which coupling is enabled but was disabled in the previous block.

**Table 4.11: Delta bit allocation exist states** 

| cpldeltbae, deltbae | Code                   |  |
|---------------------|------------------------|--|
| 00                  | Reuse previous state   |  |
| 01                  | New info follows       |  |
| 10                  | Perform no delta alloc |  |
| 11                  | Reserved               |  |

# 4.4.3.49 deltbae[ch] - Delta bit allocation exists - 2 bits

This per full bandwidth channel 2-bit code indicates the delta bit allocation strategy for the corresponding channel, as shown in Table 4.11. This parameter shall not be set to "00" in block 0.

#### 4.4.3.50 cpldeltnseg - Coupling delta bit allocation number of segments - 3 bits

This 3-bit code indicates the number of delta bit allocation segments that exist for the coupling channel. The value of this parameter ranges from 1 to 8, and is calculated by adding 1 to the 3-bit binary number represented by the code.

# 4.4.3.51 cpldeltoffst[seg] - Coupling delta bit allocation offset - 5 bits

The first 5-bit code ([seg] = 0) indicates the number of the first bit allocation band (as specified in clause 6.4.2) of the coupling channel for which delta bit allocation values are provided. Subsequent codes indicate the offset from the previous delta segment end point to the next bit allocation band for which delta bit allocation values are provided.

#### 4.4.3.52 cpldeltlen[seg] - Coupling delta bit allocation length - 4 bits

Each 4-bit code indicates the number of bit allocation bands that the corresponding segment spans.

### 4.4.3.53 cpldeltba[seg] - Coupling delta bit allocation - 3 bits

This 3-bit value is used in the bit allocation process for the coupling channel.

Each 3-bit code indicates an adjustment to the default masking curve computed in the decoder. The deltas are coded as shown in Table 4.12.

**Table 4.12: Bit allocation deltas** 

| cpldeltba, deltba | Adjustment<br>(dB) |
|-------------------|--------------------|
| 000               | -24                |
| 001               | -18                |
| 010               | -12                |
| 011               | -6                 |
| 100               | +6                 |
| 101               | +12                |
| 110               | +18                |
| 111               | +24                |

# 4.4.3.54 deltnseg[ch] - Channel delta bit allocation number of segments - 3 bits

These per full bandwidth channel elements are 3-bit codes indicating the number of delta bit allocation segments that exist for the corresponding channel. The value of this parameter ranges from 1 to 8, and is calculated by adding 1 to the 3-bit binary code.

# 4.4.3.55 deltoffst[ch][seg] - Channel delta bit allocation offset - 5 bits

The first 5-bit code ([seg] = 0) indicates the number of the first bit allocation band (see clause 6.2.2) of the corresponding channel for which delta bit allocation values are provided. Subsequent codes indicate the offset from the previous delta segment end point to the next bit allocation band for which delta bit allocation values are provided.

# 4.4.3.56 deltlen[ch][seg] - Channel delta bit allocation length - 4 bits

Each 4-bit code indicates the number of bit allocation bands that the corresponding segment spans.

#### 4.4.3.57 deltba[ch][seg] - Channel delta bit allocation - 3 bits

This 3-bit value is used in the bit allocation process for the indicated channel. Each 3-bit code indicates an adjustment to the default masking curve computed in the decoder. The deltas are coded as shown in Table 4.13.

#### 4.4.3.58 skiple - Skip length exists - 1 bit

If this bit is a 1, then the skipl parameter follows in the bit stream. If this bit is a 0, skipl does not exist.

#### 4.4.3.59 skipl - Skip length - 9 bits

This 9-bit code indicates the number of dummy bytes to skip (ignore) before unpacking the mantissas of the current audio block.

#### 4.4.3.60 skipfld - Skip field - (skipl x 8) bits

This field contains the bytes of data to be skipped, as indicated by the skipl parameter.

#### 4.4.3.61 chmant[ch][bin] - Channel mantissas - 0 bits to 16 bits

The actual quantized mantissa values for the indicated channel. Each value may contain from 0 to as many as 16 bits. The number of mantissas for the indicated channel is equal to nchmant[ch], which may be determined from chbwcod[ch] (see clause 6.1.3) if the channel is not coupled, or from cplbegf (see clause 6.4.2) if the channel is coupled. Detailed information on packed mantissa data is in clause 6.3.

#### 4.4.3.62 cplmant[bin] - Coupling mantissas - 0 bits to 16 bits

The actual quantized mantissa values for the coupling channel. Each value may contain from 0 to as many as 16 bits. The number of mantissas for the coupling channel is equal to ncplmant, which may be determined from:

ncplmant = 12 x ncplsubnd.

#### 4.4.3.63 lfemant[bin] - Low frequency effects channel mantissas - 0 bits to 16 bits

The actual quantized mantissa values for the lfe channel. Each value may contain from 0 to as many as 16 bits. The value of nlfemant is 7, so there are 7 mantissa values for the lfe channel.

# 4.4.4 auxdata - Auxiliary data field

#### 4.4.4.0 Introduction

Unused data at the end of a syncframe will exist whenever the encoder does not utilize all available data for encoding the audio signal. This may occur if the final bit allocation falls short of using all available bits, or if the input audio signal simply does not require all available bits to be coded transparently. Or, the encoder may be instructed to intentionally leave some bits unused by audio so that they are available for use by auxiliary data. Since the number of bits required for auxiliary data may be smaller than the number of bits available (which will be time varying) in any particular syncframe, a method is provided to signal the number of actual auxiliary data bits in each syncframe.

#### 4.4.4.1 auxbits - Auxiliary data bits - nauxbits bits

This field contains auxiliary data. The total number of bits in this field is:

nauxbits = (bits in syncframe) - (bits used by all bit stream elements except for auxbits).

The number of bits in the syncframe can be determined from the frame size code (frmsizcod) and Table 4.13. The number of bits used includes all bits used by bit stream elements with the exception of auxbits. Any dummy data which has been included with skip fields (skipfld) is included in the used bit count. The length of the auxbits field is adjusted by the encoder such that the crc2 element falls on the last 16-bit word of the syncframe.

**Table 4.13: Frame size code table (1 word = 16 bits) frmsizecod Nominal bit rate (kbit/s) Words/syncframe fs = 32 kHz Words/syncframe fs = 44,1 kHz Words/syncframe fs = 48 kHz**  000000 (0) 32 96 69 64 000001 (0) 32 96 70 64 000010 (1) 40 120 87 80 000011 (1) 40 120 88 80

| frmsizecod  | Nominal<br>bit rate<br>(kbit/s) | Words/syncframe<br>fs = 32 kHz | Words/syncframe<br>fs = 44,1 kHz | Words/syncframe<br>fs = 48 kHz |
|-------------|---------------------------------|--------------------------------|----------------------------------|--------------------------------|
| 000000 (0)  | 32                              | 96                             | 69                               | 64                             |
| 000001 (0)  | 32                              | 96                             | 70                               | 64                             |
| 000010 (1)  | 40                              | 120                            | 87                               | 80                             |
| 000011 (1)  | 40                              | 120                            | 88                               | 80                             |
| 000100 (2)  | 48                              | 144                            | 104                              | 96                             |
| 000101 (2)  | 48                              | 144                            | 105                              | 96                             |
| 000110 (3)  | 56                              | 168                            | 121                              | 112                            |
| 000111 (3)  | 56                              | 168                            | 122                              | 112                            |
| 001000 (4)  | 64                              | 192                            | 139                              | 128                            |
| 001001 (4)  | 64                              | 192                            | 140                              | 128                            |
| 001010 (5)  | 80                              | 240                            | 174                              | 160                            |
| 001011 (5)  | 80                              | 240                            | 175                              | 160                            |
| 001100 (6)  | 96                              | 288                            | 208                              | 192                            |
| 001101 (6)  | 96                              | 288                            | 209                              | 192                            |
| 001110 (7)  | 112                             | 336                            | 243                              | 224                            |
| 001111 (7)  | 112                             | 336                            | 244                              | 224                            |
| 010000 (8)  | 128                             | 384                            | 278                              | 256                            |
| 010001 (8)  | 128                             | 384                            | 279                              | 256                            |
| 010010 (9)  | 160                             | 480                            | 348                              | 320                            |
| 010011 (9)  | 160                             | 480                            | 349                              | 320                            |
| 010100 (10) | 192                             | 576                            | 417                              | 384                            |
| 010101 (10) | 192                             | 576                            | 418                              | 384                            |
| 010110 (11) | 224                             | 672                            | 487                              | 448                            |

| frmsizecod                        | Nominal<br>bit rate<br>(kbit/s) | Words/syncframe<br>fs = 32 kHz | Words/syncframe<br>fs = 44,1 kHz | Words/syncframe<br>fs = 48 kHz |
|-----------------------------------|---------------------------------|--------------------------------|----------------------------------|--------------------------------|
| 010111 (11)                       | 224                             | 672                            | 488                              | 448                            |
| 011000 (12)                       | 256                             | 768                            | 557                              | 512                            |
| 011001 (12)                       | 256                             | 768                            | 558                              | 512                            |
| 011010 (13)                       | 320                             | 960                            | 696                              | 640                            |
| 011011 (13)                       | 320                             | 960                            | 697                              | 640                            |
| 011100 (14)                       | 384                             | 1 152                          | 835                              | 768                            |
| 011101 (14)                       | 384                             | 1 152                          | 836                              | 768                            |
| 011110 (15)                       | 448                             | 1 344                          | 975                              | 896                            |
| 011111 (15)                       | 448                             | 1 344                          | 976                              | 896                            |
| 100000 (16)                       | 512                             | 1 536                          | 1 114                            | 1 024                          |
| 100001 (16)                       | 512                             | 1 536                          | 1 115                            | 1 024                          |
| 100010 (17)                       | 576                             | 1 728                          | 1 253                            | 1 152                          |
| 100011 (17)                       | 576                             | 1 728                          | 1 254                            | 1 152                          |
| 100100 (18)                       | 640                             | 1 920                          | 1 393                            | 1 280                          |
| 100101 (18)                       | 640                             | 1 920                          | 1 394                            | 1 280                          |
| NOTE:<br>fs : sampling frequency. |                                 |                                |                                  |                                |

If the number of user bits indicated by auxdatal is smaller than the number of available aux bits nauxbits, the user data is located at the end of the auxbits field. This allows a decoder to find and unpack the auxdatal user bits without knowing the value of nauxbits (which can only be determined by decoding the audio in the entire syncframe). The order of the user data in the auxbits field is forward. Thus the aux data decoder (which may not decode any audio) may simply look to the end of the AC-3 syncframe to find auxdatal, backup auxdatal bits (from the beginning of auxdatal) in the data stream, and then unpack auxdatal bits moving forward in the data stream.

# 4.4.4.2 auxdatal - Auxiliary data length - 14 bits

This 14-bit integer value indicates the length, in bits, of the user data in the auxbits auxiliary field.

#### 4.4.4.3 auxdatae - Auxiliary data exists - 1 bit

If this bit is a 1, then the auxdatal parameter precedes in the bit stream. If this bit is a 0, auxdatal does not exist, and there is no user data.

# 4.4.5 errorcheck - Frame error detection field

#### 4.4.5.1 crcrsv - CRC reserved bit - 1 bit

Reserved for use in specific applications to ensure crc2 will not be equal to the sync word. Use of this bit is optional by encoders. If the crc2 calculation results in a value equal to the syncword, the crcrsv bit may be inverted. This will result in a crc2 value which is not equal to the syncword.

#### 4.4.5.2 crc2 - Cyclic redundancy check 2 - 16 bits

The 16-bit CRC applies to the entire syncframe. The details of the CRC checking are described in clause 6.10.1.

# 4.5 Bit stream constraints

The following constraints shall be imposed upon the encoded bit stream by the AC-3 encoder. These constraints allow AC-3 decoders to be manufactured with smaller input memory buffers:

- 1) The combined size of the syncinfo fields, the bsi fields, block 0 and block 1 combined, shall not exceed 5/8 of the syncframe.
- 2) The combined size of the block 5 mantissa data, the auxiliary data fields, and the errorcheck fields shall not exceed the final 3/8 of the syncframe.