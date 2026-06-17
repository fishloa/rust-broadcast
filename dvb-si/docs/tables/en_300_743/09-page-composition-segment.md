# Table 9: Page composition segment

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Syntax | Size | Type  |
| --- | --- | --- |
|  page_composition_segment() { |  |   |
|  sync_byte | 8 | bslbf  |
|  segment_type | 8 | bslbf  |
|  page_id | 16 | bslbf  |
|  segment_length | 16 | uimsbf  |
|  page_time_out | 8 | uimsbf  |
|  page_version_number | 4 | uimsbf  |
|  page_state | 2 | bslbf  |
|  reserved | 2 | bslbf  |
|  while (processed_length < segment_length) { |  |   |
|  region_id | 8 | bslbf  |
|  reserved | 8 | bslbf  |
|  region_horizontal_address | 16 | uimsbf  |
|  region_vertical_address | 16 | uimsbf  |
|  } |  |   |
|  } |  |   |



# Semantics:

sync_byte: This field shall contain the value '0000 1111'.

segment_type: This field shall contain the value 0x10, as listed in table 7.

page_id: The page_id identifies the subtitle service of the data contained in this subtitling_segment. Segments with a page_id value signalled in the subtitling descriptor as the composition page id, carry subtitling data specific for one subtitle service. Accordingly, segments with the page_id signalled in the subtitling descriptor as the ancillary page id, carry data that may be shared by multiple subtitle services.

segment_length: This field shall indicate the number of bytes contained in the segment following the segment_length field.

page_time_out: The period, expressed in seconds, after which a page instance is no longer valid and consequently shall be erased from the screen, should it not have been redefined before that. The time-out period starts when the page instance is first displayed. The page_time_out value applies to each page instance until its value is redefined. The purpose of the time-out period is to avoid a page instance remaining on the screen "for ever" if the IRD happens to have missed the redefinition or deletion of the page instance. The time-out period does not need to be counted very accurately by the IRD: a reaction accuracy of  $-0/+5$  s is accurate enough.

page_version_number: The version of this page composition segment. When any of the contents of this page composition segment change, this version number is incremented (modulo 16).

page_state: This field signals the status of the subtitling page instance described in this page composition segment. The values of the page_state are defined in table 10.
