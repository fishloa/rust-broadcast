## Table 165 — Partial transport stream descriptor
_§7.2.1, PDF pp. 155-234_

| Syntax | Number of bits | Identifier |
|---|---|---|
| partial_transport_stream_descriptor() { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| reserved_future_use | 2 | bslbf |
| peak_rate | 22 | uimsbf |
| reserved_future_use | 2 | bslbf |
| minimum_overall_smoothing_rate | 22 | uimsbf |
| reserved_future_use | 2 | bslbf |
| maximum_overall_smoothing_buffer | 14 | uimsbf |
| } |
| Control code | UTF-8 encoded control code | Description |
| 0x80 to 0x85 | 0xC2 0x80 to 0xC2 0x85 | reserved for future use |
| 0x86 | 0xC2 0x86 | character emphasis on |
| 0x87 | 0xC2 0x87 | character emphasis off |
| 0x88 to 0x89 | 0xC2 0x88 to 0xC2 0x89 | reserved for future use |
| 0x8A | 0xC2 0x8A | Carriage Return/Line Feed (CR/LF) |
| 0x8B to 0x9F | 0xC2 0x8B to 0xC2 0x9F | user defined |
| Control code | UTF-8 encoded control code | Description |
| 0xE080 to 0xE085 | 0xEE 0x82 0x80 to 0xEE 0x82 0x85 | reserved for future use |
| 0xE086 | 0xEE 0x82 0x86 | character emphasis on |
| 0xE087 | 0xEE 0x82 0x87 | character emphasis off |
| 0xE088 to 0xE089 | 0xEE 0x82 0x88 to 0xEE 0x82 0x89 | reserved for future use |
| 0xE08A | 0xEE 0x82 0x8A | CR/LF |
| 0xE08B to 0xE09F | 0xEE 0x82 0x8B to 0xEE 0x82 0x9F | user defined |
| First byte | Character code table | Table description | Reproduced in |
| value |
| 0x01 | ISO/IEC 8859-5 [42] | Latin/Cyrillic alphabet | Figure A.2 |
| 0x02 | ISO/IEC 8859-6 [43] | Latin/Arabic alphabet | Figure A.3 |
| 0x03 | ISO/IEC 8859-7 [44] | Latin/Greek alphabet | Figure A.4 |
| 0x04 | ISO/IEC 8859-8 [45] | Latin/Hebrew alphabet | Figure A.5 |
| 0x05 | ISO/IEC 8859-9 [46] | Latin alphabet No. 5 | Figure A.6 |
| 0x06 | ISO/IEC 8859-10 [47] | Latin alphabet No. 6 | Figure A.7 |
| 0x07 | ISO/IEC 8859-11 [48] | Latin/Thai (draft only) | Figure A.8 |
| 0x08 | reserved for future use (see note) |
| 0x09 | ISO/IEC 8859-13 [49] | Latin alphabet No. 7 | Figure A.9 |
| 0x0A | ISO/IEC 8859-14 [50] | Latin alphabet No. 8 (Celtic) | Figure A.10 |
| 0x0B | ISO/IEC 8859-15 [51] | Latin alphabet No. 9 | Figure A.11 |
| 0x0C to | reserved for future use |
| 0x0F |
| 0x10 | dynamically selected part of ISO/IEC 8859 | See table A.4 |
|  | [38] to [51] |
| 0x11 | ISO/IEC 10646 [52] | BMP |
| 0x12 | KS X 1001-2014 [54] | Korean character set |
| 0x13 | GB-2312-1980 [53] | Simplified Chinese character set |
| 0x14 | Big5 subset of ISO/IEC 10646 [52] | Traditional Chinese character set |
| 0x15 | UTF-8 encoding of ISO/IEC 10646 [52] | BMP |
| 0x16 to | reserved for future use |
| 0x1E |
| 0x1F | Described by encoding_type_id | Described by 8-bit encoding_type_id |
|  |  | conveyed in second byte of the string |
| South-Asian languages should use the Basic Multilingual Plane (BMP) of ISO/IEC 10646 [52], where |
| appropriate glyphs are provided. |
| First byte | Second byte | Third byte | Character code table | Table description | Reproduced |
| value | value | value |  |  | in |
| 0x10 | 0x00 | 0x00 | reserved for future use |
| 0x10 | 0x00 | 0x01 | ISO/IEC 8859-1 [38] | West European |
| 0x10 | 0x00 | 0x02 | ISO/IEC 8859-2 [39] | East European |
| 0x10 | 0x00 | 0x03 | ISO/IEC 8859-3 [40] | South European |
| 0x10 | 0x00 | 0x04 | ISO/IEC 8859-4 [41] | North and North-East |
|  |  |  |  | European |
| 0x10 | 0x00 | 0x05 | ISO/IEC 8859-5 [42] | Latin/Cyrillic | Figure A.2 |
| 0x10 | 0x00 | 0x06 | ISO/IEC 8859-6 [43] | Latin/Arabic | Figure A.3 |
| 0x10 | 0x00 | 0x07 | ISO/IEC 8859-7 [44] | Latin/Greek | Figure A.4 |
| 0x10 | 0x00 | 0x08 | ISO/IEC 8859-8 [45] | Latin/Hebrew | Figure A.5 |
| 0x10 | 0x00 | 0x09 | ISO/IEC 8859-9 [46] | West European & Turkish | Figure A.6 |
| 0x10 | 0x00 | 0x0A | ISO/IEC 8859-10 [47] | North European | Figure A.7 |
| 0x10 | 0x00 | 0x0B | ISO/IEC 8859-11 [48] | Thai | Figure A.8 |
| 0x10 | 0x00 | 0x0C | reserved for future use |
| 0x10 | 0x00 | 0x0D | ISO/IEC 8859-13 [49] | Baltic | Figure A.9 |
| 0x10 | 0x00 | 0x0E | ISO/IEC 8859-14 [50] | Celtic | Figure A.10 |
| 0x10 | 0x00 | 0x0F | ISO/IEC 8859-15 [51] | West European | Figure A.11 |
| 0x10 | 0x00 | 0x10 to 0xFF | reserved for future use |
| 0x10 | 0x01 to 0xFF | 0x00 to 0xFF | reserved for future use |
| Colour |  | Description |
| light orange |  | letters of the Latin alphabet which are compatible with 7-bit US-ASCII encoding |
| light red |  | numbers of the Latin alphabet which are compatible with 7-bit US-ASCII encoding |
| light blue |  | marks, punctuation, symbols, and separators |
| light pink |  | non-spacing symbols (diacritical marks) |
| light green |  | region-specific alphabet symbols |
| Mnemonic | Description |
| SPC | space |
| NBSP | no-break space |
| SHY | soft hyphen |
| LRM | left-to-right mark |
| RLM | right-to-left mark |
| component_type bits | Description |
| b(cid:0) | Enhanced AC-3 flag (see table D.2) |
| b(cid:2) | Full service flag (see table D.3) |
| b(cid:3) to b(cid:4) | Service type flags (see table D.4) |
| b(cid:5) to b(cid:6) (see note) | Number of channels flags (see table D.5) |
| NOTE: This bit is transmitted | last (see clause 5.1.6). |
| Enhanced AC-3 flag b(cid:7) | Description |
| 0b0 | Stream is AC-3 |
| 0b1 | Stream is Enhanced AC-3 |
| Full service | Description |
| flag b(cid:8) |
| 0b0 | Decoded audio stream is an associated service intended to be combined with another decoded audio |
|  | stream before presentation to the listener |
| 0b1 | Decoded audio stream is a full service (suitable for decoding and presentation to the listener) |
| Service | type | flags | Description |  | Restrictions (see note 1) |
| b(cid:9) | b(cid:10) | b(cid:11) |  | Full service flag | Number of channels flags |
|  |  |  |  | (b(cid:8)) | (b(cid:12) to b(cid:13) (see note 2)) |
| 0b0 | 0b0 | 0b0 | Complete Main (CM) | set to 0b1 |
| 0b0 | 0b0 | 0b1 | Music and Effects (ME) | set to 0b0 |
| 0b0 | 0b1 | 0b0 | Visually Impaired (VI) |
| 0b0 | 0b1 | 0b1 | Hearing Impaired (HI) |
| 0b1 | 0b0 | 0b0 | Dialogue (D) | set to 0b0 |
| 0b1 | 0b0 | 0b1 | Commentary (C) |  | set to 0b000 |
| 0b1 | 0b1 | 0b0 | Emergency (E) | set to 0b1 | set to 0b000 |
| 0b1 | 0b1 | 0b1 | Voice Over (VO) | set to 0b0 | set to 0b000 |
| 0b1 | 0b1 | 0b1 | Karaoke | set to 0b1 | set to 0b010, 0b011, or 0b100 |
| Number |  | of channels | Description | Restrictions |
|  | flags |  |  | (see note 1) |
| b(cid:12) | b(cid:14) | b(cid:13) |  | Enhanced AC-3 flag |
|  |  | (see note 2) |  | (b(cid:7)) |
| 0b0 | 0b0 | 0b0 | Mono |
| 0b0 | 0b0 | 0b1 | 1+1 Mode |
| 0b0 | 0b1 | 0b0 | 2 channel (stereo) |
| 0b0 | 0b1 | 0b1 | 2 channel Surround encoded (stereo) |
| 0b1 | 0b0 | 0b0 | Multichannel audio (> 2 channels) |
| 0b1 | 0b0 | 0b1 | Multichannel audio (> 5.1 channels) | set to 0b1 |
| 0b1 | 0b1 | 0b0 | Elementary stream contains multiple programmes carried in | set to 0b1 |
|  |  |  | independent substreams |
| 0b1 | 0b1 | 0b1 | reserved for future use |
| Syntax | Number of bits | Identifier |
| AC-3_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| component_type_flag | 1 | bslbf |
| bsid_flag | 1 | bslbf |
| mainid_flag | 1 | bslbf |
| asvc_flag | 1 | bslbf |
| reserved_flags | 4 | bslbf |
| if (component_type_flag == 0b1) { |
| component_type | 8 | uimsbf |
| } |
| if (bsid_flag == 0b1) { |
| bsid | 8 | uimsbf |
| } |
| if (mainid_flag == 0b1) { |
| mainid | 8 | uimsbf |
| } |
| if (asvc_flag == 0b1) { |
| asvc | 8 | uimsbf |
| } |
| for (i=0;i<N;i++) { |
| additional_info_byte | 8 | uimsbf |
| } |
| } |
| Syntax | Number of bits | Identifier |
| enhanced_AC-3_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| component_type_flag | 1 | bslbf |
| bsid_flag | 1 | bslbf |
| mainid_flag | 1 | bslbf |
| asvc_flag | 1 | bslbf |
| mixinfoexists | 1 | bslbf |
| substream1_flag | 1 | bslbf |
| substream2_flag | 1 | bslbf |
| substream3_flag | 1 | bslbf |
| if (component_type_flag == 0b1) { |
| component_type | 8 | uimsbf |
| } |
| if (bsid_flag == 0b1) { |
| bsid | 8 | uimsbf |
| } |
| if (mainid_flag == 0b1) { |
| mainid | 8 | uimsbf |
| } |
| if (asvc_flag == 0b1) { |
| asvc | 8 | bslbf |
| } |
| if (substream1_flag == 0b1) { |
| substream1 | 8 | uimsbf |
| } |
| if (substream2_flag == 0b1) { |
| substream2 | 8 | uimsbf |
| } |
| if (substream3_flag == 0b1) { |
| substream3 | 8 | uimsbf |
| } |
| for (i=0;i<N;i++) { |
| additional_info_byte | 8 | uimsbf |
| } |
| } |
| substream1, substream2, and substream3 bits | Description |
| b(cid:0) | Mixing metadata flag (see table D.9) |
| b(cid:2) | Full service flag (see table D.3) |
| b(cid:3) to b(cid:4) | Service type flags (see table D.4) |
| b(cid:5) to b(cid:6) (see note) | Number of channels flags (see table D.10) |
| NOTE: This bit is transmitted last (see clause 5.1.6). |
| mixinfoexists | Description |
| 0b0 | No mixing metadata present in substream |
| 0b1 | Mixing metadata present in substream |
| Number | of | channels flags | Description |
| b(cid:12) | b(cid:14) | b(cid:13) (see note) |
| 0b0 | 0b0 | 0b0 | Mono |
| 0b0 | 0b0 | 0b1 | 1+1 Mode |
| 0b0 | 0b1 | 0b0 | 2 channel (stereo) |
| 0b0 | 0b1 | 0b1 | 2 channel Surround encoded (stereo) |
| 0b1 | 0b0 | 0b0 | Multichannel audio (> 2 channels) |
| 0b1 | 0b0 | 0b1 | Multichannel audio (> 5.1 channels) |
| 0b1 | 0b1 | 0b0 | reserved for future use |
| 0b1 | 0b1 | 0b1 | reserved for future use |
| NOTE: | This | bit is transmitted | last (see clause 5.1.6). |
| Syntax | Number of bits | Identifier |
| AC-4_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| ac4_config_flag | 1 | bslbf |
| ac4_toc_flag | 1 | bslbf |
| reserved_zero_future_use | 6 | bslbf |
| if (ac4_config_flag == 0b1) { |
| ac4_dialog_enhancement_enabled | 1 | bslbf |
| ac4_channel_mode | 2 | uimsbf |
| reserved_zero_future_use | 5 | bslbf |
| } |
| if (ac4_toc_flag == 0b1) { |
| ac4_toc_len | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| ac4_dsi_byte | 8 | uimsbf |
| } |
| } |
| for (i=0;i<N;i++) { |
| additional_info_byte | 8 | uimsbf |
| } |
| } |

#### Table D.12 — AC-4 channel mode coding

| ac4_channel_mode | Description |
|---|---|
| 0 | Mono content |
| 1 | Stereo content |
| 2 | Multichannel content |
| 3 | reserved for future use |
| Syntax | Number of bits | Identifier |
| DTS_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| sample_rate_code | 4 | bslbf |
| bit_rate_code | 6 | bslbf |
| nblks | 7 | bslbf |
| fsize | 14 | uimsbf |
| surround_mode | 6 | bslbf |
| lfe_flag | 1 | uimsbf |
| extended_surround_flag | 2 | uimsbf |
| for (i=0;i<N;i++) { |
| additional_info_byte | 8 | uimsbf |
| } |
| } |
| sample_rate_code | Description |
| 0b0000 | invalid |
| 0b0001 | 8 kHz |
| 0b0010 | 16 kHz |
| 0b0011 | 32 kHz |
| 0b0100 | 64 kHz |
| 0b0101 | 128 kHz |
| 0b0110 | 11,025 kHz |
| 0b0111 | 22,05 kHz |
| 0b1000 | 44,1 kHz |
| 0b1001 | 88,02 kHz |
| 0b1010 | 176,4 kHz |
| 0b1011 | 12 kHz |
| 0b1100 | 24 kHz |
| 0b1101 | 48 kHz |
| 0b1110 | 96 kHz |
| 0b1111 | 192 kHz |
| bit_rate_code (see note) | Description |
| 0bx0 0101 | 128 kbit/s |
| 0bx0 0110 | 192 kbit/s |
| 0bx0 0111 | 224 kbit/s |
| 0bx0 1000 | 256 kbit/s |
| 0bx0 1001 | 320 kbit/s |
| 0bx0 1010 | 384 kbit/s |
| 0bx0 1011 | 448 kbit/s |
| 0bx0 1100 | 512 kbit/s |
| 0bx0 1101 | 576 kbit/s |
| 0bx0 1110 | 640 kbit/s |
| 0bx0 1111 | 768 kbit/s |
| 0bx1 0000 | 960 kbit/s |
| 0bx1 0001 | 1 024 kbit/s |
| 0bx1 0010 | 1 152 kbit/s |
| 0bx1 0011 | 1 280 kbit/s |
| 0bx1 0100 | 1 344 kbit/s |
| 0bx1 0101 | 1 408 kbit/s |
| 0bx1 0110 | 1 411,2 kbit/s |
| 0bx1 0111 | 1 472 kbit/s |
| 0bx1 1000 | 1 536 kbit/s |
| 0bx1 1001 | 1 920 kbit/s |
| 0bx1 1010 | 2 048 kbit/s |
| 0bx1 1011 | 3 072 kbit/s |
| 0bx1 1100 | 3 840 kbit/s |
| 0bx1 1101 | open |
| 0bx1 1110 | variable |
| 0bx1 1111 | lossless |
| NOTE: "x" indicates that the bit is reserved | and should be ignored. |
| surround_mode | Number of Channels / Channel Layout (see note) |
| 0b00 0000 | 1 / mono |
| 0b00 0010 | 2 / L + R (stereo) |
| 0b00 0011 | 2 / (L+R) + (L-R) (sum-difference) |
| 0b00 0100 | 2 / LT +RT (left and right total) |
| 0b00 0101 | 3 / C + L + R |
| 0b00 0110 | 3 / L + R+ S |
| 0b00 0111 | 4 / C + L + R+ S |
| 0b00 1000 | 4 / L + R+ SL+SR |
| 0b00 1001 | 5 / C + L + R+ SL+SR |
| 0b00 1010 | user defined |
| 0b00 1011 | user defined |
| 0b00 1100 | user defined |
| 0b00 1101 | user defined |
| 0b00 1110 | user defined |
| 0b00 1111 | user defined |
| 0b01 0000 to 0b11 1111 | user defined |
| NOTE: L = left, R = right, | C = centre, S = surround, T = total. |
| extended_surround_flag | Description |
| 0b00 | no extended surround |
| 0b01 | matrixed extended surround |
| 0b10 | discrete extended surround |
| 0b11 | undefined |
| Syntax | Number of bits | Identifier |
| DTS-HD_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| substream_core_flag | 1 | bslbf |
| substream_0_flag | 1 | bslbf |
| substream_1_flag | 1 | bslbf |
| substream_2_flag | 1 | bslbf |
| substream_3_flag | 1 | bslbf |
| reserved_future_use | 3 | bslbf |
| if (substream_core_flag == 0b1) { |
| substream_info() |
| } |
| if (substream_0_flag == 0b1) { |
| substream_info() |
| } |
| if (substream_1_flag == 0b1) { |
| substream_info() |
| } |
| if (substream_2_flag == 0b1) { |
| substream_info() |
| } |
| if (substream_3_flag == 0b1) { |
| substream_info() |
| } |
| for (i=0;i<N;i++) { |
| additional_info_byte | 8 | uimsbf |
| } |
| } |
| Syntax | Number of bits | Identifier |
| substream_info() { |
| substream_length | 8 | uimsbf |
| num_assets | 3 | uimsbf |
| channel_count | 5 | uimsbf |
| lfe_flag | 1 | bslbf |
| sampling_frequency | 4 | uimsbf |
| sample_resolution | 1 | bslbf |
| reserved_future_use | 2 | bslbf |
| for (i=0;i<N;i++) { |
| asset_info() |
| } |
| } |
| sampling_frequency | Description |
| 0 | 8 kHz |
| 1 | 16 kHz |
| 2 | 32 kHz |
| 3 | 64 kHz |
| 4 (see note) | 128 kHz |
| 5 | 22,05 kHz |
| 6 | 44,1 kHz |
| 7 | 88,2 kHz |
| 8 (see note) | 176,4 kHz |
| 9 (see note) | 352,8 kHz |
| 10 | 12 kHz |
| 11 | 24 kHz |
| 12 | 48 kHz |
| 13 | 96 kHz |
| 14 (see note) | 192 kHz |
| 15 (see note) | 348 kHz |
| NOTE: This sampling frequency is | not to be used with a |
| core substream. |
| Syntax | Number of bits | Identifier |
| asset_info() { |
| asset_construction | 5 | uimsbf |
| vbr_flag | 1 | bslbf |
| post_encode_br_scaling_flag | 1 | bslbf |
| component_type_flag | 1 | bslbf |
| language_code_flag | 1 | bslbf |
| if (post_encode_br_scaling_flag == 0b1) { |
| bit_rate_scaled | 13 | bslbf |
| } else { |
| bit_rate | 13 | uimsbf |
| } |
| reserved_future_use | 2 | bslbf |
| if (component_type_flag == 0b1) { |
| component_type | 8 | bslbf |
| } |
| if (language_code_flag == 0b1) { |
| ISO_639_language_code | 24 | bslbf |
| } |
| } |
| asset_construction | Core |  | substream |  |  |  | Extension | substream |
|  | Core |  | XCH | X96 | XXCH | Core | XXCH | X96 | XBR | XLL | LBR |
|  | ✓ |
| 1 |  |  | - | - | - | - | - | - | - | - | - |
|  | ✓ |  | ✓ |
| 2 |  |  |  | - | - | - | - | - | - | - | - |
|  | ✓ |  |  |  | ✓ |
| 3 |  |  | - | - |  | - | - | - | - | - | - |
|  | ✓ |  |  | ✓ |
| 4 |  |  | - |  | - | - | - | - | - | - | - |
|  | ✓ |  |  |  |  |  | ✓ |
| 5 |  |  | - | - | - | - |  | - | - | - | - |
|  | ✓ |  |  |  |  |  |  |  | ✓ |
| 6 |  |  | - | - | - | - | - | - |  | - | - |
|  | ✓ |  | ✓ |  |  |  |  |  | ✓ |
| 7 |  |  |  | - | - | - | - | - |  | - | - |
|  | ✓ |  |  |  | ✓ |  |  |  | ✓ |
| 8 |  |  | - | - |  | - | - | - |  | - | - |
|  | ✓ |  |  |  |  |  | ✓ |  | ✓ |
| 9 |  |  | - | - | - | - |  | - |  | - | - |
|  | ✓ |  |  |  |  |  |  | ✓ |
| 10 |  |  | - | - | - | - | - |  | - | - | - |
|  | ✓ |  | ✓ |  |  |  |  | ✓ |
| 11 |  |  |  | - | - | - | - |  | - | - | - |
|  | ✓ |  |  |  | ✓ |  |  | ✓ |
| 12 |  |  | - | - |  | - | - |  | - | - | - |
|  | ✓ |  |  |  |  |  | ✓ | ✓ |
| 13 |  |  | - | - | - | - |  |  | - | - | - |
|  | ✓ |  |  |  |  |  |  |  |  | ✓ |
| 14 |  |  | - | - | - | - | - | - | - |  | - |
|  | ✓ |  | ✓ |  |  |  |  |  |  | ✓ |
| 15 |  |  |  | - | - | - | - | - | - |  | - |
|  | ✓ |  |  | ✓ |  |  |  |  |  | ✓ |
| 16 |  |  | - |  | - | - | - | - | - |  | - |
|  |  |  |  |  |  |  |  |  |  | ✓ |
| 17 | - |  | - | - | - | - | - | - | - |  | - |
|  |  |  |  |  |  |  |  |  |  |  | ✓ |
| 18 | - |  | - | - | - | - | - | - | - | - |
|  |  |  |  |  |  | ✓ |
| 19 | - |  | - | - | - |  | - | - | - | - | - |
|  |  |  |  |  |  | ✓ | ✓ |
| 20 | - |  | - | - | - |  |  | - | - | - | - |
|  |  |  |  |  |  | ✓ |  |  |  | ✓ |
| 21 | - |  | - | - | - |  | - | - | - |  | - |
| component_type bits | Description |
| b(cid:0) | reserved for future use |
| b(cid:2) | Full service flag (see table G.12) |
| b(cid:3) to b(cid:4) | Service type flags (see table G.13) |
| b(cid:5) to b(cid:6) (see note) | Number of channels flags (see table G.14) |
| NOTE: This bit is transmitted | last (see clause 5.1.6). |
| Full service | Description |
| flag b(cid:8) |
| 0b0 | Decoded audio stream is an associated service intended to be combined |
|  | with another decoded audio stream before presentation to the listener |
| 0b1 | Decoded audio stream is a full service (suitable for decoding and |
|  | presentation to the listener) |
| Service | type | flags | Description |  | Restrictions (see note 1) |
| b(cid:9) | b(cid:10) | b(cid:11) |  | Full service flag | Number of channels flags (b(cid:12) to b(cid:13) |
|  |  |  |  | (b(cid:8)) | (see note 2)) |
| 0b0 | 0b0 | 0b0 | Complete Main (CM) | set to 0b1 |
| 0b0 | 0b0 | 0b1 | Music and Effects (ME) | set to 0b0 |
| 0b0 | 0b1 | 0b0 | Visually Impaired (VI) |
| 0b0 | 0b1 | 0b1 | Hearing Impaired (HI) |
| 0b1 | 0b0 | 0b0 | Dialogue (D) | set to 0b0 |
| 0b1 | 0b0 | 0b1 | Commentary (C) |  | set to 0b000 |
| 0b1 | 0b1 | 0b0 | Emergency (E) | set to 0b1 | set to 0b000 |
| 0b1 | 0b1 | 0b1 | Voice Over (VO) | set to 0b0 | set to 0b000 |
| 0b1 | 0b1 | 0b1 | reserved for future use | set to 0b1 |
| Number | of | channels flags | Description |
| b(cid:12) | b(cid:14) | b(cid:13) (see note) |
| 0b0 | 0b0 | 0b0 | Mono |
| 0b0 | 0b0 | 0b1 | reserved for future use |
| 0b0 | 0b1 | 0b0 | 2 channel (stereo, LoRo) |
| 0b0 | 0b1 | 0b1 | 2 channel matrix encoded (stereo, LtRt) |
| 0b1 | 0b0 | 0b0 | Multichannel audio (> 2 channels) |
| 0b1 | 0b0 | 0b1 | reserved for future use |
| 0b1 | 0b1 | 0b0 | reserved for future use |
| 0b1 | 0b1 | 0b1 | reserved for future use |
| NOTE: | This | bit is transmitted | last (see clause 5.1.6). |
| Syntax | Number of bits | Identifier |
| DTS-UHD_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| DecoderProfileCode | 6 | uimsbf |
| FrameDurationCode | 2 | uimsbf |
| MaxPayloadCode | 3 | uimsbf |
| DTS_reserved | 2 | bslbf |
| StreamIndex | 3 | uimsbf |
| for (i=0;i<N;i++) { |
| codec_selector_byte | 8 | uimsbf |
| } |
| } |
| FrameDurationCode | Description |
| 0 | 512 samples |
| 1 | 1 024 samples |
| 2 | 2 048 samples |
| 3 | 4 096 samples |
| MaxPayloadCode | Description |
| 0 | 2 048 byte |
| 1 | 4 096 byte |
| 2 | 8 192 byte |
| 3 | 16 384 byte |
| 4 | 32 768 byte |
| 5 | 65 536 byte |
| 6 | 131 072 byte |
| 7 | reserved for future use |
| Syntax | Number of bits | Identifier |
| AAC_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| profile_and_level | 8 | uimsbf |
| if (descriptor_length > 1) { |
| AAC_type_flag | 1 | bslbf |
| SAOC_DE_flag | 1 | bslbf |
| reserved_zero_future_use | 6 | bslbf |
| if (AAC_type_flag == 0b1 |
| AAC_type | 8 | uimsbf |
| } |
| for (i=0;i<N;i++) { |
| additional_info_byte | 8 | uimsbf |
| } |
| } |
| } |
| SAOC_DE_flag | Parametric data | Parametric data in PES_private_data |
|  | in AAC audio ancillary data | (see note) |
| 0b0 | shall not be present | shall not be present |
| 0b1 | SAOC-DE parametric data shall be present | DE_control_data may be present |
| NOTE: PES_private_data | within the PES packet header of the audio | component as defined in ETSI |
| TS 101 154 | [14], clause E.7.2. |
| stream_content | stream_content_ext | component_type | Description |
| 0x9 | 0x0 | 0x00 | HEVC Main Profile high definition video, 50 Hz |
|  |  | 0x01 | HEVC Main 10 Profile high definition video, 50 Hz |
|  |  | 0x02 | HEVC Main Profile high definition video, 60 Hz |
|  |  | 0x03 | HEVC Main 10 Profile high definition video, 60 Hz |
| NOTE: This value should be used for HLG10 HDR services, and/or HFR services with dual PID and temporal |
| scalability as defined in ETSI TS 101 154 [14]. See also clause I.2.5.2. |
| stream_content | stream_content_ext | component_type | Description |
| 0x9 | 0x0 | 0x00 | HEVC Main Profile high definition video, 50 Hz |
|  |  | 0x01 | HEVC Main 10 Profile high definition video, 50 Hz |
|  |  | 0x02 | HEVC Main Profile high definition video, 60 Hz |
|  |  | 0x03 | HEVC Main 10 Profile high definition video, 60 Hz |
| stream_content | stream_content_ext | component_type | Description |
| 0xB | 0xF | 0x03 | plano-stereoscopic top and bottom (TaB) frame-packing |
| Service | SDT | EIT component_descriptor | Description |
| category | service_type component_descriptor |
| stream_content | stream_content_ext | component_type | Description |
| 0xB | 0xF | 0x04 | HLG10HDR |
|  |  | 0x05 | HEVC temporal video subset for a frame rate of 100 Hz, |
|  |  |  | (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
|  |  |  | (cid:15)(cid:6)(cid:6)(cid:15) Hz, or 120 Hz |
| stream_content | stream_content_ext | component_type | Description |
| 0x3 | n/a | 0x40 | Video spatial resolution has been upscaled from lower |
|  |  |  | resolution source material |
|  |  | 0x41 | Video is SDR |
|  |  | 0x42 | Video is HDR remapped from SDR source material |
|  |  | 0x43 | Video is HDR up-converted from SDR source material |
|  |  | 0x44 | Video is standard frame rate, less than or equal to 60 Hz |
|  |  | 0x45 | High frame rate video generated from lower frame rate |
|  |  |  | source material |
| HEVC Main Profile high definition video, | 0x1F | 0x9 | 0x0 | 0x00 |
| 50 Hz |
| HEVC Main 10 Profile high definition video, | 0x1F | 0x9 | 0x0 | 0x01 |
| 50 Hz |
| HEVC Main Profile high definition video, | 0x1F | 0x9 | 0x0 | 0x02 |
| 60 Hz |
| HEVC Main 10 Profile high definition video, | 0x1F | 0x9 | 0x0 | 0x03 |
| 60 Hz |
|  |  | 0x06 | HEVC ultra high definition video, frame rate of 100 Hz, |
|  |  |  | (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
|  |  |  | (cid:15)(cid:6)(cid:6)(cid:15) |
|  |  | 0x07 | HEVC ultra high definition video with PQ10 HDR, frame |
|  |  |  | (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
|  |  |  | (cid:15)(cid:6)(cid:6)(cid:15) |
| stream_content | stream_content_ext | component_type | Description |
| 0xB | 0xF | 0x04 | HLG10 HDR |
|  |  | 0x05 | HEVC temporal video subset for a frame rate of 100 Hz, |
|  |  |  | (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
|  |  |  | (cid:15)(cid:6)(cid:6)(cid:15) Hz or 120 Hz |
| stream_content | stream_content_ext | component_type | Description |
| 0xB | 0xF | 0x06 | SMPTE ST 2094-10 DMI format as defined in |
|  |  |  | clause 5.14.4.4.3.4.3 of ETSI TS 101 154 [14] |
|  |  | 0x07 | SL-HDR2 DMI format as defined in clause 5.14.4.4.3.4.4 |
|  |  |  | of ETSI TS 101 154 [14] |
|  |  | 0x08 | SMPTE ST 2094-40 DMI format as defined in |
|  |  |  | clause 5.14.4.4.3.4.5 of ETSI TS 101 154 [14] |
| HEVC ultra high definition video, frame rate of | 0x20 | 0x9 | 0x0 | 0x06 |
| (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
| (cid:15)(cid:6)(cid:6)(cid:15) |
| HEVC ultra high definition video, frame rate of | 0x20 | 0x9 | 0x0 | 0x06 | 0xB | 0xF | 0x04 |
| (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
| (cid:15)(cid:6)(cid:6)(cid:15) |
| HEVC ultra high definition video with PQ10 | 0x20 | 0x9 | 0x0 | 0x05 | 0xB | 0xF | 0x06 |
| HDR with a frame rate lower than or equal to |  |  |  |  | 0xB | 0xF | 0x07 |
| HEVC ultra high definition video with PQ10 | 0x20 | 0x9 | 0x0 | 0x05 | 0xB | 0xF | 0x05 |
| (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
| (cid:15)(cid:6)(cid:6)(cid:15) |
| HEVC ultra high definition video with PQ10 | 0x20 | 0x9 | 0x0 | 0x05 | 0xB | 0xF | 0x05 |
| (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
| HDR with a frame rate of 100 Hz, (cid:15)(cid:6)(cid:6)(cid:15) Hz, or |  |  |  |  | 0xB | 0xF | 0x06 |
|  |  |  |  |  | 0xB | 0xF | 0x07 |
| 120 Hz containing a half frame rate HEVC |
| HEVC ultra high definition video with PQ10 | 0x20 | 0x9 | 0x0 | 0x07 |
| (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
| (cid:15)(cid:6)(cid:6)(cid:15) |
| HEVC ultra high definition video with PQ10 | 0x20 | 0x9 | 0x0 | 0x7 | 0xB | 0xF | 0x06 |
| (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
| HDR, frame rate of 100 Hz, (cid:15)(cid:6)(cid:6)(cid:15) Hz, or |  |  |  |  | 0xB | 0xF | 0x07 |
| HEVC ultra high definition video with a | 0x20 | 0x9 | 0x0 | 0x08 | 0xB | 0xF | 0x06 |
| resolution up to 7 680 x 4 320, frame rate up to |  |  |  |  | 0xB | 0xF | 0x07 |
| HEVC Main Profile high definition video, 50 Hz | 0x1F | 0x9 | 0x0 | 0x00 |
| HEVC Main 10 Profile high definition video, | 0x1F | 0x9 | 0x0 | 0x01 |
| 50 Hz |
| HEVC Main Profile high definition video, 60 Hz | 0x1F | 0x9 | 0x0 | 0x02 |
| HEVC Main 10 Profile high definition video, | 0x1F | 0x9 | 0x0 | 0x03 |
| 60 Hz |
| HEVC ultra high definition video, frame rate of | 0x20 | 0x9 | 0x0 | 0x06 |
| (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
| (cid:15)(cid:6)(cid:6)(cid:15) |
| HEVC ultra high definition video, frame rate of | 0x20 | 0x9 | 0x0 | 0x06 | 0xB | 0xF | 0x04 |
| (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
| (cid:15)(cid:6)(cid:6)(cid:15) |
| HEVC ultra high definition video with | 0x20 | 0x9 | 0x0 | 0x05 | 0xB | 0xF | 0x06 |
| PQ10HDR with a frame rate lower than or |  |  |  |  | 0xB | 0xF | 0x07 |
| HEVC ultra high definition video with PQ10 | 0x20 | 0x9 | 0x0 | 0x05 | 0xB | 0xF | 0x05 |
| (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
| (cid:15)(cid:6)(cid:6)(cid:15) |
| HEVC ultra high definition video with PQ10 | 0x20 | 0x9 | 0x0 | 0x05 | 0xB | 0xF | 0x05 |
| (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
| HDR with a frame rate of 100 Hz, (cid:15)(cid:6)(cid:6)(cid:15) Hz, or |  |  |  |  | 0xB | 0xF | 0x06 |
|  |  |  |  |  | 0xB | 0xF | 0x07 |
| 120 Hz containing a half frame rate HEVC |
| HEVC ultra high definition video with PQ10 | 0x20 | 0x9 | 0x0 | 0x07 |
| (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
| (cid:15)(cid:6)(cid:6)(cid:15) |
| HEVC ultra high definition video with PQ10 | 0x20 | 0x9 | 0x0 | 0x7 | 0xB | 0xF | 0x06 |
| (cid:15)(cid:5)(cid:6)(cid:6)(cid:6)(cid:6) |
| HDR, frame rate of 100 Hz, (cid:15)(cid:6)(cid:6)(cid:15) Hz, or |  |  |  |  | 0xB | 0xF | 0x07 |
| HEVC ultra high definition video with a | 0x20 | 0x9 | 0x0 | 0x08 | 0xB | 0xF | 0x06 |
| resolution up to 7 680 x 4 320, frame rate up to |  |  |  |  | 0xB | 0xF | 0x07 |
| stream_content | stream_content_ext | component_type | Description |
| 0x3 | n/a | 0x40 | Video spatial resolution has been upscaled from lower |
|  |  |  | resolution source material |
|  |  | 0x41 | Video is SDR |
|  |  | 0x42 | Video is HDR remapped from SDR source material |
|  |  | 0x43 | Video is HDR up-converted from SDR source material |
|  |  | 0x44 | Video is standard frame rate, less than or equal to 60 Hz |
|  |  | 0x45 | High frame rate video generated from lower frame rate |
|  |  |  | source material |
| 0xB | 0xF | 0x04 | HLG10 HDR |
|  |  | 0x06 | SMPTE ST 2094-10 DMI format as defined ETSI |
|  |  |  | TS 101 154 [14] |
|  |  | 0x07 | SL-HDR2 DMI format as defined in ETSI TS 101 154 [14] |
|  |  | 0x08 | SMPTE ST 2094-40 DMI format as defined in ETSI |
|  |  |  | TS 101 154 [14] |
|  |  | 0x09 | PQ10 HDR |
| VVC bitstream with resolution up to | 0x21 | 0x9 | 0x0 | 0x10 | 0xB | 0xF | 0x06 |
| 3 840 x 2 160 (including 1 920 x 1 080) high |  |  |  |  | 0xB | 0xF | 0x07 |
| VVC bitstream with resolution up to | 0x21 | 0x9 | 0x0 | 0x11 | 0xB | 0xF | 0x06 |
| 3 840 x 2 160 (including 1 920 x 1 080) high |  |  |  |  | 0xB | 0xF | 0x07 |
| VVC bitstream with resolution up to | 0x21 | 0x9 | 0x0 | 0x12 | 0xB | 0xF | 0x06 |
| 7 680 x 4 320 high dynamic range PQ frame |  |  |  |  | 0xB | 0xF | 0x07 |
| VVC bitstream with resolution up to | 0x21 | 0x9 | 0x0 | 0x13 | 0xB | 0xF | 0x06 |
| 7 680 x 4 320 high dynamic range PQ frame |  |  |  |  | 0xB | 0xF | 0x07 |
| stream_content | stream_content_ext | component_type | Description |
| 0x3 | n/a | 0x40 | Video spatial resolution has been upscaled from lower |
|  |  |  | resolution source material |
|  |  | 0x41 | Video is SDR |
|  |  | 0x42 | Video is HDR remapped from SDR source material |
|  |  | 0x43 | Video is HDR up-converted from SDR source material |
|  |  | 0x44 | Video is standard frame rate, less than or equal to 60 Hz |
|  |  | 0x45 | High frame rate video generated from lower frame rate |
|  |  |  | source material |
| 0xB | 0xF | 0x04 | HLG10 HDR |
|  |  | 0x06 | SMPTE ST 2094-10 DMI format as defined in ETSI |
|  |  |  | TS 101 154 [14] |
|  |  | 0x07 | SL-HDR2 DMI format as defined in ETSI TS 101 154 [14] |
|  |  | 0x08 | SMPTE ST 2094-40 DMI format as defined in ETSI |
|  |  |  | TS 101 154 [14] |
|  |  | 0x09 | PQ10 HDR |
| AVS3 bitstream with resolution up to | 0x22 | 0x9 | 0x0 | 0x20 | 0xB | 0xF | 0x06 |
| 3 840 x 2 160 (including 1 920 x 1 080) high |  |  |  |  | 0xB | 0xF | 0x07 |
| AVS3 bitstream with resolution up to | 0x22 | 0x9 | 0x0 | 0x21 | 0xB | 0xF | 0x06 |
| 3 840 x 2 160 (including 1 920 x 1 080) high |  |  |  |  | 0xB | 0xF | 0x07 |
| AVS3 bitstream with resolution up to | 0x22 | 0x9 | 0x0 | 0x22 | 0xB | 0xF | 0x06 |
| 7 680 x 4 320 high dynamic range PQ frame |  |  |  |  | 0xB | 0xF | 0x07 |
| AVS3 bitstream with resolution up to | 0x22 | 0x9 | 0x0 | 0x23 | 0xB | 0xF | 0x06 |
| 7 680 x 4 320 high dynamic range PQ frame |  |  |  |  | 0xB | 0xF | 0x07 |
| Audio coding | stream_content | stream_content_ext | component_type |
| MPEG-1 Layer 2 | 0x2 | 0xF | 0x47 |
| Enhanced AC-3 | 0x4 | 0xF | 0x90 (see note 1) |
| AC-4 | 0x9 | 0x1 | 0x0C to 0x0D (see note 2) |
| HE-AAC, AAC (see note 3) | 0x6 | 0xF | 0x47 |
| HE-AAC v2 | 0x6 | 0xF | 0x49 |
| DTS-HD | 0x7 | 0xF | 0bx001 0xxx (see note 4) |
| DTS-UHD | 0x9 | 0x1 | 0x1C to 0x1D (see note 5) |
| Audio coding | stream_content | stream_content_ext | component_type |
| MPEG-1 Layer 2 | 0x2 | 0xF | 0x48 |
| AC-3 | 0x4 | 0xF | 0b0101 0xxx (see note 1) |
| Enhanced AC-3 | 0x4 | 0xF | 0b1101 0xxx (see note 1) |
| AC-4 | 0x9 | 0x1 | 0x06 to 0x0B (see note 2) |
| HE-AAC, AAC (see note 3) | 0x6 | 0xF | 0x48 |
| HE-AAC v2 | 0x6 | 0xF | 0x4A |
| DTS | 0x7 | 0xF | 0bx101 0xxx (see note 4) |
| DTS-HD | 0x7 | 0xF | 0bx101 0xxx (see note 4) |
| DTS-UHD | 0x9 | 0x1 | 0x16 to 0x1B (see note 5) |
| Audio purpose | audio_type | mix_type | editorial_classification |
|  | (see note 1) | (see note 2) | (see note 2) |
| Main audio (see note 3) | 0x00 or 0x01 | 1 | 0x00 |
| Audio description (broadcast-mix) | 0x00, 0x01, or 0x03 | 1 | 0x01 |
| Audio description (receiver-mix) | 0x03 | 0 | 0x01 |
| Clean audio (broadcast-mix) | 0x02 | 1 | 0x02 |
| Parametric data dependent stream | 0x02 | 0 | 0x04 |
| (see note 4) |
| Spoken subtitles (broadcast-mix) | 0x00, 0x01, or 0x03 | 1 | 0x03 |
| Spoken subtitles (receiver-mix) | 0x03 | 0 | 0x03 |
| Unspecific audio for the general | any | 0 or 1 | 0x17 |
| audience |
| user defined | any | 0 or 1 | 0x18 to 0x1F |
| Audio coding | stream_content | stream_content_ext | component_type |
| Dependent SAOC-DE data stream | 0x3 | 0xF | 0x80 |
| HE-AAC, HE-AAC v2, or AAC (see note) with SAOC-DE | 0x6 | 0xF | 0xA0 |
| ancillary data |
| NOTE: AAC also uses this type (see note 7 in table 26 in | clause 6.2.8). |
| SD | SD | 0xE | 0 | Link to alternate event instances also in SD |
| SD | HD | 0xE | 1 | Link to event in HD |
| SD | Frame Compatible | 0xE | 2 | Link to event in frame compatible |
|  | (FC)-3DTV |  |  | plano-stereoscopic |
| SD | SC-3DTV MVC | 0xE | 3 | Link to event in service compatible |
|  |  |  |  | plano-stereoscopic MVC |
| SD | UHD | 0xF | 0 | Link to event in UHD |
| SD | SFC-3DTV HEVC | 0xF | 1 | Link to event in service frame compatible |
|  |  |  |  | plano-stereoscopic |
| HD | SD | 0xE | 0 | Link to event in SD |
| HD | HD | 0xE | 1 | Link to alternate event instances also in HD |
| HD | FC-3DTV | 0xE | 2 | Link to event in frame compatible |
|  |  |  |  | plano-stereoscopic |
| HD | SC-3DTV MVC | 0xE | 3 | Link to event in service compatible |
|  |  |  |  | plano-stereoscopic MVC |
| HD | UHD | 0xF | 0 | Link to event in UHD |
| HD | SFC-3DTV HEVC | 0xF | 1 | Link to event in service frame compatible |
|  |  |  |  | plano-stereoscopic |
| FC-3DTV | SD | 0xE | 0 | Link to event in SD |
| FC-3DTV | HD | 0xE | 1 | Link to event in HD |
| FC-3DTV | FC-3DTV | 0xE | 2 | Link to alternate event instances also in frame |
|  |  |  |  | compatible plano-stereoscopic |
| FC-3DTV | SC-3DTV MVC | 0xE | 3 | Link to event in service compatible |
|  |  |  |  | plano-stereoscopic MVC |
| FC-3DTV | UHD | 0xF | 0 | Link to event in UHD |
| FC-3DTV | SFC-3DTV HEVC | 0xF | 1 | Link to event in service frame compatible |
|  |  |  |  | plano-stereoscopic |
| SC-3DTV | SD | 0xE | 0 | Link to event in SD |
| MVC |
| SC-3DTV | HD | 0xE | 1 | Link to event in HD |
| MVC |
| SC-3DTV | FC-3DTV | 0xE | 2 | Link to event in frame compatible |
| MVC |  |  |  | plano-stereoscopic |
| SC-3DTV | SC-3DTV MVC | 0xE | 3 | Link to alternate event instances also in |
| MVC |  |  |  | service compatible plano-stereoscopic MVC |
| SC-3DTV | UHD | 0xF | 0 | Link to event in UHD |
| MVC |
| SC-3DTV | SFC-3DTV HEVC | 0xF | 1 | Link to event in service frame compatible |
| MVC |  |  |  | plano-stereoscopic |
| SFC-3DTV | SD | 0xE | 0 | Link to event in SD |
| HEVC |
| SFC-3DTV | HD | 0xE | 1 | Link to event in HD |
| HEVC |
| SFC-3DTV | FC-3DTV | 0xE | 2 | Link to event in frame compatible |
| HEVC |  |  |  | plano-stereoscopic |
| SFC-3DTV | SC-3DTV MVC | 0xE | 3 | Link to event in service compatible |
| HEVC |  |  |  | plano-stereoscopic MVC |
| SFC-3DTV | UHD | 0xF | 0 | Link to event in UHD |
| HEVC |
| SFC-3DTV | SFC-3DTV HEVC | 0xF | 1 | Link to alternate event instances also in |
| HEVC |  |  |  | service frame compatible plano-stereoscopic |
| UHD | SD | 0xE | 0 | Link to event in SD |
| UHD | HD | 0xE | 1 | Link to event in HD |
| UHD | FC-3DTV | 0xE | 2 | Link to event in frame compatible |
|  |  |  |  | plano-stereoscopic |
| UHD | SC-3DTV MVC | 0xE | 3 | Link to event in service compatible |
|  |  |  |  | plano-stereoscopic MVC |
| UHD | UHD | 0xF | 0 | Link to alternate event instances also in UHD |
| UHD | SFC-3DTV HEVC | 0xF | 1 | Link to event in service frame compatible |
|  |  |  |  | plano-stereoscopic |
| Syntax | Number of bits | Identifier |
| DTS_Neural_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| config_id | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| additional_info_byte | 8 | uimsbf |
| } |
| } |
| config_id | Original audio configuration (see note 1) | Original channel count (see note 2) |
| 0 | unknown or undefined |
| 1 | L, R | 2 |
| 2 | L, R, C | 3 |
| 3 | L, R, Ls, Rs | 4 |
| 4 | L, R, C, Ls, Rs | 5 |
| 5 | L, R, C, Ls, Rs, Cs | 6 |
| 6 | L, R, C, Ls, Rs, Lb, Rb | 7 |
| 7 | L, R, Ls, Rs, Cs | 5 |
| 8 | L, R, Ls, Rs, Lb, Rb | 6 |
| 9 to 255 | reserved for future use |
| config_id | Original audio configuration (see note) | Original channel count |
| 0 | unknown or undefined |
| 1 | L, R, C, LFE, Ls, Rs | 5.1 |
| 2 | L, R, C, LFE, Ls, Rs, Cs | 6.1 |
| 3 | L, R, C, LFE, Ls, Rs, Lb, Rb | 7.1 |
| 4 to 255 | reserved for future use |
| NOTE: L = | left, R = right, C = centre, LFE = low frequency effects, | s = surround, b = |
| back. |
| NGA | Field in the | Mapping |
| codec | audio_preselection_descriptor |
| AC-4 | num_preselections | num_presentation field (within the AC-4 TOC according to clause 6.7 |
| Part-2 |  | of ETSI TS 101 154 [14]). |
| AC-4 | preselection_id | presentation_group_index field of the Preselection (within the AC-4 |
| Part-2 |  | TOC according to clause 6.7 of ETSI TS 101 154 [14]). |
| NGA | Field in the | Mapping |
| codec | audio_preselection_descriptor |
| MPEG-H | num_preselections | mae_numGroupPresets field as specified in clause 6.8 of ETSI |
|  |  | TS 101 154 [14]. |
| MPEG-H | preselection_id | mae_GroupPresetID field as specified in clause 6.8 of ETSI |
|  |  | TS 101 154 [14]. |
| NGA | Field in the | Mapping |
| codec | audio_preselection_descriptor |
|  |  | Document history |
| Edition 1 | October 1995 | Publication as ETSI ETS 300 468 |
| Edition 2 | January 1997 | Publication as ETSI ETS 300 468 |
| V1.3.1 | February 1998 | Publication |
| V1.4.1 | November 2000 | Publication |
| V1.5.1 | May 2003 | Publication |
| V1.6.1 | November 2004 | Publication |
| V1.7.1 | May 2006 | Publication |
| V1.8.1 | July 2008 | Publication |
| V1.9.1 | March 2009 | Publication |
| V1.10.1 | November 2009 | Publication |
| V1.11.1 | April 2010 | Publication |
| V1.12.1 | October 2011 | Publication |
| V1.13.1 | August 2012 | Publication |
| V1.14.1 | May 2014 | Publication |
| V1.15.1 | March 2016 | Publication |
| V1.16.1 | August 2019 | Publication |
| V1.17.1 | October 2022 | Publication |
| V1.18.1 | December 2023 | Publication |
| V1.19.0 | November 2024 | EN Approval Procedure AP 20250216: 2024-11-18 to 2025-02-17 |
| V1.19.1 | February 2025 | Publication |


