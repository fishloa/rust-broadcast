# Table 4.13: Frame size code table (1 word = 16 bits)

_Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope._


|  frmsizecod | Nominal bit rate (kbit/s) | Words/syncframe fs = 32 kHz | Words/syncframe fs = 44,1 kHz | Words/syncframe fs = 48 kHz  |
| --- | --- | --- | --- | --- |
|  000000 (0) | 32 | 96 | 69 | 64  |
|  000001 (0) | 32 | 96 | 70 | 64  |
|  000010 (1) | 40 | 120 | 87 | 80  |
|  000011 (1) | 40 | 120 | 88 | 80  |
|  000100 (2) | 48 | 144 | 104 | 96  |
|  000101 (2) | 48 | 144 | 105 | 96  |
|  000110 (3) | 56 | 168 | 121 | 112  |
|  000111 (3) | 56 | 168 | 122 | 112  |
|  001000 (4) | 64 | 192 | 139 | 128  |
|  001001 (4) | 64 | 192 | 140 | 128  |
|  001010 (5) | 80 | 240 | 174 | 160  |
|  001011 (5) | 80 | 240 | 175 | 160  |
|  001100 (6) | 96 | 288 | 208 | 192  |
|  001101 (6) | 96 | 288 | 209 | 192  |
|  001110 (7) | 112 | 336 | 243 | 224  |
|  001111 (7) | 112 | 336 | 244 | 224  |
|  010000 (8) | 128 | 384 | 278 | 256  |
|  010001 (8) | 128 | 384 | 279 | 256  |
|  010010 (9) | 160 | 480 | 348 | 320  |
|  010011 (9) | 160 | 480 | 349 | 320  |
|  010100 (10) | 192 | 576 | 417 | 384  |
|  010101 (10) | 192 | 576 | 418 | 384  |
|  010110 (11) | 224 | 672 | 487 | 448  |



|  frmsizecod | Nominal bit rate (kbit/s) | Words/syncframe fs = 32 kHz | Words/syncframe fs = 44,1 kHz | Words/syncframe fs = 48 kHz  |
| --- | --- | --- | --- | --- |
|  010111 (11) | 224 | 672 | 488 | 448  |
|  011000 (12) | 256 | 768 | 557 | 512  |
|  011001 (12) | 256 | 768 | 558 | 512  |
|  011010 (13) | 320 | 960 | 696 | 640  |
|  011011 (13) | 320 | 960 | 697 | 640  |
|  011100 (14) | 384 | 1 152 | 835 | 768  |
|  011101 (14) | 384 | 1 152 | 836 | 768  |
|  011110 (15) | 448 | 1 344 | 975 | 896  |
|  011111 (15) | 448 | 1 344 | 976 | 896  |
|  100000 (16) | 512 | 1 536 | 1 114 | 1 024  |
|  100001 (16) | 512 | 1 536 | 1 115 | 1 024  |
|  100010 (17) | 576 | 1 728 | 1 253 | 1 152  |
|  100011 (17) | 576 | 1 728 | 1 254 | 1 152  |
|  100100 (18) | 640 | 1 920 | 1 393 | 1 280  |
|  100101 (18) | 640 | 1 920 | 1 394 | 1 280  |
|  NOTE: f_{s} : sampling frequency.  |   |   |   |   |

If the number of user bits indicated by auxdatal is smaller than the number of available aux bits nauxbits, the user data is located at the end of the auxbits field. This allows a decoder to find and unpack the auxdatal user bits without knowing the value of nauxbits (which can only be determined by decoding the audio in the entire syncframe). The order of the user data in the auxbits field is forward. Thus the aux data decoder (which may not decode any audio) may simply look to the end of the AC-3 syncframe to find auxdatal, backup auxdatal bits (from the beginning of auxdatal) in the data stream, and then unpack auxdatal bits moving forward in the data stream.

## 4.4.4.2 auxdatal - Auxiliary data length - 14 bits

This 14-bit integer value indicates the length, in bits, of the user data in the auxbits auxiliary field.

## 4.4.4.3 auxdatae - Auxiliary data exists - 1 bit

If this bit is a 1, then the auxdatal parameter precedes in the bit stream. If this bit is a 0, auxdatal does not exist, and there is no user data.

## 4.4.5 errorcheck - Frame error detection field

### 4.4.5.1 crcrsv - CRC reserved bit - 1 bit

Reserved for use in specific applications to ensure crc2 will not be equal to the sync word. Use of this bit is optional by encoders. If the crc2 calculation results in a value equal to the syncword, the crcrsv bit may be inverted. This will result in a crc2 value which is not equal to the syncword.

### 4.4.5.2 crc2 - Cyclic redundancy check 2 - 16 bits

The 16-bit CRC applies to the entire syncframe. The details of the CRC checking are described in clause 6.10.1.

## 4.5 Bit stream constraints

The following constraints shall be imposed upon the encoded bit stream by the AC-3 encoder. These constraints allow AC-3 decoders to be manufactured with smaller input memory buffers:

1) The combined size of the syncinfo fields, the bsi fields, block 0 and block 1 combined, shall not exceed 5/8 of the syncframe.

2) The combined size of the block 5 mantissa data, the auxiliary data fields, and the errorcheck fields shall not exceed the final 3/8 of the syncframe.



3) Block 0 shall contain all necessary information to begin correctly decoding the bit stream.

4) Whenever the state of cplinu changes from off to on, all coupling information shall be included in the block in which coupling is turned on. No coupling related information shall be reused from any previous blocks where coupling may have been on.

5) Coupling shall not be used in dual mono $(1 + 1)$ or mono $(1/0)$ modes. For blocks in which coupling is used, there shall be at least two channels in coupling.

6) Bit stream elements shall not be reused from a previous block if other bit stream parameters change the dimensions of the elements to be reused. For example, exponents shall not be reused if the start or end mantissa bin changes from the previous block.

# 5 Decoding the AC-3 bit stream

## 5.1 Introduction

Clause 4 specifies the details of the AC-3 bit stream syntax. The following clauses provide an overview of the AC-3 decoding process as diagrammed in Figure 5.1, where the decoding process flow is shown as a sequence of blocks down the centre of the page, and some of the information flow is indicated by arrowed lines at the sides of the page. More detailed information on some of the processing blocks will be found in clause 6. The decoder described in the following clauses should be considered one example of a decoder. Other methods may exist to implement decoders, and these other methods may have advantages in certain areas (such as instruction count, memory requirement, number of transforms required, etc.).

## 5.2 Summary of the decoding process

### 5.2.1 Input bit stream

#### 5.2.1.0 Introduction

The input bit stream will typically come from a transmission or storage system. The interface between the source of AC-3 data and the AC-3 decoder is not specified in the present document. The details of the interface affect a number of decoder implementation details.

#### 5.2.1.1 Continuous or burst input

The encoded AC-3 data may be input to the decoder as a continuous data stream at the nominal bit rate, or chunks of data may be burst into the decoder at a high rate with a low duty cycle. For burst mode operation, either the data source or the decoder may be the master controlling the burst timing. The AC-3 decoder input buffer may be smaller in size if the decoder can request bursts of data on an as-needed basis. However, the external buffer memory may be larger in this case.

#### 5.2.1.2 Byte or word alignment

Most applications of the present document will convey the bit AC-3 bit stream with byte or (16-bit) word alignment. The syncframe is always an integral number of words in length. The decoder may receive data as a continuous serial stream of bits without any alignment. Or, the data may be input to the decoder with either byte or word (16-bit) alignment. Byte or word alignment of the input data may allow some simplification of the decoder. Alignment does reduce the probability of false detection of the sync word.


---


## 5.2.2 Synchronization and error detection

The AC-3 bit-stream format allows rapid synchronization. The 16-bit sync word has a low probability of false detection. With no input stream alignment the probability of false detection of the sync word is 0,0015 % per input stream bit position. For a bit rate of 384 kbit/s, the probability of false sync word detection is 19 % per syncframe. Byte alignment of the input stream drops this probability to 2,5 %, and word alignment drops it to 1,2 %.

When a sync pattern is detected the decoder may be estimated to be in sync and one of the CRC words (crc1 or crc2) may be checked. Since crc1 comes first and covers the first 5/8 of the syncframe, the result of a crc1 check may be available after only 5/8 of the syncframe has been received. Or, the entire syncframe size can be received and crc2 checked. If either CRC checks, the decoder may safely be presumed to be in sync and decoding and reproduction of audio may proceed. The chance of false sync in this case would be the concatenation of the probabilities of a false sync word detection and a CRC misdetection of error. The CRC check is reliable to 0,0015 %. This probability, concatenated with the probability of a false sync detection in a byte aligned input bit stream, yield a probability of false synchronization of 0,000035 % (or about once in 3 million synchronization attempts).

If this small probability of false sync is too large for an application, there are several methods which may reduce it. The decoder may only presume correct sync in the case that both CRC words check properly. The decoder may require multiple sync words to be received with the proper alignment. If the data transmission or storage system is aware that data is in error, this information may be made known to the decoder.



# Annex A (normative): AC-3 bit streams in the MPEG-2 multiplex

## A.0 Scope

This annex contains specifications on how to combine one or more AC-3 bit streams into the ATSC (Recommendation ITU-R BT.1300 [i.5], System A) or DVB (Recommendation ITU-R BT.1300 [i.5], System B) MPEG-2 transport stream (ISO/IEC 13818-1 [i.4]).

## A.1 Introduction

The AC-3 bit stream is included in an MPEG-2 multiplex bit stream in much the same way an MPEG-1 audio stream would be included. The AC-3 bit stream is packetized into PES packets. An MPEG-2 multiplex bit stream containing AC-3 bit streams shall meet all constraints described in the STD model in clause A.2.6. It is necessary to unambiguously indicate that an AC-3 stream is, in fact, an AC-3 stream (and not an MPEG audio stream). The MPEG-2 standard does not explicitly indicate codes to be used to indicate an AC-3 stream. Also, the MPEG-2 standard does not have an audio descriptor adequate to describe the contents of the AC-3 bit stream in the PSI tables.

The AC-3 audio access unit (AU) or presentation unit (PU) is an AC-3 syncframe. The AC-3 syncframe contains 1536 audio samples. The duration of an AC-3 access (or presentation) unit is 32 ms for audio sampled at 48 kHz, approximately 34,83 ms for audio sampled at 44,1 kHz, and 48 ms for audio sampled at 32 kHz.

The items which need to be specified in order to include AC-3 within the MPEG-2 bit stream are: stream_type, stream_id, AC-3 audio descriptor, and, for system A only, registration descriptor. The registration descriptor is not required in System B since the AC-3_descriptor is regarded as a public descriptor in this system. The ISO 639-1 [i.1] language descriptor may be employed to indicate language. Some constraints are placed on the PES layer for the case of multiple audio streams intended to be reproduced in exact sample synchronism. In System A (ATSC) the AC-3 audio descriptor is titled "AC-3_audio_stream_descriptor" while in System B (DVB) the AC-3 audio descriptor is titled "AC-3_descriptor". It should be noted that the syntax of these descriptors differs significantly between the two systems.

## A.2 Detailed specification for System A (ATSC)

### A.2.1 Stream_type

The value of stream_type for AC-3 shall be 0×81.

### A.2.2 Stream_id

The value of stream_id in the PES header shall be 0×BD (indicating private_stream_1). Multiple AC-3 streams may share the same value of stream_id since each stream is carried with a unique PID value. The mapping of values of PID to stream_type is indicated in the transport stream programme map table (PMT).



# A.2.3 Registration_descriptor

The syntax of the AC-3 registration_descriptor is shown below. The AC-3 registration_descriptor shall be included in the TS programme_map_section.

Table A.2.0a

|  Syntax | No. of bits | Mnemonic  |
| --- | --- | --- |
|  registration_descriptor() |  |   |
|  descriptor_tag | 8 | uimsbf  |
|  descriptor_length | 8 | uimsbf  |
|  format_identifier | 32 | uimsbf  |
|  descriptor_tag - 0X05. |  |   |
|  descriptor_length - 0X04. |  |   |
|  format_identifier - The AC-3 format_identifier is 0X41432D33 ("AC-3"). |  |   |

# A.2.4 AC-3 audio_descriptor

The AC-3 audio_stream_descriptor allows information about individual AC-3 bit streams to be included in the programme specific information (PSI) tables. This information is useful to enable decision making as to the appropriate AC-3 stream(s) that are present in the current broadcast to be directed to the audio decoder, and also to enable the announcement of characteristics of audio streams that will be included in future broadcasts. Note that horizontal lines in table A.2.0b indicate allowable termination points for the descriptor.

Table A.2.0b

|  Syntax | No. of bits | Mnemonic  |
| --- | --- | --- |
|  AC-3.audio_stream_descriptor() |  |   |
|  descriptor_tag | 8 | uimsbf  |
|  descriptor_length | 8 | uimsbf  |
|  sample_rate_code | 3 | bslbf  |
|  bsid | 5 | bslbf  |
|  bit_rate_code | 6 | bslbf  |
|  surround_mode | 2 | bslbf  |
|  bsmod | 3 | bslbf  |
|  num_channels | 4 | bslbf  |
|  full_svc | 1 | bslbf  |
|  langcod | 8 | bslbf  |
|  if(num_channels == 0) /* 1+1 mode */ |  |   |
|  langcod2 | 8 | bslbf  |
|  if(bsmod < 2) |  |   |
|  mainid | 3 | uimsbf  |
|  priority | 2 | bslbf  |
|  reserved | 3 | '111'  |
|  else asvcflags | 8 | bslbf  |
|  textlen | 7 | uimsbf  |
|  text_code | 1 | bslbf  |
|  for(i = 0; i < m; i++) |  |   |
|  text[i] | 8 | bslbf  |
|  language_flag | 1 | bslbf  |
|  language_flag_2 | 1 | bslbf  |
|  reserved | 6 | '111111'  |
|  if(language_flag == 1) {language} | 3x8 | uimsbf  |
|  if(language_flag_2 == 1) {language_2} | 3x8 | uimsbf  |
|  for(i = 0; i < n; i++) |  |   |
|  additional_info[i] | nx8 | bslbf  |



descriptor_tag - The value for the AC-3 descriptor_tag is 0×81.

descriptor_length - This is an 8-bit field specifying the number of bytes of the descriptor immediately following descriptor_length field.

sample_rate_code - This is a 3-bit field which indicates the sample rate of the encoded audio. The indication may be of one specific sample rate, or may be of a set of values which include the sample rate of the encoded audio (see Table A.2.1).

Table A.2.1: Sample_rate_code table

|  sample_rate_code | Sample rate (kHz)  |
| --- | --- |
|  000 | 48  |
|  001 | 44,1  |
|  010 | 32  |
|  011 | Reserved  |
|  100 | 48 or 44,1  |
|  101 | 48 or 32  |
|  110 | 44,1 or 32  |
|  111 | 48 or 44,1 or 32  |

bsid - This is a 5-bit field which is set to the same value as the bsid field in the AC-3 bit stream.

bit_rate_code - This is a 6-bit field. The lower 5 bits indicate a nominal bit rate. The MSB indicates whether the indicated bit rate is exact (MSB = 0) or an upper limit (MSB = 1) (see Table A.2.2).

Table A.2.2: Bit_rate_code table

|  bit_rate_code | Exact bit rate (kbit/s)  |
| --- | --- |
|  000000 (0) | 32  |
|  000001 (1) | 40  |
|  000010 (2) | 48  |
|  000011 (3) | 56  |
|  000100 (4) | 64  |
|  000101 (5) | 80  |
|  000110 (6) | 96  |
|  000111 (7) | 112  |
|  001000 (8) | 128  |
|  001001 (9) | 160  |
|  001010 (10) | 192  |
|  001011 (11) | 224  |
|  001100 (12) | 256  |
|  001101 (13) | 320  |
|  001110 (14) | 384  |
|  001111 (15) | 448  |
|  010000 (16) | 512  |
|  010001 (17) | 576  |
|  010010 (18) | 640  |

|  bit_rate_code | Bit rate upper limit (kbit/s)  |
| --- | --- |
|  100000 (32) | 32  |
|  100001 (33) | 40  |
|  100010 (34) | 48  |
|  100011 (35) | 56  |
|  100100 (36) | 64  |
|  100101 (37) | 80  |
|  100110 (38) | 96  |
|  100111 (39) | 112  |
|  101000 (40) | 128  |
|  101001 (41) | 160  |
|  101010 (42) | 192  |
|  101011 (43) | 224  |
|  101100 (44) | 256  |
|  101101 (45) | 320  |
|  101110 (46) | 384  |
|  101111 (47) | 448  |
|  110000 (48) | 512  |
|  110001 (49) | 576  |
|  110010 (50) | 640  |

dsurmod - This is a 2-bit field which may be set to the same value as the dsurmod field in the AC-3 bit stream, or which may be set to "00" (not indicated) (see Table A.2.3).

Table A.2.3: dsurmod table

|  surround_mode | Meaning  |
| --- | --- |
|  00 | Not indicated  |
|  01 | NOT Dolby® surround encoded  |
|  10 | Dolby® surround encoded  |
|  11 | Reserved  |



bsmod - This is a 3-bit field which is set to the same value as the bsmod field in the AC-3 bit stream.

num_channels - This is a 4-bit field which indicates the number of channels in the AC-3 bit stream. When the MSB is 0, the lower 3 bits are set to the same value as the acmod field in the AC-3 bit stream. When the MSB field is 1, the lower 3 bits indicate the maximum number of encoded audio channels (counting the life channel as 1). If the value of acmod in the AC-3 bit stream is "000" (1 + 1 mode), then the value of num_channels shall be set to "0000" (see Table A.2.4).

Table A.2.4: Num_channels table

|  num_channels | Audio coding mode (acmod)  |
| --- | --- |
|  0000 | 1 + 1  |
|  0001 | 1/0  |
|  0010 | 2/0  |
|  0011 | 3/0  |
|  0100 | 2/1  |
|  0101 | 3/1  |
|  0110 | 2/2  |
|  0111 | 3/2  |

|  num_channels | Number of encoded channels  |
| --- | --- |
|  1000 | 1  |
|  1001 | ≤ 2  |
|  1010 | ≤ 3  |
|  1011 | ≤ 4  |
|  1100 | ≤ 5  |
|  1101 | ≤ 6  |
|  1110 | Reserved  |
|  1111 | Reserved  |

full_svc - This is a 1-bit field which indicates whether or not this audio service is a full service suitable for presentation, or whether this audio service is only a partial service which should be combined with another audio service before presentation. This bit should be set to a "1" if this audio service is sufficiently complete to be presented to the listener without being combined with another audio service (for example, a visually impaired service which contains all elements of the programme; music, effects, dialogue, and the visual content descriptive narrative). This bit should be set to a "0" if the service is not sufficiently complete to be presented without being combined with another audio service (e.g. a visually impaired service which only contains a narrative description of the visual programme content and which needs to be combined with another audio service which contains music, effects, and dialogue).

langcod - This is an 8-bit field which is set to the same value as the langcod field in the AC-3 bit stream. If the AC-3 bit stream langcod field is not present, then this 8-bit field shall be set to 0xFF if present.

langcod2 - This is an 8-bit field which is set to the value of the langcod2 field in the AC-3 bit stream. If the AC-3 bit stream langcod2 field is not present, then this 8-bit field shall be set to 0xFF if present.

NOTE 1: The langcod and langcod2 fields are not (that is, are no longer) used to indicate language. The ISO 639 [i.1] language descriptor is used to indicate language. However, the AC-3 audio descriptor may optionally include the ISO_639_language_code, see below "language" and "language_2" fields.

mainid - This is a 3-bit field which contains a number in the range 0 - 7 which identifies a main audio service. Each main service should be tagged with a unique number. This value is used as an identifier to link associated services with particular main services.

priority - This is a 2-bit field that indicates the priority of the audio service. This field allows a Main audio service (bsmod equal to 0 or 1) to be marked as the primary audio service. Other audio services may be explicitly marked or not specified. Table A.2.5 shows how this field is encoded.

Table A.2.5: Priority Field Coding

|  Bit Field | Meaning  |
| --- | --- |
|  00 | reserved  |
|  01 | Primary Audio  |
|  10 | Other Audio  |
|  11 | Not specified  |

asvcflags - This is an 8-bit field. Each bit (0 - 7) indicates with which main service(s) this associated service is associated. The left most bit, bit 7, indicates whether this associated service may be reproduced along with main service number 7. If the bit has a value of 1, the service is associated with main service number 7. If the bit has a value of 0, the service is not associated with main service number 7.

textlen - This is an unsigned integer which indicates the length, in bytes, of a descriptive text field which follows.



text_code - This is a 1-bit field which indicates how the following text field is encoded. If this bit is a "1", the text is encoded as 1-byte characters using the ISO Latin-1 alphabet (ISO 8859-1 [i.3]). If this bit is a "0", the text is encoded with 2-byte unicode characters.

text[i] - The text field may contain a brief textual description of the audio service.

language_flag - This is a 1-bit flag that indicates whether or not the 3-byte language field is present in the descriptor. If this bit is set to "1", then the 3-byte language field is present. If this bit is set to "0", then the language field is not present.

language_flag_2 - This is a 1-bit flag that indicates whether or not the 3-byte language_2 field is present in the descriptor. If this bit is set to "1", then the 3-byte language_2 field is present. If this bit is set to "0", then the language_2 field is not present. This bit shall always be set to "0", unless the num_channels field is set to "0000" indicating the audio coding mode is 1+1 (dual mono). If the num_channels field is set to "0000" then this bit may be set to "1" and the language_2 field may be included in this descriptor.

language - This field is a 3-byte language code per ISO 639-2/B [i.2] defining the language of this audio service. If the AC-3 stream audio coding mode is 1+1 (dual mono), this field indicates the language of the first channel (channel 1, or "left" channel). The language field shall contain a three-character code as specified by ISO 639-2/B [i.2]. Each character is coded into 8 bits according to ISO 8859-1 [i.3] (ISO Latin-1) and inserted in order into the 24-bit field. The coding is identical to that used in the ISO_639_language_code value in the ISO_639_language_descriptor specified in ISO/IEC 13818-1 [i.4].

language_2 - This field is only present if the AC-3 stream audio coding mode is 1+1 (dual mono). This field is a 3-byte language code per ISO 639-2/B [i.2] defining the language of the second channel (channel 2, or "right" channel) in the AC-3 bit stream. The language_2 field shall contain a three-character code as specified by ISO n order into the 24-bit field. The coding is identical to that used in the ISO_639_language_code value in the ISO_639_language_descriptor specified in ISO/IEC 13818-1 [i.4].

additional_info[j] - This is a set of additional bytes filling out the remainder of the descriptor. The purpose of these bytes is not currently defined. This field is provided to allow the ATSC to extend this descriptor. No other use is permitted.

NOTE 2: In the event that there is a single Main service that alternates between different languages, the ISO 639-1 [i.1] Language descriptor may be used to communicate that additional information.

## A.2.5 ISO_639_language_code

The ISO_639_language_code descriptor allows a stream to be tagged with the 24-bit ISO 639-1 [i.1] language code.

## A.2.6 STD audio buffer size

For an MPEG-2 transport stream, the T-STD model defines the main audio buffer size $BS_{n}$ as:

$$
BS_n = BS_{mux} + BS_{dec} + BS_{oh}
$$

where:

- $BS_{mux} = 736$ bytes
- $BS_{oh}$: PES header overhead
- $BS_{dec}$: access unit buffer

MPEG-2 specifies a fixed value for $BS_n$ (3584 bytes) and indicates that any excess buffer may be used for additional multiplexing.

When an AC-3 bit stream is carried by an MPEG-2 transport stream, the transport stream shall be compliant with a main audio buffer size of:

$$
BS_n = BS_{mux} + BS_{pad} + BS_{dec}
$$
