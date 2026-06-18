## Table 2-83 — Content labeling descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.56, Table 2-83; PDF p.99._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Content_labeling_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;metadata_application_format | 16 | uimsbf |
| &nbsp;&nbsp;if (metadata_application_format == 0xFFFF) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;metadata_application_format_identifier | 32 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;content_reference_id_record_flag | 1 | bslbf |
| &nbsp;&nbsp;content_time_base_indicator | 4 | uimsbf |
| &nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;if (content_reference_id_record_flag == '1') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;content_reference_id_record_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;for (i=0; i<content_reference_id_record_length; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;content_reference_id_byte | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;if (content_time_base_indicator == 1 \|\| content_time_base_indicator == 2) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;content_time_base_value | 33 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;metadata_time_base_value | 33 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;if (content_time_base_indicator == 2) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;contentId | 7 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;if (content_time_base_indicator >= 3 && content_time_base_indicator <= 7) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;time_base_association_data_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;for (i=0; i<time_base_association_data_length; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;for (i=0; i<N; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;private_data_byte | 8 | bslbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

content_time_base_indicator per Table 2-85; metadata_application_format per Table 2-84.
