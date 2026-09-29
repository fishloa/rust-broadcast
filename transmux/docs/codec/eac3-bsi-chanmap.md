# E-AC-3 `bsi()` walk + `chanmap`/`chan_loc` — ETSI TS 102 366 V1.4.1 (2017-09)

Transcription of the tables and syntax the `transmux::ac3` E-AC-3 path depends on.
Extracted from the vendored PDF (`private/specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf`)
with `pypdf`, page by page. Quoted text is verbatim; only the table layout is
normalised.

Relevant clauses:

| Clause        | PDF page (0-based) | Content                                    |
|---------------|--------------------|--------------------------------------------|
| E.1.2.2       | 113, 114, 115      | `bsi()` syntax                             |
| E.1.3.1.7/.8  | 126, 127           | `chanmape` / `chanmap` + Table E.1.4       |
| F.6.1         | 197                | `EC3SpecificBox` syntax                    |
| F.6.2.2–.13   | 198, 199           | `data_rate`, `num_ind_sub`, … `chan_loc`   |
| Table F.6.1   | 200                | `chan_loc` bit assignments                 |
| F.6.2.14      | 200                | reserved trailer                           |

## E.1.2.2 `bsi()` — order and nesting (PDF page 113/114/115)

Field order, verbatim from the syntax table (word sizes that matter to us):

```
bsi()
{
 strmtyp ....................................................................................2
 substreamid ................................................................................3
 frmsiz ....................................................................................11
 fscod ......................................................................................2
 numblkscod .................................................................................2

 acmod ......................................................................................3
 lfeon ......................................................................................1
 bsid .......................................................................................5
 dialnorm ...................................................................................5
 compre .....................................................................................1
 if(compre) {compr} .........................................................................8
 if(acmod == 0x0) /* if 1+1 mode (dual mono, so some items need a second value) */
 {
  dialnorm2 ...............................................................................5
  compr2e .................................................................................1
  if(compr2e) {compr2} .................................................................... 8
 }
 if(strmtyp == 0x1) /* if dependent stream */
 {
  chanmape ................................................................................1
  if(chanmape) {chanmap} .................................................................16
 }
 mixmdate ...................................................................................1
 if(mixmdate) /* mixing metadata */
 {
  if(acmod > 0x2) /* if more than 2 channels */ {dmixmod} .................................2
  if((acmod & 0x1) && (acmod > 0x2)) /* if three front channels exist */
  {
   ltrtcmixlev ..........................................................................3
   lorocmixlev ..........................................................................3
  }
  if(acmod & 0x4) /* if a surround channel exists */
  {
   ltrtsurmixlev ........................................................................3
   lorosurmixlev ........................................................................3
  }
  if(lfeon) /* if the LFE channel exists */
  {
   lfemixlevcode  ........................................................................ 1
   if(lfemixlevcode) {lfemixlevcod} ..................................................... 5
  }
  if(strmtyp == 0x0) /* if independent stream */
  {
   pgmscle ..............................................................................1
   if(pgmscle) {pgmscl} .................................................................6
   if(acmod == 0x0)
   {
    pgmscl2e .......................................................................... 1
    if(pgmscl2e) {pgmscl2} ............................................................6
   }
   extpgmscle ...........................................................................1
   if(extpgmscle) {extpgmscl} ...........................................................6
   mixdef ...............................................................................2
   if(mixdef == 0x1) /* mixing option 2 */
   {
    premixcmpsel ...................................................................... 1
    drcsrc ............................................................................ 1
    premixcmpscl ...................................................................... 3
   }
   else if(mixdef == 0x2) /* mixing option 3 */ {mixdata} ..............................12
   else if(mixdef == 0x3) /* mixing option 4 */
   {
    mixdeflen .........................................................................5
    mixdata2e .........................................................................1
    if (mixdata2e)
    {
     premixcmpsel ................................................................... 1
     drcsrc ......................................................................... 1
     premixcmpscl ................................................................... 3
     extpgmlscle .................................................................... 1
     if(extpgmlscle) {extpgmlscl} ................................................... 4
     extpgmcscle .................................................................... 1
     if(extpgmcscle) {extpgmcscl} ................................................... 4
     extpgmrscle .................................................................... 1
     if(extpgmrscle) {extpgmrscl} ................................................... 4
     extpgmlsscle ................................................................... 1
     if(extpgmlsscle) {extpgmlsscl} ................................................. 4
     extpgmrsscle ................................................................... 1
     if(extpgmrsscle) {extpgmrsscl} ................................................. 4
     extpgmlfescle .................................................................. 1
     if(extpgmlfescle) {extpgmlfescl} ............................................... 4
     dmixscle ....................................................................... 1
     if(dmixscle) {dmixscl} ......................................................... 4
     addche ......................................................................... 1
     if (addche)
     {
      extpgmaux1scle  .............................................................. 1
      if(extpgmaux1scle) {extpgmaux1scl} .......................................... 4
      extpgmaux2scle  .............................................................. 1
      if(extpgmaux2scle) {extpgmaux2scl} .......................................... 4
     }
    }
    mixdata3e .........................................................................1
    if(mixdata3e)
    {
     spchdat ........................................................................ 5
     addspchdate .................................................................... 1
     if(addspchdate)
     {
      spchdat1 .................................................................... 5
      spchan1att .................................................................. 2
      addspchdat1e  ................................................................ 1
      if(addspdat1e)
      {
       spchdat2 ................................................................. 5
       spchan2att ............................................................... 3
      }
     }
    }
    mixdata ........................................ (8*(mixdeflen+2)) - no. mixdata bits
    mixdatafill ................................................................... 0 - 7
   }
   if(acmod < 0x2) /* if mono or dual mono source */
   {
    paninfoe .......................................................................... 1
    if(paninfoe)
    {
     panmean ........................................................................ 8
     paninfo ........................................................................ 6
    }
    if(acmod == 0x0)
    {
     paninfo2e ...................................................................... 1
     if(paninfo2e)
     {
      panmean2 .................................................................... 8
      paninfo2 .................................................................... 6
     }
    }
   }
   frmmixcfginfoe  ....................................................................... 1
   if(frmmixcfginfoe)
   {
    if(numblkscod == 0x0) {blkmixcfginfo[0]} .......................................... 5
    else
    {
     for(blk = 0; blk < number_of_blocks_per_sync_frame; blk++)
     {
      blkmixcfginfoe  .............................................................. 1
      if(blkmixcfginfoe) {blkmixcfginfo[blk]} ...................................... 5
     }
    }
   }
  }
 }
 infomdate ..................................................................................1
 if(infomdate) /* informational metadata */
 {
  bsmod ...................................................................................3
  copyrightb ..............................................................................1
  origbs ..................................................................................1
  if(acmod == 0x2) /* if in 2/0 mode */
  {
   dsurmod ..............................................................................2
   dheadphonmod .........................................................................2
  }
  if(acmod >= 0x6) /* if both surround channels exist */ {dsurexmod} ......................2
  ...
 }
```

Points the code depends on, and that a careless reading gets wrong:

1. **`mixdata2e` / `mixdata3e` are siblings of `mixdeflen`, not inside its
   5-bit block.** `mixdeflen` is read, then `mixdata2e` is a *separate* 1-bit
   flag; only when set does its sub-block follow. The same for `mixdata3e`.
2. **The `mixdata` payload is `8 * (mixdeflen + 2)` bits, followed by 0..=7
   `mixdatafill` bits.** The fixed part is byte-aligned (`8 * n`), so the fill
   is implicit: re-align to the next byte boundary rather than reading a count.
3. **The whole programme-mix block is gated on `strmtyp == 0x0`** — *not* on
   "not dependent". `strmtyp == 0x2` (a stream converted from AC-3) is an
   independent stream with no programme-mix block, so a `!= 0x1` test reads
   fields that are not there.
4. `mixmdate` itself, and the `dmixmod`/`*mixlev`/`lfemixlevcode` prefix inside
   it, are present for **both** independent and dependent frames.

## E.1.3.1.7 / .8 — `chanmape` and `chanmap` (PDF pages 126, 127)

> **E.1.3.1.7 chanmape - Custom channel map exists - 1 bit**
> If the chanmape bit is set to '0', the channel map for a dependent substream is defined by the audio coding mode
> (acmod) and LFE on (lfeon) parameters. If this bit is a 1, the following 16 bits define the custom channel map for this
> dependent substream.
> Only dependent substreams can have a custom channel map.

> **E.1.3.1.8 chanmap - Custom channel map - 16 bits**
> The 16-bit chanmap field shall specify the custom channel map for a dependent substream. The channel locations
> supported by the custom channel map are as defined in Table E.1.4. Shaded entries in Table E.1.4 represent channel
> locations present in the independent substream with which the dependent substream is associated. Non-shaded entries in
> Table E.1.4 represent channel locations not present in the independent substream with which the dependent substream is
> associated. These channel locations are defined in SMPTE 428-3 [i.8].

> **Table E.1.4: Custom channel map locations**
>
> | chanmap bit | Location       |
> |-------------|----------------|
> | 0 (MSB)     | Left           |
> | 1           | Centre         |
> | 2           | Right          |
> | 3           | Left Surround  |
> | 4           | Right Surround |
> | 5           | Lc/Rc pair     |
> | 6           | Lrs/Rrs pair   |
> | 7           | Cs             |
> | 8           | Ts             |
> | 9           | Lsd/Rsd pair   |
> | 10          | Lw/Rw pair     |
> | 11          | Vhl/Vhr pair   |
> | 12          | Vhc            |
> | 13          | Lts/Rts pair   |
> | 14          | LFE2           |
> | 15          | LFE            |

> The custom channel map indicates which coded channels are present in the dependent substream and the order of the
> coded channels in the dependent substream. Bit 0, which indicates the presence of the left channel, is stored in the most
> significant bit of the chanmap field. For each channel present in the dependent substream, the corresponding location bit
> in the chanmap is set to 1. The order of the coded channels in the dependent substream is the same as the order of the
> enabled location bits in the chanmap. For example, if bits 0, 3, and 4 of the chanmap field are set to 1, and the
> dependent stream is coded with acmod = 3 and lfeon = 0, the first coded channel in the dependent stream is the Left
> channel, the second coded channel is the Left Surround channel, and the third coded channel is the Right Surround
> channel. When the enabled location bit in the chanmap field refers to a pair of channels, this defines the channel
> location of two adjacent channels in the dependent substream. For example, if bits 3, 4 and 6 of the chanmap field are
> set to 1, and the dependent stream is coded with acmod = 6 and lfeon = 0, the first coded channel in the dependent
> stream is the Left Surround channel, the second coded channel is the Right Surround channel, and the third and fourth
> channels are the Left Rear Surround and Right Rear Surround channels. The number of channel locations indicated by
> the chanmap field shall equal the total number of coded channels present in the dependent substream, as indicated by
> the acmod and lfeon bit stream parameters.

**The bit numbering is MSB-first.** "chanmap bit 0 ... is stored in the most
significant bit of the chanmap field." So on the wire the *first* bit read is
Location `Left`, the last is `LFE`. A `1 << n` test against the 16-bit value as
an integer is therefore **inverted** — it must be `1 << (15 - n)`.

## F.6.1 / Annex F.6.2 — `EC3SpecificBox` (`dec3`) (PDF pages 197–199)

Syntax (PDF page 197):

```
data_rate  .................................................................. 13 uimsbf
num_ind_sub  .................................................................. 3 uimsbf
for(i = 0; i < num_ind_sub + 1; i++)
{
 fscod ..................................................................... 2 uimsbf
 bsid ...................................................................... 5 uimsbf
 reserved .................................................................. 1 bslbf
 asvc ...................................................................... 1 bslbf
 bsmod ..................................................................... 3 uimsbf
 acmod ..................................................................... 3 uimsbf
 lfeon ..................................................................... 1 bslbf
 reserved .................................................................. 3 uimbsf
 num_dep_sub ............................................................... 4 uimsbf
 if num_dep_sub > 0
 {
  chan_loc ............................................................... 9 uimsbf
 }
 else
 {
  reserved 1 ............................................................. bslbf
 }
}
reserved variable ........................................................ bslbf
```

Semantics:

> **F.6.2.2 data\_rate - 13 bits**
> The 13-bit data\_rate field indicates the data rate (in kbps) of the entire bitstream. The value is the sum of the data rates
> of all the substreams. When a bitstream uses variable data-rate encoding, data\_rate indicates the maximum data rate of
> the bitstream.
> The data rate of each substream is calculated using this equation:
> `data_rate_sub = ((frmsiz+1) * fs) / (numblks * 16)`

(`fs` in kHz, `frmsiz` per E.1.3.1.3, `numblks` per E.1.3.1.5.)

> **F.6.2.3 num\_ind\_sub - 3 bits**
> The 3-bit num\_ind\_sub field shall indicate the number of independent substreams that are present in the Enhanced
> AC-3 bit stream. The value of this field shall be equal to the substreamID value of the last independent substream of
> the bit stream.

> **F.6.2.12 num\_dep\_sub - 4 bits**
> The 4-bit num\_dep\_sub field shall be set to the value of the substreamid field found in the frame with a strmtyp value of 1
> (that is, in the dependent substream) immediately preceding a frame with a strmtyp value of 0 (that is, in the independent
> substream).

> **F.6.2.13 chan\_loc - 9 bits**
> The chan\_loc field indicates channel locations (beyond the standard 5.1 channels) that are carried by dependent
> substreams associated with an independent substream. The contents of the chan\_loc field are determined by parsing the
> chanmap bit field in every dependent substream associated with a particular independent substream, and setting the
> corresponding channel locations in the chan\_loc field to a value of 1.

> Because this field is used by the system only to indicate the unique channel locations present in the bitstream, it is not
> necessary to reflect replacement channels in this field. Therefore, duplicate channel locations in the chanmap field
> indicate replacement channels and can be ignored.

> **Table F.6.1: chan\_loc field bit assignments**
>
> | Bit       | Location    |
> |-----------|-------------|
> | 0         | Lc/Rc pair  |
> | 1         | Lrs/Rrs pair|
> | 2         | Cs          |
> | 3         | Ts          |
> | 4         | Lsd/Rsd pair|
> | 5         | Lw/Rw pair  |
> | 6         | Lvh/Rvh pair|
> | 7         | Cvh         |
> | 8 (MSB)   | LFE2        |

Note the two tables use **opposite** bit orders for their own fields. Table
F.6.1 labels bit 8 as the MSB while Table E.1.4 labels bit 0 as the MSB; each is
numbered from its own field's MSB. So the `chan_loc` value is built with
`1 << n` for Table F.6.1's `n` (bit 0 = Lc/Rc is the *least* significant of the
9), while `chanmap` is read with `1 << (15 - n)` for Table E.1.4's `n`.

> **F.6.2.14 reserved - variable**
> Additional reserved bytes may follow at the end of the EC3SpecificBox. The number of reserved bytes present is
> determined by subtracting the number of bytes used by the EC3SpecificBox fields specified above from the total
> length of the EC3SpecificBox as specified by the value of the BoxHeader.Size field of the EC3SpecificBox.

## `chanmap` → `chan_loc` mapping

Table E.1.4 and Table F.6.1 name the same locations in a different order and
numbering. The mapping the code uses (`(chanmap bit, chan_loc bit)`):

| Table E.1.4 `chanmap` bit | Location     | Table F.6.1 `chan_loc` bit |
|---------------------------|--------------|----------------------------|
| 5                         | Lc/Rc pair   | 0                          |
| 6                         | Lrs/Rrs pair | 1                          |
| 7                         | Cs           | 2                          |
| 8                         | Ts           | 3                          |
| 9                         | Lsd/Rsd pair | 4                          |
| 10                        | Lw/Rw pair   | 5                          |
| 11                        | Vhl/Vhr pair | 6                          |
| 12                        | Vhc          | 7                          |
| 14                        | LFE2         | 8                          |

Table E.1.4's other locations (bits 0–4: Left, Centre, Right, Left Surround,
Right Surround — and bit 15, LFE) are the standard 5.1 set the *independent*
substream already carries, which is exactly what §F.6.2.13 excludes ("channel
locations beyond the standard 5.1 channels"). Location bit 13 (`Lts/Rts pair`)
has no `chan_loc` bit and is likewise dropped. The pair names differ between the
tables (`Vhl/Vhr` vs `Lvh/Rvh`, `Vhc` vs `Cvh`) but denote the same locations
(SMPTE 428-3, cited by both clauses).
