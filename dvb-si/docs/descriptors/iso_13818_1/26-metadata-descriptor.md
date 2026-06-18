## Table 2-89 — Metadata descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.60, Table 2-89; PDF p.103._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Metadata_descriptor() { |  |  |
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
| &nbsp;&nbsp;decoder_config_flags | 3 | bslbf |
| &nbsp;&nbsp;DSM-CC_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;if (DSM-CC_flag == '1') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;service_identification_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;for (i=0; i<service_identification_length; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;service_identification_record_byte | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;if (decoder_config_flags == '001') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;decoder_config_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;for (i=0; i<decoder_config_length; i++) { decoder_config_byte | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;if (decoder_config_flags == '011') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;dec_config_identification_record_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;for (i=0; i<...; i++) { dec_config_identification_record_byte | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;if (decoder_config_flags == '100') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;decoder_config_metadata_service_id | 8 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;if (decoder_config_flags == '101' \|\| decoder_config_flags == '110') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved_data_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;for (i=0; i<reserved_data_length; i++) { reserved | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;for (i=0; i<N; i++) { private_data_byte | 8 | bslbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

decoder_config_flags per Table 2-90.
