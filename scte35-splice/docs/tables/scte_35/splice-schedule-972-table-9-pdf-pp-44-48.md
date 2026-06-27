## splice_schedule() — §9.7.2, Table 9, PDF pp. 44-48

Allows a schedule of splice events to be conveyed in advance. Component
Splice Mode (`program_splice_flag == '0'`) is **deprecated**.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `splice_schedule() {` |  |  |
| &nbsp;&nbsp;splice_count | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<splice_count; i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;splice_event_id | 32 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;splice_event_cancel_indicator | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;event_id_compliance_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (splice_event_cancel_indicator == '0') {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;out_of_network_indicator | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;program_splice_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;duration_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if (program_splice_flag == '1')` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;utc_splice_time | 32 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if (program_splice_flag == '0') {` (deprecated) |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;component_count | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for(j=0;j<component_count;j++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;component_tag | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;utc_splice_time | 32 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if (duration_flag)` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;break_duration() |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;unique_program_id | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;avail_num | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;avails_expected | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

Field semantics (§9.7.2.1):

- **splice_count** — 8-bit unsigned integer; the number of splice events
  specified in the loop that follows.
- **splice_event_id** — a 32-bit unique splice event identifier (see Event
  id partitioning, §9.9.3, below).
- **splice_event_cancel_indicator** — 1-bit flag; when '1', a previously
  sent splice event identified by `splice_event_id` has been cancelled.
- **event_id_compliance_flag** — when '0', the `splice_event_id` value for
  each event in this command complies with §9.9.3; when '1', compliance is
  not specified.
- **out_of_network_indicator** — 1-bit flag; '1' = the splice event is an
  opportunity to exit from the network feed and `utc_splice_time` refers to
  an intended Out Point / Program Out Point; '0' = an opportunity to return
  to the network feed and `utc_splice_time` refers to an intended In Point /
  Program In Point.
- **program_splice_flag** — should be '1': Program Splice Mode, all
  PIDs/components of the program are spliced. '0' = Component Splice Mode
  (**deprecated**), each component to be spliced is listed separately.
- **duration_flag** — 1-bit flag indicating the presence of the
  `break_duration()` field.
- **utc_splice_time** — 32-bit unsigned integer: the time of the signaled
  splice event as the number of seconds since 00 hours UTC, January 6th,
  1980, **with** the count of intervening leap seconds included (i.e.
  GPS-epoch seconds; convertible to UTC without the System Time table's
  GPS_UTC_offset). Used only in the splice_schedule() command.
- **component_count** (deprecated) — 8-bit count of elementary PID stream
  entries in the loop; shall be ≥ 1 when `program_splice_flag` == '0'.
- **component_tag** (deprecated) — 8-bit value identifying the elementary
  PID stream containing the Splice Point; the same value as in the PMT
  stream_identifier_descriptor() for that stream.
- **unique_program_id** — should provide a unique identification for a
  viewing event within the service (see SCTE 118-2 / SCTE 224 for guidance).
- **avail_num** — (previously 'avail') identifies a specific avail within
  one `unique_program_id`; expected to reset to one for the first avail in a
  new viewing event and to increment for each new avail; may carry a zero
  value to indicate non-usage.
- **avails_expected** — (previously 'avail_count') the expected number of
  individual avails within the current viewing event; zero means `avail_num`
  has no meaning.

