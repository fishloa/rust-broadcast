**Table E.1: Timescale for Media Header Box** 

| base_samp_freq<br>[kHz] | frame_rate_index     | frame_rate<br>[fps] | Media time<br>scale<br>[1/sec] | sample_delta<br>[units of media time<br>scale] |
|-------------------------|----------------------|---------------------|--------------------------------|------------------------------------------------|
|                         |                      | 0 23,976            | 48 000                         | 2 002                                          |
|                         |                      | 1 24                | 48 000                         | 2 000                                          |
|                         |                      | 2 25                | 48 000                         | 1 920                                          |
|                         |                      |                     | 240 000                        | 8 008                                          |
|                         | 3 29,97 (see note 1) |                     | 48 000                         | 1 601, 1 602 (see note 2)                      |
|                         |                      | 4 30                | 48 000                         | 1 600                                          |
|                         |                      | 5 47,95             | 48 000                         | 1 001                                          |
|                         |                      | 6 48                | 48 000                         | 1 000                                          |
|                         |                      | 7 50                | 48 000                         | 960                                            |
| 48                      |                      | 8 59,94             | 240 000                        | 4 004                                          |
|                         |                      | 9 60                | 48 000                         | 800                                            |
|                         |                      | 10 100              | 48 000                         | 480                                            |
|                         |                      | 11 119,88           | 240 000                        | 2 002                                          |
|                         |                      | 12 120              | 48 000                         | 400                                            |
|                         |                      | 13 (23,44)          | 48 000                         | 2 048                                          |
|                         |                      | 14 Reserved         |                                |                                                |
|                         |                      | 15 Reserved         |                                |                                                |
| 44,1                    |                      | 0…12 Reserved       |                                |                                                |
|                         |                      | 13 (21,53)          | 44 100                         | 2 048                                          |
|                         |                      | 14, 15 Reserved     |                                |                                                |

NOTE 1: There are two possible choices for the media time scale.

NOTE 2: The sample\_delta value is non-constant and it changes between the two specified values.

# E.3 AC-4 sample definition

For the purpose of carrying AC-4 in the ISO base media file format, an AC-4 sample corresponds to one raw\_ac4\_frame(), as defined in ETSI TS 103 190-1 [1], clause 4.2.1.

Sync samples are defined as samples that have the b\_iframe\_global flag set in the ac4\_toc(). AC-4 sync samples are marked as specified in clause E.2. Each AC-4 sync sample is a random access point and stream access point.

The first sample of an AC-4 segment or fragment shall be an AC-4 sync sample.

NOTE: A segment or fragment may contain multiple sync samples.

## E.4 AC4SampleEntry Box

Definition

**Table E.2: Definition of AC4SampleEntry** 

| 'ac-4'                          |
|---------------------------------|
| Sample Description Box ('stsd') |
| Yes                             |
| Exactly one                     |
|                                 |

The AC4SampleEntry shall contain an AC4SpecificBox, as defined in clause E.5.

The following optional boxes inherited from AudioSampleEntry from ISO/IEC 14496-12 [3] should not be present:

- DownMixInstructions()
- DRCCoefficientsBasic()
- DRCInstructionsBasic()
- DRCCoefficientsUniDRC()
- DRCInstructionsUniDRC()

#### Syntax

#### **Pseudocode E.1: Syntax of AC4SampleEntry()**

```
 aligned(8) class AC4SampleEntry extends AudioSampleEntry('ac-4') { 
 AC4SpecificBox(); 
 // we permit any number of AC4PresentationLabel boxes:
 AC4PresentationLabelBox() []; 
 Box () []; // further boxes as needed
 }
```

#### Semantics

The layout of the AC4SampleEntry box is identical to that of AudioSampleEntry defined in ISO/IEC 14496-12 [3], clause 12.2.3 (including the reserved fields and their values), except that AC4SampleEntry additionally contains AC-4 specific boxes at the end, i.e. AC-4 bitstream information called AC4SpecificBox and, optionally, one or more AC4PresentationLabelBox. The AC4SpecificBox field structure for AC-4 is defined in clause E.5 and the AC4PresentationLabelBox field structure is defined in clause E.5a.

Additional AC-4 specific requirements on the elements in the AudioSampleEntry are provided in table E.3.

**Table E.3: AC-4 specific requirements on the elements in AC4SampleEntry box** 

| Element                                                                                       | Data Type           | On decoding, the<br>value indicates | On encoding, the value                                                                                             |
|-----------------------------------------------------------------------------------------------|---------------------|-------------------------------------|--------------------------------------------------------------------------------------------------------------------|
| Box.size                                                                                      | unsigned<br>int(32) | the size of the<br>sampleEntry box  | See note                                                                                                           |
| Box.type                                                                                      | unsigned<br>int(32) | N/A                                 | shall be set to 0x61632D34 ('ac-4')                                                                                |
| SampleEntry.data_reference_index                                                              | unsigned<br>int(16) | N/A                                 | See note                                                                                                           |
| AudioSampleEntry.channelcount                                                                 | unsigned<br>int(16) | shall be ignored                    | should be set to the total number of audio<br>output channels of the first presentation of<br>that track; see NOTE |
| AudioSampleEntry.samplesize                                                                   | unsigned<br>int(16) | N/A                                 | shall be set to 16                                                                                                 |
| AudioSampleEntry.samplingrate                                                                 | unsigned<br>int(32) | shall be ignored                    | should be set to the base sampling<br>frequency of the track, as indicated by<br>ac4_toc/fs_index; see note        |
| NOTE:<br>set according to the Sample Entry definition in ISO/IEC 14496-12 [3], clause 12.2.3. |                     |                                     |                                                                                                                    |

## E.5 AC4SpecificBox

#### Definition

**Table E.4: Definition of AC4SpecificBox()** 

| Box Type:  | 'dac4'                      |
|------------|-----------------------------|
| Container: | AC4SampleEntry Box ('ac-4') |
| Mandatory: | Yes                         |
| Quantity:  | Exactly one                 |

The AC4SpecificBox shall contain an ac4\_dsi\_v1() data structure as specified in clause E.6.1.

#### Syntax

#### **Pseudocode E.2: Syntax of AC4SpecificBox()**

```
aligned(8) class AC4SpecificBox extends Box('dac4') { 
 bit(8)[] ac4_dsi_v1(); // to end of the box
}
```

#### Semantics

**Table E.5: Element description for AC4SpecificBox** 

| Element      | Data Type        | On decoding, the value indicates | On encoding, the value                       |
|--------------|------------------|----------------------------------|----------------------------------------------|
| Box.size     | unsigned int(32) | the size of the box              | shall be specified by [3]                    |
| Box.type     | unsigned int(32) | N/A                              | shall contain the value 0x64616334 ('dac4'). |
| ac4_dsi_v1() | bit(8)[]         | see clause E.6.1                 | see clause E.6.1                             |

## E.5a AC4 Presentation Label Box

#### Definition

**Table E.5a: Definition of AC4PresentationLabelBox()** 

| Box Type:  | 'lac4'                      |
|------------|-----------------------------|
| Container: | AC4SampleEntry Box ('ac-4') |
| Mandatory: | No                          |
| Quantity:  | Zero or more                |

The AC-4 Presentation Label Box provides labels that can be used in a user interface to guide user-driven presentation selection.

The AC4PresentationLabelBox may occur zero or more times after AC4SampleEntryBox/AC4SpecificBox. If there are multiple occurrences of the AC-4 Presentation Label Box, then they shall each have a different language\_tag.

#### Syntax

#### **Pseudocode E.2a: Syntax of AC4PresentationLabelBox()**

```
aligned(8) class AC4PresentationLabelBox extends FullBox('lac4', version = 0, 0) { 
 unsigned int(16) num_presentation_labels; 
 utf8string language_tag; 
 for (i=0; i < num_presentation_labels; i++) { 
 unsigned int(16) presentation_id; 
 utf8string presentation_label; 
 } 
}
```

#### Semantics

**Table E.5b: Element Description for AC4PresentationLabelBox** 

| Element                                                                                      | Data Type                                                 | On decoding, the value indicates                                                                                                                                            | On encoding, the value                                                                                                                                                                                                        |
|----------------------------------------------------------------------------------------------|-----------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Box.size                                                                                     | unsigned                                                  | the size of the                                                                                                                                                             | See note 1                                                                                                                                                                                                                    |
|                                                                                              | int(32)                                                   | AC4PresentationLabel box                                                                                                                                                    |                                                                                                                                                                                                                               |
| Box.type                                                                                     | unsigned<br>int(32)                                       | N/A                                                                                                                                                                         | shall be set to 0x6C616334 ('lac4')                                                                                                                                                                                           |
| FullBox.version                                                                              | unsigned<br>int(8)                                        | N/A                                                                                                                                                                         | shall be set to 0                                                                                                                                                                                                             |
| FullBox.flags                                                                                | bit(24)                                                   | N/A                                                                                                                                                                         | shall be set to 0                                                                                                                                                                                                             |
| num_presentation_labels                                                                      | unsigned<br>int(16)                                       | the number of presentation labels<br>contained in the<br>AC4PresentationLabelBox                                                                                            | shall be set to the number of<br>presentation labels that are included<br>in this box.                                                                                                                                        |
| language_tag                                                                                 | null<br>terminated<br>string using<br>UTF-8<br>characters | the language of all labels in the<br>containing<br>AC4PresentationLabelBox. For<br>different display languages,<br>multiple AC-4 Presentation Label<br>Boxes shall be used. | shall be specified as in IETF<br>BCP 47 [7], encoded as a null<br>terminated UTF-8 string. If no<br>language tag is provided, the<br>terminating null-byte shall be written.                                                  |
| presentation_id                                                                              | unsigned<br>int(16)                                       | indicates the matching<br>presentation in ac4_dsi_v1() for a<br>label.<br>See note 2                                                                                        | shall be set to the presentation_id<br>or extended_presentation_id of the<br>presentation in the ac4_toc() that the<br>following presentation_label<br>corresponds to.                                                        |
| presentation_label                                                                           | null<br>terminated<br>string using<br>UTF-8<br>characters | a textual label for a presentation                                                                                                                                          | shall be stored as a null-terminated<br>UTF-8-encoded string containing a<br>textual label for the matching<br>presentation. On writing, the string<br>shall be terminated with a null-byte,<br>even when the label is empty. |
| NOTE 1: set according to the Sample Entry definition in ISO/IEC 14496-12 [3], clause 12.2.3. |                                                           |                                                                                                                                                                             |                                                                                                                                                                                                                               |
| NOTE 2: The value does not exceed 511 as it is limited by the maximum possible number in     |                                                           |                                                                                                                                                                             |                                                                                                                                                                                                                               |
| ac4_presentation_v1_dsi/extended_presentation_id.                                            |                                                           |                                                                                                                                                                             |                                                                                                                                                                                                                               |

See also clause E.10.2.

## E.6 ac4\_dsi\_v1

### E.6.1 Syntax and Semantics

The ac4\_dsi\_v1() structure summarizes the content of all samples referenced by Ac4SampleEntry containing the DSI, with elements aligned and sized such that parsing the information involves less bit operations. This information may be used to populate manifest files.

On decoding, if the ac4\_dsi\_v1() structure is available to the decoder, it shall be used for presentation selection. In this case, selection criteria shall be only applied to the information in the ac4\_dsi\_v1() and its substructures.

On encoding, there are certain constraints placed on the values in the ac4\_dsi\_v1() and its substructures, as further detailed in the rest of the present Annex.

Inside the ac4\_dsi\_v1() structure, presentations are represented in an array of ac4\_presentation\_v1\_dsi() elements. The number and order of presentations in the ac4\_dsi\_v1() structure need not be the same as in the ac4\_toc(); in fact, both structures may contain a different number of presentations. Therefore, to identify a presentation in the ac4\_toc(), a decoder shall match a presentation selected through its ac4\_presentation\_v1\_dsi() element to a presentation in the ac4\_toc() as specified in table E.11; the decoder shall decode the matching presentation.

NOTE 1: In the following, the "matching presentation" is the presentation contained in the ac4\_toc() that matches as specified in presentation\_id. See table E.11.

On encoding, each ac4\_presentation\_v1\_dsi() should match one presentation.

NOTE 2: Therefore, entries in the ac4\_dsi\_v1() element apply to all samples that reference the Ac4SampleEntry containing the DSI. No configuration change can occur inside an Ac4SampleEntry.

```
Syntax No of bits
ac4_dsi_v1() 
{ 
 ac4_dsi_version; ............................................................................. 3 
 bitstream_version; ........................................................................... 7 
 fs_index; .................................................................................... 1 
 frame_rate_index; ............................................................................ 4 
 n_presentations; ............................................................................. 9 
 if (bitstream_version > 1) { 
 b_program_id; .............................................................................. 1 
 if (b_program_id) { 
 short_program_id; ....................................................................... 16 
 b_uuid; .................................................................................. 1 
 if (b_uuid) { 
 program_uuid; ....................................................................... 16*8 
 } 
 } 
 } 
 ac4_bitrate_dsi(); 
 byte_align; ................................................................................ 0…7 
 for (i = 0; i < n_presentations; i++) { 
 presentation_version; ...................................................................... 8 
 pres_bytes; ................................................................................ 8 
 if (pres_bytes == 255) { 
 add_pres_bytes; ......................................................................... 16 
 pres_bytes += add_pres_bytes; 
 } 
 if (presentation_version == 0) { 
 presentation_bytes = ac4_presentation_v0_dsi(); 
 } 
 else { 
 if (presentation_version == 1) { 
 presentation_bytes = ac4_presentation_v1_dsi(pres_bytes); 
 } 
 else { 
 presentation_bytes = 0; 
 } 
 } 
 skip_bytes = pres_bytes - presentation_bytes; 
 skip_area; ...................................................................... skip_bytes*8 
 } 
}
```

NOTE: The number of bits in byte\_align pads the number of bits, counted from the start of ac4\_dsi\_v1 to a multiple of 8.