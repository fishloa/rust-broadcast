# Table 27: Progressive pixel block

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Syntax | Size | Type  |
| --- | --- | --- |
|  progressivepixel_block() { |  |   |
|  bitmap_width | 16 | uimsbf  |
|  bitmap_height | 16 | uimsbf  |
|  compressed_data_block_length | 16 | uimsbf  |
|  for (i=0; i<compressed_data_block_length; i++) { |  |   |
|  compressed_bitmap_data_byte | 8 | bslbf  |
|  } |  |   |
|  } |  |   |

# Semantics:

bitmap_width: This field shall indicate the width of the subtitle bitmap image in pixels.

bitmap_height: This field shall indicate the height of the subtitle bitmap image in pixels.

compressed_data_block_length: This field shall indicate the number of compressed_bitmap_data_byte following this field.



compressed_bitmap_data_byte: This field is formed of the sequence of bytes of the subtitle bitmap image in compressed form, which is according to the zlib container format [14], in the same way as is specified for the Portable Network Graphics (PNG) format [16]. This format applies the DEFLATE compression algorithm [15]. The compressed bitmap data shall consist of the raw zlib datastream and shall not contain any PNG format overhead such as chunk headers or chunk CRC values.

Annex E provides an informative description of the conversion process for a suitably coded PNG file to be converted into a progressively-coded subtitle bitmap.

# 7.2.6 End of display set segment

The end_of_display_set_segment provides an explicit indication to the decoder that transmission of a display set is complete. The end_of_display_set_segment shall be inserted into the stream as the last segment for each display set. It shall be present for each subtitle service in a subtitle stream, although decoders need not take advantage of this segment and may apply other strategies to determine when they have sufficient information from a display set to commence decoding. The syntax of the end_of_display_set_segment is shown in table 28.
