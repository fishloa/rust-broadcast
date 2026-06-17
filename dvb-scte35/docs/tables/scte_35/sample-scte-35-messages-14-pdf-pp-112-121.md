## Sample SCTE 35 Messages — §14, PDF pp. 112-121
_Informative. Transcribed verbatim from the vendored PDF via `pdftotext` (deterministic), 2026-06-13. Per the spec's §14 intro these decodes are "the output from the open source decoder available on github" (threefive). Each `Base64` string was machine-verified to decode to the stated byte length; the `Hex` shown is canonically re-derived from that base64 (identical to the spec's `Hex=` field for 14.1–14.7; for 14.8 the spec's printed hex line-wrapped, so the base64-derived hex is authoritative). These are the crate's known-vector test inputs._

### 14.1. time_signal – Placement Opportunity Start
This is an example of using the time_signal command with a segmentation descriptor. The programmer that created this message used the Web Delivery Allowed flag to indicate the broadcast Advertisements should be blacked out. This would tend to also indicate that digital ad insertion would be used to insert new Advertisements in their place. The programmer is also using the Segment number in a non-normative manner to indicate that there are Distributor Placement Opportunities within this Provider Placement Opportunity. A standardized method of doing this would be to use the MID format of the segmentation UPID type and insert a UPID type 0x0E (ADS) with this information. Also note the Tier value in this message is 0xfff as displayed on the output of a receiver. It is likely that the value of Tier in transmission had a different value that this receiver was authorized to receive, and the receiver obfuscated that by changing the value to 0xfff.

```text
2018-07-16 00:04:57 M274P29528596539
Hex=0xFC3034000000000000FFFFF00506FE72BD0050001E021C435545494800008E7FCF0001A599B00808000000002CA0A18A3402009AC9D17E
Base64=/DA0AAAAAAAA///wBQb+cr0AUAAeAhxDVUVJSAAAjn/PAAGlmbAICAAAAAAsoKGKNAIAmsnRfg==

Decoded length = 55
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 52
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x5
Time Signal
Time = 0x072bd0050 - 21388.766756
Descriptor Loop Length = 30
Segmentation Descriptor - Length=28
Segmentation Event ID = 0x4800008e
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 0
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
Segmentation Duration = 0x0001a599b0 = 307.000000 seconds
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca0a18a
Type = Placement Opportunity Start
Segment num = 2 Segments Expected = 0
CRC32 = 0x9ac9d17e
```

### 14.2. splice_insert
This is the legacy standard for a Distributor Placement Opportunity. As a significant number of existing ad servers will not respond to the newer time_signal command, it is likely that this message will be in use until the legacy components are removed and replaced. The programmer that generated this message uses the Break duration and auto return mode for the splice_insert. The Break duration at 60.293567 seconds is slightly longer than the contracted 60 second local avail; it is, however, the exact duration of the content that the local avail will overlay. This means that the encoder will generate a key frame at that specified Break duration from the splice time and the affiliate should fill the duration with a slate or black (in some countries a blue color is used). Some splicers or fragmented file delivery systems may be able to adjust the duration and boundaries to match the key frames as well.

```text
2018-07-16 00:06:59 M274P29540838841
Hex=0xFC302F000000000000FFFFF014054800008F7FEFFE7369C02EFE0052CCF500000000000A0008435545490000013562DBA30A
Base64=/DAvAAAAAAAA///wFAVIAACPf+/+c2nALv4AUsz1AAAAAAAKAAhDVUVJAAABNWLbowo=

Decoded length = 50
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 47
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x14
Splice Insert
Splice Event ID = 0x4800008f
Flags OON=1 Prog=1 Duration=1 Immediate=0
Splice time = 0x07369c02e - 21514.559089
Auto Return
break duration = 0x00052ccf5 = 60.293567 seconds
Unique Program ID = 0
Avail Num = 0
Avails Expected = 0
Descriptor Loop Length = 10
Avail Descriptor - Length=8
Avail Descriptor = 0x00000135 - 309
CRC32 = 0x62dba30a
```

### 14.3. time_signal – Placement Opportunity End

```text
2018-07-16 00:10:04 M274P29559224252
Hex=0xFC302F000000000000FFFFF00506FE746290A000190217435545494800008E7F9F0808000000002CA0A18A350200A9CC6758
Base64=/DAvAAAAAAAA///wBQb+dGKQoAAZAhdDVUVJSAAAjn+fCAgAAAAALKChijUCAKnMZ1g=

Decoded length = 50
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 47
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x5
Time Signal
Time = 0x0746290a0 - 21695.740089
Descriptor Loop Length = 25
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x4800008e
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca0a18a
Type = Placement Opportunity End
Segment num = 2 Segments Expected = 0
CRC32 = 0xa9cc6758
```

### 14.4. time_signal – Program Start/End

```text
2018-07-16 00:00:15 M274P29500484335
Hex=0xFC3048000000000000FFFFF00506FE7A4D88B60032021743554549480000187F9F0808000000002CCBC344110000021743554549480000197F9F0808000000002CA4DBA01000009972E343
Base64=/DBIAAAAAAAA///wBQb+ek2ItgAyAhdDVUVJSAAAGH+fCAgAAAAALMvDRBEAAAIXQ1VFSUgAABl/nwgIAAAAACyk26AQAACZcuND

Decoded length = 75
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 72
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x5
Time Signal
Time = 0x07a4d88b6 - 22798.906911
Descriptor Loop Length = 50
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x48000018
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ccbc344
Type = Program End
Segment num = 0 Segments Expected = 0
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x48000019
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca4dba0
Type = Program Start
Segment num = 0 Segments Expected = 0
CRC32 = 0x9972e343
```

### 14.5. time_signal – Program Overlap Start

```text
2018-07-16 02:59:52 M274P30575324060
Hex=0xFC302F000000000000FFFFF00506FEAEBFFF640019021743554549480000087F9F0808000000002CA56CF5170000951DB0A8
Base64=/DAvAAAAAAAA///wBQb+rr//ZAAZAhdDVUVJSAAACH+fCAgAAAAALKVs9RcAAJUdsKg=

Decoded length = 50
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 47
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x5
Time Signal
Time = 0x0aebfff64 - 32575.759333
Descriptor Loop Length = 25
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x48000008
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca56cf5
Type = Program Overlap Start
Segment num = 0 Segments Expected = 0
CRC32 = 0x951db0a8
```

### 14.6. time_signal – Program Blackout Override / Program End
Since the restriction flags are not evaluated on an End message, the use of the Program blackout override can be used in the case of an overlap start or other condition where the restrictions may need to be changed during a Program playback.

```text
2018-07-16 01:45:45 M274P30131806863
Hex=0xFC3048000000000000FFFFF00506FE932E380B00320217435545494800000A7F9F0808000000002CA0A1E3180000021743554549480000097F9F0808000000002CA0A18A110000B4217EB0
Base64=/DBIAAAAAAAA///wBQb+ky44CwAyAhdDVUVJSAAACn+fCAgAAAAALKCh4xgAAAIXQ1VFSUgAAAl/nwgIAAAAACygoYoRAAC0IX6w

Decoded length = 75
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 72
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x5
Time Signal
Time = 0x0932e380b - 27436.441722
Descriptor Loop Length = 50
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x4800000a
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca0a1e3
Type = Program Blackout Override
Segment num = 0 Segments Expected = 0
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x48000009
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca0a18a
Type = Program End
Segment num = 0 Segments Expected = 0
CRC32 = 0xb4217eb0
```

### 14.7. time_signal – Program End

```text
2018-07-16 03:00:28 M274P30578915636
Hex=0xFC302F000000000000FFFFF00506FEAEF17C4C0019021743554549480000077F9F0808000000002CA56C97110000C4876A2E
Base64=/DAvAAAAAAAA///wBQb+rvF8TAAZAhdDVUVJSAAAB3+fCAgAAAAALKVslxEAAMSHai4=

Decoded length = 50
Table ID = 0xFC
MPEG Short Section
Not Private
Reserved = 0x3
Section Length = 47
Protocol Version = 0
unencrypted Packet
PTS Adjustment = 0x000000000
Tier = 0xfff
Splice Command Length = 0x5
Time Signal
Time = 0x0aef17c4c - 32611.795333
Descriptor Loop Length = 25
Segmentation Descriptor - Length=23
Segmentation Event ID = 0x48000007
Segmentation Event Cancel Indicator NOT set
Delivery Not Restricted flag = 0
Web Delivery Allowed flag = 1
No Regional Blackout flag = 1
Archive Allowed flag = 1
Device Restrictions = 3
Program Segmentation flag SET
UPID Type = Turner Identifier length = 8
Turner Identifier = 0x000000002ca56c97
Type = Program End
Segment num = 0 Segments Expected = 0
CRC32 = 0xc4876a2e
```

### 14.8. time_signal – Program Start/End - Placement Opportunity End
This is a complex message, although one that can occur frequently as many ad Breaks are placed at the end of the Program. The implementer should take care though to find the length and current practice is to try and keep the message in a single transport packet.

```text
2018-07-16 03:00:33 M274P30579401569
Hex=0xFC3061000000000000FFFFF00506FEA8CD44ED004B021743554549480000AD7F9F0808000000002CB2D79D350200021743554549480000267F9F0808000000002CB2D79D110000021743554549480000277F9F0808000000002CB2D7B31000008A18869F
Base64=/DBhAAAAAAAA///wBQb+qM1E7QBLAhdDVUVJSAAArX+fCAgAAAAALLLXnTUCAAIXQ1VFSUgAACZ/nwgIAAAAACyy150RAAACF0NVRUlIAAAnf58ICAAAAAAsstezEAAAihiGnw==

Decoded length = 100
Table ID = 0xFC
MPEG Short Section
```
