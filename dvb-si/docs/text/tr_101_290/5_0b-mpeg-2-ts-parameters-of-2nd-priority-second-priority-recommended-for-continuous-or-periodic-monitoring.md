## Table 5.0b — MPEG-2 TS parameters of 2nd priority (Second priority: recommended for continuous or periodic monitoring)
_§5.2.2 "Second priority: recommended for continuous or periodic monitoring", PDF pp. 23-24_

| No. | Indicator | Precondition | Reference |
|---|---|---|---|
| 2.1 | `Transport_error` | Transport_error_indicator in the TS-Header is set to "1" | ISO/IEC 13818-1 [i.1]: clauses 2.4.3.2, 2.4.3.3 |
| 2.2 | `CRC_error` | CRC error occurred in CAT, PAT, PMT, NIT, EIT, BAT, SDT or TOT table | ISO/IEC 13818-1 [i.1]: clauses 2.4.4, annex A<br>ETSI EN 300 468 [i.7]: clause 5.2 |
| 2.3 | `PCR_error` (see notes 1 and 2) | PCR discontinuity of more than 100 ms occurring without specific indication.<br>Time interval between two consecutive PCR values more than 100 ms | ISO/IEC 13818-1 [i.1]: clauses 2.4.3.4, 2.4.3.5<br>ISO/IEC 13818-4 [i.2]: clause 9.11.3<br>ETSI TS 101 154 [i.30]: clause 4.1.5.3 |
| 2.3a | `PCR_repetition_error` (see notes 1 and 2) | Time interval between two consecutive PCR values more than 100 ms | ETSI TS 101 154 [i.30]: clause 4.1.5.3 |
| 2.3b | `PCR_discontinuity_indicator_error` | The difference between two consecutive PCR values (PCR<sub>i+1</sub> – PCR<sub>i</sub>) is outside the range of 0...100 ms without the discontinuity_indicator set | ISO/IEC 13818-1 [i.1]: clauses 2.4.3.4, 2.4.3.5<br>ISO/IEC 13818-4 [i.2]: clause 9.1.1.3 |
| 2.4 | `PCR_accuracy_error` | PCR accuracy of selected programme is not within ±500 ns | ISO/IEC 13818-1 [i.1]: clause 2.4.2.2 |
| 2.5 | `PTS_error` (see note 3) | PTS repetition period more than 700 ms | ISO/IEC 13818-1 [i.1]: clauses 2.4.3.6, 2.4.3.7, 2.7.4 |
| 2.6 | `CAT_error` | Packets with transport_scrambling_control not 00 present, but no section with table_id = 0x01 (i.e. a CAT) present<br>Section with table_id other than 0x01 (i.e. not a CAT) found on PID 0x0001 | ISO/IEC 13818-1 [i.1]: clause 2.4.4 |

- NOTE 1: The old version of PCR_error (2.3) is a combination of the more specific errors PCR_repetition_error (2.3.a) and PCR_discontinuity_indicator_error (2.3.b) by a logical 'or' function. It is kept in the present document for reasons of consistency of existing implementations. For new implementations it is recommended that the indicators 2.3.a and 2.3.b are used only.
- NOTE 2: The limitation to 40 ms in the 'Preconditions' of 2.3 PCR_error and 2.3a PCR_repetition_error was removed from ETSI TS 101 154 [i.30] in 2005. The respective clause there now refers only to the 100 ms limitation in [i.1] which is recommended to be applied generally.
- NOTE 3: The limitation to 700 ms should not be applied to still pictures.

Accompanying text (§5.2.2, PDF p. 24): for `PCR_error` /
`PCR_repetition_error`, "In DVB a repetition period of not more than 100 ms
is permitted, previously a maximum of 40ms was recommended (see note 2 in
table 5.0b)." For `PCR_accuracy_error`, "The accuracy of ±500 ns is intended
to be sufficient for the colour subcarrier to be synthesized from system
clock. This test should only be performed on a constant bitrate TS as
defined in ISO/IEC 13818-1 [i.1] clause 2.4.2.2." For `PTS_error`, "The
Presentation Time Stamps (PTS) should occur at least every 700 ms"; they are
only accessible if the TS is not scrambled.

