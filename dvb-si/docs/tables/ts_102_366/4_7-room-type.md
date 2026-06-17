# Table 4.7: Room type

_Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope._


|  roomtyp | Type of mixing room  |
| --- | --- |
|  00 | Not indicated  |
|  01 | Large room, X curve monitor  |
|  10 | Small room, flat monitor  |
|  11 | Reserved  |

## 4.4.2.16 dialnorm2 - Dialogue normalization, Ch2 - 5 bits

This 5-bit code has the same meaning as dialnorm, except that it applies to the second audio channel when acmod indicates two independent channels (dual mono $1 + 1$ mode).

## 4.4.2.17 compr2e - Compression gain word exists, Ch2 - 1 bit

If this bit is a 1, the following 8 bits represent a compression gain word for Ch2.

## 4.4.2.18 compr2 - Compression gain word, Ch2 - 8 bits

This 8-bit word has the same meaning as compr, except that it applies to the second audio channel when acmod indicates two independent channels (dual mono $1 + 1$ mode).

## 4.4.2.19 langcod2e - Language code exists, Ch2 - 1 bit

If this bit is a 1, the following 8 bits (i.e. the element langcod2) shall be reserved. If this bit is a 0, the element langcod2 does not exist in the bit stream.

## 4.4.2.20 langcod2 - Language code, Ch2 - 8 bits

This is an 8 bit reserved value. See langcod, clause 4.4.2.12.

## 4.4.2.21 audprodi2e - Audio production information exists, Ch2 - 1 bit

If this bit is a 1, the following two data fields exist indicating information about the audio production for Ch2.

## 4.4.2.22 mixlevel2 - Mixing level, Ch2 - 5 bits

This 5-bit code has the same meaning as mixlevel, except that it applies to the second audio channel when acmod indicates two independent channels (dual mono $1 + 1$ mode).



4.4.2.23 roomtyp2 - Room type, Ch2 - 2 bits

This 2-bit code has the same meaning as roomtyp, except that it applies to the second audio channel when acmod indicates two independent channels (dual mono 1 + 1 mode).

4.4.2.24 copyrightb - Copyright bit - 1 bit

If this bit has a value of 1, the information in the bit stream is indicated as protected by copyright. It has a value of 0 if the information is not indicated as protected.

4.4.2.25 origbs - Original bit stream - 1 bit

This bit has a value of 1 if this is an original bit stream. This bit has a value of 0 if this is a copy of another bit stream.

4.4.2.26 timecod1e, timecod2e - Time code (first and second) halves exists - 2 bits

These values indicate, as shown in Table 4.8, whether time codes follow in the bit stream. The time code can have a resolution of 1/64th of a frame (one frame = 1/30th of a second). Since only the high resolution portion of the time code is needed for fine synchronization, the 28 bit time code is broken into two 14 bit halves. The low resolution first half represents the code in 8 second increments up to 24 hours. The high resolution second half represents the code in 1/64th frame increments up to 8 seconds.

4.4.2.27 timecod1 - Time code first half - 14 bits

The first 5 bits of this 14 bit field represent the time in hours, with valid values of 0 to 23. The next 6 bits represent the time in minutes, with valid values of 0 to 59. The final 3 bits represents the time in 8 second increments, with valid values of 0 - 7 (representing 0, 8, 16, ..., 56 seconds).
