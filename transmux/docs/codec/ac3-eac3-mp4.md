# Annex F (normative): AC-3 and Enhanced AC-3 bit streams in the ISO Base Media File Format

## F.0 Scope

The purpose of this annex is to define the necessary structures for the storage and identification of AC-3 and Enhanced AC-3 bit streams in a file format that is compliant with the ISO Base Media File Format. Examples of file formats that are derived from the ISO Base Media File Format include the MP4 file format and the 3GPP file format.

# F.1 AC-3 and Enhanced AC-3 Track definition

In the terminology of the ISO Base Media File Format specification [i.9], AC-3 and Enhanced AC-3 tracks are both audio tracks. Therefore the following rules shall apply to the media box in the AC-3 or Enhanced AC-3 track:

- In the Handler Reference Box, the handler\_type field shall be set to "soun".
- The Media Information Header Box shall contain a Sound Media Header Box.
- The Sample Description Box shall contain a box derived from AudioSampleEntry. For AC-3 tracks, this box is called AC3SampleEntry and is defined in clause F.3. For Enhanced AC-3 tracks this box is called EC3SampleEntry and is defined in clause F.5.
- The value of the timescale parameter in the Media Header Box, and the value of the SamplingRate parameter in the AC3SampleEntry Box or EC3SampleEntry Box shall be equal to the sample rate (in Hz) of the AC-3 or Enhanced AC-3 bit stream respectively.
- AC-3 bit streams shall always be identified by using the AC3SampleEntry Box. The use of the MP4AudioSampleEntry Box in combination with the Object Type Indicator value assigned for AC-3 to identify AC-3 bit streams is prohibited.
- Enhanced AC-3 bit streams shall always be identified by using the EC3SampleEntry Box. The use of the MP4AudioSampleEntry Box in combination with the Object Type Indicator value assigned for Enhanced AC-3 to identify Enhanced AC-3 bit streams is prohibited.
- The following bit stream parameters shall remain constant within an AC-3 bit stream that is identified by the AC3SampleEntry Box:
  - bsid
  - bsmod
  - acmod
  - lfeon
  - fscod
  - frmsizcod
- The following bit stream parameters shall remain constant within an Enhanced AC-3 bit stream that is identified by the EC3SampleEntry Box:
  - Number of independent substreams
  - Number of dependent substreams

- Within each independent substream:
  - bsid
  - bsmod
  - acmod
  - Ifeon
  - fscod
- Within each dependent substream:
  - bsid
  - acmod
  - Ifeon
  - fscod
  - chanmap

While the present document defines the AC-3 and Enhanced AC-3 bit stream syntax in big-endian byte order, it should be noted that little-endian byte order streams are valid and may be present within files that are compliant with the ISO Base Media File Format. The byte order of the AC-3 or Enhanced AC-3 bit stream can be determined from the order of the bytes in the syncword field at the start of each AC-3 or Enhanced AC-3 syncframe. A bit stream stored in bigendian byte order has a syncword value of 0x0B77, and a bit stream stored in little-endian byte order has a syncword value of 0x770B. It is strongly recommended that AC-3 and Enhanced AC-3 bit streams are stored within ISO Base Media Files in big-endian byte order.

# F.2 AC-3 and Enhanced AC-3 Sample definition

AC-3 or Enhanced AC-3 Samples contain six audio blocks, equivalent in duration to 1 536 contiguous samples of PCM audio data. Consequently, the value of the sample\_delta field in the Decoding Time to Sample Box shall be 1 536.

An AC-3 Sample shall be defined as follows:

• Exactly one AC-3 syncframe, as defined in clause 4.1 of the present document.

An Enhanced AC-3 Sample shall be defined as follows:

The number of Enhanced AC-3 syncframes required to deliver six blocks of audio data from every substream present in the Enhanced AC-3 bit stream, beginning with independent substream 0.

![](_page_2_Figure_2.jpeg)

**Figure F.2.1: Composition of ISOBMFF samples** 

How data is structured within an ISOBMFF sample depends on the configuration of the bitstream. Figures F.2.2 through F.2.4 provide different examples of ISOBMFF sample makeup.

Figure F.2.2 shows the construction of an ISOBMFF sample that contains a single Enhanced AC-3 Sample consisting of six audio blocks.

![](_page_2_Figure_6.jpeg)

**Figure F.2.2: ISOBMFF sample with a single substream with six blocks per frame** 

The six audio blocks represent 1, 536 PCM samples of audio from a single substream (substream 0).

Figure F.2.3 shows an ISOBMFF sample that contains a single Enhanced AC-3 Sample consisting of four frames.

![](_page_3_Figure_2.jpeg)

**Figure F.2.3: ISOBMFF sample with two substreams with three blocks per frame** 

Each frame contains three audio blocks (denoted 'AB0' for substream 0 and 'AB1' for substream 1), each representing 256 PCM samples of audio from all channels in each substream.

Figure F.2.4 shows an ISOBMFF sample that contains a single Enhanced AC-3 Sample consisting of six frames. Each frame contains one audio block, each representing 256 PCM samples of audio from every channel in the substream.

![](_page_3_Figure_6.jpeg)

**Figure F.2.4: ISOBMFF sample with a single substream with one block per frame** 

AC-3 and Enhanced AC-3 Samples shall be byte-aligned. If necessary, up to 7 zero-valued padding bits shall be added to the end of an AC-3 or Enhanced AC-3 Sample to achieve byte-alignment. The Padding Bits Box (defined in section 8.23 of ISO/IEC 14496-12 [i.9] need not be used to record padding bits that are added to a Sample to align its size to the nearest byte boundary.

## F.3 AC3SampleEntry Box

## F.3.0 Introduction

The bitstream format does not require external metadata to set up the decoder, as it is fully contained in that regard. Descriptor data is present, however, to provide information to the system without requiring access to the elementary stream, as the bitstream can be encrypted.

## F.3.1 Syntax

| Syntax                 | No. of bits | Identifier | Value |
|------------------------|-------------|------------|-------|
| AC3SampleEntry()       |             |            |       |
| {                      |             |            |       |
| BoxHeader.Size32       |             | uimsbf     |       |
| BoxHeader.Type32       |             | uimsbf     |       |
| Reserved [6]8          |             | uimsbf     | 0     |
| Data-reference-index16 |             | uimsbf     |       |
| Reserved [2]32         |             | uimsbf     | 0     |
| ChannelCount16         |             | uimsbf     | 2     |
| SampleSize16           |             | uimsbf     | 16    |
| Reserved32             |             | uimsbf     |       |
| SamplingRate16         |             | uimsbf     | 0     |
| Reserved16             |             | uimsbf     | 0     |
| AC3SpecificBox         |             |            |       |
| }                      |             |            |       |

## F.3.2 Semantics

The layout of the AC3SampleEntry box is identical to that of AudioSampleEntry defined in ISO/IEC 14496-12 [i.9] (including the reserved fields and their values), except that AC3SampleEntry ends with a box containing AC-3 bit stream information called AC3SpecificBox. The AC3SpecificBox field structure for AC-3 is defined in clause F.4.

For unencrypted tracks, The value of the BoxHeader.Type field shall be set to 0x61632d33 ('ac-3'). For encrypted tracks, the value should be set according to the encryption scheme.

The values of the ChannelCount and SampleSize fields within the AC3SampleEntry Box shall be ignored.

# F.4 AC3SpecificBox

## F.4.1 Syntax

| Syntax<br>No. of bits |  | Identifier |  |
|-----------------------|--|------------|--|
| AC3SpecificBox ()     |  |            |  |
| {                     |  |            |  |
| BoxHeader.Size 32     |  | uimsbf     |  |
| BoxHeader.Type 32     |  | uimsbf     |  |
| fscod 2               |  | uimsbf     |  |
| bsid 5                |  | uimsbf     |  |
| bsmod 3               |  | uimsbf     |  |
| acmod 3               |  | uimbsf     |  |
| lfeon 1               |  | bslbf      |  |
| bit_rate_code 5       |  | uimsbf     |  |
| reserved 5            |  | uimbsf     |  |
| }                     |  |            |  |

## F.4.2 Semantics

### F.4.2.1 BoxHeader.Type - 32 bits

The value of the 32-bit BoxHeader.Type field shall be set to 0x64616333 ('dac3').

### F.4.2.2 fscod - 2 bits

The 2-bit fscod field shall have the same meaning and shall be set to the same value as the fscod field in the AC-3 bit stream.

## F.4.2.3 bsid - 5 bits

The 5-bit bsid field shall have the same meaning and shall be set to the same value as the bsid field in the AC-3 bit stream.

#### F.4.2.4 bsmod - 3 bits

The 3-bit bsmod field shall have the same meaning and shall be set to the same value as the bsmod field in the AC-3 bit stream.

## F.4.2.5 acmod - 3 bits

The 3-bit acmod field shall have the same meaning and shall be set to the same value as the acmod field in the AC-3 bit stream.

#### F.4.2.6 lfeon - 1 bit

The 1-bit lfeon field has the same meaning and is set to the same value as the lfeon field in the AC-3 bit stream.

## F.4.2.7 bit\_rate\_code - 5 bits

The 5-bit bit\_rate\_code field shall indicate the data rate of the AC-3 bit stream in kbit/s, as shown in Table F.4.1. The value of this field shall be derived from the value of the frmsizcod parameter (see Table 4.13).

**Table F.4.1: bit\_rate\_code** 

| Bit_rate_code | Nominal bit rate (kbit/s) |
|---------------|---------------------------|
| 00000         | 32                        |
| 00001         | 40                        |
| 00010         | 48                        |
| 00011         | 56                        |
| 00100         | 64                        |
| 00101         | 80                        |
| 00110         | 96                        |
| 00111         | 112                       |
| 01000         | 128                       |
| 01001         | 160                       |
| 01010         | 192                       |
| 01011         | 224                       |
| 01100         | 256                       |
| 01101         | 320                       |
| 01110         | 384                       |
| 01111         | 448                       |
| 10000         | 512                       |
| 10001         | 576                       |
| 10010         | 640                       |

#### F.4.2.8 reserved - 5 bits

These bits are reserved, and shall be set to 0.

## F.5 EC3SampleEntry Box

## F.5.1 Syntax

| Syntax<br>No. of bits   | Identifier | Value |
|-------------------------|------------|-------|
| EC3SampleEntry()        |            |       |
| {                       |            |       |
| BoxHeader.Size 32       | uimsbf     |       |
| BoxHeader.Type 32       | uimsbf     |       |
| Reserved [6] 8          | uimsbf     | 0     |
| Data-reference-index 16 | uimsbf     |       |
| Reserved [2] 32         | uimsbf     | 0     |
| ChannelCount 16         | uimsbf     | 2     |
| SampleSize 16           | uimsbf     | 16    |
| Reserved 32             | uimsbf     | 0     |
| SamplingRate 16         | uimsbf     |       |
| Reserved 16             | uimsbf     | 0     |
| EC3SpecificBox          |            |       |
| }                       |            |       |

## F.5.2 Semantics

The layout of the EC3SampleEntry box is identical to that of AudioSampleEntry defined in ISO/IEC 14496-12 [i.9] (including the reserved fields and their values), except that EC3SampleEntry ends with a box containing Enhanced AC-3 bit stream information called EC3SpecificBox. The EC3SpecificBox field structure for Enhanced AC-3 is defined in clause F.6.

For unencrypted tracks, the value of the BoxHeader.Type field shall be set to 0x65632d33 ('ec-3'). For encrypted tracks, the value should be set according to the encryption scheme.

The values of the ChannelCount and SampleSize fields within the EC3SampleEntry Box shall be ignored.

# F.6 EC3SpecificBox

## F.6.1 Syntax

```
Syntax No. of bits Identifier
EC3SpecificBox() 
{ 
  BoxHeader.Size .............................................................. 32 uimsbf 
  BoxHeader.Type ............................................................. 32 uimsbf 
  data_rate .................................................................. 13 uimsbf 
 num_ind_sub .................................................................. 3 uimsbf 
 for(i = 0; i < num_ind_sub + 1; i++) 
 { 
 fscod ..................................................................... 2 uimsbf 
 bsid ...................................................................... 5 uimsbf 
 reserved .................................................................. 1 bslbf 
 asvc ...................................................................... 1 bslbf 
 bsmod ..................................................................... 3 uimsbf 
 acmod ..................................................................... 3 uimsbf 
 lfeon ..................................................................... 1 bslbf 
 reserved .................................................................. 3 uimbsf 
 num_dep_sub ............................................................... 4 uimsbf 
 if num_dep_sub > 0 
 { 
 chan_loc ............................................................... 9 uimsbf
```

| Syntax            | No. of bits | Identifier |
|-------------------|-------------|------------|
| }                 |             |            |
| else              |             |            |
| {<br>reserved 1   |             | bslbf      |
| }                 |             |            |
| }                 |             |            |
| reserved variable |             | bslbf      |
| }                 |             |            |

## F.6.2 Semantics

#### F.6.2.1 BoxHeader.Type - 32 bits

The value of the 32-bit BoxHeader.Type field shall be set to 0x64656333 ('dec3').

### F.6.2.2 data\_rate - 13 bits

The 13-bit data\_rate field indicates the data rate (in kbps) of the entire bitstream. The value is the sum of the data rates of all the substreams. When a bitstream uses variable data-rate encoding, data\_rate indicates the maximum data rate of the bitstream.

The data rate of each substream is calculated using this equation:

$$data\_rate\_sub = \frac{(frmsiz+1)*fs}{numblks*16}$$

In this equation:

- frmsiz is the value of the frmsiz field in the frame as defined in clause E.1.3.1.3.
- fs is the sample frequency of the bitstream (in kHz) as defined in clause E.1.3.1.4. (The fs value is derived from the fscod parameter in the frame.)
- numblks is the number of audio blocks per frame as defined in clause E.1.3.1.5. (The numblks value is derived from the numblkscod parameter in the frame.)

## F.6.2.3 num\_ind\_sub - 3 bits

The 3-bit num\_ind\_sub field shall indicate the number of independent substreams that are present in the Enhanced AC-3 bit stream. The value of this field shall be equal to the substreamID value of the last independent substream of the bit stream.

NOTE: This is the frame with a strmtyp value of 0 that precedes the frame with both a strmtyp value of 0 and a substreamid value of 0 (indicating that this frame belongs to the first independent substream of the bitstream).

#### F.6.2.4 fscod - 2 bits

The 2-bit fscod field shall have the same meaning and shall be set to the same value as the fscod field in the independent substream.

#### F.6.2.5 bsid - 5 bits

The 5-bit bsid field shall have the same meaning and shall be set to the same value as the bsid field in the independent substream.

#### F.6.2.6 reserved - 1 bit

This bit is reserved, and shall be set to 0.

#### F.6.2.7 asvc - 1 bit

The asvc field is used to signal whether the audio service carried by the independent substream (and dependent substreams associated with the independent substream, if present) is a main audio service intended to be presented on its own, or is an associated audio service that is intended to be decoded and mixed with a main audio service prior to presentation. The asvc parameter shall be set to '1' if the audio service is an associated audio service, and is set to '0' if the audio service is a main audio service.

Decoding devices that are not capable of simultaneously decoding two independent substreams and mixing the decoded audio together shall ignore independent substreams with a substreamID value greater than 0.

If the asvc parameter is set to '1' for independent substream 0, then the audio service carried in independent substream 0 is intended to be mixed with a main audio service carried in a separate Enhanced AC-3 bit stream stored in a different track in the ISO base media file. Decoding devices that are not capable of simultaneously decoding two Enhanced AC-3 bit streams and mixing the decoded audio together shall ignore Enhanced AC-3 audio tracks that have the asvc parameter set to '1' for independent substream 0.

### F.6.2.8 bsmod - 3 bits

The 3-bit bsmod field has the same meaning and is set to the same value as the bsmod field in the independent substream. If the bsmod field is not present in the independent substream (i.e. when the infomdate field in the independent substream is set to '0'), this field shall be set to 0.

#### F.6.2.9 acmod - 3 bits

The 3-bit acmod field shall have the same meaning and shall be set to the same value as the acmod field in the independent substream.

#### F.6.2.10 lfeon - 1 bit

The 1-bit lfeon field shall have the same meaning and shall be set to the same value as the lfeon field in the independent substream.

#### F.6.2.11 reserved - 3 bits

These bits are reserved, and shall be set to 0.

## F.6.2.12 num\_dep\_sub - 4 bits

The 4-bit num\_dep\_sub field shall be set to the value of the substreamid field found in the frame with a strmtyp value of 1 (that is, in the dependent substream) immediately preceding a frame with a strmtyp value of 0 (that is, in the independent substream).

### F.6.2.13 chan\_loc - 9 bits

The chan\_loc field indicates channel locations (beyond the standard 5.1 channels) that are carried by dependent substreams associated with an independent substream. The contents of the chan\_loc field are determined by parsing the chanmap bit field in every dependent substream associated with a particular independent substream, and setting the corresponding channel locations in the chan\_loc field to a value of 1.

Because this field is used by the system only to indicate the unique channel locations present in the bitstream, it is not necessary to reflect replacement channels in this field. Therefore, duplicate channel locations in the chanmap field indicate replacement channels and can be ignored.