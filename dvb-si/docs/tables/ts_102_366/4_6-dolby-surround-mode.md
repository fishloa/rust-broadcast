# Table 4.6: Dolby® Surround mode

_Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope._


|  dsurmod | Indication  |
| --- | --- |
|  00 | Not indicated  |
|  01 | NOT Dolby® Surround encoded  |
|  10 | Dolby® Surround encoded  |
|  11 | Reserved  |

## 4.4.2.7 Ifeon - Low frequency effects channel on - 1 bit

This bit has a value of 1 if the Ife (sub woofer) channel is on, and a value of 0 if the Ife channel is off.

## 4.4.2.8 dialnorm - Dialogue normalization - 5 bits

This 5-bit code indicates how far the average dialogue level is below digital 100 percent. Valid values are 1 to 31. The value of 0 is reserved. The values of 1 to 31 are interpreted as -1 dB to -31 dB with respect to digital 100%. If the reserved value of 0 is received, the decoder shall use -31 dB. The value of dialnorm shall affect the sound reproduction level. If the value is not used by the AC-3 decoder itself, the value shall be used by other parts of the audio reproduction equipment. Dialogue normalization is further explained in clause 6.6.

## 4.4.2.9 compre - Compression gain word exists - 1 bit

If this bit is a 1, the following 8 bits represent a compression control word.

## 4.4.2.10 compr - Compression gain word - 8 bits

This encoder generated gain word may be present in the bit stream. If so, it may be used to scale the reproduced audio level in order to reproduce a very narrow dynamic range, with an assured upper limit of instantaneous peak reproduced signal level in the monophonic downmix. The meaning and use of compr is described further in clause 6.7.3.

## 4.4.2.11 langcode - Language code exists - 1 bit

If this bit is a 1, the following 8 bits (i.e. the element langcod) shall be reserved. If this bit is a 0, the element langcod does not exist in the bit stream.

## 4.4.2.12 langcod - Language code - 8 bits

This is an 8 bit reserved value. (This element was originally intended to carry an 8-bit value that would, via a table lookup, indicate the language of the audio programme. Because modern delivery systems provide the ISO 639-2 [i.2] language code in the multiplexing layer, indication of language within the AC-3 bit stream was unnecessary, and so was removed from the AC-3 syntax.)

## 4.4.2.13 audprodie - Audio production information exists - 1 bit

If this bit is a 1, the mixlevel and roomtyp fields exist, indicating information about the audio production environment (mixing room).



## 4.4.2.14 mixlevel - Mixing level - 5 bits

This 5-bit code indicates the absolute acoustic sound pressure level of an individual channel during the final audio mixing session. The 5-bit code represents a value in the range 0 to 31. The peak mixing level is 80 plus the value of mixlevel dB SPL, or 80 dB to 111 dB SPL. The peak mixing level is the acoustic level of a sine wave in a single channel whose peaks reach 100 percent in the PCM representation. The absolute SPL value is typically measured by means of pink noise with an RMS value of -20 dB or -30 dB with respect to the peak RMS sine wave level. The value of mixlevel is not typically used within the AC-3 decoder, but may be used by other parts of the audio reproduction equipment.

## 4.4.2.15 roomtyp - Room type - 2 bits

This 2-bit code, shown in Table 4.7, indicates the type and calibration of the mixing room used for the final audio mixing session. The value of roomtyp is not typically used by the AC-3 decoder, but may be used by other parts of the audio reproduction equipment. If roomtyp is set to the reserved code, the decoder should still reproduce audio. The reserved code may be interpreted as "not indicated".
