## Related interval statements elsewhere in the document (hand-transcribed)

| Clause | Statement |
|---|---|
| §4.1.1 item d), PDF p. 13 | the SI stream will have at least 8 TS packets per 10 s carrying NIT data or NULL packets. This rule simplifies the replacement of the NIT at broadcast delivery system boundaries. |
| §4.1.5, PDF p. 17 | The TDT will be transmitted at least every 30 s. |
| §4.1.6, PDF p. 17 | (The TOT will be transmitted at) least every 30 seconds. |

NOTE: Clause 4.4 covers DVB SI tables only — TR 101 211 v1.9.1 does not specify
repetition rates for PAT, CAT or PMT, and it states no minimum interval between
sections. The 25 ms floor ("the minimum time interval between the arrival of the
last byte of a section to the first byte of the next transmitted section with the
same PID, table_id and table_id_extension and with the same or different
section_number shall be 25 ms", applicable to TSs with a total data rate of up to
100 Mbit/s) is in EN 300 468 §5.1.4.1, not in this document.

