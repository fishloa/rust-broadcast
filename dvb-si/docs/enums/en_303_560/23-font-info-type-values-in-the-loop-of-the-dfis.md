## Table 23 — font_info_type values in the loop of the DFIS
_§5.3.2.3.2.1, PDF pp. 32-32_

| font_info_type value | font_info_type label | Permitted Occurrences | Description |
|---|---|---|---|
| 0x00 | font_style_weight | 1 or more | This 8-bit field describes the font_style and the font_weight of the downloadable font. See clause 5.3.2.3.2.2. |
| 0x01 | font file URI | 1 or more | Specifies the DVB URI location of a font file which can be downloaded from the internet or an object data carousel (see clause 5.3.2.3.2.3). |
| 0x02 | font_size | 0 or more | The font size (height) in pixels |
| 0x03 | font_family | 1 | It specifies the font_family (see clause 8.3.5 in [2]) of the downloadable font. This string shall be encoded in UTF-8. |
| 0x04 - 0xFF | reserved | n/a | These field values are reserved for future use. |

