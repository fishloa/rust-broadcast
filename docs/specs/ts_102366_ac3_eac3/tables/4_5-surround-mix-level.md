# Table 4.5: Surround mix level

_Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope._


|  surmixlev | slev  |
| --- | --- |
|  00 | 0,707 (-3 dB)  |
|  01 | 0,500 (-6 dB)  |
|  10 | 0  |
|  11 | Reserved  |

## 4.4.2.6 dsurmod - Dolby® Surround mode - 2 bits

When operating in the two channel mode, this 2-bit code, as shown in Table 4.6, indicates whether or not the programme has been encoded in Dolby® Surround. This information is not used by the AC-3 decoder, but may be used by other portions of the audio reproduction equipment. If dsurmod is set to the reserved code, the decoder should still reproduce audio. The reserved code may be interpreted as "not indicated".



NOTE: "Dolby®", "Pro Logic®", "Surround EX™" and the double -D symbol are trademarks of Dolby® Laboratories. This information is given for the convenience of users of the present document and does not constitute an endorsement by ETSI of the product named. Equivalent products may be used if they can be shown to lead to the same results."
