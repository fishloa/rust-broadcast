## Table 2-86 — Metadata pointer descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.58, Table 2-86; PDF p.101._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Metadata_pointer_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;metadata_application_format | 16 | uimsbf |
| &nbsp;&nbsp;if (metadata_application_format == 0xFFFF) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;metadata_application_format_identifier | 32 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;metadata_format | 8 | uimsbf |
| &nbsp;&nbsp;if (metadata_format == 0xFF) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;metadata_format_identifier | 32 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;metadata_service_id | 8 | uimsbf |
| &nbsp;&nbsp;metadata_locator_record_flag | 1 | bslbf |
| &nbsp;&nbsp;MPEG_carriage_flags | 2 | uimsbf |
| &nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;if (metadata_locator_record_flag == '1') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;metadata_locator_record_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;for (i=0; i<metadata_locator_record_length; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;metadata_locator_record_byte | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;if (MPEG_carriage_flags <= 2) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;program_number | 16 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;if (MPEG_carriage_flags == 1) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;transport_stream_location | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;transport_stream_id | 16 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;for (i=0; i<N; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;private_data_byte | 8 | bslbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

metadata_format per Table 2-87; MPEG_carriage_flags per Table 2-88.
