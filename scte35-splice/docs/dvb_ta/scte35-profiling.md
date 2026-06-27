# SCTE 35 profiling for DVB-TA (constraints, NO new wire syntax)

_Source: ETSI TS 103 752-1 V1.2.1 §5.3.4–5.3.5 (PDF pp.14–17), render-verified_

This clause does **not define new wire structures**. It pins the values / usage
of fields that base ANSI/SCTE 35 [1] already defines and that `scte35-splice`
already parses (`splice_info_section()`, `splice_insert()`, `time_signal()`,
`segmentation_descriptor()`). Each item below is a constraint the typed DVB-TA
layer can enforce; the citation is included so an implementation can assert it.

The one **new** structure these clauses reference is the `DVB_DAS_descriptor()`
(§5.3.5.16) — transcribed separately in [`das-descriptor.md`](das-descriptor.md).

## §5.3.4 SCTE 35 section structure

- **§5.3.4.1 Section encryption** — sections may be encrypted
  (`encrypted_packet = 1`) or unencrypted (`= 0`). Encryption algorithm per
  SCTE 35 [1] Table 27. Decryption key (if needed) delivered out-of-band via the
  DAS application (out of scope here).
- **§5.3.4.2 Maximum section length** — SCTE 35 [1] constrains sections to start
  at the beginning of a TS-packet payload. DVB DAS sections may be up to **4 096
  bytes** (per SCTE 35) and may span multiple TS packets. The max is reduced when
  encapsulated in DSM-CC stream events (see
  [`dsmcc-stream-event.md`](dsmcc-stream-event.md): 180 / 178 byte limits).
- **§5.3.4.3 PTS adjustment field** — `pts_adjustment` may be used by message
  generation / re-multiplexing equipment; its value **shall be added** to the
  times in `pts_time` fields to give the correct time reference.

## §5.3.5 `segmentation_descriptor()` / `splice_insert()` content constraints

The descriptor may be of type **DPO, PPO, distributor advertisement, or provider
advertisement** (§5.3.5.1). Differing field names for the same function across the
two methods are given together below.

| § | Field(s) | DVB-TA constraint |
|---|----------|-------------------|
| 5.3.5.2 | `segmentation_event_id` / `splice_event_id` | identifier for the signalled point in time; usable by the DAS application |
| 5.3.5.3 | `segmentation_event_cancel_indicator` / `splice_event_cancel_indicator` | **shall be `0`** — event cancellation is not permitted for DVB DAS |
| 5.3.5.4 | DPO/PPO start & end messages / `out_of_network_indicator` | both start and end messages should be signalled per SCTE 35; the End message conveys no extra info (end time = start time + duration). **Applicable PO `segmentation_type_id` values are `0x34, 0x35, 0x36, 0x37`.** A `splice_insert()` with `out_of_network_indicator = 1` is equivalent to a PPO/DPO **start** message; for `splice_insert()` it is **recommended only `out_of_network_indicator = 1` messages are used** |
| 5.3.5.5 | `segmentation_duration_flag` / `duration_flag` | **shall be `1`** (duration specified) — not applicable to End messages |
| 5.3.5.6 | `splice_immediate_flag` (`splice_insert()` only) | **shall be `0`** — splice-immediate mode not permitted for DVB DAS |
| 5.3.5.7 | `time_specified_flag` | **shall be `1`** in the `splice_time()` structure — time always specified |
| 5.3.5.8 | `pts_time` | the `splice_time()` `pts_time` **shall contain a PTS** giving frame-accurate boundary info. Boundary is immediately prior to the presentation unit whose presentation time most closely matches the signalled PTS (PTS value = signalled `pts_time` + `pts_adjustment`). NOTE: for a Start message the PTS refers to the **first frame of the segment**; for an End message to the **first frame after the segment** (aligned with SCTE 35 In/Out Points) |
| 5.3.5.9 | `auto_return` (`splice_insert()` only) | **shall be `1`** — a `splice_insert()` with `out_of_network_indicator = 0` at the end of the PO is not required |
| 5.3.5.10 | `segmentation_upid_type` (`segmentation_descriptor()` only) | **shall be `0x0F`** — indicates `segmentation_upid()` contains a URI (IETF RFC 3986 [3]) |
| 5.3.5.11 | `unique_program_id` / `segmentation_upid()` | identify a content instance or a collection of segments. `unique_program_id` (in `splice_insert()`) is a **16-bit** field; `segmentation_upid` (in `segmentation_descriptor()`) is **variable-length**, typed by `segmentation_upid_type`. UPID **shall conform to URI format** (RFC 3986 [3]): `urn:<reverse-domain-name-of-broadcaster>:<identifier>`. `<identifier>` defined by the broadcaster; recommended to be an Airing ID as 16 hex characters. Examples: `urn:com.broadcaster:112210F47DE98115`, `urn:tv.acme:B637643-50A9-4C2D-BC7B-09FD8312190F` |
| 5.3.5.12 | `sub_segment_num` / `sub_segments_expected` (PPO/DPO) | convey position of the PO and number of POs expected within the break; usable by the DAS application |
| 5.3.5.13 | `segment_num` / `segments_expected` (PA/DA) | convey position of the advertisement and number of advertisements expected within the break |
| 5.3.5.14 | `avail_num` / `avails_expected` (`splice_insert()`) | convey position of the PO and number of POs expected within the break |
| 5.3.5.15 | `segment_num` / `segments_expected` (DPO/PPO) | convey number of the break within the programme and total breaks expected within the programme. **No equivalent in `splice_insert()` per SCTE 35** (the `DVB_DAS_descriptor()` `break_num`/`breaks_expected` provide it — see [`das-descriptor.md`](das-descriptor.md)) |

### Segmentation-type IDs for placement opportunities

§5.3.5.4 fixes the four base-SCTE 35 `segmentation_type_id` values used for POs
(start/end pairs). These are base SCTE 35 codes, not new — the DVB profile simply
mandates their use:

| `segmentation_type_id` | Meaning (SCTE 35) |
|------------------------|-------------------|
| `0x34` | Provider Placement Opportunity Start |
| `0x35` | Provider Placement Opportunity End |
| `0x36` | Distributor Placement Opportunity Start |
| `0x37` | Distributor Placement Opportunity End |

⚠ §5.3.5.4 names "`0x34, 0x35, 0x36, 0x37`" as "the applicable
`segmentation_type_id` values for POs" without spelling out the Start/End/Provider/
Distributor label of each individually; the labels above are the standard SCTE 35
assignments for that contiguous block (Provider PO Start/End = 0x34/0x35,
Distributor PO Start/End = 0x36/0x37). Confirm against SCTE 35 [1] Table 22 (which
`scte35-splice` already encodes) rather than treating these labels as set by
TS 103 752-1.

## Partial replacement (§5.3.3) — signalling pattern, not new syntax

- **time_signal() method (§5.3.3.2):** signal the start/end boundaries of
  individually replaceable segments within a PO using **Advertisement Start/End
  `segmentation_descriptor`s** in a `time_signal()` structure.
- **splice_insert() method (§5.3.3.3):** signal starts and durations of the
  individually replaceable segments using `splice_insert()` structures (e.g. three
  `splice_insert()`s for a 2-ad PO: one whole-PO + one per commercial).
