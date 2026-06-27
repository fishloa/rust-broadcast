2.4.3 Specification of the transport stream syntax and semantics

The following syntax describes a stream of bytes . Transport stream packets shall be 188 bytes long .

2.4.3.1 Transport stream

See Table 2-1 .

Table 2-1 – Transport stream

Syntax No . of bits Mnemonic

MPEG_transport_stream( ) { do { transport_packet( ) } while (nextbits( ) = = sync_byte ) }

2.4.3.2 Transport stream packet layer

See Table 2-2 .

Table 2-2 – Transport packet of this Recommendation | International Standard

| Syntax | No . of bits | Mnemonic |
|---|---|---|
| transport_packet(){ |  |  |
| sync_byte | 8 | bslbf |
| transport_error_indicator | 1 | bslbf |
| payload_unit_start_indicator | 1 | bslbf |
| transport_priority | 1 | bslbf |
| PID | 13 | uimsbf |
| transport_scrambling_control | 2 | bslbf |
| adaptation_field_control | 2 | bslbf |
| continuity_counter | 4 | uimsbf |

if(adaptation_field_control = = '10' || adaptation_field_control = = '11'){ adaptation_field( ) } if(adaptation_field_control = = '01' || adaptation_field_control = = '11' ) { for ( i = 0 ; i < N ; i++){ data_byte 8 bslbf

} } }

2.4.3.3 Semantic definition of fields in transport stream packet layer

sync_byte – The sync_byte is a fixed 8-bit field whose value is '0100 0111' (0x47 ) . Sync_byte emulation in the choice of values for other regularly occurring fields , such as PID , should be avoided .

transport_error_indicator – The transport_error_indicator is a 1-bit flag . When set to '1' it indicates that at least 1 uncorrectable bit error exists in the associated transport stream packet . This bit may be set to '1' by entities external to the transport layer . When set to '1' this bit shall not be reset to '0' unless the bit value(s ) in error have been corrected .

payload_unit_start_indicator – The payload_unit_start_indicator is a 1-bit flag which has normative meaning for transport stream packets that carry PES packets ( refer to 2.4.3.6 ) or transport stream section data ( refer to Table 2-31 in 2.4.4.5 ) .

When the payload of the transport stream packet contains PES packet data , the payload_unit_start_indicator has the following significance : a '1' indicates that the payload of this transport stream packet will commence with the first byte of a PES packet and a '0' indicates no PES packet shall start in this transport stream packet . If the

payload_unit_start_indicator is set to '1' , then one and only one PES packet starts in this transport stream packet . This also applies to private streams of stream_type 6 ( refer to Table 2-34 ) .

When the payload of the transport stream packet contains transport stream section data , the payload_unit_start_indicator has the following significance : if the transport stream packet carries the first byte of a section , the payload_unit_start_indicator value shall be '1' , indicating that the first byte of the payload of this transport stream packet carries the pointer_field . If the transport stream packet does not carry the first byte of a section , the payload_unit_start_indicator value shall be '0' , indicating that there is no pointer_field in the payload . Refer to 2.4.4.2 and 2.4.4.3 . This also applies to private streams of stream_type 5 ( refer to Table 2-34 ) .

For null packets the payload_unit_start_indicator shall be set to '0' .

The meaning of this bit for transport stream packets carrying only private data is not defined in this Specification .

transport_priority – The transport_priority is a 1-bit indicator . When set to '1' it indicates that the associated packet is of greater priority than other packets having the same PID which do not have the bit set to '1' . The transport mechanism can use this to prioritize its data within an elementary stream . Depending on the application the transport_priority field may be coded regardless of the PID or within one PID only . This field may be changed by channel-specific encoders or decoders .

PID – The PID is a 13-bit field , indicating the type of the data stored in the packet payload . PID value 0x0000 is reserved for the program association table ( see Table 2-30 ) . PID value 0x0001 is reserved for the conditional access table ( see Table 2-32 ) . PID value 0x0002 is reserved for the transport stream description table ( see Table 2-36 ) , PID value 0x0003 is reserved for IPMP control information table ( see ISO/IEC 13818-11 ) and PID values 0x0004-0x000F are reserved . PID value 0x1FFF is reserved for null packets ( see Table 2-3 ) .

Table 2-3 – PID table

| Value | Description |
|---|---|
| 0x0000 | Program association table |
| 0x0001 | Conditional access table |
| 0x0002 | Transport stream description table |
| 0x0003 | IPMP control information table |
| 0x0004 | Adaptive streaming information ( see Note 2 ) |
| 0x0005 . . 0x000F | Reserved |
| 0x0010 . . | May be assigned as network_PID , Program_map_PID , elementary_PID , or for other purposes |
| 0x1FFE |  |
| 0x1FFF | Null packet |

NOTE 1 – The transport packets with PID values 0x0000 , 0x0001 , and 0x0010-0x1FFE are allowed to carry a PCR . NOTE 2 – Payload syntax is defined in 5.10.3.3.5 of ISO/IEC 23009-1 .

transport_scrambling_control – This 2-bit field indicates the scrambling mode of the transport stream packet payload . The transport stream packet header , and the adaptation field when present , shall not be scrambled . In the case of a null packet the value of the transport_scrambling_control field shall be set to '00' ( see Table 2-4 ) .

Table 2-4 – Scrambling control values

| Value | Description |
|---|---|
| '00' | Not scrambled |
| '01' | User-defined |
| '10' | User-defined |
| '11' | User-defined |

adaptation_field_control – This 2-bit field indicates whether this transport stream packet header is followed by an adaptation field and/or payload ( see Table 2-5 ) .

Table 2-5 – Adaptation field control values

| Value | Description |
|---|---|
| '00' | Reserved for future use by ISO/IEC |
| '01' | No adaptation_field , payload only |
| '10' | Adaptation_field only , no payload |
| '11' | Adaptation_field followed by payload |

Rec . ITU-T H.222.0 | ISO/IEC 13818-1 decoders shall discard transport stream packets with the adaptation_field_control field set to a value of '00' . In the case of a null packet the value of the adaptation_field_control shall be set to '01' .

continuity_counter – The continuity_counter is a 4-bit field incrementing with each transport stream packet with the same PID . The continuity_counter wraps around to 0 after its maximum value . The continuity_counter shall not be incremented when the adaptation_field_control of the packet equals '00' or '10' .

In transport streams , duplicate packets may be sent as two , and only two , consecutive transport stream packets of the same PID . The duplicate packets shall have the same continuity_counter value as the original packet and the adaptation_field_control field shall be equal to '01' or '11' . In duplicate packets each byte of the original packet shall be duplicated , with the exception that in the program clock reference fields , if present , a valid value shall be encoded .

The continuity_counter in a particular transport stream packet is continuous when it differs by a positive value of one from the continuity_counter value in the previous transport stream packet of the same PID , or when either of the non-incrementing conditions ( adaptation_field_control set to '00' or '10' , or duplicate packets as described above ) are met . The continuity counter may be discontinuous when the discontinuity_indicator is set to '1' ( refer to 2.4.3.4 ) . In the case of a null packet the value of the continuity_counter is undefined .

data_byte – Data bytes shall be contiguous bytes of data from the PES packets ( refer to 2.4.3.6 ) , transport stream sections ( refer to 2.4.4 ) , packet stuffing bytes after transport stream sections , or private data not in these structures as indicated by the PID . In the case of null packets with PID value 0x1FFF , data_bytes may be assigned any value . The number of data_bytes , N , is specified by 184 minus the number of bytes in the adaptation_field( ) , as described in 2.4.3.4 .

2.4.3.4 Adaptation field

See Table 2-6 .

Table 2-6 – Transport stream adaptation field

| Syntax | No . of bits | Mnemonic |
|---|---|---|
| adaptation_field( ) { |  |  |
| adaptation_field_length | 8 | uimsbf |

if ( adaptation_field_length > 0 ) {

| discontinuity_indicator | 1 | bslbf |
|---|---|---|
| random_access_indicator | 1 | bslbf |
| elementary_stream_priority_indicator | 1 | bslbf |
| PCR_flag | 1 | bslbf |
| OPCR_flag | 1 | bslbf |
| splicing_point_flag | 1 | bslbf |
| transport_private_data_flag | 1 | bslbf |
| adaptation_field_extension_flag | 1 | bslbf |

if ( PCR_flag = = '1' ) {

| program_clock_reference_base | 33 | uimsbf |
|---|---|---|
| reserved | 6 | bslbf |
| program_clock_reference_extension | 9 | uimsbf |
| } |  |  |

if ( OPCR_flag = = '1' ) {

| original_program_clock_reference_base | 33 | uimsbf |
|---|---|---|
| reserved | 6 | bslbf |
| original_program_clock_reference_extension | 9 | uimsbf |
| } |  |  |

if ( splicing_point_flag = = '1' ) { splice_countdown 8 tcimsbf } if ( transport_private_data_flag = = '1' ) { transport_private_data_length 8 uimsbf

Table 2-6 – Transport stream adaptation field

Syntax No . of bits Mnemonic for ( i = 0 ; i < transport_private_data_length ; i++ ) {

private_data_byte 8 bslbf } }

if ( adaptation_field_extension_flag = = '1' ) {

| adaptation_field_extension_length | 8 | uimsbf |
|---|---|---|
| ltw_flag | 1 | bslbf |
| piecewise_rate_flag | 1 | bslbf |
| seamless_splice_flag | 1 | bslbf |
| af_descriptor_not_present_flag | 1 | bslbf |
| reserved | 4 | bslbf |

if ( ltw_flag = = '1' ) {

| ltw_valid_flag | 1 | bslbf |
|---|---|---|
| ltw_offset | 15 | uimsbf |
| } |  |  |

if ( piecewise_rate_flag = = '1' ) {

| reserved | 2 | bslbf |
|---|---|---|
| piecewise_rate | 22 | uimsbf |
| } |  |  |

if ( seamless_splice_flag = = '1' ) {

| Splice_type | 4 | bslbf |
|---|---|---|
| DTS_next_AU[32..30 ] | 3 | bslbf |
| marker_bit | 1 | bslbf |
| DTS_next_AU[29..15 ] | 15 | bslbf |
| marker_bit | 1 | bslbf |
| DTS_next_AU[14..0 ] | 15 | bslbf |
| marker_bit | 1 | bslbf |

} if ( af_descriptor_not_present_flag = = '0' ) { for ( i = 0 ; i  N1 ; i++ ) { af_descriptor( ) } } else { for ( i = 0 ; i < N2 ; i++ ) {

reserved 8 bslbf } } }

for ( i = 0 ; i < N3 ; i++ ) { stuffing_byte 8 bslbf } } }

2.4.3.5 Semantic definition of fields in adaptation field

adaptation_field_length – The adaptation_field_length is an 8-bit field specifying the number of bytes in the adaptation_field immediately following the adaptation_field_length . The value '0' is for inserting a single stuffing byte in the adaptation field of a transport stream packet . When the adaptation_field_control value is '11' , the value of the adaptation_field_length shall be in the range 0 to 182 . When the adaptation_field_control value is '10' , the value of the adaptation_field_length shall be 183 . For transport stream packets carrying PES packets , stuffing is needed when there is insufficient PES packet data to completely fill the transport stream packet payload bytes . Stuffing is accomplished by defining an adaptation field longer than the sum of the lengths of the data elements in it , so that the payload bytes remaining after the adaptation field exactly accommodates the available PES packet data . The extra space in the adaptation field is filled with stuffing bytes .

This is the only method of stuffing allowed for transport stream packets carrying PES packets . For transport stream packets carrying sections , an alternative stuffing method is described in 2.4.4.1 .

discontinuity_indicator – This is a 1-bit field which when set to '1' indicates that the discontinuity state is true for the current transport stream packet . When the discontinuity_indicator is set to '0' or is not present , the discontinuity state is false . The discontinuity indicator is used to indicate two types of discontinuities , system time-base discontinuities and continuity_counter discontinuities .

A system time-base discontinuity is indicated by the use of the discontinuity_indicator in transport stream packets of a PID designated as a PCR_PID ( refer to 2.4.4.10 ) . When the discontinuity state is true for a transport stream packet of a

PID designated as a PCR_PID , the next PCR in a transport stream packet with that same PID represents a sample of a new system time clock for the associated program . The system time-base discontinuity point is defined to be the instant in time when the first byte of a packet containing a PCR of a new system time-base arrives at the input of the T-STD . The discontinuity_indicator shall be set to '1' in the packet in which the system time-base discontinuity occurs . The discontinuity_indicator bit may also be set to '1' in transport stream packets of the same PCR_PID prior to the packet which contains the new system time-base PCR . In this case , once the discontinuity_indicator has been set to '1' , it shall continue to be set to '1' in all transport stream packets of the same PCR_PID up to and including the transport stream packet which contains the first PCR of the new system time-base . After the occurrence of a system time-base discontinuity , no fewer than two PCRs for the new system time-base shall be received before another system time-base discontinuity can occur . Further , except when trick mode status is true , data from no more than two system time-bases shall be present in the set of T-STD buffers for one program at any time .

Prior to the occurrence of a system time-base discontinuity , the first byte of a transport stream packet which contains a PTS or DTS which refers to the new system time-base shall not arrive at the input of the T-STD . After the occurrence of a system time-base discontinuity , the first byte of a transport stream packet which contains a PTS or DTS which refers to the previous system time-base shall not arrive at the input of the T-STD .

A continuity_counter discontinuity is indicated by the use of the discontinuity_indicator in any transport stream packet . When the discontinuity state is true in any transport stream packet of a PID not designated as a PCR_PID , the continuity_counter in that packet may be discontinuous with respect to the previous transport stream packet of the same PID . When the discontinuity state is true in a transport stream packet of a PID that is designated as a PCR_PID , the continuity_counter may only be discontinuous in the packet in which a system time-base discontinuity occurs . A continuity counter discontinuity point occurs when the discontinuity state is true in a transport stream packet and the continuity_counter in the same packet is discontinuous with respect to the previous transport stream packet of the same PID . A continuity counter discontinuity point shall occur at most one time from the initiation of the discontinuity state until the conclusion of the discontinuity state . Furthermore , for all PIDs that are not designated as PCR_PIDs , when the discontinuity_indicator is set to '1' in a packet of a specific PID , the discontinuity_indicator may be set to '1' in the next transport stream packet of that same PID , but shall not be set to '1' in three consecutive transport stream packets of that same PID .

For the purpose of this clause , an elementary stream access point is defined as follows :

| • | ISO/IEC 11172-2 video and Rec . ITU-T H.262 | ISO/IEC 13818-2 video – The first byte of a video |
|---|---|
|  | sequence header . |
| • | ISO/IEC 14496-2 visual – The first byte of the visual object sequence header . |
| • | AVC video streams conforming to one or more profiles defined in Annex A of Rec . ITU-T H.264 | ISO/IEC |

14496-10 – The first byte of an AVC access unit . The SPS and PPS parameter sets referenced in this and all subsequent AVC access units in the coded video stream shall be provided after this access point in the byte stream and prior to their activation .

• Video sub-bitstreams of AVC video streams conforming to one or more profiles defined in Annex G of Rec . ITU-T H.264 | ISO/IEC 14496-10 – The first byte of an SVC dependency representation is an elementary stream access point if the following conditions are met :

• The subset sequence parameter sets and picture parameter sets referenced in this and all subsequent SVC dependency representation in the video sub-bitstream shall be provided after this access point in the byte stream and prior to their activation .

• If this SVC video sub-bitstream access point requires the elementary stream access point of the same AVC access unit , if any , contained in the corresponding elementary stream that needs to be present in decoding order before decoding the elementary stream associated with this elementary stream access point , then the corresponding elementary stream shall also include an elementary stream access point . NOTE 1 – If the hierarchy descriptor is present for this SVC video sub-bitstream then the video sub￾bitstream of which the hierarchy_layer_index equals the hierarchy_embedded_layer_index of this SVC sub-bitstream should have an elementary stream access point in the same access unit .

• MVC video sub-bitstreams of AVC video streams conforming to one or more profiles defined in Annex H of Rec . ITU-T H.264 | ISO/IEC 14496-10 – The first byte of an MVC view-component subset is an elementary stream access point if the following two conditions are met :

– The subset sequence parameter sets and picture parameter sets referenced in this and all subsequent MVC view-component subsets in the MVC video sub-bitstream shall be provided after this access point in the byte stream and prior to their activation .

– If this MVC video sub-bitstream access point requires the elementary stream access point of the same AVC access unit , if any , contained in the corresponding elementary stream that needs to be present in decoding order before decoding the elementary stream associated with this elementary stream