# Table 32: CLUT parameters

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Syntax | Size | Type  |
| --- | --- | --- |
|  CLUT_parameters() { |  |   |
|  CLUT_entry_max_number | 2 | bslbf  |
|  colour_component_type | 2 | bslbf  |
|  output_bit_depth | 3 | bslbf  |
|  reserved_zero_future_use | 1 | bslbf  |
|  dynamic_range_and_colour_gamut | 8 | bslbf  |
|  } |  |   |

# Semantics:

CLUT_entry_max_number: This two-bit field shall indicate the maximum number of CLUT entries. A value of '0' corresponds to a maximum number of 256 entries. All other values are reserved. Any number of CLUT entries can be provided, up to the maximum number.

colour_component_type: This two-bit field shall indicate the type of colour coding used in the chroma1-value and chroma2-value fields. A value of '0' corresponds to colour coding type YCbCr, whereby chroma1-value is Cb and chroma2-value is Cr. All other values are reserved.

output_bit_depth: This three-bit field shall indicate the bit-depth of the output of each component, as shown in table 33. If the graphics plane of the IRD has a bit-depth different from the output_bit_depth setting, then the IRD shall perform the appropriate conversion for each component value of the CLUT.
