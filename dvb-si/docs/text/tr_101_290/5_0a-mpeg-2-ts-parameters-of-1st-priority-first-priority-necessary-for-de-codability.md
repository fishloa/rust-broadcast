## Table 5.0a — MPEG-2 TS parameters of 1st priority (First priority: necessary for de-codability)
_§5.2.1 "First priority: necessary for de-codability (basic monitoring)", PDF pp. 22-22_

| No. | Indicator | Precondition | Reference |
|---|---|---|---|
| 1.1 | `TS_sync_loss` | Loss of synchronization with consideration of hysteresis parameters | ISO/IEC 13818-1 [i.1], clause 2.4.3.3 and annex G.1 |
| 1.2 | `Sync_byte_error` | Sync_byte not equal 0x47 | ISO/IEC 13818-1 [i.1], clause 2.4.3.3 |
| 1.3 | `PAT_error` | PID 0x0000 does not occur at least every 0,5 s<br>a PID 0x0000 does not contain a table_id 0x00 (i.e. a PAT)<br>Scrambling_control_field is not 00 for PID 0x0000 | ISO/IEC 13818-1 [i.1], clauses 2.4.4.3, 2.4.4.4 |
| 1.3.a | `PAT_error_2` (note 1) | Sections with table_id 0x00 do not occur at least every 0,5 s on PID 0x0000.<br>Section with table_id other than 0x00 found on PID 0x0000.<br>Scrambling_control_field is not 00 for PID 0x0000 | ETSI TS 101 154 [i.30], clause 4.1.7<br>ISO/IEC 13818-1 [i.1], clauses 2.4.4.3, 2.4.4.4 |
| 1.4 | `Continuity_count_error` | Incorrect packet order<br>a packet occurs more than twice<br>lost packet | ISO/IEC 13818-1 [i.1], clauses 2.4.3.2, 2.4.3.3 |
| 1.5 | `PMT_error` | Sections with table_id 0x02, (i.e. a PMT), do not occur at least every 0,5 s on the PID which is referred to in the PAT<br>Scrambling_control_field is not 00 for all PIDs containing sections with table_id 0x02 (i.e. a PMT) | ISO/IEC 13818-1 [i.1], clauses 2.4.4.3, 2.4.4.4, 2.4.4.8 |
| 1.5.a | `PMT_error_2` (note 2) | Sections with table_id 0x02, (i.e. a PMT), do not occur at least every 0,5 s on each program_map_PID which is referred to in the PAT<br>Scrambling_control_field is not 00 for all packets containing information of sections with table_id 0x02 (i.e. a PMT) on each program_map_PID which is referred to in the PAT | ETSI TS 101 154 [i.30], clause 4.1.7 (note 3)<br>ISO/IEC 13818-1 [i.1], clauses 2.4.4.3, 2.4.4.4, 2.4.4.8 |
| 1.6 | `PID_error` | Referred PID does not occur for a user specified period. | ISO/IEC 13818-1 [i.1], clause 2.4.4.8 |

- NOTE 1: Recommended for future implementations as a replacement of 1.3.
- NOTE 2: Recommended for future implementations as a replacement of 1.5; this excludes specifically network_PIDs.
- NOTE 3: In ETSI TS 101 154 [i.30], it is recommended that the interval between two sections should not exceed 100 ms. For many applications it may be sufficient to check that the interval is no longer than 0,5 s.

Accompanying text (§5.2.1, PDF pp. 22-23): for `TS_sync_loss`, "It is
proposed that five consecutive correct sync bytes (ISO/IEC 13818-1 [i.1],
clause G.1) should be sufficient for sync acquisition, and two or more
consecutive corrupted sync bytes should indicate sync loss."
`Sync_byte_error` "is set as soon as the correct sync byte (0x47) does not
appear after 188 or 204 bytes." For `PID_error`, "The user specified period
should not exceed 5 s for video or audio PIDs"; data services and audio
services with ISO 639 [i.17] language descriptor with type greater than '0'
should be excluded from this 5 s limit, and in principle a different user
specified period could be defined for each PID.

