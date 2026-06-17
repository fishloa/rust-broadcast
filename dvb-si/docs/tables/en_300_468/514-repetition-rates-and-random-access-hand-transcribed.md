## §5.1.4 — Repetition rates and random access (hand-transcribed)
_§5.1.4 / §5.1.4.1, PDF pp. 24-25_

Prose clause (outside the extractor's bit-syntax-table model), **hand-transcribed**
verbatim (2026-06-11) from the vendored PDF. This is the authoritative home of
the **25 ms inter-section floor** used by the dvb-si `SiMux` scheduler; the
per-table maximum intervals it defers to live in `tr_101_211.md` §4.4.

> **5.1.4.1 Rates for DVB PSI and SI** — In systems where acquisition time of
> PSI and SI in DVB transport streams is important, it is recommended to
> continuously re-transmit these sections at regular intervals, even when no
> changes occur. Clause 4.4 of ETSI TS 101 211 makes recommendations for how
> often PSI and SI sections should be re-transmitted.
>
> For SI specified within the present document the minimum time interval between
> the arrival of the last byte of a section to the first byte of the next
> transmitted section with the same PID, table_id and table_id_extension and
> with the same or different section_number **shall be 25 ms**. This limit
> applies for TSs with a total data rate of up to 100 Mbit/s.
>
> NOTE: These requirements do not apply to SAT.

