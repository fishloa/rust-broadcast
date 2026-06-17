# Table 24: 4-bits per pixel code string

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Syntax | Size | Type  |
| --- | --- | --- |
|  4-bit/pixel_code_string() { |  |   |
|  if (next_bits(4) != '0000') { |  |   |
|  4-bit_pixel-code | 4 | bslbf  |
|  } else { |  |   |
|  4-bit_zero | 4 | bslbf  |
|  switch_1 | 1 | bslbf  |
|  if (switch_1 == '0') { |  |   |
|  if (next_bits(3) != '000') |  |   |
|  run_length_3-9 | 3 | uimsbf  |
|  Else |  |   |
|  end_of_string_signal | 3 | bslbf  |
|  } else { |  |   |
|  switch_2 | 1 | bslbf  |
|  if (switch_2 == '0') { |  |   |
|  run_length_4-7 | 2 | bslbf  |
|  4-bit_pixel-code | 4 | bslbf  |
|  } else { |  |   |
|  switch_3 | 2 | bslbf  |
|  if (switch_3 == '10') { |  |   |
|  run_length_9-24 | 4 | uimsbf  |
|  4-bit_pixel-code | 4 | bslbf  |
|  } |  |   |
|  if (switch_3 == '11') { |  |   |
|  run_length_25-280 | 8 | uimsbf  |
|  4-bit_pixel-code | 4 | bslbf  |
|  } |  |   |
|  } |  |   |
|  } |  |   |
|  } |  |   |



# Semantics:

4-bitpixel-code: A 4-bit code, specifying the pseudo-colour of a pixel as either an entry number of a CLUT with sixteen entries or an entry number of a map-table.

4-bit_zero: A 4-bit field filled with '0000'.

switch_1: A 1-bit switch that identifies the meaning of the following fields.

run_length_3-9: Number of pixels minus 2 that shall be set to pseudo-colour (entry) '0000'.

end_of_string_signal: A 3-bit field filled with '000'. The presence of this field, i.e. next_bits(3) == '000', signals the end of the 4-bit/pixel_code_string.

switch_2: A 1-bit switch. If set to '0', it signals that the following 6-bits contain run-length coded pixel-data, else it indicates the presence of the following fields.

switch_3: A 2-bit switch that may signal one of the properties listed in table 25.
