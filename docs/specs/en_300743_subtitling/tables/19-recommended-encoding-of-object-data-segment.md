# Table 19: Recommended encoding of object_data_segment

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  top_field_data_block_length + bottom_field_data_block_length | 8_stuff_bits | stuffing_length (implied) | segment_length  |
| --- | --- | --- | --- |
|  Is an Odd number | Not present | 0 | 7 + stuffing_length + top_field_data_block_length + bottom_field_data_block_length (Always an even number)  |
|  Is an Even number | Present | 1  |   |

8_stuff_bits: If present, this field shall be coded as '0000 0000'.

number_of_codes: Specifies the number of character codes in the string.

character_code: Specifies a character through its index number in a character table, the definition of which is not included in the present document. The specification and provision of such a character code table is part of the local agreement between the subtitle service provider and IRD manufacturer that is needed to put this mode of subtitles into operation.

progressivepixel_block(): Contains the data for the progressively coded object. Its structure is defined in clause 7.2.5.3.



## 7.2.5.1 Pixel-data sub-block

The pixel-data sub-block structure is used with object coding method 0x0, i.e. "coding of pixels".

For each object the pixel-data sub-block for the top field and the pixel-data sub-block for the bottom field shall be carried in the same object_data_segment. If this segment carries no data for the bottom field, i.e. the bottom_field_data_block_length contains the value '0x0000', then the data for the top field shall be valid for the bottom field also.

NOTE: This effectively forbids an object from having a height of only one TV picture line. Isolated objects of this height would be liable to suffer unpleasant flicker effects at the TV display frame rate when displayed on an interlaced display.

Table 20 defines the syntax of the pixel-data sub-block structure.
