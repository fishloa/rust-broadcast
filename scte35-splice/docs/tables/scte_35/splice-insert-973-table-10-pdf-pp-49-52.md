## splice_insert() — §9.7.3, Table 10, PDF pp. 49-52

Signals an upcoming splice event. Shall be sent at least once before each
legacy Splice Point; packets containing the entirety of the table shall
always precede the packet containing the related Splice Point. Component
Splice Mode is **deprecated**.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `splice_insert() {` |  |  |
| &nbsp;&nbsp;splice_event_id | 32 | uimsbf |
| &nbsp;&nbsp;splice_event_cancel_indicator | 1 | bslbf |
| &nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;`if(splice_event_cancel_indicator == '0') {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;out_of_network_indicator | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;program_splice_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;duration_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;splice_immediate_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;event_id_compliance_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if((program_splice_flag == '1') && (splice_immediate_flag == '0'))` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;splice_time() |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`if(program_splice_flag == '0') {` (deprecated) |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;component_count | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for(i=0;i<component_count;i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;component_tag | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if(splice_immediate_flag == '0')` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;splice_time() |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`if(duration_flag == '1')` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;break_duration() |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;unique_program_id | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;avail_num | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;avails_expected | 8 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

Field semantics (§9.7.3.1) — `splice_event_id`,
`splice_event_cancel_indicator`, `event_id_compliance_flag`,
`program_splice_flag`, `duration_flag`, `component_count`, `component_tag`,
`unique_program_id`, `avail_num`, `avails_expected` are as defined for
splice_schedule() above, except that times are PTS-based:

- **out_of_network_indicator** — '1' = opportunity to exit the network feed;
  the value of `splice_time()`, **as modified by `pts_adjustment`**, refers
  to an intended Out Point / Program Out Point. '0' = opportunity to return
  to the network feed; `splice_time()` as modified by `pts_adjustment`
  refers to an intended In Point / Program In Point.
- **splice_immediate_flag** — when '1', indicates the absence of the
  `splice_time()` field and that the splice mode shall be the Splice
  Immediate Mode, whereby the splicing device shall choose the nearest
  opportunity in the stream, relative to the splice information packet, to
  splice. When '0', indicates the presence of `splice_time()` in at least
  one location within the splice_insert() command.
- Timing constraints (§9.9.2.1): for a network Out Point at least one
  message shall arrive at least 4 seconds in advance of the signaled splice
  time. The intended Out Point shall be **before** the first presentation
  unit with presentation time ≥ the signaled `pts_time` as modified by
  `pts_adjustment`; the intended In Point shall **be** the first
  presentation unit with presentation time ≥ that value. In Component Splice
  Mode (deprecated), the first listed component shall carry a valid
  `pts_time` (the default pts_time), used by subsequent components that omit
  theirs.

