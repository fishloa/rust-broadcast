## segmentation_descriptor() — §10.3.3, Table 20, PDF pp. 68-79

Optional extension to the time_signal() and splice_insert() commands that
allows segmentation messages to be sent in a time/video accurate method.
Shall only be used with the time_signal(), splice_insert() and splice_null()
commands. The time_signal() or splice_insert() message should be sent at
least once, a minimum of 4 seconds in advance of the signaled splice_time().
Devices that do not recognize a value in any field shall ignore the message
and take no action. Component Segmentation Mode
(`program_segmentation_flag == '0'`) is **deprecated**.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `segmentation_descriptor() {` |  |  |
| &nbsp;&nbsp;splice_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;identifier | 32 | uimsbf |
| &nbsp;&nbsp;segmentation_event_id | 32 | uimsbf |
| &nbsp;&nbsp;segmentation_event_cancel_indicator | 1 | bslbf |
| &nbsp;&nbsp;segmentation_event_id_compliance_indicator | 1 | bslbf |
| &nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;`if(segmentation_event_cancel_indicator == '0') {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;program_segmentation_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_duration_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;delivery_not_restricted_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if(delivery_not_restricted_flag == '0') {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;web_delivery_allowed_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;no_regional_blackout_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;archive_allowed_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;device_restrictions | 2 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`} else {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`if(program_segmentation_flag == '0') {` (deprecated) |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;component_count | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for(i=0;i<component_count;i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;component_tag | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;pts_offset | 33 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`if(segmentation_duration_flag == '1')` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;segmentation_duration | 40 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_upid_type | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_upid_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_upid() |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_type_id | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segment_num | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segments_expected | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if(segmentation_type_id == '0x34' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x30' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x32' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x36' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x38' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x3A' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x44' \|\|` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`segmentation_type_id == '0x46') {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;sub_segment_num | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;sub_segments_expected | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

> Transcription note: in the published PDF this conditional is typeset with
> two misprints — a stray closing parenthesis on the `'0x3A'` line
> (`== '0x3A') ||`) and the `8 / uimsbf` cells for `sub_segment_num`
> vertically drifted onto the `'0x38'` row (verified by word-coordinate
> inspection of PDF p. 69). The brace structure and the
> sub_segment_num/sub_segments_expected semantics (§10.3.3.1) make the
> intended reading above unambiguous: the condition closes after `'0x46'`,
> and both `sub_segment_num` and `sub_segments_expected` are 8-bit uimsbf.

Field semantics (§10.3.3.1):

- **splice_descriptor_tag** — shall be **0x02**.
- **descriptor_length** — length in bytes of the descriptor following this
  field. Note: `sub_segment_num` and `sub_segments_expected` can form an
  optional appendix to the segmentation descriptor; the presence or absence
  of this optional data block is determined by **descriptor_length**.
- **identifier** — shall be **0x43554549** (ASCII "CUEI").
- **segmentation_event_id** — a 32-bit unique segmentation event identifier.
  Only one occurrence of a given value shall be active at any one time (see
  §9.9.3 and §10.3.3.7).
- **segmentation_event_cancel_indicator** — 1-bit flag; when '1', a
  previously sent segmentation event identified by `segmentation_event_id`
  has been cancelled. The `segmentation_type_id` does not need to match
  between the original and cancelling messages; once cancelled, the
  `segmentation_event_id` may be reused for content identification or to
  start a new Segment.
- **segmentation_event_id_compliance_indicator** — when '0', the
  `segmentation_event_id` is compliant with §9.9.3; when '1', compliance is
  not specified.
- **program_segmentation_flag** — should be '1': Program Segmentation Mode,
  all PIDs/components of the program are to be segmented. '0' = Component
  Segmentation Mode (**deprecated**), each component to be segmented is
  listed separately. May be set to different states in different descriptor
  messages within a program.
- **segmentation_duration_flag** — should be '1', indicating the presence of
  the `segmentation_duration` field. The accuracy of the start time of this
  duration is constrained by the splice_command_type used (e.g. with
  splice_null() the precise position in the stream is not deterministic).
- **delivery_not_restricted_flag** — when '1', the next five bits are
  reserved. When '0', the following five bits
  (`web_delivery_allowed_flag` … `device_restrictions`) have the meanings
  below; they facilitate implementations that use out-of-scope methods to
  process and manage this Segment.
- **web_delivery_allowed_flag** — '1' = no restrictions with respect to web
  delivery of this Segment; '0' = restrictions related to web delivery are
  asserted.
- **no_regional_blackout_flag** — '1' = no regional blackout of this
  Segment; '0' = this Segment is restricted due to regional blackout rules.
- **archive_allowed_flag** — '1' = no assertion about recording this
  Segment; '0' = restrictions related to recording this Segment are
  asserted.
- **device_restrictions** — 2 bits, per Table 21 below; signals three
  pre-defined groups of devices whose population is independent and
  non-hierarchical (delivery/format of the group-defining messaging is out
  of scope).
- **component_count / component_tag** (Component mode, deprecated) — as in
  splice_insert(): count shall be ≥ 1 when `program_segmentation_flag` ==
  '0'; `component_tag` matches the PMT stream_identifier_descriptor() value
  and its presence denotes the presence of this component of the asset.
- **pts_offset** — 33-bit unsigned integer; an offset to be **added** to the
  `pts_time`, as modified by `pts_adjustment`, in the time_signal() message
  to obtain the intended splice time(s); zero = use `pts_time` without
  offset. If `time_specified_flag` = 0, or the command carrying this
  descriptor does not have a splice_time() field, this field shall be used
  to offset the derived immediate splice time.
- **segmentation_duration** — 40-bit unsigned integer; the duration of the
  Segment in ticks of the program's 90 kHz clock; may indicate when the
  Segment will be over and when the next segmentation message will occur.
  **Shall be 0 for end messages.**
- **segmentation_upid_type** — a value from Table 22 below.
- **segmentation_upid_length** — length in bytes of segmentation_upid() as
  indicated by Table 22; shall be set to **zero** if no segmentation_upid()
  is present. For UPID type MID() it reflects the total length of the nested
  UPID types structure.
- **segmentation_upid()** — contents and length determined by
  `segmentation_upid_type` and `segmentation_upid_length` (e.g. type 0x06
  ISAN with length 12 carries the ISAN identifier of the content this
  descriptor refers to).
- **segmentation_type_id** — 8 bits, one of the values in Table 23 below to
  designate the type of segmentation; all unused values are reserved. When
  `segmentation_type_id` is 0x01 (Content Identification), the value of
  `segmentation_upid_type` shall be non-zero. If `segmentation_upid_length`
  is zero, then `segmentation_type_id` shall be set to 0x00 (Not Indicated).
- **segment_num** — supports numbering Segments within a given collection of
  Segments (such as Chapters or Advertisements); when utilized, expected to
  reset to one at the beginning of a collection and to increment for each
  new Segment. Value as indicated in Table 23.
- **segments_expected** — a count of the expected number of individual
  Segments within the collection. Value as indicated in Table 23.
- **sub_segment_num** — optional, for the applicable segmentation_type_id
  values in Table 23; identifies a specific sub-Segment within a collection
  of sub-Segments, expected to be set to one for the first and to increment
  by one for each new sub-Segment. If present, `descriptor_length` shall
  include it in the byte count and serve as the indication that it is
  present in the descriptor.
- **sub_segments_expected** — shall be present if `sub_segment_num` is
  present; a count of the expected number of individual sub-Segments within
  the collection. Same `descriptor_length` rule as `sub_segment_num`.

### device_restrictions — §10.3.3.1, Table 21, PDF p. 73

| Segmentation Message | device_restrictions |
|---|---|
| Restrict Group 0 | 00 |
| Restrict Group 1 | 01 |
| Restrict Group 2 | 10 |
| None | 11 |

Restrict Group 0/1/2 — this Segment is restricted for a class of devices
defined by an out-of-band message that describes which devices are excluded.
None — this Segment has no device restrictions.

### segmentation_upid_type — §10.3.3.1, Table 22, PDF pp. 74-75

| segmentation_upid_type | segmentation_upid_length (bytes) | segmentation_upid() (Name) | Description |
|---|---|---|---|
| 0x00 | 0 | Not Used | The segmentation_upid is not defined and is not present in the descriptor. |
| 0x01 | variable | User Defined | **Deprecated: use type 0x0C**; the segmentation_upid does not follow a standard naming scheme. |
| 0x02 | 8 | ISCI | **Deprecated: use type 0x03**, 8 characters; 4 alpha characters followed by 4 numbers. |
| 0x03 | 12 | Ad-ID | Defined by the Advertising Digital Identification, LLC group. 12 characters; 4 alpha characters (company identification prefix) followed by 8 alphanumeric characters. (See [Ad-ID], [SMPTE 2092-1].) |
| 0x04 | 32 | UMID | See [SMPTE 330]. |
| 0x05 | 8 | ISAN | **Deprecated: use type 0x06**, ISO 15706 binary encoding. |
| 0x06 | 12 | ISAN | Formerly known as V-ISAN. ISO 15706-2 binary encoding ("versioned" ISAN). See [ISO 15706-2]. |
| 0x07 | 12 | TID | Tribune Media Systems Program identifier. 12 characters; 2 alpha characters followed by 10 numbers. |
| 0x08 | 8 | TI | AiringID (formerly Turner ID), used to indicate a specific airing of a Program that is unique within a network. |
| 0x09 | variable | ADI | CableLabs metadata identifier as defined in §10.3.3.2: `<element>:<providerID>/<assetID>`, where `<element>` is one of PREVIEW, MPEG2HD, MPEG2SD, AVCHD, AVCSD, HEVCSD, HEVCHD, SIGNAL, PO (PlacementOpportunity), BLACKOUT, BREAK, OTHER, in 7-bit printable ASCII (0x20–0x7E). |
| 0x0A | 12 | EIDR | An EIDR (see [EIDR]) represented in Compact Binary encoding as defined in Section 2.1.1 of the EIDR ID Format (see [EIDR ID FORMAT]). [SMPTE 2079] |
| 0x0B | variable | ATSC Content Identifier | ATSC_content_identifier() structure as defined in [ATSC A/57B]. |
| 0x0C | variable | MPU() | Managed Private UPID structure as defined in §10.3.3.3 (Table 24 below). |
| 0x0D | variable | MID() | Multiple UPID types structure as defined in §10.3.3.4 (Table 25 below). |
| 0x0E | variable | ADS Information | Advertising information as described in §10.3.3.5 (key=value pairs, e.g. `type=LA&dur=60&pos=90&tier=1`). |
| 0x0F | variable | URI | Universal Resource Identifier (see [RFC 3986]). |
| 0x10 | 16 | UUID | Universally Unique Identifier (see [RFC 4122]). This segmentation_upid_type can be used instead of a URI if it is desired to transfer the UUID payload only. |
| 0x11 | variable | SCR | Segment Content Reference as described in §10.3.3.6 (key=value pairs, e.g. `type=PI&tier=1`). |
| 0x12–0xFF | variable | Reserved | Reserved for future standardization. |

ADS (0x0E) and SCR (0x11) key-value conventions (§10.3.3.5 / §10.3.3.6):
key-value separator `=`, key-value delimiter `&`, serial separator `;`. ADS
keys: `type` (LA = Local Operator Availability, NA = National Addressable,
NR = National Replacement, CV = Creative Versioning, C3/C7 = Nielsen VOD
ratings credit for 3/7 days), `pos` (start point in milliseconds of the
referenced advertising relative to the start of the Segment), `dur`
(duration in milliseconds), `tier` (replacement authorization designation).

### segmentation_type_id — §10.3.3.1, Table 23, PDF pp. 76-79

| Segmentation Message | segmentation_type_id hex (decimal) | segment_num | segments_expected | sub_segment_num | sub_segments_expected |
|---|---|---|---|---|---|
| Not Indicated | 0x00 (00) | 0 | 0 | Not used | Not used |
| Content Identification | 0x01 (01) | 0 | 0 | Not used | Not used |
| Private | 0x02 (02) |  |  |  |  |
| Reserved | 0x03–0x0F |  |  |  |  |
| Program Start | 0x10 (16) | 1 | 1 | Not used | Not used |
| Program End | 0x11 (17) | 1 | 1 | Not used | Not used |
| Program Early Termination | 0x12 (18) | 1 | 1 | Not used | Not used |
| Program Breakaway | 0x13 (19) | 1 | 1 | Not used | Not used |
| Program Resumption | 0x14 (20) | 1 | 1 | Not used | Not used |
| Program Runover Planned | 0x15 (21) | 1 | 1 | Not used | Not used |
| Program Runover Unplanned | 0x16 (22) | 1 | 1 | Not used | Not used |
| Program Overlap Start | 0x17 (23) | 1 | 1 | Not used | Not used |
| Program Blackout Override | 0x18 (24) | 0 | 0 | Not used | Not used |
| Program Join | 0x19 (25) | 1 | 1 | Not used | Not used |
| Program Immediate Resumption | 0x1A (26) | 0 | 0 | Not used | Not used |
| Reserved | 0x1B–0x1F |  |  |  |  |
| Chapter Start | 0x20 (32) | Non-zero | Non-zero | Not used | Not used |
| Chapter End | 0x21 (33) | Non-zero | Non-zero | Not used | Not used |
| Break Start | 0x22 (34) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Break End | 0x23 (35) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Opening Credit Start (deprecated) | 0x24 (36) | 1 | 1 | Not used | Not used |
| Opening Credit End (deprecated) | 0x25 (37) | 1 | 1 | Not used | Not used |
| Closing Credit Start (deprecated) | 0x26 (38) | 1 | 1 | Not used | Not used |
| Closing Credit End (deprecated) | 0x27 (39) | 1 | 1 | Not used | Not used |
| Reserved | 0x28–0x2F |  |  |  |  |
| Provider Advertisement Start | 0x30 (48) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Provider Advertisement End | 0x31 (49) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Distributor Advertisement Start | 0x32 (50) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Distributor Advertisement End | 0x33 (51) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Provider Placement Opportunity Start | 0x34 (52) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Provider Placement Opportunity End | 0x35 (53) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Distributor Placement Opportunity Start | 0x36 (54) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Distributor Placement Opportunity End | 0x37 (55) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Provider Overlay Placement Opportunity Start | 0x38 (56) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Provider Overlay Placement Opportunity End | 0x39 (57) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Distributor Overlay Placement Opportunity Start | 0x3A (58) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Distributor Overlay Placement Opportunity End | 0x3B (59) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Provider Promo Start | 0x3C (60) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Provider Promo End | 0x3D (61) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Distributor Promo Start | 0x3E (62) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Distributor Promo End | 0x3F (63) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Unscheduled Event Start | 0x40 (64) | 0 | 0 | Not used | Not used |
| Unscheduled Event End | 0x41 (65) | 0 | 0 | Not used | Not used |
| Alternate Content Opportunity Start | 0x42 (66) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Alternate Content Opportunity End | 0x43 (67) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Provider Ad Block Start | 0x44 (68) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Provider Ad Block End | 0x45 (69) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Distributor Ad Block Start | 0x46 (70) | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero | 0 or Non-zero |
| Distributor Ad Block End | 0x47 (71) | 0 or Non-zero | 0 or Non-zero | Not used | Not used |
| Reserved | 0x48–0x4F |  |  |  |  |
| Network Start | 0x50 (80) | 0 | 0 | Not used | Not used |
| Network End | 0x51 (81) | 0 | 0 | Not used | Not used |
| Reserved | 0x52–0xFF |  |  |  |  |

Notes (from Table 23):

1. Only one Program Overlap Start is allowed to be active at a time. A
   Program End shall occur before a subsequent Program Overlap Start can
   occur.
2. See [SCTE 223] for further usage of segmentation_type_id.
3. The opening credit and closing credit start/end types are deprecated. New
   implementations should use the Segment Content Reference for this
   purpose.

(The final reserved row is printed as "0x52-0FF" in the PDF; the intended
range is 0x52–0xFF.)

### MPU() — §10.3.3.3, Table 24, PDF p. 80

Managed Private UPID, `segmentation_upid_type` 0x0C.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `MPU() {` |  |  |
| &nbsp;&nbsp;format_identifier | 32 | uimsbf |
| &nbsp;&nbsp;private_data | N*8 | uimsbf |
| `}` |  |  |

- **format_identifier** — a 32-bit unique identifier as defined in ISO/IEC
  13818-1 and registered with the SMPTE Registration Authority.
- **private_data** — a variable length, byte-aligned set of data as defined
  by the registered owner of the `format_identifier` value. The length is
  defined by `segmentation_upid_length`, **which includes the
  format_identifier field length**.

### MID() — §10.3.3.4, Table 25, PDF pp. 80-81

Multiple UPID types structure, `segmentation_upid_type` 0x0D.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `MID() {` |  |  |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_upid_type | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;segmentation_upid | N*8 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

- **segmentation_upid_type** — as defined in Table 22.
- **length** — segmentation_upid_length for the following
  segmentation_upid.
- **segmentation_upid** — segmentation_upid according to
  segmentation_upid_type as defined in Table 22.
- Note: the number of segmentation_upids present ("N") is not explicitly
  signaled; it is discovered by repeatedly parsing the fields above until
  the outer `segmentation_upid_length` is exhausted.

