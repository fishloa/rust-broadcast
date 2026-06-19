# Table 18: Object coding method

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | Object coding method  |
| --- | --- |
|  0x0 | coding of pixels (see note 1)  |
|  0x1 | coded as a string of characters  |
|  0x2 | progressive coding of pixels (see note 2)  |
|  0x3 | reserved  |
|  NOTE 1: The value 0x0 indicates interlaced coding of pixels, the only method available for coding of pixels prior to version V1.6.1 of the present document. NOTE 2: This object coding method is introduced in version 1.6.1 of the present document, hence subtitle decoders that are compliant with an earlier version of the present document will be unable to process this mode.  |   |

non_modifying_colour_flag: If set to '1' this indicates that the CLUT entry value '1' is a non modifying colour. When the non modifying colour is assigned to an object pixel, then the pixel of the underlying region background or object shall not be modified. This can be used to create "transparent holes" in objects.

top_field_data_block_length: Specifies the number of bytes contained in the pixel-data_sub-blocks for the top field.

bottom_field_data_block_length: Specifies the number of bytes contained in the data_sub-block for the bottom field.

pixel-data_sub-block(): Contains the run-length encoded data for each field of the object. Its structure is defined in clause 7.2.5.1.

processed_length: The number of bytes from the field(s) within the while-loop that have been processed by the decoder.

stuffing_length: The value is not signalled but it can be calculated from other fields and shall be either zero or one.

NOTE: In earlier versions of the present document the presence or absence of the 8_stuff_bits field was determined by an undefined wordaligned() function which created an ambiguity. This was replaced by the stuffing_length value to remove the ambiguity.

Some legacy subtitle encoders may operate differently to the recommended behaviour defined below in table 19. However in all cases subtitle decoders shall calculate the stuffing_length value using the following equation:

$$
\text{stuffing_length} = \text{segment_length} - 7 - \text{top_field_data_block_length} - \text{bottom_field_data_block_length}
$$

Subtitle encoders should add an 8_stuff_bits field only if the sum of top_field_data_block_length and bottom_field_data_block_length is an even number. Therefore the segment_length field will always be set to an even number. The recommended encoder behaviour is summarized in table 19.
