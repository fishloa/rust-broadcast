# Annex E (normative): Enhanced AC-3

## E.0 Scope

This annex defines the audio coding algorithm denoted as Enhanced AC-3 ("E-AC-3") and the alterations to the AC-3 bit stream necessary to convey E-AC-3 data along with a reference decoding process. E-AC-3 bit streams are similar in nature to standard AC-3 bit streams but are not backwardly compatible (i.e., they are not decodable by standard AC-3 decoders). This annex specifies either directly or by reference the bit stream syntax of E-AC-3. When an AC-3 bit stream carries E-AC-3 bit stream syntax, it is referred herein to as an E-AC-3 bit stream.

## E.1 Bit stream syntax and semantics specification

### E.1.1 Indication of Enhanced AC-3 bit stream syntax

An AC-3 bit stream is indicated as using the Enhanced AC-3 bit stream syntax described in this annex when the bit stream identification (bsid) field is set to 16. To enable differentiation between an AC-3 bit stream and an E-AC-3 bit stream, the bsid field is placed the same number of bits from the beginning of the syncframe as defined in the syntax below.

### E.1.2 Syntax specification

### E.1.2.0 E-AC-3\_bit\_stream and syncframe

Unless otherwise specified, all bit stream elements shall have the same meaning and purpose as described in the body and Annex D of the present document. Single bit boolean values shall be treated as '1' equals TRUE. A continuous audio bit stream consists of a sequence of synchronization frames.

```
Syntax
E-AC-3_bit_stream() 
{ 
   while(true) 
   { 
 syncframe() ; 
   } 
} /* end of bit stream */
```

The syncframe consists of the syncinfo, bsi and audfrm fields, up to 6 coded audblk fields, the auxdata field, and the errorcheck field.

```
Syntax
syncframe() 
{ 
 syncinfo() ; 
 bsi() ; 
 audfrm() ; 
  for(blk = 0; blk < number_of_blocks_per_syncframe; blk++) 
  { 
 audblk() ; 
  } 
 auxdata() ; 
 errorcheck() ; 
} /* end of syncframe */
```

Each of the bit stream elements, and their length, are itemized in the following pseudo code. Note that all bit stream elements arrive most significant bit first, or left bit first, in time.

#### E.1.2.1 syncinfo - Synchronization information

```
Syntax
syncinfo()
{
syncword
} /* end of syncinfo */

Syntax
Word size
```

#### E.1.2.2 bsi - Bit stream information

```
Word size
Syntax
bsi()
         substreamid
        numblkscod ....................................
        if(compre) {compr} . . . . . . . . . . . . . . . . . . .
        if (acmod == 0x0) /* if 1+1 mode (dual mono, so some items need a second value) */
                 if(compr2e) {compr2} 8
         if(strmtyp == 0x1) /* if dependent stream */
                 chanmape ....................................
                 if(chanmape) {chanmap} . . . . . . . . . . . . . . . . . . .
         if(mixmdate) /* mixing metadata */
                 if(acmod > 0x2) /* if more than 2 channels */ {dmixmod} ....................................
                 if((acmod & 0x1) && (acmod > 0x2)) /* if three front channels exist */
                         lorocmixlev ....................................
                 if(acmod & 0x4) /* if a surround channel exists */
                         lorosurmixlev ....................................
                 if(lfeon) /* if the LFE channel exists */
                          if(lfemixlevcode) {lfemixlevcod} . . . . . . . . . . . . . . . . . . .
                 if (strmtyp == 0x0) /* if independent stream */
                          if(pgmscle) {pgmscl}....................................
                          if (acmod == 0x0) /* if 1+1 mode (dual mono, so some items need a second value) */
                                  pqmsc12e ....................................
                                  if(pqmsc12e) {pqmsc12} ....................................
                          extpqmscle ....................................
                          if(extpgmscle) {extpgmscl} ....................................
                          if (mixdef == 0x1) /* mixing option 2 */
                                  {\tt premixcmpsel} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.5cm} \underline{\phantom{a}} \hspace{1.
                                  dresre ..................................
                          else if(mixdef == 0x2) /* mixing option 3 */ {mixdata} ...................................
```

```
Word size
Syntax
     else if (mixdef == 0x3) /* mixing option 4 */
      mixdeflen ....................................
      mixdata2e ....................................
       if (mixdata2e)
        drcsrc ..................................
        premixcmpscl ....................................
        extpgmlscle ....................................
        if(extpgmlscle) {extpgmlscl} .....
        extpomcscle
        if(extpgmcscle) {extpgmcscl} . . . . . . . . . . . . . . . . . . .
        extpgmrscle .....
        if(extpgmrscle) {extpgmrscl} ....................................
        extpgmlsscle .....
        if(extpgmlsscle) {extpgmlsscl} .....
        extpgmrsscle ....................................
        if(extpgmrsscle) {extpgmrsscl} .....
        {\tt extpgmlfescle} \dots \dots \dots \dots \dots \dots \dots \dots \dots \dots \dots \dots \dots \dots \dots \dots \dots \dots \dots
        if(extpgmlfescle) {extpgmlfescl} ......4
        dmixscle ....................................
        if(dmixscle) {dmixscl} ....................................
        addche
        if (addche)
          extpgmaux1scle ....................................
          if(extpgmaux1scle) {extpgmaux1scl} ..... 4
          extpgmaux2scle ....................................
          if(extpomaux2scle) {extpomaux2scl} . . . . . . . . . . . . . . . . . . .
      mixdata3e ....................................
      if(mixdata3e)
        addspchdate ....................................
        if (addspchdate)
          spchdat1 ..... 5
          spchanlatt ...................................
          addspchdatle ....
          if (addspdat1e)
            spchdat2 ....................................
      mixdata ..... (8*(mixdeflen+2)) - no. mixdata bits
      mixdatafill ...................................
     if (acmod < 0x2) /* if mono or dual mono source */
      paninfoe ....................................
       if (paninfoe)
        paninfo ....................................
       if(acmod == 0x0) /* if 1+1 mode (dual mono - some items need a second value) */
        paninfo2e ....................................
        if (paninfo2e)
          panmean2 ..... 8
          paninfo2 ...... 6
       }
     frmmixcfginfoe ...
     if(frmmixcfginfoe) /* mixing configuration information */
       if(numblkscod == 0x0) {blkmixcfginfo[0]} ......
      else
        for(blk = 0; blk < number of blocks per sync frame; blk++)</pre>
```

```
Word size
Syntax
          blkmixcfginfoe ......
          if(blkmixcfginfoe){blkmixcfginfo[blk]} ....................................
       }
     }
 infomdate ......
                             . . . . . . . . . . . . . . . . . . . . 
 if(infomdate) /* informational metadata */
   copyrightb ....................................
   origbs ...... 1
   if (acmod == 0x2) /* if in 2/0 mode */
     dsurmod ....................................
     dheadphonmod ....................................
   if(acmod >= 0x6) /* if both surround channels exist */ {dsurexmod} ....................................
   audprodie 1
   if (audprodie)
     roomtyp . . . . . . . . . . . . . . . . . . .
     adconvtyp ....................................
   if(acmod == 0x0) /* if 1+1 mode (dual mono, so some items need a second value) */
     audprodi2e ....................................
     if(audprodi2e)
       roomtvp2 ..... 2
       adconvtyp2 ..... 1
   if(fscod < 0x3) /* if not half sample rate */ {sourcefscod}....................................
 if( (strmtyp == 0x0) && (numblkscod != 0x3) ) {convsync} ....................................
 if(strmtyp == 0x2) /* if bit stream converted from AC-3 */
   if(numblkscod == 0x3) /* 6 blocks per syncframe */ {blkid = 1}
   else {blkid} ....................................
   if(blkid) {frmsizecod} ......6
 addbsie ....................................
 if(addbsie)
   addbsil ..... 6
```

#### E.1.2.3 audfrm - Audio frame

```
Syntax
audfrm()
{
/* these fields for audio frame exist flags and strategy data */
    if (numblkscod == 0x3) /* six blocks per syncframe */
        expstre
```

```
Syntax Word size
 bamode ..................................................................................... 1 
 frmfgaincode ............................................................................... 1 
 dbaflde .................................................................................... 1 
 skipflde ................................................................................... 1 
  spxattene .................................................................................. 1 
/* these fields for coupling data */ 
  if(acmod > 0x1) 
  { 
 cplstre[0] = 1 
 cplinu[0] ............................................................................... 1 
 for(blk = 1; blk < number_of_blocks_per_sync_frame; blk++) 
 { 
 cplstre[blk] ......................................................................... 1 
 if(cplstre[blk] == 1) {cplinu[blk]} .................................................. 1 
 else {cplinu[blk] = cplinu[blk-1]} 
 } 
  } 
  else 
  { 
 for(blk = 0; blk < number_of_blocks_per_sync_frame; blk++) {cplinu[blk] = 0} 
  } 
/* these fields for exponent strategy data */ 
  if(expstre) 
  { 
 for(blk = 0; blk < number_of_blocks_per_sync_frame; blk++) 
 { 
 if(cplinu[blk] == 1) {cplexpstr[blk]} ................................................ 2 
 for(ch = 0; ch < nfchans; ch++) {chexpstr[blk][ch]} .................................. 2 
 } 
  } 
  else 
  { 
 ncplblks = 0 
 for(blk = 0; blk < number_of_blocks_per_sync_frame; blk++) {ncplblks += cplinu[blk]} 
 if( (acmod > 0x1) && (ncplblks > 0) ) {frmcplexpstr} .................................... 5 
 for(ch = 0; ch < nfchans; ch++) {frmchexpstr[ch]} ....................................... 5 
 /* cplexpstr[blk] and chexpstr[blk][ch] derived from table lookups - see Table E.1.8*/ 
  } 
  if(lfeon) 
  { 
 for(blk = 0; blk < number_of_blocks_per_sync_frame; blk++) {lfeexpstr[blk]} ............. 1 
  } 
/* These fields for converter exponent strategy data */ 
 if(strmtyp == 0x0) 
 { 
 if(numblkscod != 0x3) {convexpstre} ..................................................... 1 
 else {convexpstre = 1} 
 if(convexpstre == 1) 
 { 
 for(ch = 0; ch < nfchans; ch++) {convexpstr[ch]} ..................................... 5 
 } 
 } 
/* these fields for AHT data */ 
  if(ahte) 
  { 
  /* coupling can use AHT only when coupling in use for all blocks */ 
 /* ncplregs derived from cplstre and cplexpstr - see clause E.2.4.2 */ 
 if( (ncplblks == 6) && (ncplregs ==1) ) {cplahtinu} ..................................... 1 
 else {cplahtinu = 0} 
 for(ch = 0; ch < nfchans; ch++) 
 { 
 /* nchregs derived from chexpstr - see clause E.2.4.2 */ 
 if(nchregs[ch] == 1) {chahtinu[ch]} .................................................. 1 
 else {chahtinu[ch] = 0} 
 } 
 if(lfeon) 
 { 
 /* nlferegs derived from lfeexpstr - see clause E.2.4.2 */ 
 if(nlferegs == 1) {lfeahtinu} ........................................................ 1 
 else {lfeahtinu = 0} 
 } 
  } 
/* these fields for audio frame SNR offset data */ 
  if(snroffststr == 0x0) 
  { 
 frmcsnroffst ............................................................................ 6 
 frmfsnroffst ............................................................................ 4
```

```
Syntax Word size
  } 
/* these fields for audio frame transient pre-noise processing data */ 
  if(transproce) 
  { 
 for(ch = 0; ch < nfchans; ch++) 
 { 
 chintransproc[ch] .................................................................... 1 
 if(chintransproc[ch]) 
 { 
 transprocloc[ch] ................................................................. 10 
 transproclen[ch] .................................................................. 8 
 } 
 } 
  } 
/* These fields for spectral extension attenuation data */ 
 if(spxattene) 
 { 
 for(ch = 0; ch < nfchans; ch++) 
 { 
 chinspxatten[ch] ..................................................................... 1 
 if(chinspxatten[ch]) 
 { 
 spxattencod[ch] ................................................................... 5 
 } 
 } 
 } 
/* these fields for block start information */ 
  if (numblkscod != 0x0) {blkstrtinfoe} ...................................................... 1 
  else {blkstrtinfoe = 0} 
  if(blkstrtinfoe) 
  { 
 /* nblkstrtbits determined from frmsiz (see clause E.1.3.2.27) */ 
 blkstrtinfo .................................................................. nblkstrtbits 
  } 
/* these fields for syntax state initialization */ 
  for(ch = 0; ch < nfchans; ch++) 
  { 
 firstspxcos[ch] = 1 
 firstcplcos[ch] = 1 
  } 
  firstcplleak = 1 
} /* end of audfrm */
```

#### E.1.2.4 audblk - Audio block

```
Syntax Word size
audblk() 
{ 
/* these fields for block switch and dither flags */ 
  if(blkswe) 
  { 
 for(ch = 0; ch < nfchans; ch++) {blksw[ch]} ............................................. 1 
  } 
  else 
  { 
 for(ch = 0; ch < nfchans; ch++) {blksw[ch] = 0} 
  } 
  if(dithflage) 
  { 
 for(ch = 0; ch < nfchans; ch++) {dithflag[ch]} .......................................... 1 
  } 
  else 
  { 
 for(ch = 0; ch < nfchans; ch++) {dithflag[ch] = 1} /* dither on */ 
  } 
/* these fields for dynamic range control */ 
 dynrnge 1 
  if(dynrnge) {dynrng} ....................................................................... 8 
  if(acmod == 0x0) /* if 1+1 mode */ 
  { 
 dynrng2e ................................................................................ 1 
 if(dynrng2e) {dynrng2} .................................................................. 8 
  } 
/* these fields for spectral extension strategy information */
```

```
Syntax Word size
  if(blk == 0) {spxstre = 1} 
  else {spxstre} ............................................................................. 1 
  if(spxstre) 
  { 
 spxinu .................................................................................. 1 
 if(spxinu) 
 { 
 if(acmod == 0x1) 
 { 
 chinspx[0] = 1 
 } 
 else 
 { 
 for(ch = 0; ch < nfchans; ch++) {chinspx[ch]} ..................................... 1 
 } 
 spxstrtf ............................................................................. 2 
 spxbegf .............................................................................. 3 
 spxendf .............................................................................. 3 
 if(spxbegf < 6) {spx_begin_subbnd = spxbegf + 2} 
 else {spx_begin_subbnd = spxbegf * 2 - 3} 
 if(spxendf < 3) {spx_end_subbnd = spxendf + 5} 
 else {spx_end_subbnd = spxendf * 2 + 3} 
 spxbndstrce .......................................................................... 1 
 if(spxbndstrce) 
 { 
 for(bnd = spx_begin_subbnd+1; bnd < spx_end_subbnd ; bnd++) {spxbndstrc[bnd]} ..... 1 
 } 
 } 
 else /* !spxinu */ 
 { 
 for(ch = 0; ch < nfchans; ch++) 
 { 
 chinspx[ch] = 0 
 firstspxcos[ch] = 1 
 } 
 } 
  } 
/* these fields for spectral extension coordinates */ 
  if(spxinu) 
  { 
 for(ch = 0; ch < nfchans; ch++) 
 { 
 if(chinspx[ch]) 
 { 
 if(firstspxcos[ch]) 
 { 
 spxcoe[ch] = 1 
 firstspxcos[ch] = 0 
 } 
 else /* !firstspxcos[ch] */ {spxcoe[ch]} .......................................... 1 
 if(spxcoe[ch]) 
 { 
 spxblnd[ch] .................................................................... 5 
 mstrspxco[ch] .................................................................. 2 
 /* nspxbnds determined from spx_begin_subbnd, spx_end_subbnd, and spxbndstrc[ ] */ 
 for(bnd = 0; bnd < nspxbnds; bnd++) 
 { 
 spxcoexp[ch][bnd] ........................................................... 4 
 spxcomant[ch][bnd] .......................................................... 2 
 } 
 } 
 } 
 else /* !chinspx[ch] */ 
 { 
 firstspxcos[ch] = 1 
 } 
 } 
  } 
/* These fields for coupling strategy and enhanced coupling strategy information */ 
 if(cplstre[blk]) 
 { 
 if (cplinu[blk]) 
 { 
 ecplinu .............................................................................. 1 
 if (acmod == 0x2) 
 {
```

```
Syntax Word size
 chincpl[0] = 1 
 chincpl[1] = 1 
 } 
 else 
 { 
 for(ch = 0; ch < nfchans; ch++) {chincpl[ch]} ..................................... 1 
 } 
 if (ecplinu == 0) /* standard coupling in use */ 
 { 
 if(acmod == 0x2) {phsflginu} /* if in 2/0 mode */ ................................. 1 
 cplbegf ........................................................................... 4
 if (spxinu == 0) /* if SPX not in use */ 
 { 
 cplendf ........................................................................ 4
 } 
 else /* SPX in use */ 
 { 
 if (spxbegf < 6) 
 { 
 /* note that in this case the value of cplendf may be negative */ 
 cplendf = spxbegf - 2 
 } 
 else 
 { 
 cplendf = (spxbegf * 2) - 7 
 } 
 } 
 /* ncplsubnd = 3 + cplendf - cplbegf */ 
 cplbndstrce ....................................................................... 1
 if(cplbndstrce) 
 { 
 for(bnd = 1; bnd < ncplsubnd; bnd++) {cplbndstrc[bnd]} ......................... 1 
 } 
 } 
 else /* enhanced coupling in use */ 
 { 
 ecplbegf .......................................................................... 4
 if(ecplbegf < 3) {ecpl_begin_subbnd = ecplbegf * 2} 
 else if(ecplbegf < 13) {ecpl_begin_subbnd = ecplbegf + 2} 
 else {ecpl_begin_subbnd = ecplbegf * 2 - 10} 
 if (spxinu == 0) /* if SPX not in use */ 
 { 
 ecplendf ....................................................................... 4
 ecpl_end_subbnd = ecplendf + 7 
 } 
 else /* SPX in use */ 
 { 
 if (spxbegf < 6) 
 { 
 ecpl_end_subbnd = spxbegf + 5 
 } 
 else 
 { 
 ecpl_end_subbnd = spxbegf * 2 
 } 
 } 
 ecplbndstrce ...................................................................... 1
 if (ecplbndstrce) 
 { 
 for(sbnd = max(9, ecpl_begin_subbnd+1); sbnd < ecpl_end_subbnd; sbnd++) 
 { 
 ecplbndstrc[sbnd] ........................................................... 1
 } 
 } 
 } /* ecplinu[blk] */ 
 } 
 else /* !cplinu[blk] */ 
 { 
 for(ch = 0; ch < nfchans; ch++) 
 { 
 chincpl[ch] = 0 
 firstcplcos[ch] = 1 
 } 
 firstcplleak = 1 
 phsflginu = 0 
 ecplinu = 0;
```
## Semantic clauses (verified against ETSI TS 102 366 V1.4.1, added for #556)

- **§E.1.3.1.3 frmsiz — Frame size — 11 bits:** "The frmsiz field indicates a
  value one less than the overall size of the coded syncframe in 16-bit words.
  That is, this field may assume a value ranging from 0 to 2 047, and these
  values correspond to syncframe sizes ranging from 1 to 2 048."
  (`words_per_syncframe = frmsiz + 1`, §E.1.3.2.28.)
- **§E.1.3.1.2 substreamid (context):** "Dependent substreams shall
  immediately follow the independent substream with which they are
  associated." Independent substreams 1..7 may exist (multi-program);
  dependent substreams are assigned IDs 0..7 sequentially.
