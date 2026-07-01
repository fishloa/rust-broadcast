## **8.7.7.2 Syntax**

```
aligned(8) class SubSampleInformationBox 
 extends FullBox('subs', version, flags) { 
 unsigned int(32) entry_count; 
 int i,j; 
 for (i=0; i < entry_count; i++) { 
 unsigned int(32) sample_delta; 
 unsigned int(16) subsample_count; 
 if (subsample_count > 0) { 
 for (j=0; j < subsample_count; j++) { 
 if(version == 1) 
 { 
 unsigned int(32) subsample_size; 
 } 
 else 
 { 
 unsigned int(16) subsample_size; 
 } 
 unsigned int(8) subsample_priority; 
 unsigned int(8) discardable; 
 unsigned int(32) codec_specific_parameters; 
 } 
 } 
 } 
}
```

### **8.7.7.3 Semantics**

version is an integer that specifies the version of this box (0 or 1 in this specification) entry\_count is an integer that gives the number of entries in the following table. 

sample\_delta is an integer that specifies the sample number of the sample having sub‐sample structure. It is coded as the difference between the desired sample number, and the sample number indicated in the previous entry. If the current entry is the first entry, the value indicates the sample number of the first sample having sub‐sample information, that is, the value is the difference between the sample number and zero (0). 

subsample\_count is an integer that specifies the number of sub‐sample for the current sample. If there is no sub‐sample structure, then this field takes the value 0. 

subsample\_size is an integer that specifies the size, in bytes, of the current sub‐sample. 

subsample\_priority is an integer specifying the degradation priority for each sub‐sample. Higher values of subsample\_priority, indicate sub‐samples which are important to, and have a greater impact on, the decoded quality. 

discardable equal to 0 means that the sub‐sample is required to decode the current sample, while equal to 1 means the sub‐sample is not required to decode the current sample but may be used for enhancements, e.g., the sub‐sample consists of supplemental enhancement information (SEI) messages. 

codec\_specific\_parameters is defined by the codec in use. If no such definition is available, this field shall be set to 0. 

### **8.7.8 Sample Auxiliary Information Sizes Box**

### **8.7.8.1 Definition**

Box Type: 'saiz'

Container: Sample Table Box ('stbl') or Track Fragment Box ('traf') 

Mandatory: No 

Quantity: Zero or More 

Per‐sample sample auxiliary information may be stored anywhere in the same file as the sample data itself; for self‐contained media files, this is typically in a MediaData box or a box from a derived

# **ISO/IEC 14496-12:2015(E)**

specification. It is stored either (a) in multiple chunks, with the number of samples per chunk, as well as the number of chunks, matching the chunking of the primary sample data or (b) in a single chunk for all the samples in a movie sample table (or a movie fragment). The Sample Auxiliary Information for all samples contained within a single chunk (or track run) is stored contiguously (similarly to sample data). 

Sample Auxiliary Information, when present, is always stored in the same file as the samples to which it relates as they share the same data reference ('dref') structure. However, this data may be located anywhere within this file, using auxiliary information offsets ('saio') to indicate the location of the data. 

Whether sample auxiliary information is permitted or required may be specified by the brands or the coding format in use. The format of the sample auxiliary information is determined by aux\_info\_type. If aux\_info\_type and aux\_info\_type\_parameter are omitted then the implied value of aux\_info\_type is either (a) in the case of transformed content, such as protected content, the scheme\_type included in the Protection Scheme Information box or otherwise (b) the sample entry type. The default value of the aux\_info\_type\_parameter is 0. Some values of aux\_info\_type may be restricted to be used only with particular track types. A track may have multiple streams of sample auxiliary information of different types. The types are registered at the registration authority. 

While aux\_info\_type determines the format of the auxiliary information, several streams of auxiliary information having the same format may be used when their value of aux\_info\_type\_parameter differs. The semantics of aux\_info\_type\_parameter for a particular aux\_info\_type value must be specified along with specifying the semantics of the particular aux\_info\_type value and the implied auxiliary information format. 

This box provides the size of the auxiliary information for each sample. For each instance of this box, there must be a matching SampleAuxiliaryInformationOffsetsBox with the same values of aux\_info\_type and aux\_info\_type\_parameter, providing the offset information for this auxiliary information. 

NOTE For discussions on the use of sample auxiliary information versus other mechanisms, see Annex C.8. 

# **8.7.8.2 Syntax**

```
aligned(8) class SampleAuxiliaryInformationSizesBox 
 extends FullBox('saiz', version = 0, flags) 
{ 
 if (flags & 1) { 
 unsigned int(32) aux_info_type; 
 unsigned int(32) aux_info_type_parameter; 
 } 
 unsigned int(8) default_sample_info_size; 
 unsigned int(32) sample_count; 
 if (default_sample_info_size == 0) { 
 unsigned int(8) sample_info_size[ sample_count ]; 
 } 
}
```

## **8.7.8.3 Semantics**

- aux\_info\_type is an integer that identifies the type of the sample auxiliary information. At most one occurrence of this box with the same values for aux\_info\_type and aux\_info\_type\_parameter shall exist in the containing box.
- aux\_info\_type\_parameter identifies the "stream" of auxiliary information having the same value of aux\_info\_type and associated to the same track. The semantics of aux\_info\_type\_parameter are determined by the value of aux\_info\_type.
- default\_sample\_info\_size is an integer specifying the sample auxiliary information size for the case where all the indicated samples have the same sample auxiliary information size. If the size varies then this field shall be zero.
- sample\_count is an integer that gives the number of samples for which a size is defined. For a Sample Auxiliary Information Sizes box appearing in the Sample Table Box this must be the same as, or less than, the sample\_count within the Sample Size Box or Compact Sample Size Box. For a Sample Auxiliary Information Sizes box appearing in a Track Fragment box this must be the same as, or less than, the sum of the sample\_count entries within the Track Fragment Run boxes of the Track Fragment. If this is less than the number of samples, then auxiliary information is supplied for the initial samples, and the remaining samples have no associated auxiliary information.
- sample\_info\_size gives the size of the sample auxiliary information in bytes. This may be zero to indicate samples with no associated auxiliary information.

# **8.7.9 Sample Auxiliary Information Offsets Box**

## **8.7.9.1 Definition**

Box Type: 'saio' Container: Sample Table Box ('stbl') or Track Fragment Box ('traf') Mandatory: No Quantity: Zero or More 

For an introduction to sample auxiliary information, see the definition of the Sample Auxiliary Information Size Box. 

This box provides the position information for the sample auxiliary information, in a way similar to the chunk offsets for sample data. 

# **8.7.9.2 Syntax**

```
aligned(8) class SampleAuxiliaryInformationOffsetsBox 
 extends FullBox('saio', version, flags) 
{ 
 if (flags & 1) { 
 unsigned int(32) aux_info_type; 
 unsigned int(32) aux_info_type_parameter; 
 } 
 unsigned int(32) entry_count; 
 if ( version == 0 ) { 
 unsigned int(32) offset[ entry_count ]; 
 } 
 else { 
 unsigned int(64) offset[ entry_count ]; 
 } 
}
```

## **8.9.2 Sample to Group Box**

### **8.9.2.1 Definition**

```
Box	Type:	 'sbgp'
Container:	 Sample	Table	Box	('stbl')	or	Track	Fragment	Box	('traf')		
Mandatory:	No	
Quantity:	 Zero	or	more.
```

This table can be used to find the group that a sample belongs to and the associated description of that sample group. The table is compactly coded with each entry giving the index of the first sample of a run of samples with the same sample group descriptor. The sample group description ID is an index that refers to a SampleGroupDescription box, which contains entries describing the characteristics of each sample group. 

There may be multiple instances of this box if there is more than one sample grouping for the samples in a track. Each instance of the SampleToGroup box has a type code that distinguishes different sample groupings. There shall be at most one instance of this box with a particular grouping type in a Sample Table Box or Track Fragment Box. The associated SampleGroupDescription shall indicate the same value for the grouping type. 

Version 1 of this box should only be used if a grouping type parameter is needed. 

## **8.9.2.2 Syntax**

```
aligned(8) class SampleToGroupBox 
 extends FullBox('sbgp', version, 0) 
{ 
 unsigned int(32) grouping_type; 
 if (version == 1) { 
 unsigned int(32) grouping_type_parameter; 
 } 
 unsigned int(32) entry_count; 
 for (i=1; i <= entry_count; i++) 
 { 
 unsigned int(32) sample_count; 
 unsigned int(32) group_description_index; 
 } 
}
```

### **8.9.2.3 Semantics**

version is an integer that specifies the version of this box, either 0 or 1. 

grouping\_type is an integer that identifies the type (i.e. criterion used to form the sample groups) of the sample grouping and links it to its sample group description table with the same value for grouping type. At most one occurrence of this box with the same value for grouping\_type (and, if used, grouping\_type\_parameter) shall exist for a track. 

grouping\_type\_parameter is an indication of the sub‐type of the grouping entry\_count is an integer that gives the number of entries in the following table. 

sample\_count is an integer that gives the number of consecutive samples with the same sample group descriptor. If the sum of the sample count in this box is less than the total sample count, or there is no sample‐to‐group box that applies to some samples (e.g. it is absent from a track fragment), then the reader should associates the samples that have no explicit group association with the default group defined in the SampleDescriptionGroup box, if any, or else with no group. It is an error for the total in this box to be greater than the sample\_count documented elsewhere, and the reader behaviour would then be undefined.

group\_description\_index is an integer that gives the index of the sample group entry which describes the samples in this group. The index ranges from 1 to the number of sample group entries in the SampleGroupDescription Box, or takes the value 0 to indicate that this sample is a member of no group of this type. 

### **8.9.3 Sample Group Description Box**

### **8.9.3.1 Definition**

Box Type: 'sgpd'

Container: Sample Table Box ('stbl') or Track Fragment Box ('traf') 

Mandatory: No 

Quantity: Zero or more, with one for each Sample to Group Box. 

This description table gives information about the characteristics of sample groups. The descriptive information is any other information needed to define or characterize the sample group. 

There may be multiple instances of this box if there is more than one sample grouping for the samples in a track. Each instance of the SampleGroupDescription box has a type code that distinguishes different sample groupings. There shall be at most one instance of this box with a particular grouping type in a Sample Table Box or Track Fragment Box. The associated SampleToGroup shall indicate the same value for the grouping type. 

The information is stored in the sample group description box after the entry‐count. An abstract entry type is defined and sample groupings shall define derived types to represent the description of each sample group. For video tracks, an abstract VisualSampleGroupEntry is used with similar types for audio and hint tracks. 

NOTE In version 0 of the entries the base classes for sample group description entries are neither boxes nor have a size that is signaled. For this reason, use of version 0 entries is deprecated. When defining derived classes, ensure either that they have a fixed size, or that the size is explicitly indicated with a length field. An implied size (e.g. achieved by parsing the data) is not recommended as this makes scanning the array difficult. 

# **8.9.3.2 Syntax**

```
// Sequence Entry 
abstract class SampleGroupDescriptionEntry (unsigned int(32) grouping_type) 
{ 
} 
abstract class VisualSampleGroupEntry (unsigned int(32) grouping_type) extends 
SampleGroupDescriptionEntry (grouping_type) 
{ 
} 
abstract class AudioSampleGroupEntry (unsigned int(32) grouping_type) extends 
SampleGroupDescriptionEntry (grouping_type) 
{ 
} 
abstract class HintSampleGroupEntry (unsigned int(32) grouping_type) extends 
SampleGroupDescriptionEntry (grouping_type) 
{ 
}
```

# **ISO/IEC 14496-12:2015(E)**

```
abstract class SubtitleSampleGroupEntry (unsigned int(32) grouping_type) extends 
SampleGroupDescriptionEntry (grouping_type) 
{ 
} 
abstract class TextSampleGroupEntry (unsigned int(32) grouping_type) extends 
SampleGroupDescriptionEntry (grouping_type) 
{ 
} 
aligned(8) class SampleGroupDescriptionBox (unsigned int(32) handler_type) 
 extends FullBox('sgpd', version, 0){ 
 unsigned int(32) grouping_type; 
 if (version==1) { unsigned int(32) default_length; } 
 if (version>=2) { 
 unsigned int(32) default_sample_description_index; 
 } 
 unsigned int(32) entry_count; 
 int i; 
 for (i = 1 ; i <= entry_count ; i++){ 
 if (version==1) { 
 if (default_length==0) { 
 unsigned int(32) description_length; 
 } 
 } 
 SampleGroupEntry (grouping_type); 
 // an instance of a class derived from SampleGroupEntry 
 // that is appropriate and permitted for the media type 
 } 
}
```

### **8.9.3.3 Semantics**

version is an integer that specifies the version of this box. 

grouping\_type is an integer that identifies the SampleToGroup box that is associated with this sample group description. If grouping\_type\_parameter is not defined for a given grouping\_type, then there shall be only one occurrence of this box with this grouping\_type. 

default\_sample\_description\_index: specifies the index of the sample group description entry which applies to all samples in the track for which no sample to group mapping is provided through a SampleToGroup box. The default value of this field is zero (indicating that the samples are mapped to no group of this type).

entry\_count is an integer that gives the number of entries in the following table. 

default\_length indicates the length of every group entry (if the length is constant), or zero (0) if it is variable 

description\_length indicates the length of an individual group entry, in the case it varies from entry to entry and default\_length is therefore 0 

# **8.9.4 Representation of group structures in Movie Fragments**

Support for Sample Group structures within Movie fragments is provided by the use of the SampleToGroup Box with the container for this Box being the Track Fragment Box ('traf'). The definition, syntax and semantics of this Box is as specified in subclause 8.9.2. 

The SampleToGroup Box can be used to find the group that a sample in a track fragment belongs to and the associated description of that sample group. The table is compactly coded with each entry giving the index of the first sample of a run of samples with the same sample group descriptor. The sample group description ID is an index that refers to a SampleGroupDescription Box, which

contains entries describing the characteristics of each sample group and present in the SampleTableBox. 

There may be multiple instances of the SampleToGroup Box if there is more the one sample grouping for the samples in a track fragment. Each instance of the SampleToGroup Box has a type code that distinguishes different sample groupings. The associated SampleGroupDescription shall indicate the same value for the grouping type. 

The total number of samples represented in any SampleToGroup Box in the track fragment must match the total number of samples in all the track fragment runs. Each SampleToGroup Box documents a different grouping of the same samples. 

Zero or more SampleGroupDescription boxes may also be present in a Track Fragment Box. These definitions are additional to the definitions provided in the Sample Table of the track in the Movie Box. Group definitions within a movie fragment can also be referenced and used from within that same movie fragment. 

Within the SampleToGroup box in that movie fragment, the group description indexes for groups defined within the same fragment start at 0x10001, i.e. the index value 1, with the value 1 in the top 16 bits. This means there must be fewer than 65536 group definitions for this track and grouping type in the sample table in the Movie Box. 

When changing the size of movie fragments, or removing them, these fragment‐local group definitions will need to be merged into the definitions in the movie box, or into the new movie fragments, and the index numbers in the SampleToGroup box(es) adjusted accordingly. It is recommended that, in this process, identical (and hence duplicate) definitions not be made in any SampleGroupDescription box, but that duplicates be merged and the indexes adjusted accordingly. 

# **8.10 User Data**

## **8.10.1 User Data Box**

# **8.10.1.1 Definition**

Box Type: 'udta'

Container: Movie Box ('moov'), Track Box ('trak'), 

 Movie Fragment Box ('moof') or Track Fragment Box ('traf') 

Mandatory: No 

Quantity: Zero or one 

This box contains objects that declare user information about the containing box and its data (presentation or track). 

The User Data Box is a container box for informative user‐data. This user data is formatted as a set of boxes with more specific box types, which declare more precisely their content. 

The handling of user‐data in movie fragments is described in 8.8.17.

# **ISO/IEC 14496-12:2015(E)**

## **8.10.1.2 Syntax**

```
aligned(8) class UserDataBox extends Box('udta') { 
}
```

## **8.10.2 Copyright Box**

## **8.10.2.1 Definition**

```
Box	Type:	 'cprt'
Container:	 User	data	box	('udta')	
Mandatory:	 No
```

Quantity: Zero or more 

The Copyright box contains a copyright declaration which applies to the entire presentation, when contained within the Movie Box, or, when contained in a track, to that entire track. There may be multiple copyright boxes using different language codes. 

## **8.10.2.2 Syntax**

```
aligned(8) class CopyrightBox 
 extends FullBox('cprt', version = 0, 0) { 
 const bit(1) pad = 0; 
 unsigned int(5)[3] language; // ISO-639-2/T language code 
 string notice; 
}
```

### **8.10.2.3 Semantics**

language declares the language code for the following text. See ISO 639‐2/T for the set of three character codes. Each character is packed as the difference between its ASCII value and 0x60. The code is confined to being three lower‐case letters, so these values are strictly positive. 

notice is a null‐terminated string in either UTF‐8 or UTF‐16 characters, giving a copyright notice. If UTF‐16 is used, the string shall start with the BYTE ORDER MARK (0xFEFF), to distinguish it from a UTF‐8 string. This mark does not form part of the final string. 

# **8.10.3 Track Selection Box**

# **8.10.3.1 Introduction**

A typical presentation stored in a file contains one alternate group per media type: one for video, one for audio, etc. Such a file may include several video tracks, although, at any point in time, only one of them should be played or streamed. This is achieved by assigning all video tracks to the same alternate group. (See subclause 8.3.2 for the definition of alternate groups.) 

All tracks in an alternate group are candidates for media selection, but it may not make sense to switch between some of those tracks during a session. One may for instance allow switching between video tracks at different bitrates and keep frame size but not allow switching between tracks of different frame size. In the same manner it may be desirable to enable selection – but not switching – between tracks of different video codecs or different audio languages. 

The distinction between tracks for *selection* and *switching* is addressed by assigning tracks to switch groups in addition to alternate groups. One alternate group may contain one or more switch groups. All tracks in an alternate group are candidates for media selection, while tracks in a switch group are also