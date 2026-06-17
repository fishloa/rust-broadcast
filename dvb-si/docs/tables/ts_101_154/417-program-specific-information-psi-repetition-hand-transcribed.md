## §4.1.7 — Program Specific Information (PSI) repetition (hand-transcribed)
_§4.1.7, PDF pp. 49-49 (cites Rec. ITU-T H.222.0 / ISO/IEC 13818-1 §2.4.4)_

The geometry-based extractor targets bit-syntax/value tables; §4.1.7 is prose,
so it is **hand-transcribed** here verbatim (2026-06-11) from the vendored PDF.
This is the authoritative source for the PAT/PMT **100 ms** repetition figure
(distinct from the 0,5 s monitoring ceiling in TR 101 290 §5.2.1 — see
`tr_101_290.md`).

> The Program Association Table (PAT) and Program Map Table (PMT) should be
> repeated with a maximum time interval of 100 ms between repetitions. In
> distribution applications, the maximum time interval between repetitions of
> each of these tables **shall be 100 ms**.

Reading for the dvb-si `SiMux` defaults: PAT/PMT carry **no repetition rate in
TR 101 211** (which covers DVB SI only) and **none in ISO/IEC 13818-1**
(§2.4.1 is general coding structure; the spec's only timing bounds are PCR
100 ms / SCR 700 ms). TS 101 154 §4.1.7 is therefore the tightest authoritative
mandate (a `shall` for distribution), and TR 101 290 §5.2.1 is the looser
monitoring ceiling (0,5 s). The SiMux default cites this clause.
