# Table 26: 8-bits per pixel code string

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Syntax | Size | Type  |
| --- | --- | --- |
|  8-bit/pixel_code_string() { |  |   |
|  if (next_bits(8) != '0000 0000') { |  |   |
|  8-bitpixel-code | 8 | bslbf  |
|  } else { |  |   |
|  8-bit_zero | 8 | bslbf  |
|  switch_1 | 1 | bslbf  |
|  if switch_1 == '0' { |  |   |
|  if next_bits(7) != '000 0000' |  |   |
|  run_length_1-127 | 7 | uimsbf  |
|  else |  |   |
|  end_of_string_signal | 7 | bslbf  |
|  } else { |  |   |
|  run_length_3-127 | 7 | uimsbf  |
|  8-bitpixel-code | 8 | bslbf  |
|  } |  |   |
|  } |  |   |
|  } |  |   |



# Semantics:

8-bitpixel-code: An 8-bit code, specifying the pseudo-colour of a pixel as an entry number of a CLUT with 256 entries.

8-bit_zero: An 8-bit field filled with '0000 0000'.

switch_1: A 1-bit switch that identifies the meaning of the following fields.

run_length_1-127: Number of pixels that shall be set to pseudo-colour (entry) '0x00'.

end_of_string_signal: A 7-bit field filled with '000 0000'. The presence of this field, i.e. next_bits(7) == '000 0000', signals the end of the 8-bit/pixel_code_string.

run_length_3-127: Number of pixels that shall be set to the pseudo-colour defined next. This field shall not have a value of less than three.

## 7.2.5.3 Progressive pixel block

The progressive pixel block format is used with object coding method 0x2, i.e. "progressive coding of pixels".

This object coding method is introduced in V1.6.1 of the present document, hence it shall not be used in systems where subtitle decoders are in operation that were designed to be compliant with ETSI EN 300 743 (V1.5.1) [8] or an earlier version.

Subtitle streams with progressive object coding type shall use subtitling_type value 0x16 or 0x26 in the subtitling descriptor signalled in the PMT for the service in which they are carried. Subtitle streams that have subtitling_type value not equal to either 0x16 or 0x26 shall not use the progressive coding object type. This ensures that IRDs that are compliant with V1.5.1 or an earlier version of the present document should not be presented with subtitle services that use object coding method 0x2.

The progressive pixel block format shall not be used to carry interlace-scan subtitle segments.

Progressively coded subtitle bitmaps shall be carried in the zlib datastream format, as defined in IETF RFC 1950 [14]. This format applies the DEFLATE compression method as defined by IETF RFC 1951 [15]. The parameters for zlib and DEFLATE usage shall be the same as those applied in the Portable Network Graphics (PNG) format [16] with "Compression method 0" applied to the sequence of filtered scanlines, without any further PNG format overhead, i.e. without the PNG "chunk" structure.

The syntax of the progressive pixel block is shown in table 27.
