## **ISO/IEC 14496-12:2015(E)**

### **8.16.5.2 Syntax**

```
aligned(8) class ProducerReferenceTimeBox extends FullBox('prft', version, 0) { 
 unsigned int(32) reference_track_ID; 
 unsigned int(64) ntp_timestamp; 
 if (version==0) { 
 unsigned int(32) media_time; 
 } else { 
 unsigned int(64) media_time; 
 } 
}
```

### **8.16.5.3 Semantics**

reference\_track\_ID provides the track\_ID for the reference track. ntp\_timestamp indicates a UTC time in NTP format corresponding to decoding\_time. media\_time corresponds to the same time as ntp\_timestamp, but in the time units used for the reference track, and is measured on this media clock as the media is produced. 

NOTE in most cases this timestamp will not be equal to the timestamp of the first sample of the adjacent segment of the reference track, but it is recommended it be in the range of the segment containing this producer reference time box. 

# **8.17 Support for Incomplete Tracks**

### **8.17.1 General**

This Subclause documents the sample entry formats for tracks that are incomplete. Incomplete tracks may contain samples that are marked empty or not received using the sample format. 

Incomplete tracks may result, for example, when subsegments are received partially according to level assignments and padding\_flag in the Level Assignment box indicates that the data in a Media Data box that is not received can be replaced by zeros. Consequently, sample data assigned to non‐accessed levels is not present, and care should be taken not to attempt to process such samples. However, in partially received subsegments some tracks might remain complete in content while other tracks might be incomplete and only contain data that is included by reference into the complete tracks. 

This Subclause specifies support for sample entry formats for incomplete tracks. With this support, readers can detect incomplete tracks from their sample entries and avoid processing such tracks or take the possibility of empty or not received samples into account when processing such tracks.

The support for incomplete tracks is similar to the content protection transformation where sample entries are hidden behind generic sample entries, such as 'encv' and 'enca'. Because the format of a sample entry varies with media‐type, a different encapsulating four‐character‐code is used for incomplete tracks of each media type (audio, video, text etc.). They are: 

| Stream (Track) Type | Sample-Entry Code |
|---------------------|-------------------|
| Video               | icpv              |
| Audio               | icpa              |
| Text                | icpt              |
| System              | icps              |
| Hint                | icph              |
| Timed	Metadata      | icpm              |

Sample data of incomplete tracks may be included into samples of other tracks by reference, and hence an incomplete track should not be removed as long as any track reference points to it. 

NOTE – The choice of level by the original recording client may vary over time, and at times represent the complete track. The level is not indicated here, and it is not required that the sample entry change from 'incomplete' to 'complete' when all levels were, in fact, received, for a period. Note also that the 'original format' may have indicated encryption, if partial reception and decryption works for that encryption format. 

### **8.17.2 Transformation**

The sample entry for a track that becomes incomplete e.g. through partial reception, should be modified as follows: 

- 1) The four‐character‐code of the sample entry, e.g. 'avc1', is replaced by a new sample entry code 'icpv' meaning an incomplete track.
- 2) A Complete Track Information box is added to the sample description, leaving all other boxes unmodified.
- 3) The original sample entry type, e.g. 'avc1', is stored within an Original Format box contained in the Complete Track Information box.

After transformation, an example AVC sample entry might look like: 

```
class IncompleteAVCSampleEntry() extends VisualSampleEntry ('icpv'){ 
 CompleteTrackInfoBox(); 
 AVCConfigurationBox config; 
 MPEG4BitRateBox (); // optional 
 MPEG4ExtensionDescriptorsBox (); // optional 
}
```