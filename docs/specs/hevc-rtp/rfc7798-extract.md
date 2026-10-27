Source file: specs/rfc7798_rtp_hevc.txt
RFC number: 7798
Licence: IETF Trust, BCP 78 / RFC 5378 allow reproduction with attribution.
Command: tools/spec_extract.py --source specs/rfc7798_rtp_hevc.txt --sections 4.1 4.2 4.3 4.4.1 4.4.2 4.4.3 4.4.4 4.5 5 6 7.1 --output docs/specs/hevc-rtp/rfc7798-extract.md

## 4.1. RTP Header Usage (specs/rfc7798_rtp_hevc.txt:1089-1211)

```text
4.1.  RTP Header Usage

   The format of the RTP header is specified in [RFC3550] (reprinted as
   Figure 2 for convenience).  This payload format uses the fields of
   the header in a manner consistent with that specification.

   The RTP payload (and the settings for some RTP header bits) for
   aggregation packets and fragmentation units are specified in Sections
   4.4.2 and 4.4.3, respectively.

    0                   1                   2                   3
    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |V=2|P|X|  CC   |M|     PT      |       sequence number         |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                           timestamp                           |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |           synchronization source (SSRC) identifier            |
   +=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+
   |            contributing source (CSRC) identifiers             |
   |                             ....                              |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

               Figure 2: RTP Header According to [RFC3550]











   The RTP header information to be set according to this RTP payload
   format is set as follows:

   Marker bit (M): 1 bit

      Set for the last packet of the access unit, carried in the current
      RTP stream.  This is in line with the normal use of the M bit in
      video formats to allow an efficient playout buffer handling.  When
      MRST or MRMT is in use, if an access unit appears in multiple RTP
      streams, the marker bit is set on each RTP stream's last packet of
      the access unit.

         Informative note: The content of a NAL unit does not tell
         whether or not the NAL unit is the last NAL unit, in decoding
         order, of an access unit.  An RTP sender implementation may
         obtain this information from the video encoder.  If, however,
         the implementation cannot obtain this information directly from
         the encoder, e.g., when the bitstream was pre-encoded, and also
         there is no timestamp allocated for each NAL unit, then the
         sender implementation can inspect subsequent NAL units in
         decoding order to determine whether or not the NAL unit is the
         last NAL unit of an access unit as follows.  A NAL unit is
         determined to be the last NAL unit of an access unit if it is
         the last NAL unit of the bitstream.  A NAL unit naluX is also
         determined to be the last NAL unit of an access unit if both
         the following conditions are true: 1) the next VCL NAL unit
         naluY in decoding order has the high-order bit of the first
         byte after its NAL unit header equal to 1, and 2) all NAL units
         between naluX and naluY, when present, have nal_unit_type in
         the range of 32 to 35, inclusive, equal to 39, or in the ranges
         of 41 to 44, inclusive, or 48 to 55, inclusive.

   Payload Type (PT): 7 bits

      The assignment of an RTP payload type for this new packet format
      is outside the scope of this document and will not be specified
      here.  The assignment of a payload type has to be performed either
      through the profile used or in a dynamic way.

         Informative note: It is not required to use different payload
         type values for different RTP streams in MRST or MRMT.

   Sequence Number (SN): 16 bits

      Set and used in accordance with [RFC3550].








   Timestamp: 32 bits

      The RTP timestamp is set to the sampling timestamp of the content.
      A 90 kHz clock rate MUST be used.

      If the NAL unit has no timing properties of its own (e.g.,
      parameter set and SEI NAL units), the RTP timestamp MUST be set to
      the RTP timestamp of the coded picture of the access unit in which
      the NAL unit (according to Section 7.4.2.4.4 of [HEVC]) is
      included.

      Receivers MUST use the RTP timestamp for the display process, even
      when the bitstream contains picture timing SEI messages or
      decoding unit information SEI messages as specified in [HEVC].
      However, this does not mean that picture timing SEI messages in
      the bitstream should be discarded, as picture timing SEI messages
      may contain frame-field information that is important in
      appropriately rendering interlaced video.

   Synchronization source (SSRC): 32 bits

      Used to identify the source of the RTP packets.  When using SRST,
      by definition a single SSRC is used for all parts of a single
      bitstream.  In MRST or MRMT, different SSRCs are used for each RTP
      stream containing a subset of the sub-layers of the single
      (temporally scalable) bitstream.  A receiver is required to
      correctly associate the set of SSRCs that are included parts of
      the same bitstream.
```

## 4.2. Payload Header Usage (specs/rfc7798_rtp_hevc.txt:1212-1238)

```text
4.2.  Payload Header Usage

   The first two bytes of the payload of an RTP packet are referred to
   as the payload header.  The payload header consists of the same
   fields (F, Type, LayerId, and TID) as the NAL unit header as shown in
   Section 1.1.4, irrespective of the type of the payload structure.

   The TID value indicates (among other things) the relative importance
   of an RTP packet, for example, because NAL units belonging to higher
   temporal sub-layers are not used for the decoding of lower temporal
   sub-layers.  A lower value of TID indicates a higher importance.
   More-important NAL units MAY be better protected against transmission
   losses than less-important NAL units.
```

## 4.3. Transmission Modes (specs/rfc7798_rtp_hevc.txt:1239-1294)

```text
4.3.  Transmission Modes

   This memo enables transmission of an HEVC bitstream over:

      o a Single RTP stream on a Single media Transport (SRST),

      o Multiple RTP streams over a Single media Transport (MRST), or

      o Multiple RTP streams on Multiple media Transports (MRMT).

      Informative note: While this specification enables the use of MRST
      within the H.265 RTP payload, the signaling of MRST within SDP
      offer/answer is not fully specified at the time of this writing.
      See [RFC5576] and [RFC5583] for what is supported today as well as
      [RTP-MULTI-STREAM] and [SDP-NEG] for future directions.

   When in MRMT, the dependency of one RTP stream on another RTP stream
   is typically indicated as specified in [RFC5583].  [RFC5583] can also
   be utilized to specify dependencies within MRST, but only if the RTP
   streams utilize distinct payload types.

   SRST or MRST SHOULD be used for point-to-point unicast scenarios,
   whereas MRMT SHOULD be used for point-to-multipoint multicast
   scenarios where different receivers require different operation
   points of the same HEVC bitstream, to improve bandwidth utilizing
   efficiency.

      Informative note: A multicast may degrade to a unicast after all
      but one receivers have left (this is a justification of the first
      "SHOULD" instead of "MUST"), and there might be scenarios where
      MRMT is desirable but not possible, e.g., when IP multicast is not
      deployed in certain network (this is a justification of the second
      "SHOULD" instead of "MUST").

   The transmission mode is indicated by the tx-mode media parameter
   (see Section 7.1).  If tx-mode is equal to "SRST", SRST MUST be used.
   Otherwise, if tx-mode is equal to "MRST", MRST MUST be used.
   Otherwise (tx-mode is equal to "MRMT"), MRMT MUST be used.

      Informative note: When an RTP stream does not depend on other RTP
      streams, any of SRST, MRST, or MRMT may be in use for the RTP
      stream.

   Receivers MUST support all of SRST, MRST, and MRMT.

      Informative note: The required support of MRMT by receivers does
      not imply that multicast must be supported by receivers.
```

## 4.4.1. Single NAL Unit Packets (specs/rfc7798_rtp_hevc.txt:1319-1363)

```text
4.4.1.  Single NAL Unit Packets

   A single NAL unit packet contains exactly one NAL unit, and consists
   of a payload header (denoted as PayloadHdr), a conditional 16-bit
   DONL field (in network byte order), and the NAL unit payload data
   (the NAL unit excluding its NAL unit header) of the contained NAL
   unit, as shown in Figure 3.

    0                   1                   2                   3
    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |           PayloadHdr          |      DONL (conditional)       |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                                                               |
   |                  NAL unit payload data                        |
   |                                                               |
   |                               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                               :...OPTIONAL RTP padding        |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

            Figure 3: The Structure of a Single NAL Unit Packet








   The payload header SHOULD be an exact copy of the NAL unit header of
   the contained NAL unit.  However, the Type (i.e., nal_unit_type)
   field MAY be changed, e.g., when it is desirable to handle a CRA
   picture to be a BLA picture [JCTVC-J0107].

   The DONL field, when present, specifies the value of the 16 least
   significant bits of the decoding order number of the contained NAL
   unit.  If sprop-max-don-diff is greater than 0 for any of the RTP
   streams, the DONL field MUST be present, and the variable DON for the
   contained NAL unit is derived as equal to the value of the DONL
   field.  Otherwise (sprop-max-don-diff is equal to 0 for all the RTP
   streams), the DONL field MUST NOT be present.
```

## 4.4.2. Aggregation Packets (APs) (specs/rfc7798_rtp_hevc.txt:1364-1603)

```text
4.4.2.  Aggregation Packets (APs)

   Aggregation Packets (APs) are introduced to enable the reduction of
   packetization overhead for small NAL units, such as most of the non-
   VCL NAL units, which are often only a few octets in size.

   An AP aggregates NAL units within one access unit.  Each NAL unit to
   be carried in an AP is encapsulated in an aggregation unit.  NAL
   units aggregated in one AP are in NAL unit decoding order.

   An AP consists of a payload header (denoted as PayloadHdr) followed
   by two or more aggregation units, as shown in Figure 4.

    0                   1                   2                   3
    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |    PayloadHdr (Type=48)       |                               |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+                               |
   |                                                               |
   |             two or more aggregation units                     |
   |                                                               |
   |                               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                               :...OPTIONAL RTP padding        |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

            Figure 4: The Structure of an Aggregation Packet

   The fields in the payload header are set as follows.  The F bit MUST
   be equal to 0 if the F bit of each aggregated NAL unit is equal to
   zero; otherwise, it MUST be equal to 1.  The Type field MUST be equal
   to 48.  The value of LayerId MUST be equal to the lowest value of
   LayerId of all the aggregated NAL units.  The value of TID MUST be
   the lowest value of TID of all the aggregated NAL units.







      Informative note: All VCL NAL units in an AP have the same TID
      value since they belong to the same access unit.  However, an AP
      may contain non-VCL NAL units for which the TID value in the NAL
      unit header may be different than the TID value of the VCL NAL
      units in the same AP.

   An AP MUST carry at least two aggregation units and can carry as many
   aggregation units as necessary; however, the total amount of data in
   an AP obviously MUST fit into an IP packet, and the size SHOULD be
   chosen so that the resulting IP packet is smaller than the MTU size
   so to avoid IP layer fragmentation.  An AP MUST NOT contain FUs
   specified in Section 4.4.3.  APs MUST NOT be nested; i.e., an AP must
   not contain another AP.

   The first aggregation unit in an AP consists of a conditional 16-bit
   DONL field (in network byte order) followed by a 16-bit unsigned size
   information (in network byte order) that indicates the size of the
   NAL unit in bytes (excluding these two octets, but including the NAL
   unit header), followed by the NAL unit itself, including its NAL unit
   header, as shown in Figure 5.

    0                   1                   2                   3
    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
                   :       DONL (conditional)      |   NALU size   |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |   NALU size   |                                               |
   +-+-+-+-+-+-+-+-+         NAL unit                              |
   |                                                               |
   |                               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                               :
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

     Figure 5: The Structure of the First Aggregation Unit in an AP

   The DONL field, when present, specifies the value of the 16 least
   significant bits of the decoding order number of the aggregated NAL
   unit.

   If sprop-max-don-diff is greater than 0 for any of the RTP streams,
   the DONL field MUST be present in an aggregation unit that is the
   first aggregation unit in an AP, and the variable DON for the
   aggregated NAL unit is derived as equal to the value of the DONL
   field.  Otherwise (sprop-max-don-diff is equal to 0 for all the RTP
   streams), the DONL field MUST NOT be present in an aggregation unit
   that is the first aggregation unit in an AP.







   An aggregation unit that is not the first aggregation unit in an AP
   consists of a conditional 8-bit DOND field followed by a 16-bit
   unsigned size information (in network byte order) that indicates the
   size of the NAL unit in bytes (excluding these two octets, but
   including the NAL unit header), followed by the NAL unit itself,
   including its NAL unit header, as shown in Figure 6.

    0                   1                   2                   3
    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
                   : DOND (cond)   |          NALU size            |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                                                               |
   |                       NAL unit                                |
   |                               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                               :
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

   Figure 6: The Structure of an Aggregation Unit That Is Not the
   First Aggregation Unit in an AP

   When present, the DOND field plus 1 specifies the difference between
   the decoding order number values of the current aggregated NAL unit
   and the preceding aggregated NAL unit in the same AP.

   If sprop-max-don-diff is greater than 0 for any of the RTP streams,
   the DOND field MUST be present in an aggregation unit that is not the
   first aggregation unit in an AP, and the variable DON for the
   aggregated NAL unit is derived as equal to the DON of the preceding
   aggregated NAL unit in the same AP plus the value of the DOND field
   plus 1 modulo 65536.  Otherwise (sprop-max-don-diff is equal to 0 for
   all the RTP streams), the DOND field MUST NOT be present in an
   aggregation unit that is not the first aggregation unit in an AP, and
   in this case the transmission order and decoding order of NAL units
   carried in the AP are the same as the order the NAL units appear in
   the AP.

   Figure 7 presents an example of an AP that contains two aggregation
   units, labeled as 1 and 2 in the figure, without the DONL and DOND
   fields being present.













    0                   1                   2                   3
    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                          RTP Header                           |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |   PayloadHdr (Type=48)        |         NALU 1 Size           |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |          NALU 1 HDR           |                               |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+         NALU 1 Data           |
   |                   . . .                                       |
   |                                                               |
   +               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |  . . .        | NALU 2 Size                   | NALU 2 HDR    |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   | NALU 2 HDR    |                                               |
   +-+-+-+-+-+-+-+-+              NALU 2 Data                      |
   |                   . . .                                       |
   |                               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                               :...OPTIONAL RTP padding        |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

   Figure 7: An Example of an AP Packet Containing Two Aggregation
   Units without the DONL and DOND Fields






























   Figure 8 presents an example of an AP that contains two aggregation
   units, labeled as 1 and 2 in the figure, with the DONL and DOND
   fields being present.

    0                   1                   2                   3
    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                          RTP Header                           |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |   PayloadHdr (Type=48)        |        NALU 1 DONL            |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |          NALU 1 Size          |            NALU 1 HDR         |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                                                               |
   |                 NALU 1 Data   . . .                           |
   |                                                               |
   +     . . .     +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |               |  NALU 2 DOND  |          NALU 2 Size          |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |          NALU 2 HDR           |                               |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+          NALU 2 Data          |
   |                                                               |
   |        . . .                  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                               :...OPTIONAL RTP padding        |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

   Figure 8: An Example of an AP Containing Two Aggregation Units
   with the DONL and DOND Fields
```

## 4.4.3. Fragmentation Units (specs/rfc7798_rtp_hevc.txt:1604-1742)

```text
4.4.3.  Fragmentation Units

   Fragmentation Units (FUs) are introduced to enable fragmenting a
   single NAL unit into multiple RTP packets, possibly without
   cooperation or knowledge of the HEVC encoder.  A fragment of a NAL
   unit consists of an integer number of consecutive octets of that NAL
   unit.  Fragments of the same NAL unit MUST be sent in consecutive
   order with ascending RTP sequence numbers (with no other RTP packets
   within the same RTP stream being sent between the first and last
   fragment).

   When a NAL unit is fragmented and conveyed within FUs, it is referred
   to as a fragmented NAL unit.  APs MUST NOT be fragmented.  FUs MUST
   NOT be nested; i.e., an FU must not contain a subset of another FU.

   The RTP timestamp of an RTP packet carrying an FU is set to the NALU-
   time of the fragmented NAL unit.







   An FU consists of a payload header (denoted as PayloadHdr), an FU
   header of one octet, a conditional 16-bit DONL field (in network byte
   order), and an FU payload, as shown in Figure 9.

    0                   1                   2                   3
    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |    PayloadHdr (Type=49)       |   FU header   | DONL (cond)   |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-|
   | DONL (cond)   |                                               |
   |-+-+-+-+-+-+-+-+                                               |
   |                         FU payload                            |
   |                                                               |
   |                               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                               :...OPTIONAL RTP padding        |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

                Figure 9: The Structure of an FU

   The fields in the payload header are set as follows.  The Type field
   MUST be equal to 49.  The fields F, LayerId, and TID MUST be equal to
   the fields F, LayerId, and TID, respectively, of the fragmented NAL
   unit.

   The FU header consists of an S bit, an E bit, and a 6-bit FuType
   field, as shown in Figure 10.

   +---------------+
   |0|1|2|3|4|5|6|7|
   +-+-+-+-+-+-+-+-+
   |S|E|  FuType   |
   +---------------+

   Figure 10: The Structure of FU Header

   The semantics of the FU header fields are as follows:

   S: 1 bit
      When set to 1, the S bit indicates the start of a fragmented NAL
      unit, i.e., the first byte of the FU payload is also the first
      byte of the payload of the fragmented NAL unit.  When the FU
      payload is not the start of the fragmented NAL unit payload, the S
      bit MUST be set to 0.










   E: 1 bit
      When set to 1, the E bit indicates the end of a fragmented NAL
      unit, i.e., the last byte of the payload is also the last byte of
      the fragmented NAL unit.  When the FU payload is not the last
      fragment of a fragmented NAL unit, the E bit MUST be set to 0.

   FuType: 6 bits
      The field FuType MUST be equal to the field Type of the fragmented
      NAL unit.

   The DONL field, when present, specifies the value of the 16 least
   significant bits of the decoding order number of the fragmented NAL
   unit.

   If sprop-max-don-diff is greater than 0 for any of the RTP streams,
   and the S bit is equal to 1, the DONL field MUST be present in the
   FU, and the variable DON for the fragmented NAL unit is derived as
   equal to the value of the DONL field.  Otherwise (sprop-max-don-diff
   is equal to 0 for all the RTP streams, or the S bit is equal to 0),
   the DONL field MUST NOT be present in the FU.

   A non-fragmented NAL unit MUST NOT be transmitted in one FU; i.e.,
   the Start bit and End bit must not both be set to 1 in the same FU
   header.

   The FU payload consists of fragments of the payload of the fragmented
   NAL unit so that if the FU payloads of consecutive FUs, starting with
   an FU with the S bit equal to 1 and ending with an FU with the E bit
   equal to 1, are sequentially concatenated, the payload of the
   fragmented NAL unit can be reconstructed.  The NAL unit header of the
   fragmented NAL unit is not included as such in the FU payload, but
   rather the information of the NAL unit header of the fragmented NAL
   unit is conveyed in F, LayerId, and TID fields of the FU payload
   headers of the FUs and the FuType field of the FU header of the FUs.
   An FU payload MUST NOT be empty.

   If an FU is lost, the receiver SHOULD discard all following
   fragmentation units in transmission order corresponding to the same
   fragmented NAL unit, unless the decoder in the receiver is known to
   be prepared to gracefully handle incomplete NAL units.

   A receiver in an endpoint or in a MANE MAY aggregate the first n-1
   fragments of a NAL unit to an (incomplete) NAL unit, even if fragment
   n of that NAL unit is not received.  In this case, the
   forbidden_zero_bit of the NAL unit MUST be set to 1 to indicate a
   syntax violation.
```

## 4.4.4. PACI Packets (specs/rfc7798_rtp_hevc.txt:1743-1972)

```text
4.4.4.  PACI Packets

   This section specifies the PACI packet structure.  The basic payload
   header specified in this memo is intentionally limited to the 16 bits
   of the NAL unit header so to keep the packetization overhead to a
   minimum.  However, cases have been identified where it is advisable
   to include control information in an easily accessible position in
   the packet header, despite the additional overhead.  One such control
   information is the TSCI as specified in Section 4.5.  PACI packets
   carry this and future, similar structures.

   The PACI packet structure is based on a payload header extension
   mechanism that is generic and extensible to carry payload header
   extensions.  In this section, the focus lies on the use within this
   specification.  Section 4.4.4.2 provides guidance for the
   specification designers in how to employ the extension mechanism in
   future specifications.

   A PACI packet consists of a payload header (denoted as PayloadHdr),
   for which the structure follows what is described in Section 4.2.
   The payload header is followed by the fields A, cType, PHSsize,
   F[0..2], and Y.

   Figure 11 shows a PACI packet in compliance with this memo, i.e.,
   without any extensions.

    0                   1                   2                   3
    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |    PayloadHdr (Type=50)       |A|   cType   | PHSsize |F0..2|Y|
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |        Payload Header Extension Structure (PHES)              |
   |=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=|
   |                                                               |
   |                  PACI payload: NAL unit                       |
   |                   . . .                                       |
   |                                                               |
   |                               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                               :...OPTIONAL RTP padding        |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

               Figure 11: The Structure of a PACI











   The fields in the payload header are set as follows.  The F bit MUST
   be equal to 0.  The Type field MUST be equal to 50.  The value of
   LayerId MUST be a copy of the LayerId field of the PACI payload NAL
   unit or NAL-unit-like structure.  The value of TID MUST be a copy of
   the TID field of the PACI payload NAL unit or NAL-unit-like
   structure.

   The semantics of other fields are as follows:

   A: 1 bit
      Copy of the F bit of the PACI payload NAL unit or NAL-unit-like
      structure.

   cType: 6 bits
      Copy of the Type field of the PACI payload NAL unit or NAL-unit-
      like structure.

   PHSsize: 5 bits
      Indicates the length of the PHES field.  The value is limited to
      be less than or equal to 32 octets, to simplify encoder design for
      MTU size matching.

   F0:
      This field equal to 1 specifies the presence of a temporal
      scalability support extension in the PHES.

   F1, F2:
      MUST be 0, available for future extensions, see Section 4.4.4.2.
      Receivers compliant with this version of the HEVC payload format
      MUST ignore F1=1 and/or F2=1, and also ignore any information in
      the PHES indicated as present by F1=1 and/or F2=1.

         Informative note: The receiver can do that by first decoding
         information associated with F0=1, and then skipping over any
         remaining bytes of the PHES based on the value of PHSsize.

   Y: 1 bit
      MUST be 0, available for future extensions, see Section 4.4.4.2.
      Receivers compliant with this version of the HEVC payload format
      MUST ignore Y=1, and also ignore any information in the PHES
      indicated as present by Y.

   PHES: variable number of octets
      A variable number of octets as indicated by the value of PHSsize.

   PACI Payload:
      The single NAL unit packet or NAL-unit-like structure (such as: FU
      or AP) to be carried, not including the first two octets.





         Informative note: The first two octets of the NAL unit or NAL-
         unit-like structure carried in the PACI payload are not
         included in the PACI payload.  Rather, the respective values
         are copied in locations of the PayloadHdr of the RTP packet.
         This design offers two advantages: first, the overall structure
         of the payload header is preserved, i.e., there is no special
         case of payload header structure that needs to be implemented
         for PACI.  Second, no additional overhead is introduced.

      A PACI payload MAY be a single NAL unit, an FU, or an AP.  PACIs
      MUST NOT be fragmented or aggregated.  The following subsection
      documents the reasons for these design choices.

4.4.4.1.  Reasons for the PACI Rules (Informative)

   A PACI cannot be fragmented.  If a PACI could be fragmented, and a
   fragment other than the first fragment got lost, access to the
   information in the PACI would not be possible.  Therefore, a PACI
   must not be fragmented.  In other words, an FU must not carry
   (fragments of) a PACI.

   A PACI cannot be aggregated.  Aggregation of PACIs is inadvisable
   from a compression viewpoint, as, in many cases, several to be
   aggregated NAL units would share identical PACI fields and values
   which would be carried redundantly for no reason.  Most, if not all,
   of the practical effects of PACI aggregation can be achieved by
   aggregating NAL units and bundling them with a PACI (see below).
   Therefore, a PACI must not be aggregated.  In other words, an AP must
   not contain a PACI.

   The payload of a PACI can be a fragment.  Both middleboxes and
   sending systems with inflexible (often hardware-based) encoders
   occasionally find themselves in situations where a PACI and its
   headers, combined, are larger than the MTU size.  In such a scenario,
   the middlebox or sender can fragment the NAL unit and encapsulate the
   fragment in a PACI.  Doing so preserves the payload header extension
   information for all fragments, allowing downstream middleboxes and
   the receiver to take advantage of that information.  Therefore, a
   sender may place a fragment into a PACI, and a receiver must be able
   to handle such a PACI.

   The payload of a PACI can be an aggregation NAL unit.  HEVC
   bitstreams can contain unevenly sized and/or small (when compared to
   the MTU size) NAL units.  In order to efficiently packetize such
   small NAL units, APs were introduced.  The benefits of APs are
   independent from the need for a payload header extension.  Therefore,
   a sender may place an AP into a PACI, and a receiver must be able to
   handle such a PACI.





4.4.4.2.  PACI Extensions (Informative)

   This section includes recommendations for future specification
   designers on how to extent the PACI syntax to accommodate future
   extensions.  Obviously, designers are free to specify whatever
   appears to be appropriate to them at the time of their design.
   However, a lot of thought has been invested into the extension
   mechanism described below, and we suggest that deviations from it
   warrant a good explanation.

   This memo defines only a single payload header extension (TSCI,
   described in Section 4.5); therefore, only the F0 bit carries
   semantics.  F1 and F2 are already named (and not just marked as
   reserved, as a typical video spec designer would do).  They are
   intended to signal two additional extensions.  The Y bit allows one
   to, recursively, add further F and Y bits to extend the mechanism
   beyond three possible payload header extensions.  It is suggested to
   define a new packet type (using a different value for Type) when
   assigning the F1, F2, or Y bits different semantics than what is
   suggested below.

   When a Y bit is set, an 8-bit flag-extension is inserted after the Y
   bit.  A flag-extension consists of 7 flags F[n..n+6], and another Y
   bit.

   The basic PACI header already includes F0, F1, and F2.  Therefore,
   the Fx bits in the first flag-extensions are numbered F3, F4, ...,
   F9; the F bits in the second flag-extension are numbered F10, F11,
   ..., F16, and so forth.  As a result, at least three Fx bits are
   always in the PACI, but the number of Fx bits (and associated types
   of extensions) can be increased by setting the next Y bit and adding
   an octet of flag-extensions, carrying seven flags and another Y bit.
   The size of this list of flags is subject to the limits specified in
   Section 4.4.4 (32 octets for all flag-extensions and the PHES
   information combined).

   Each of the F bits can indicate either the presence or the absence of
   certain information in the Payload Header Extension Structure (PHES).

   When a spec developer devises a new syntax that takes advantage of
   the PACI extension mechanism, he/she must follow the constraints
   listed below; otherwise, the extension mechanism may break.

      1) The fields added for a particular Fx bit MUST be fixed in
         length and not depend on what other Fx bits are set (no parsing
         dependency).

      2) The Fx bits must be assigned in order.





      3) An implementation that supports the n-th Fn bit for any value
         of n must understand the syntax (though not necessarily the
         semantics) of the fields Fk (with k < n), so as to be able to
         either use those bits when present, or at least be able to skip
         over them.
```

## 4.5. Temporal Scalability Control Information (specs/rfc7798_rtp_hevc.txt:1973-2059)

```text
4.5.  Temporal Scalability Control Information

   This section describes the single payload header extension defined in
   this specification, known as TSCI.  If, in the future, additional
   payload header extensions become necessary, they could be specified
   in this section of an updated version of this document, or in their
   own documents.

   When F0 is set to 1 in a PACI, this specifies that the PHES field
   includes the TSCI fields TL0PICIDX, IrapPicID, S, and E as follows:

   0                   1                   2                   3
   0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |    PayloadHdr (Type=50)       |A|   cType   | PHSsize |F0..2|Y|
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |   TL0PICIDX   |   IrapPicID   |S|E|    RES    |               |
   |-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+               |
   |                           ....                                |
   |               PACI payload: NAL unit                          |
   |                                                               |
   |                               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
   |                               :...OPTIONAL RTP padding        |
   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

   Figure 12: The Structure of a PACI with a PHES Containing a TSCI

   TL0PICIDX (8 bits)
      When present, the TL0PICIDX field MUST be set to equal to
      temporal_sub_layer_zero_idx as specified in Section D.3.22 of
      [HEVC] for the access unit containing the NAL unit in the PACI.

   IrapPicID (8 bits)
      When present, the IrapPicID field MUST be set to equal to
      irap_pic_id as specified in Section D.3.22 of [HEVC] for the
      access unit containing the NAL unit in the PACI.











   S (1 bit)
      The S bit MUST be set to 1 if any of the following conditions is
      true and MUST be set to 0 otherwise:

      o  The NAL unit in the payload of the PACI is the first VCL NAL
         unit, in decoding order, of a picture.

      o  The NAL unit in the payload of the PACI is an AP, and the NAL
         unit in the first contained aggregation unit is the first VCL
         NAL unit, in decoding order, of a picture.

      o  The NAL unit in the payload of the PACI is an FU with its S bit
         equal to 1 and the FU payload containing a fragment of the
         first VCL NAL unit, in decoding order, of a picture.

   E (1 bit)
      The E bit MUST be set to 1 if any of the following conditions is
      true and MUST be set to 0 otherwise:

      o  The NAL unit in the payload of the PACI is the last VCL NAL
         unit, in decoding order, of a picture.

      o  The NAL unit in the payload of the PACI is an AP and the NAL
         unit in the last contained aggregation unit is the last VCL NAL
         unit, in decoding order, of a picture.

      o  The NAL unit in the payload of the PACI is an FU with its E bit
         equal to 1 and the FU payload containing a fragment of the last
         VCL NAL unit, in decoding order, of a picture.

   RES (6 bits)
      MUST be equal to 0.  Reserved for future extensions.

   The value of PHSsize MUST be set to 3.  Receivers MUST allow other
   values of the fields F0, F1, F2, Y, and PHSsize, and MUST ignore any
   additional fields, when present, than specified above in the PHES.
```

## 5. Packetization Rules (specs/rfc7798_rtp_hevc.txt:2157-2193)

```text
5.  Packetization Rules

   The following packetization rules apply:

   o  If sprop-max-don-diff is greater than 0 for any of the RTP
      streams, the transmission order of NAL units carried in the RTP
      stream MAY be different than the NAL unit decoding order and the
      NAL unit output order.  Otherwise (sprop-max-don-diff is equal to
      0 for all the RTP streams), the transmission order of NAL units
      carried in the RTP stream MUST be the same as the NAL unit
      decoding order and, when tx-mode is equal to "MRST" or "MRMT",
      MUST also be the same as the NAL unit output order.

   o  A NAL unit of a small size SHOULD be encapsulated in an
      aggregation packet together with one or more other NAL units in
      order to avoid the unnecessary packetization overhead for small
      NAL units.  For example, non-VCL NAL units such as access unit
      delimiters, parameter sets, or SEI NAL units are typically small
      and can often be aggregated with VCL NAL units without violating
      MTU size constraints.

   o  Each non-VCL NAL unit SHOULD, when possible from an MTU size match
      viewpoint, be encapsulated in an aggregation packet together with
      its associated VCL NAL unit, as typically a non-VCL NAL unit would
      be meaningless without the associated VCL NAL unit being
      available.





   o  For carrying exactly one NAL unit in an RTP packet, a single NAL
      unit packet MUST be used.
```

## 6. De-packetization Process (specs/rfc7798_rtp_hevc.txt:2194-2322)

```text
6.  De-packetization Process

   The general concept behind de-packetization is to get the NAL units
   out of the RTP packets in an RTP stream and all RTP streams the RTP
   stream depends on, if any, and pass them to the decoder in the NAL
   unit decoding order.

   The de-packetization process is implementation dependent.  Therefore,
   the following description should be seen as an example of a suitable
   implementation.  Other schemes may be used as well, as long as the
   output for the same input is the same as the process described below.
   The output is the same when the set of output NAL units and their
   order are both identical.  Optimizations relative to the described
   algorithms are possible.

   All normal RTP mechanisms related to buffer management apply.  In
   particular, duplicated or outdated RTP packets (as indicated by the
   RTP sequences number and the RTP timestamp) are removed.  To
   determine the exact time for decoding, factors such as a possible
   intentional delay to allow for proper inter-stream synchronization
   must be factored in.

   NAL units with NAL unit type values in the range of 0 to 47,
   inclusive, may be passed to the decoder.  NAL-unit-like structures
   with NAL unit type values in the range of 48 to 63, inclusive, MUST
   NOT be passed to the decoder.

   The receiver includes a receiver buffer, which is used to compensate
   for transmission delay jitter within individual RTP streams and
   across RTP streams, to reorder NAL units from transmission order to
   the NAL unit decoding order, and to recover the NAL unit decoding
   order in MRST or MRMT, when applicable.  In this section, the
   receiver operation is described under the assumption that there is no
   transmission delay jitter within an RTP stream and across RTP
   streams.  To make a difference from a practical receiver buffer that
   is also used for compensation of transmission delay jitter, the
   receiver buffer is hereafter called the de-packetization buffer in
   this section.  Receivers should also prepare for transmission delay
   jitter; that is, either reserve separate buffers for transmission
   delay jitter buffering and de-packetization buffering or use a
   receiver buffer for both transmission delay jitter and de-
   packetization.  Moreover, receivers should take transmission delay
   jitter into account in the buffering operation, e.g., by additional
   initial buffering before starting of decoding and playback.






   When sprop-max-don-diff is equal to 0 for all the received RTP
   streams, the de-packetization buffer size is zero bytes, and the
   process described in the remainder of this paragraph applies.  When
   there is only one RTP stream received, the NAL units carried in the
   single RTP stream are directly passed to the decoder in their
   transmission order, which is identical to their decoding order.  When
   there is more than one RTP stream received, the NAL units carried in
   the multiple RTP streams are passed to the decoder in their NTP
   timestamp order.  When there are several NAL units of different RTP
   streams with the same NTP timestamp, the order to pass them to the
   decoder is their dependency order, where NAL units of a dependee RTP
   stream are passed to the decoder prior to the NAL units of the
   dependent RTP stream.  When there are several NAL units of the same
   RTP stream with the same NTP timestamp, the order to pass them to the
   decoder is their transmission order.

      Informative note: The mapping between RTP and NTP timestamps is
      conveyed in RTCP SR packets.  In addition, the mechanisms for
      faster media timestamp synchronization discussed in [RFC6051] may
      be used to speed up the acquisition of the RTP-to-wall-clock
      mapping.

   When sprop-max-don-diff is greater than 0 for any the received RTP
   streams, the process described in the remainder of this section
   applies.

   There are two buffering states in the receiver: initial buffering and
   buffering while playing.  Initial buffering starts when the reception
   is initialized.  After initial buffering, decoding and playback are
   started, and the buffering-while-playing mode is used.

   Regardless of the buffering state, the receiver stores incoming NAL
   units, in reception order, into the de-packetization buffer.  NAL
   units carried in RTP packets are stored in the de-packetization
   buffer individually, and the value of AbsDon is calculated and stored
   for each NAL unit.  When MRST or MRMT is in use, NAL units of all RTP
   streams of a bitstream are stored in the same de-packetization
   buffer.  When NAL units carried in any two RTP streams are available
   to be placed into the de-packetization buffer, those NAL units
   carried in the RTP stream that is lower in the dependency tree are
   placed into the buffer first.  For example, if RTP stream A depends
   on RTP stream B, then NAL units carried in RTP stream B are placed
   into the buffer first.










   Initial buffering lasts until condition A (the difference between the
   greatest and smallest AbsDon values of the NAL units in the de-
   packetization buffer is greater than or equal to the value of sprop-
   max-don-diff of the highest RTP stream) or condition B (the number of
   NAL units in the de-packetization buffer is greater than the value of
   sprop-depack-buf-nalus) is true.

   After initial buffering, whenever condition A or condition B is true,
   the following operation is repeatedly applied until both condition A
   and condition B become false:

      o  The NAL unit in the de-packetization buffer with the smallest
         value of AbsDon is removed from the de-packetization buffer and
         passed to the decoder.

   When no more NAL units are flowing into the de-packetization buffer,
   all NAL units remaining in the de-packetization buffer are removed
   from the buffer and passed to the decoder in the order of increasing
   AbsDon values.
```

## 7.1. Media Type Registration (specs/rfc7798_rtp_hevc.txt:2334-3563)

```text
7.1.  Media Type Registration

   The media subtype for the HEVC codec is allocated from the IETF tree.

   The receiver MUST ignore any unrecognized parameter.

   Type name:     video

   Subtype name:  H265

   Required parameters: none

   OPTIONAL parameters:

      profile-space, tier-flag, profile-id, profile-compatibility-
      indicator, interop-constraints, and level-id:






         These parameters indicate the profile, tier, default level, and
         some constraints of the bitstream carried by the RTP stream and
         all RTP streams the RTP stream depends on, or a specific set of
         the profile, tier, default level, and some constraints the
         receiver supports.

         The profile and some constraints are indicated collectively by
         profile-space, profile-id, profile-compatibility-indicator, and
         interop-constraints.  The profile specifies the subset of
         coding tools that may have been used to generate the bitstream
         or that the receiver supports.

            Informative note: There are 32 values of profile-id, and
            there are 32 flags in profile-compatibility-indicator, each
            flag corresponding to one value of profile-id.  According to
            HEVC version 1 in [HEVC], when more than one of the 32 flags
            is set for a bitstream, the bitstream would comply with all
            the profiles corresponding to the set flags.  However, in a
            draft of HEVC version 2 in [HEVCv2], Subclause A.3.5, 19
            Format Range Extensions profiles have been specified, all
            using the same value of profile-id (4), differentiated by
            some of the 48 bits in interop-constraints; this (rather
            unexpected way of profile signaling) means that one of the
            32 flags may correspond to multiple profiles.  To be able to
            support whatever HEVC extension profile that might be
            specified and indicated using profile-space, profile-id,
            profile-compatibility-indicator, and interop-constraints in
            the future, it would be safe to require symmetric use of
            these parameters in SDP offer/answer unless recv-sub-layer-
            id is included in the SDP answer for choosing one of the
            sub-layers offered.

         The tier is indicated by tier-flag.  The default level is
         indicated by level-id.  The tier and the default level specify
         the limits on values of syntax elements or arithmetic
         combinations of values of syntax elements that are followed
         when generating the bitstream or that the receiver supports.

         A set of profile-space, tier-flag, profile-id, profile-
         compatibility-indicator, interop-constraints, and level-id
         parameters ptlA is said to be consistent with another set of
         these parameters ptlB if any decoder that conforms to the
         profile, tier, level, and constraints indicated by ptlB can
         decode any bitstream that conforms to the profile, tier, level,
         and constraints indicated by ptlA.








         In SDP offer/answer, when the SDP answer does not include the
         recv-sub-layer-id parameter that is less than the sprop-sub-
         layer-id parameter in the SDP offer, the following applies:

            o  The profile-space, tier-flag, profile-id, profile-
               compatibility-indicator, and interop-constraints
               parameters MUST be used symmetrically, i.e., the value of
               each of these parameters in the offer MUST be the same as
               that in the answer, either explicitly signaled or
               implicitly inferred.

            o  The level-id parameter is changeable as long as the
               highest level indicated by the answer is either equal to
               or lower than that in the offer.  Note that the highest
               level is indicated by level-id and max-recv-level-id
               together.

         In SDP offer/answer, when the SDP answer does include the recv-
         sub-layer-id parameter that is less than the sprop-sub-layer-id
         parameter in the SDP offer, the set of profile-space, tier-
         flag, profile-id, profile-compatibility-indicator, interop-
         constraints, and level-id parameters included in the answer
         MUST be consistent with that for the chosen sub-layer
         representation as indicated in the SDP offer, with the
         exception that the level-id parameter in the SDP answer is
         changeable as long as the highest level indicated by the answer
         is either lower than or equal to that in the offer.

         More specifications of these parameters, including how they
         relate to the values of the profile, tier, and level syntax
         elements specified in [HEVC] are provided below.

      profile-space, profile-id:

         The value of profile-space MUST be in the range of 0 to 3,
         inclusive.  The value of profile-id MUST be in the range of 0
         to 31, inclusive.

         When profile-space is not present, a value of 0 MUST be
         inferred.  When profile-id is not present, a value of 1 (i.e.,
         the Main profile) MUST be inferred.

         When used to indicate properties of a bitstream, profile-space
         and profile-id are derived from the profile, tier, and level
         syntax elements in SPS or VPS NAL units as follows, where
         general_profile_space, general_profile_idc,
         sub_layer_profile_space[j], and sub_layer_profile_idc[j] are
         specified in [HEVC]:





            If the RTP stream is the highest RTP stream, the following
            applies:

            o profile-space = general_profile_space
            o profile-id = general_profile_idc

            Otherwise (the RTP stream is a dependee RTP stream), the
            following applies, with j being the value of the sprop-sub-
            layer-id parameter:

            o profile-space = sub_layer_profile_space[j]
            o profile-id = sub_layer_profile_idc[j]

      tier-flag, level-id:

         The value of tier-flag MUST be in the range of 0 to 1,
         inclusive.  The value of level-id MUST be in the range of 0 to
         255, inclusive.

         If the tier-flag and level-id parameters are used to indicate
         properties of a bitstream, they indicate the tier and the
         highest level the bitstream complies with.

         If the tier-flag and level-id parameters are used for
         capability exchange, the following applies.  If max-recv-level-
         id is not present, the default level defined by level-id
         indicates the highest level the codec wishes to support.
         Otherwise, max-recv-level-id indicates the highest level the
         codec supports for receiving.  For either receiving or sending,
         all levels that are lower than the highest level supported MUST
         also be supported.

         If no tier-flag is present, a value of 0 MUST be inferred; if
         no level-id is present, a value of 93 (i.e., level 3.1) MUST be
         inferred.

         When used to indicate properties of a bitstream, the tier-flag
         and level-id parameters are derived from the profile, tier, and
         level syntax elements in SPS or VPS NAL units as follows, where
         general_tier_flag, general_level_idc, sub_layer_tier_flag[j],
         and sub_layer_level_idc[j] are specified in [HEVC]:

            If the RTP stream is the highest RTP stream, the following
            applies:

            o tier-flag = general_tier_flag
            o level-id = general_level_idc






            Otherwise (the RTP stream is a dependee RTP stream), the
            following applies, with j being the value of the sprop-sub-
            layer-id parameter:

            o tier-flag = sub_layer_tier_flag[j]
            o level-id = sub_layer_level_idc[j]

      interop-constraints:

         A base16 [RFC4648] (hexadecimal) representation of six bytes of
         data, consisting of progressive_source_flag,
         interlaced_source_flag, non_packed_constraint_flag,
         frame_only_constraint_flag, and reserved_zero_44bits.

         If the interop-constraints parameter is not present, the
         following MUST be inferred:

            o progressive_source_flag = 1
            o interlaced_source_flag = 0
            o non_packed_constraint_flag = 1
            o frame_only_constraint_flag = 1
            o reserved_zero_44bits = 0

         When the interop-constraints parameter is used to indicate
         properties of a bitstream, the following applies, where
         general_progressive_source_flag,
         general_interlaced_source_flag,
         general_non_packed_constraint_flag,
         general_non_packed_constraint_flag,
         general_frame_only_constraint_flag,
         general_reserved_zero_44bits,
         sub_layer_progressive_source_flag[j],
         sub_layer_interlaced_source_flag[j],
         sub_layer_non_packed_constraint_flag[j],
         sub_layer_frame_only_constraint_flag[j], and
         sub_layer_reserved_zero_44bits[j] are specified in [HEVC]:

            If the RTP stream is the highest RTP stream, the following
            applies:

            o progressive_source_flag = general_progressive_source_flag

            o interlaced_source_flag = general_interlaced_source_flag

            o non_packed_constraint_flag =
                 general_non_packed_constraint_flag







            o frame_only_constraint_flag =
                 general_frame_only_constraint_flag

            o reserved_zero_44bits = general_reserved_zero_44bits

            Otherwise (the RTP stream is a dependee RTP stream), the
            following applies, with j being the value of the sprop-sub-
            layer-id parameter:

            o progressive_source_flag =
                 sub_layer_progressive_source_flag[j]

            o interlaced_source_flag =
                 sub_layer_interlaced_source_flag[j]

            o non_packed_constraint_flag =
                 sub_layer_non_packed_constraint_flag[j]

            o frame_only_constraint_flag =
                 sub_layer_frame_only_constraint_flag[j]

            o reserved_zero_44bits = sub_layer_reserved_zero_44bits[j]

            Using interop-constraints for capability exchange results in
            a requirement on any bitstream to be compliant with the
            interop-constraints.

      profile-compatibility-indicator:

         A base16 [RFC4648] representation of four bytes of data.

         When profile-compatibility-indicator is used to indicate
         properties of a bitstream, the following applies, where
         general_profile_compatibility_flag[j] and
         sub_layer_profile_compatibility_flag[i][j] are specified in
         [HEVC]:

            The profile-compatibility-indicator in this case indicates
            additional profiles to the profile defined by profile-space,
            profile-id, and interop-constraints the bitstream conforms
            to.  A decoder that conforms to any of all the profiles the
            bitstream conforms to would be capable of decoding the
            bitstream.  These additional profiles are defined by
            profile-space, each set bit of profile-compatibility-
            indicator, and interop-constraints.








            If the RTP stream is the highest RTP stream, the following
            applies for each value of j in the range of 0 to 31,
            inclusive:

            o bit j of profile-compatibility-indicator =
                 general_profile_compatibility_flag[j]

            Otherwise (the RTP stream is a dependee RTP stream), the
            following applies for i equal to sprop-sub-layer-id and for
            each value of j in the range of 0 to 31, inclusive:

            o bit j of profile-compatibility-indicator =
                 sub_layer_profile_compatibility_flag[i][j]

         Using profile-compatibility-indicator for capability exchange
         results in a requirement on any bitstream to be compliant with
         the profile-compatibility-indicator.  This is intended to
         handle cases where any future HEVC profile is defined as an
         intersection of two or more profiles.

         If this parameter is not present, this parameter defaults to
         the following: bit j, with j equal to profile-id, of profile-
         compatibility-indicator is inferred to be equal to 1, and all
         other bits are inferred to be equal to 0.

      sprop-sub-layer-id:

         This parameter MAY be used to indicate the highest allowed
         value of TID in the bitstream.  When not present, the value of
         sprop-sub-layer-id is inferred to be equal to 6.

         The value of sprop-sub-layer-id MUST be in the range of 0 to 6,
         inclusive.

      recv-sub-layer-id:

         This parameter MAY be used to signal a receiver's choice of the
         offered or declared sub-layer representations in the sprop-vps.
         The value of recv-sub-layer-id indicates the TID of the highest
         sub-layer of the bitstream that a receiver supports.  When not
         present, the value of recv-sub-layer-id is inferred to be equal
         to the value of the sprop-sub-layer-id parameter in the SDP
         offer.

         The value of recv-sub-layer-id MUST be in the range of 0 to 6,
         inclusive.







      max-recv-level-id:

         This parameter MAY be used to indicate the highest level a
         receiver supports.  The highest level the receiver supports is
         equal to the value of max-recv-level-id divided by 30.

         The value of max-recv-level-id MUST be in the range of 0 to
         255, inclusive.

         When max-recv-level-id is not present, the value is inferred to
         be equal to level-id.

         max-recv-level-id MUST NOT be present when the highest level
         the receiver supports is not higher than the default level.

      tx-mode:

         This parameter indicates whether the transmission mode is SRST,
         MRST, or MRMT.

         The value of tx-mode MUST be equal to "SRST", "MRST" or "MRMT".
         When not present, the value of tx-mode is inferred to be equal
         to "SRST".

         If the value is equal to "MRST", MRST MUST be in use.
         Otherwise, if the value is equal to "MRMT", MRMT MUST be in
         use.  Otherwise (the value is equal to "SRST"), SRST MUST be in
         use.

         The value of tx-mode MUST be equal to "MRST" for all RTP
         streams in an MRST.

         The value of tx-mode MUST be equal to "MRMT" for all RTP
         streams in an MRMT.

      sprop-vps:

         This parameter MAY be used to convey any video parameter set
         NAL unit of the bitstream for out-of-band transmission of video
         parameter sets.  The parameter MAY also be used for capability
         exchange and to indicate sub-stream characteristics (i.e.,
         properties of sub-layer representations as defined in [HEVC]).
         The value of the parameter is a comma-separated (',') list of
         base64 [RFC4648] representations of the video parameter set NAL
         units as specified in Section 7.3.2.1 of [HEVC].








         The sprop-vps parameter MAY contain one or more than one video
         parameter set NAL unit. However, all other video parameter sets
         contained in the sprop-vps parameter MUST be consistent with
         the first video parameter set in the sprop-vps parameter.  A
         video parameter set vpsB is said to be consistent with another
         video parameter set vpsA if any decoder that conforms to the
         profile, tier, level, and constraints indicated by the 12 bytes
         of data starting from the syntax element general_profile_space
         to the syntax element general_level_idc, inclusive, in the
         first profile_tier_level( ) syntax structure in vpsA can decode
         any bitstream that conforms to the profile, tier, level, and
         constraints indicated by the 12 bytes of data starting from the
         syntax element general_profile_space to the syntax element
         general_level_idc, inclusive, in the first profile_tier_level(
         ) syntax structure in vpsB.

      sprop-sps:

         This parameter MAY be used to convey sequence parameter set NAL
         units of the bitstream for out-of-band transmission of sequence
         parameter sets.  The value of the parameter is a comma-
         separated (',') list of base64 [RFC4648] representations of the
         sequence parameter set NAL units as specified in Section
         7.3.2.2 of [HEVC].

      sprop-pps:

         This parameter MAY be used to convey picture parameter set NAL
         units of the bitstream for out-of-band transmission of picture
         parameter sets.  The value of the parameter is a comma-
         separated (',') list of base64 [RFC4648] representations of the
         picture parameter set NAL units as specified in Section 7.3.2.3
         of [HEVC].

      sprop-sei:

         This parameter MAY be used to convey one or more SEI messages
         that describe bitstream characteristics.  When present, a
         decoder can rely on the bitstream characteristics that are
         described in the SEI messages for the entire duration of the
         session, independently from the persistence scopes of the SEI
         messages as specified in [HEVC].

         The value of the parameter is a comma-separated (',') list of
         base64 [RFC4648] representations of SEI NAL units as specified
         in Section 7.3.2.4 of [HEVC].







            Informative note: Intentionally, no list of applicable or
            inapplicable SEI messages is specified here.  Conveying
            certain SEI messages in sprop-sei may be sensible in some
            application scenarios and meaningless in others.  However, a
            few examples are described below:

               1) In an environment where the bitstream was created from
                  film-based source material, and no splicing is going
                  to occur during the lifetime of the session, the film
                  grain characteristics SEI message or the tone mapping
                  information SEI message are likely meaningful, and
                  sending them in sprop-sei rather than in the bitstream
                  at each entry point may help with saving bits and
                  allows one to configure the renderer only once,
                  avoiding unwanted artifacts.

               2) The structure of pictures information SEI message in
                  sprop-sei can be used to inform a decoder of
                  information on the NAL unit types, picture-order count
                  values, and prediction dependencies of a sequence of
                  pictures.  Having such knowledge can be helpful for
                  error recovery.

               3) Examples for SEI messages that would be meaningless to
                  be conveyed in sprop-sei include the decoded picture
                  hash SEI message (it is close to impossible that all
                  decoded pictures have the same hashtag), the display
                  orientation SEI message when the device is a handheld
                  device (as the display orientation may change when the
                  handheld device is turned around), or the filler
                  payload SEI message (as there is no point in just
                  having more bits in SDP).

      max-lsr, max-lps, max-cpb, max-dpb, max-br, max-tr, max-tc:

         These parameters MAY be used to signal the capabilities of a
         receiver implementation.  These parameters MUST NOT be used for
         any other purpose.  The highest level (specified by max-recv-
         level-id) MUST be the highest that the receiver is fully
         capable of supporting.  max-lsr, max-lps, max-cpb, max-dpb,
         max-br, max-tr, and max-tc MAY be used to indicate capabilities
         of the receiver that extend the required capabilities of the
         highest level, as specified below.

         When more than one parameter from the set (max-lsr, max-lps,
         max-cpb, max-dpb, max-br, max-tr, max-tc) is present, the
         receiver MUST support all signaled capabilities simultaneously.
         For example, if both max-lsr and max-br are present, the





         highest level with the extension of both the picture rate and
         bitrate is supported.  That is, the receiver is able to decode
         bitstreams in which the luma sample rate is up to max-lsr
         (inclusive), the bitrate is up to max-br (inclusive), the coded
         picture buffer size is derived as specified in the semantics of
         the max-br parameter below, and the other properties comply
         with the highest level specified by max-recv-level-id.

            Informative note: When the OPTIONAL media type parameters
            are used to signal the properties of a bitstream, and max-
            lsr, max-lps, max-cpb, max-dpb, max-br, max-tr, and max-tc
            are not present, the values of profile-space, tier-flag,
            profile-id, profile-compatibility-indicator, interop-
            constraints, and level-id must always be such that the
            bitstream complies fully with the specified profile, tier,
            and level.

      max-lsr:

         The value of max-lsr is an integer indicating the maximum
         processing rate in units of luma samples per second.  The max-
         lsr parameter signals that the receiver is capable of decoding
         video at a higher rate than is required by the highest level.

         When max-lsr is signaled, the receiver MUST be able to decode
         bitstreams that conform to the highest level, with the
         exception that the MaxLumaSR value in Table A-2 of [HEVC] for
         the highest level is replaced with the value of max-lsr.
         Senders MAY use this knowledge to send pictures of a given size
         at a higher picture rate than is indicated in the highest
         level.

         When not present, the value of max-lsr is inferred to be equal
         to the value of MaxLumaSR given in Table A-2 of [HEVC] for the
         highest level.

         The value of max-lsr MUST be in the range of MaxLumaSR to 16 *
         MaxLumaSR, inclusive, where MaxLumaSR is given in Table A-2 of
         [HEVC] for the highest level.

      max-lps:

         The value of max-lps is an integer indicating the maximum
         picture size in units of luma samples.  The max-lps parameter
         signals that the receiver is capable of decoding larger picture
         sizes than are required by the highest level.  When max-lps is
         signaled, the receiver MUST be able to decode bitstreams that
         conform to the highest level, with the exception that the





         MaxLumaPS value in Table A-1 of [HEVC] for the highest level is
         replaced with the value of max-lps.  Senders MAY use this
         knowledge to send larger pictures at a proportionally lower
         picture rate than is indicated in the highest level.

         When not present, the value of max-lps is inferred to be equal
         to the value of MaxLumaPS given in Table A-1 of [HEVC] for the
         highest level.

         The value of max-lps MUST be in the range of MaxLumaPS to 16 *
         MaxLumaPS, inclusive, where MaxLumaPS is given in Table A-1 of
         [HEVC] for the highest level.

      max-cpb:

         The value of max-cpb is an integer indicating the maximum coded
         picture buffer size in units of CpbBrVclFactor bits for the VCL
         HRD parameters and in units of CpbBrNalFactor bits for the NAL
         HRD parameters, where CpbBrVclFactor and CpbBrNalFactor are
         defined in Section A.4 of [HEVC].  The max-cpb parameter
         signals that the receiver has more memory than the minimum
         amount of coded picture buffer memory required by the highest
         level.  When max-cpb is signaled, the receiver MUST be able to
         decode bitstreams that conform to the highest level, with the
         exception that the MaxCPB value in Table A-1 of [HEVC] for the
         highest level is replaced with the value of max-cpb.  Senders
         MAY use this knowledge to construct coded bitstreams with
         greater variation of bitrate than can be achieved with the
         MaxCPB value in Table A-1 of [HEVC].

         When not present, the value of max-cpb is inferred to be equal
         to the value of MaxCPB given in Table A-1 of [HEVC] for the
         highest level.

         The value of max-cpb MUST be in the range of MaxCPB to 16 *
         MaxCPB, inclusive, where MaxLumaCPB is given in Table A-1 of
         [HEVC] for the highest level.

            Informative note: The coded picture buffer is used in the
            hypothetical reference decoder (Annex C of [HEVC]).  The use
            of the hypothetical reference decoder is recommended in HEVC
            encoders to verify that the produced bitstream conforms to
            the standard and to control the output bitrate.  Thus, the
            coded picture buffer is conceptually independent of any
            other potential buffers in the receiver, including de-
            packetization and de-jitter buffers.  The coded picture
            buffer need not be implemented in decoders as specified in
            Annex C of [HEVC], but rather standard-compliant decoders





            can have any buffering arrangements provided that they can
            decode standard-compliant bitstreams.  Thus, in practice,
            the input buffer for a video decoder can be integrated with
            de-packetization and de-jitter buffers of the receiver.

      max-dpb:

         The value of max-dpb is an integer indicating the maximum
         decoded picture buffer size in units decoded pictures at the
         MaxLumaPS for the highest level, i.e., the number of decoded
         pictures at the maximum picture size defined by the highest
         level.  The value of max-dpb MUST be in the range of 1 to 16,
         respectively.  The max-dpb parameter signals that the receiver
         has more memory than the minimum amount of decoded picture
         buffer memory required by default, which is MaxDpbPicBuf as
         defined in [HEVC] (equal to 6).  When max-dpb is signaled, the
         receiver MUST be able to decode bitstreams that conform to the
         highest level, with the exception that the MaxDpbPicBuff value
         defined in [HEVC] as 6 is replaced with the value of max-dpb.
         Consequently, a receiver that signals max-dpb MUST be capable
         of storing the following number of decoded pictures
         (MaxDpbSize) in its decoded picture buffer:

           if( PicSizeInSamplesY <= ( MaxLumaPS >> 2 ) )
              MaxDpbSize = Min( 4 * max-dpb, 16 )
           else if ( PicSizeInSamplesY <= ( MaxLumaPS >> 1 ) )
              MaxDpbSize = Min( 2 * max-dpb, 16 )
           else if ( PicSizeInSamplesY <= ( ( 3 * MaxLumaPS ) >> 2
         ) )
              MaxDpbSize = Min( (4 * max-dpb) / 3, 16 )
           else
              MaxDpbSize = max-dpb

         Wherein MaxLumaPS given in Table A-1 of [HEVC] for the highest
         level and PicSizeInSamplesY is the current size of each decoded
         picture in units of luma samples as defined in [HEVC].

         The value of max-dpb MUST be greater than or equal to the value
         of MaxDpbPicBuf (i.e., 6) as defined in [HEVC].  Senders MAY
         use this knowledge to construct coded bitstreams with improved
         compression.

         When not present, the value of max-dpb is inferred to be equal
         to the value of MaxDpbPicBuf (i.e., 6) as defined in [HEVC].

            Informative note: This parameter was added primarily to
            complement a similar codepoint in the ITU-T Recommendation
            H.245, so as to facilitate signaling gateway designs.  The





            decoded picture buffer stores reconstructed samples.  There
            is no relationship between the size of the decoded picture
            buffer and the buffers used in RTP, especially de-
            packetization and de-jitter buffers.

      max-br:

         The value of max-br is an integer indicating the maximum video
         bitrate in units of CpbBrVclFactor bits per second for the VCL
         HRD parameters and in units of CpbBrNalFactor bits per second
         for the NAL HRD parameters, where CpbBrVclFactor and
         CpbBrNalFactor are defined in Section A.4 of [HEVC].

         The max-br parameter signals that the video decoder of the
         receiver is capable of decoding video at a higher bitrate than
         is required by the highest level.

         When max-br is signaled, the video codec of the receiver MUST
         be able to decode bitstreams that conform to the highest level,
         with the following exceptions in the limits specified by the
         highest level:

            o  The value of max-br replaces the MaxBR value in Table A-2
               of [HEVC] for the highest level.

            o  When the max-cpb parameter is not present, the result of
               the following formula replaces the value of MaxCPB in
               Table A-1 of [HEVC]:

               (MaxCPB of the highest level) * max-br / (MaxBR of the
               highest level)

         For example, if a receiver signals capability for Main profile
         Level 2 with max-br equal to 2000, this indicates a maximum
         video bitrate of 2000 kbits/sec for VCL HRD parameters, a
         maximum video bitrate of 2200 kbits/sec for NAL HRD parameters,
         and a CPB size of 2000000 bits (2000000 / 1500000 * 1500000).

         Senders MAY use this knowledge to send higher bitrate video as
         allowed in the level definition of Annex A of [HEVC] to achieve
         improved video quality.

         When not present, the value of max-br is inferred to be equal
         to the value of MaxBR given in Table A-2 of [HEVC] for the
         highest level.








         The value of max-br MUST be in the range of MaxBR to 16 *
         MaxBR, inclusive, where MaxBR is given in Table A-2 of [HEVC]
         for the highest level.

            Informative note: This parameter was added primarily to
            complement a similar codepoint in the ITU-T Recommendation
            H.245, so as to facilitate signaling gateway designs.  The
            assumption that the network is capable of handling such
            bitrates at any given time cannot be made from the value of
            this parameter.  In particular, no conclusion can be drawn
            that the signaled bitrate is possible under congestion
            control constraints.

      max-tr:

         The value of max-tr is an integer indication the maximum number
         of tile rows.  The max-tr parameter signals that the receiver
         is capable of decoding video with a larger number of tile rows
         than the value allowed by the highest level.

         When max-tr is signaled, the receiver MUST be able to decode
         bitstreams that conform to the highest level, with the
         exception that the MaxTileRows value in Table A-1 of [HEVC] for
         the highest level is replaced with the value of max-tr.

         Senders MAY use this knowledge to send pictures utilizing a
         larger number of tile rows than the value allowed by the
         highest level.

         When not present, the value of max-tr is inferred to be equal
         to the value of MaxTileRows given in Table A-1 of [HEVC] for
         the highest level.

         The value of max-tr MUST be in the range of MaxTileRows to 16 *
         MaxTileRows, inclusive, where MaxTileRows is given in Table A-1
         of [HEVC] for the highest level.

      max-tc:

         The value of max-tc is an integer indication the maximum number
         of tile columns.  The max-tc parameter signals that the
         receiver is capable of decoding video with a larger number of
         tile columns than the value allowed by the highest level.

         When max-tc is signaled, the receiver MUST be able to decode
         bitstreams that conform to the highest level, with the
         exception that the MaxTileCols value in Table A-1 of [HEVC] for
         the highest level is replaced with the value of max-tc.





         Senders MAY use this knowledge to send pictures utilizing a
         larger number of tile columns than the value allowed by the
         highest level.

         When not present, the value of max-tc is inferred to be equal
         to the value of MaxTileCols given in Table A-1 of [HEVC] for
         the highest level.

         The value of max-tc MUST be in the range of MaxTileCols to 16 *
         MaxTileCols, inclusive, where MaxTileCols is given in Table A-1
         of [HEVC] for the highest level.

      max-fps:

         The value of max-fps is an integer indicating the maximum
         picture rate in units of pictures per 100 seconds that can be
         effectively processed by the receiver.  The max-fps parameter
         MAY be used to signal that the receiver has a constraint in
         that it is not capable of processing video effectively at the
         full picture rate that is implied by the highest level and,
         when present, one or more of the parameters max-lsr, max-lps,
         and max-br.

         The value of max-fps is not necessarily the picture rate at
         which the maximum picture size can be sent, it constitutes a
         constraint on maximum picture rate for all resolutions.

            Informative note: The max-fps parameter is semantically
            different from max-lsr, max-lps, max-cpb, max-dpb, max-br,
            max-tr, and max-tc in that max-fps is used to signal a
            constraint, lowering the maximum picture rate from what is
            implied by other parameters.

         The encoder MUST use a picture rate equal to or less than this
         value.  In cases where the max-fps parameter is absent, the
         encoder is free to choose any picture rate according to the
         highest level and any signaled optional parameters.

         The value of max-fps MUST be smaller than or equal to the full
         picture rate that is implied by the highest level and, when
         present, one or more of the parameters max-lsr, max-lps, and
         max-br.











      sprop-max-don-diff:

         If tx-mode is equal to "SRST" and there is no NAL unit naluA
         that is followed in transmission order by any NAL unit
         preceding naluA in decoding order (i.e., the transmission order
         of the NAL units is the same as the decoding order), the value
         of this parameter MUST be equal to 0.

         Otherwise, if tx-mode is equal to "MRST" or "MRMT", the
         decoding order of the NAL units of all the RTP streams is the
         same as the NAL unit transmission order and the NAL unit output
         order, the value of this parameter MUST be equal to either 0 or
         1.

         Otherwise, if tx-mode is equal to "MRST" or "MRMT" and the
         decoding order of the NAL units of all the RTP streams is the
         same as the NAL unit transmission order but not the same as the
         NAL unit output order, the value of this parameter MUST be
         equal to 1.

         Otherwise, this parameter specifies the maximum absolute
         difference between the decoding order number (i.e., AbsDon)
         values of any two NAL units naluA and naluB, where naluA
         follows naluB in decoding order and precedes naluB in
         transmission order.

         The value of sprop-max-don-diff MUST be an integer in the range
         of 0 to 32767, inclusive.

         When not present, the value of sprop-max-don-diff is inferred
         to be equal to 0.

      sprop-depack-buf-nalus:

         This parameter specifies the maximum number of NAL units that
         precede a NAL unit in transmission order and follow the NAL
         unit in decoding order.

         The value of sprop-depack-buf-nalus MUST be an integer in the
         range of 0 to 32767, inclusive.

         When not present, the value of sprop-depack-buf-nalus is
         inferred to be equal to 0.

         When sprop-max-don-diff is present and greater than 0, this
         parameter MUST be present and the value MUST be greater than 0.







      sprop-depack-buf-bytes:

         This parameter signals the required size of the de-
         packetization buffer in units of bytes.  The value of the
         parameter MUST be greater than or equal to the maximum buffer
         occupancy (in units of bytes) of the de-packetization buffer as
         specified in Section 6.

         The value of sprop-depack-buf-bytes MUST be an integer in the
         range of 0 to 4294967295, inclusive.

         When sprop-max-don-diff is present and greater than 0, this
         parameter MUST be present and the value MUST be greater than 0.
         When not present, the value of sprop-depack-buf-bytes is
         inferred to be equal to 0.

            Informative note: The value of sprop-depack-buf-bytes
            indicates the required size of the de-packetization buffer
            only.  When network jitter can occur, an appropriately sized
            jitter buffer has to be available as well.

      depack-buf-cap:

         This parameter signals the capabilities of a receiver
         implementation and indicates the amount of de-packetization
         buffer space in units of bytes that the receiver has available
         for reconstructing the NAL unit decoding order from NAL units
         carried in one or more RTP streams.  A receiver is able to
         handle any RTP stream, and all RTP streams the RTP stream
         depends on, when present, for which the value of the sprop-
         depack-buf-bytes parameter is smaller than or equal to this
         parameter.

         When not present, the value of depack-buf-cap is inferred to be
         equal to 4294967295.  The value of depack-buf-cap MUST be an
         integer in the range of 1 to 4294967295, inclusive.

            Informative note: depack-buf-cap indicates the maximum
            possible size of the de-packetization buffer of the receiver
            only, without allowing for network jitter.













      sprop-segmentation-id:

         This parameter MAY be used to signal the segmentation tools
         present in the bitstream and that can be used for
         parallelization.  The value of sprop-segmentation-id MUST be an
         integer in the range of 0 to 3, inclusive.  When not present,
         the value of sprop-segmentation-id is inferred to be equal to
         0.

         When sprop-segmentation-id is equal to 0, no information about
         the segmentation tools is provided.  When sprop-segmentation-id
         is equal to 1, it indicates that slices are present in the
         bitstream.  When sprop-segmentation-id is equal to 2, it
         indicates that tiles are present in the bitstream.  When sprop-
         segmentation-id is equal to 3, it indicates that WPP is used in
         the bitstream.

      sprop-spatial-segmentation-idc:

         A base16 [RFC4648] representation of the syntax element
         min_spatial_segmentation_idc as specified in [HEVC].  This
         parameter MAY be used to describe parallelization capabilities
         of the bitstream.

      dec-parallel-cap:

         This parameter MAY be used to indicate the decoder's additional
         decoding capabilities given the presence of tools enabling
         parallel decoding, such as slices, tiles, and WPP, in the
         bitstream.  The decoding capability of the decoder may vary
         with the setting of the parallel decoding tools present in the
         bitstream, e.g., the size of the tiles that are present in a
         bitstream.  Therefore, multiple capability points may be
         provided, each indicating the minimum required decoding
         capability that is associated with a parallelism requirement,
         which is a requirement on the bitstream that enables parallel
         decoding.

         Each capability point is defined as a combination of 1) a
         parallelism requirement, 2) a profile (determined by profile-
         space and profile-id), 3) a highest level, and 4) a maximum
         processing rate, a maximum picture size, and a maximum video
         bitrate that may be equal to or greater than that determined by
         the highest level.  The parameter's syntax in ABNF [RFC5234] is
         as follows:








         dec-parallel-cap = "dec-parallel-cap={" cap-point *(","
                            cap-point) "}"

         cap-point = ("w" / "t") ":" spatial-seg-idc 1*(";"
                      cap-parameter)

         spatial-seg-idc = 1*4DIGIT ; (1-4095)

         cap-parameter = tier-flag / level-id / max-lsr
                         / max-lps / max-br

         tier-flag = "tier-flag" EQ ("0" / "1")

         level-id  = "level-id" EQ 1*3DIGIT ; (0-255)

         max-lsr   = "max-lsr" EQ  1*20DIGIT ; (0-
         18,446,744,073,709,551,615)

         max-lps   = "max-lps" EQ 1*10DIGIT ; (0-4,294,967,295)

         max-br    = "max-br"  EQ 1*20DIGIT ; (0-
         18,446,744,073,709,551,615)

         EQ = "="

         The set of capability points expressed by the dec-parallel-cap
         parameter is enclosed in a pair of curly braces ("{}").  Each
         set of two consecutive capability points is separated by a
         comma (',').  Within each capability point, each set of two
         consecutive parameters, and, when present, their values, is
         separated by a semicolon (';').

         The profile of all capability points is determined by profile-
         space and profile-id, which are outside the dec-parallel-cap
         parameter.

         Each capability point starts with an indication of the
         parallelism requirement, which consists of a parallel tool
         type, which may be equal to 'w' or 't', and a decimal value of
         the spatial-seg-idc parameter.  When the type is 'w', the
         capability point is valid only for H.265 bitstreams with WPP in
         use, i.e., entropy_coding_sync_enabled_flag equal to 1.  When
         the type is 't', the capability point is valid only for H.265
         bitstreams with WPP not in use (i.e.,
         entropy_coding_sync_enabled_flag equal to 0).  The capability-
         point is valid only for H.265 bitstreams with
         min_spatial_segmentation_idc equal to or greater than spatial-
         seg-idc.





         After the parallelism requirement indication, each capability
         point continues with one or more pairs of parameter and value
         in any order for any of the following parameters:

            o tier-flag
            o level-id
            o max-lsr
            o max-lps
            o max-br

         At most, one occurrence of each of the above five parameters is
         allowed within each capability point.

         The values of dec-parallel-cap.tier-flag and dec-parallel-
         cap.level-id for a capability point indicate the highest level
         of the capability point.  The values of dec-parallel-cap.max-
         lsr, dec-parallel-cap.max-lps, and dec-parallel-cap.max-br for
         a capability point indicate the maximum processing rate in
         units of luma samples per second, the maximum picture size in
         units of luma samples, and the maximum video bitrate (in units
         of CpbBrVclFactor bits per second for the VCL HRD parameters
         and in units of CpbBrNalFactor bits per second for the NAL HRD
         parameters where CpbBrVclFactor and CpbBrNalFactor are defined
         in Section A.4 of [HEVC]).

         When not present, the value of dec-parallel-cap.tier-flag is
         inferred to be equal to the value of tier-flag outside the dec-
         parallel-cap parameter.  When not present, the value of dec-
         parallel-cap.level-id is inferred to be equal to the value of
         max-recv-level-id outside the dec-parallel-cap parameter.  When
         not present, the value of dec-parallel-cap.max-lsr, dec-
         parallel-cap.max-lps, or dec-parallel-cap.max-br is inferred to
         be equal to the value of max-lsr, max-lps, or max-br,
         respectively, outside the dec-parallel-cap parameter.

         The general decoding capability, expressed by the set of
         parameters outside of dec-parallel-cap, is defined as the
         capability point that is determined by the following
         combination of parameters: 1) the parallelism requirement
         corresponding to the value of sprop-segmentation-id equal to 0
         for a bitstream, 2) the profile determined by profile-space,
         profile-id, profile-compatibility-indicator, and interop-
         constraints, 3) the tier and the highest level determined by
         tier-flag and max-recv-level-id, and 4) the maximum processing
         rate, the maximum picture size, and the maximum video bitrate
         determined by the highest level.  The general decoding
         capability MUST NOT be included as one of the set of capability
         points in the dec-parallel-cap parameter.





         For example, the following parameters express the general
         decoding capability of 720p30 (Level 3.1) plus an additional
         decoding capability of 1080p30 (Level 4) given that the
         spatially largest tile or slice used in the bitstream is equal
         to or less than 1/3 of the picture size:

            a=fmtp:98 level-id=93;dec-parallel-cap={t:8;level- id=120}

         For another example, the following parameters express an
         additional decoding capability of 1080p30, using dec-parallel-
         cap.max-lsr and dec-parallel-cap.max-lps, given that WPP is
         used in the bitstream:

            a=fmtp:98 level-id=93;dec-parallel-cap={w:8;
                        max-lsr=62668800;max-lps=2088960}

            Informative note: When min_spatial_segmentation_idc is
            present in a bitstream and WPP is not used, [HEVC] specifies
            that there is no slice or no tile in the bitstream
            containing more than 4 * PicSizeInSamplesY / (
            min_spatial_segmentation_idc + 4 ) luma samples.

      include-dph:

         This parameter is used to indicate the capability and
         preference to utilize or include Decoded Picture Hash (DPH) SEI
         messages (see Section D.3.19 of [HEVC]) in the bitstream. DPH
         SEI messages can be used to detect picture corruption so the
         receiver can request picture repair, see Section 8.  The value
         is a comma-separated list of hash types that is supported or
         requested to be used, each hash type provided as an unsigned
         integer value (0-255), with the hash types listed from most
         preferred to the least preferred.  Example: "include-dph=0,2",
         which indicates the capability for MD5 (most preferred) and
         Checksum (less preferred).  If the parameter is not included or
         the value contains no hash types, then no capability to utilize
         DPH SEI messages is assumed.  Note that DPH SEI messages MAY
         still be included in the bitstream even when there is no
         declaration of capability to use them, as in general SEI
         messages do not affect the normative decoding process and
         decoders are allowed to ignore SEI messages.

   Encoding considerations:

      This type is only defined for transfer via RTP (RFC 3550).








   Security considerations:

      See Section 9 of RFC 7798.

   Published specification:

      Please refer to RFC 7798 and its Section 12.

   Additional information: None

   File extensions: none

   Macintosh file type code: none

   Object identifier or OID: none

   Person & email address to contact for further information:

      Ye-Kui Wang (yekui.wang@gmail.com)

   Intended usage: COMMON

   Author: See Authors' Addresses section of RFC 7798.

   Change controller:

      IETF Audio/Video Transport Payloads working group delegated from
      the IESG.
```
