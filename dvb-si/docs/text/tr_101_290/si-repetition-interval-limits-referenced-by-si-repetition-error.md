## SI repetition-interval limits referenced by SI_repetition_error
_§5.2.3 (indicator 3.2 discussion), PDF p. 27; numeric values from Tables 5.0a/5.0c rows, PDF pp. 22, 25-26_

TR 101 290 v1.4.1 contains **no standalone numeric table** of SI section
intervals. The `SI_repetition_error` discussion (§5.2.3, PDF p. 27) reads,
verbatim:

> For SI tables a maximum and minimum periodicity are specified in
> ETSI EN 300 468 [i.7] and ETSI TR 101 211 [i.8]. This is checked for this
> indicator. This indicator should be set in addition to other indicators of
> repetition errors for specific tables.

The authoritative numeric interval table is therefore ETSI TR 101 211 §4.4
(vendored as `specs/etsi_tr_101_211_v01.09.01_dvb_si_guidelines.pdf`) with
ETSI EN 300 468 §5.1.4. The numeric limits that TR 101 290 itself states are
embedded in the indicator rows above; collected here for reference (each
value cited to its row):

| Table / item | Maximum interval (presence check) | Minimum interval (gap check) | Source row(s) |
|---|---|---|---|
| PAT (table_id 0x00, PID 0x0000) | 0,5 s (TS 101 154 recommends interval between two sections ≤ 100 ms — Table 5.0a note 3) | — | 1.3, 1.3.a |
| PMT (table_id 0x02, PID per PAT) | 0,5 s (TS 101 154 recommends ≤ 100 ms — Table 5.0a note 3) | — | 1.5, 1.5.a |
| NIT_actual (table_id 0x40, PID 0x0010) | 10 s | specified value (25 ms or lower) | 3.1, 3.1.a |
| NIT_other (table_id 0x41, PID 0x0010) | specified value (10 s or higher), same section_number | — | 3.1.b |
| SDT_actual (table_id 0x42, PID 0x0011) | 2 s | specified value (25 ms or lower) | 3.5, 3.5.a |
| SDT_other (table_id 0x46, PID 0x0011) | specified value (10s or higher), same section_number | — | 3.5.b |
| EIT-P/F actual (table_id 0x4E, PID 0x0012) | 2 s (each of section '0' (EIT-P) and section '1' (EIT-F)) | specified value (25 ms or lower) | 3.6, 3.6.a |
| EIT-P/F other (table_id 0x4F, PID 0x0012) | specified value (10 s or higher), sections '0' and '1' each | — | 3.6.b |
| RST (table_id 0x71, PID 0x0013) | — | specified value (25 ms or lower) | 3.7 |
| TDT (table_id 0x70, PID 0x0014) | 30 s | specified value (25 ms or lower) | 3.8 |

Related second-priority timing limits (Table 5.0b): PCR interval more than
100 ms (the earlier 40 ms limit was removed from TS 101 154 in 2005 — note 2);
PCR accuracy not within ±500 ns (2.4); PTS repetition period more than
700 ms (2.5, not applied to still pictures — note 3).
