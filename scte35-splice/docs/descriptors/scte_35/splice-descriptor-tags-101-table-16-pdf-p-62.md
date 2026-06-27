## Splice Descriptor Tags — §10.1, Table 16, PDF p. 62

Splice descriptors are only used within a splice_info_section (never in MPEG
syntax such as the PMT). Receiving equipment should skip descriptors with
unknown identifiers; for known identifiers it should skip descriptors with
an unknown splice_descriptor_tag. Multiple descriptors of the same or
different types in a single command are allowed and may be common; the only
limit on the number of descriptors is `section_length`.

| Tag | XML Element | Descriptors for Identifier "CUEI" |
|---|---|---|
| 0x00 | AvailDescriptor | avail_descriptor |
| 0x01 | DTMFDescriptor | DTMF_descriptor |
| 0x02 | SegmentationDescriptor | segmentation_descriptor |
| 0x03 | TimeDescriptor | time_descriptor |
| 0x04 | AudioDescriptor | audio_descriptor |
| 0x05 – 0xEF |  | Reserved for future SCTE splice_descriptors |
| 0xF0 – 0xFF |  | Reserved for DVB use (as specified in ETSI 103 752-1) |

