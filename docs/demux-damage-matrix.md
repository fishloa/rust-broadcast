# Demux damage matrix

The measurement for the best-effort-demux lane (rz1): one committed real
fixture per format, damaged at named positions, demuxed again, and compared
with the undamaged run. `transmux/tests/it/damage_matrix.rs` is the harness;
`test-harness/src/damage.rs` applies the damage and compares the outcomes.

## Baseline (`origin/main`)

Per damage class: how many cells errored, lost the tail (more than two clean
samples neither kept before nor after the damage), or were non-monotonic.

| format | damage | cells | errored | lost tail | non-monotonic | ok |
|---|---|---:|---:|---:|---:|---:|
| flv | delete | 11 | 1 | 1 | 0 | 10 |
| flv | field-max | 1 | 0 | 1 | 0 | 0 |
| flv | field-past | 1 | 0 | 1 | 0 | 0 |
| flv | field-zero | 1 | 0 | 0 | 0 | 1 |
| flv | insert | 11 | 1 | 1 | 0 | 10 |
| flv | random | 11 | 1 | 1 | 0 | 10 |
| flv | truncate | 11 | 1 | 0 | 0 | 10 |
| flv | zero | 11 | 1 | 1 | 0 | 10 |
| mkv | delete | 11 | 10 | 10 | 1 | 1 |
| mkv | field-max | 1 | 0 | 0 | 1 | 1 |
| mkv | field-past | 1 | 0 | 1 | 1 | 0 |
| mkv | field-zero | 1 | 0 | 1 | 1 | 0 |
| mkv | insert | 11 | 2 | 2 | 9 | 9 |
| mkv | random | 11 | 0 | 1 | 10 | 10 |
| mkv | truncate | 11 | 0 | 0 | 0 | 11 |
| mkv | zero | 11 | 1 | 1 | 10 | 10 |
| ps | delete | 11 | 0 | 9 | 5 | 2 |
| ps | field-max | 1 | 0 | 1 | 0 | 0 |
| ps | field-past | 1 | 0 | 1 | 0 | 0 |
| ps | field-zero | 1 | 0 | 0 | 0 | 1 |
| ps | insert | 11 | 0 | 8 | 5 | 3 |
| ps | random | 11 | 0 | 4 | 3 | 7 |
| ps | truncate | 11 | 0 | 0 | 0 | 11 |
| ps | zero | 11 | 0 | 4 | 3 | 7 |
| ts | delete | 11 | 0 | 4 | 0 | 7 |
| ts | field-max | 1 | 0 | 0 | 0 | 1 |
| ts | field-past | 1 | 0 | 0 | 0 | 1 |
| ts | field-zero | 1 | 0 | 0 | 0 | 1 |
| ts | insert | 11 | 0 | 1 | 0 | 10 |
| ts | random | 11 | 0 | 0 | 0 | 11 |
| ts | truncate | 11 | 0 | 0 | 0 | 11 |
| ts | zero | 11 | 0 | 0 | 0 | 11 |

## Baseline cells, verbatim

```

## ts clean samples=206 tracks=2 findings=0 error=None
ts	start	truncate	at=0	err=None	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
ts	start	zero	at=0	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=true	findings=1
ts	start	random	at=0	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=true	findings=1
ts	start	delete	at=0	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=true	findings=1
ts	start	insert	at=0	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=true	findings=1
ts	first-unit	truncate	at=8253	err=None	del=8	before=8	after=0	lost=198	dropped_tracks=1	extra=0	mono=true	findings=1
ts	first-unit	zero	at=8253	err=None	del=206	before=77	after=128	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	first-unit	random	at=8253	err=None	del=206	before=77	after=128	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	first-unit	delete	at=8253	err=None	del=196	before=75	after=121	lost=10	dropped_tracks=0	extra=0	mono=true	findings=2
ts	first-unit	insert	at=8253	err=None	del=206	before=77	after=128	lost=1	dropped_tracks=0	extra=1	mono=true	findings=1
ts	mid	truncate	at=41266	err=None	del=87	before=87	after=0	lost=119	dropped_tracks=0	extra=0	mono=true	findings=1
ts	mid	zero	at=41266	err=None	del=206	before=134	after=71	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	mid	random	at=41266	err=None	del=206	before=134	after=71	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	mid	delete	at=41266	err=None	del=206	before=134	after=71	lost=1	dropped_tracks=0	extra=1	mono=true	findings=2
ts	mid	insert	at=41266	err=None	del=206	before=134	after=71	lost=1	dropped_tracks=0	extra=1	mono=true	findings=1
ts	last-unit	truncate	at=74279	err=None	del=180	before=180	after=0	lost=26	dropped_tracks=0	extra=0	mono=true	findings=1
ts	last-unit	zero	at=74279	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=true	findings=0
ts	last-unit	random	at=74279	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=true	findings=0
ts	last-unit	delete	at=74279	err=None	del=205	before=201	after=4	lost=1	dropped_tracks=0	extra=0	mono=true	findings=2
ts	last-unit	insert	at=74279	err=None	del=205	before=201	after=4	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
ts	stride1	truncate	at=10316	err=None	del=8	before=8	after=0	lost=198	dropped_tracks=1	extra=0	mono=true	findings=1
ts	stride1	zero	at=10316	err=None	del=206	before=84	after=121	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	stride1	random	at=10316	err=None	del=206	before=84	after=121	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	stride1	delete	at=10316	err=None	del=205	before=17	after=187	lost=2	dropped_tracks=0	extra=1	mono=true	findings=2
ts	stride1	insert	at=10316	err=None	del=206	before=84	after=121	lost=1	dropped_tracks=0	extra=1	mono=true	findings=1
ts	stride2	truncate	at=20633	err=None	del=39	before=39	after=0	lost=167	dropped_tracks=0	extra=0	mono=true	findings=1
ts	stride2	zero	at=20633	err=None	del=206	before=103	after=102	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	stride2	random	at=20633	err=None	del=206	before=103	after=102	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	stride2	delete	at=20633	err=None	del=196	before=95	after=101	lost=10	dropped_tracks=0	extra=0	mono=true	findings=2
ts	stride2	insert	at=20633	err=None	del=206	before=103	after=102	lost=1	dropped_tracks=0	extra=1	mono=true	findings=1
ts	stride3	truncate	at=30949	err=None	del=66	before=66	after=0	lost=140	dropped_tracks=0	extra=0	mono=true	findings=1
ts	stride3	zero	at=30949	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=true	findings=0
ts	stride3	random	at=30949	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=true	findings=0
ts	stride3	delete	at=30949	err=None	del=204	before=156	after=48	lost=2	dropped_tracks=0	extra=0	mono=true	findings=2
ts	stride3	insert	at=30949	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=true	findings=1
ts	stride4	truncate	at=41266	err=None	del=87	before=87	after=0	lost=119	dropped_tracks=0	extra=0	mono=true	findings=1
ts	stride4	zero	at=41266	err=None	del=206	before=134	after=71	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	stride4	random	at=41266	err=None	del=206	before=134	after=71	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	stride4	delete	at=41266	err=None	del=206	before=134	after=71	lost=1	dropped_tracks=0	extra=1	mono=true	findings=2
ts	stride4	insert	at=41266	err=None	del=206	before=134	after=71	lost=1	dropped_tracks=0	extra=1	mono=true	findings=1
ts	stride5	truncate	at=51582	err=None	del=118	before=118	after=0	lost=88	dropped_tracks=0	extra=0	mono=true	findings=1
ts	stride5	zero	at=51582	err=None	del=206	before=152	after=53	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	stride5	random	at=51582	err=None	del=206	before=152	after=53	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	stride5	delete	at=51582	err=None	del=196	before=145	after=51	lost=10	dropped_tracks=0	extra=0	mono=true	findings=2
ts	stride5	insert	at=51582	err=None	del=205	before=152	after=51	lost=3	dropped_tracks=0	extra=2	mono=true	findings=1
ts	stride6	truncate	at=61899	err=None	del=145	before=145	after=0	lost=61	dropped_tracks=0	extra=0	mono=true	findings=1
ts	stride6	zero	at=61899	err=None	del=206	before=186	after=19	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	stride6	random	at=61899	err=None	del=206	before=186	after=19	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	stride6	delete	at=61899	err=None	del=205	before=186	after=19	lost=1	dropped_tracks=0	extra=0	mono=true	findings=2
ts	stride6	insert	at=61899	err=None	del=206	before=186	after=19	lost=1	dropped_tracks=0	extra=1	mono=true	findings=1
ts	stride7	truncate	at=72215	err=None	del=166	before=166	after=0	lost=40	dropped_tracks=0	extra=0	mono=true	findings=1
ts	stride7	zero	at=72215	err=None	del=206	before=183	after=22	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	stride7	random	at=72215	err=None	del=206	before=183	after=22	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ts	stride7	delete	at=72215	err=None	del=196	before=175	after=21	lost=10	dropped_tracks=0	extra=0	mono=true	findings=2
ts	stride7	insert	at=72215	err=None	del=206	before=183	after=22	lost=1	dropped_tracks=0	extra=1	mono=true	findings=1
ts	mid	field-zero	at=41878	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=true	findings=0
ts	mid	field-max	at=41878	err=None	del=205	before=168	after=37	lost=1	dropped_tracks=0	extra=0	mono=true	findings=0
ts	mid	field-past	at=41878	err=None	del=205	before=168	after=37	lost=1	dropped_tracks=0	extra=0	mono=true	findings=0

## ps clean samples=163 tracks=2 findings=0 error=None
ps	start	truncate	at=0	err=None	del=0	before=0	after=0	lost=163	dropped_tracks=2	extra=0	mono=true	findings=0
ps	start	zero	at=0	err=None	del=162	before=88	after=74	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
ps	start	random	at=0	err=None	del=162	before=88	after=74	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
ps	start	delete	at=0	err=None	del=162	before=88	after=74	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
ps	start	insert	at=0	err=None	del=163	before=163	after=0	lost=0	dropped_tracks=0	extra=0	mono=true	findings=1
ps	first-unit	truncate	at=6144	err=None	del=6	before=5	after=0	lost=158	dropped_tracks=0	extra=1	mono=true	findings=0
ps	first-unit	zero	at=6144	err=None	del=150	before=88	after=61	lost=14	dropped_tracks=0	extra=1	mono=false	findings=1
ps	first-unit	random	at=6144	err=None	del=150	before=88	after=61	lost=14	dropped_tracks=0	extra=1	mono=false	findings=1
ps	first-unit	delete	at=6144	err=None	del=150	before=88	after=61	lost=14	dropped_tracks=0	extra=1	mono=false	findings=1
ps	first-unit	insert	at=6144	err=None	del=162	before=88	after=73	lost=2	dropped_tracks=0	extra=1	mono=false	findings=1
ps	mid	truncate	at=30720	err=None	del=84	before=83	after=0	lost=80	dropped_tracks=0	extra=1	mono=true	findings=0
ps	mid	zero	at=30720	err=None	del=150	before=75	after=73	lost=15	dropped_tracks=0	extra=2	mono=false	findings=1
ps	mid	random	at=30720	err=None	del=150	before=75	after=73	lost=15	dropped_tracks=0	extra=2	mono=false	findings=1
ps	mid	delete	at=30720	err=None	del=150	before=75	after=73	lost=15	dropped_tracks=0	extra=2	mono=false	findings=1
ps	mid	insert	at=30720	err=None	del=155	before=130	after=24	lost=9	dropped_tracks=0	extra=1	mono=false	findings=1
ps	last-unit	truncate	at=55296	err=None	del=152	before=152	after=0	lost=11	dropped_tracks=0	extra=0	mono=true	findings=0
ps	last-unit	zero	at=55296	err=None	del=153	before=147	after=5	lost=11	dropped_tracks=0	extra=1	mono=true	findings=1
ps	last-unit	random	at=55296	err=None	del=153	before=147	after=5	lost=11	dropped_tracks=0	extra=1	mono=true	findings=1
ps	last-unit	delete	at=55296	err=None	del=153	before=147	after=5	lost=11	dropped_tracks=0	extra=1	mono=true	findings=1
ps	last-unit	insert	at=55296	err=None	del=158	before=147	after=10	lost=6	dropped_tracks=0	extra=1	mono=true	findings=1
ps	stride1	truncate	at=7680	err=None	del=6	before=5	after=0	lost=158	dropped_tracks=0	extra=1	mono=true	findings=1
ps	stride1	zero	at=7680	err=None	del=163	before=97	after=65	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ps	stride1	random	at=7680	err=None	del=163	before=97	after=65	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ps	stride1	delete	at=7680	err=None	del=151	before=89	after=61	lost=13	dropped_tracks=0	extra=1	mono=false	findings=1
ps	stride1	insert	at=7680	err=None	del=151	before=89	after=61	lost=13	dropped_tracks=0	extra=1	mono=false	findings=1
ps	stride2	truncate	at=15360	err=None	del=40	before=39	after=0	lost=124	dropped_tracks=0	extra=1	mono=true	findings=1
ps	stride2	zero	at=15360	err=None	del=163	before=113	after=49	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ps	stride2	random	at=15360	err=None	del=163	before=113	after=49	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ps	stride2	delete	at=15360	err=None	del=163	before=113	after=49	lost=1	dropped_tracks=0	extra=1	mono=true	findings=1
ps	stride2	insert	at=15360	err=None	del=163	before=113	after=49	lost=1	dropped_tracks=0	extra=1	mono=true	findings=1
ps	stride3	truncate	at=23040	err=None	del=62	before=61	after=0	lost=102	dropped_tracks=0	extra=1	mono=true	findings=1
ps	stride3	zero	at=23040	err=None	del=163	before=95	after=67	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ps	stride3	random	at=23040	err=None	del=163	before=95	after=67	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ps	stride3	delete	at=23040	err=None	del=158	before=94	after=63	lost=6	dropped_tracks=0	extra=1	mono=true	findings=1
ps	stride3	insert	at=23040	err=None	del=158	before=94	after=63	lost=6	dropped_tracks=0	extra=1	mono=true	findings=1
ps	stride4	truncate	at=30720	err=None	del=84	before=83	after=0	lost=80	dropped_tracks=0	extra=1	mono=true	findings=0
ps	stride4	zero	at=30720	err=None	del=150	before=75	after=73	lost=15	dropped_tracks=0	extra=2	mono=false	findings=1
ps	stride4	random	at=30720	err=None	del=150	before=75	after=73	lost=15	dropped_tracks=0	extra=2	mono=false	findings=1
ps	stride4	delete	at=30720	err=None	del=150	before=75	after=73	lost=15	dropped_tracks=0	extra=2	mono=false	findings=1
ps	stride4	insert	at=30720	err=None	del=155	before=130	after=24	lost=9	dropped_tracks=0	extra=1	mono=false	findings=1
ps	stride5	truncate	at=38400	err=None	del=94	before=93	after=0	lost=70	dropped_tracks=0	extra=1	mono=true	findings=1
ps	stride5	zero	at=38400	err=None	del=163	before=147	after=15	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ps	stride5	random	at=38400	err=None	del=163	before=147	after=15	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ps	stride5	delete	at=38400	err=None	del=152	before=138	after=13	lost=12	dropped_tracks=0	extra=1	mono=false	findings=1
ps	stride5	insert	at=38400	err=None	del=152	before=138	after=13	lost=12	dropped_tracks=0	extra=1	mono=false	findings=1
ps	stride6	truncate	at=46080	err=None	del=119	before=119	after=0	lost=44	dropped_tracks=0	extra=0	mono=true	findings=1
ps	stride6	zero	at=46080	err=None	del=163	before=155	after=7	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ps	stride6	random	at=46080	err=None	del=163	before=155	after=7	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ps	stride6	delete	at=46080	err=None	del=150	before=150	after=0	lost=13	dropped_tracks=0	extra=0	mono=true	findings=1
ps	stride6	insert	at=46080	err=None	del=150	before=150	after=0	lost=13	dropped_tracks=0	extra=0	mono=true	findings=1
ps	stride7	truncate	at=53760	err=None	del=147	before=147	after=0	lost=16	dropped_tracks=0	extra=0	mono=true	findings=1
ps	stride7	zero	at=53760	err=None	del=163	before=148	after=14	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ps	stride7	random	at=53760	err=None	del=163	before=148	after=14	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
ps	stride7	delete	at=53760	err=None	del=158	before=147	after=10	lost=6	dropped_tracks=0	extra=1	mono=true	findings=1
ps	stride7	insert	at=53760	err=None	del=158	before=147	after=10	lost=6	dropped_tracks=0	extra=1	mono=true	findings=1
ps	mid	field-zero	at=30738	err=None	del=163	before=163	after=0	lost=0	dropped_tracks=0	extra=0	mono=true	findings=0
ps	mid	field-max	at=30738	err=None	del=84	before=83	after=0	lost=80	dropped_tracks=0	extra=1	mono=true	findings=1
ps	mid	field-past	at=30738	err=None	del=84	before=83	after=0	lost=80	dropped_tracks=0	extra=1	mono=true	findings=1

## flv clean samples=206 tracks=2 findings=0 error=None
flv	start	truncate	at=0	err=Some("FLV codec config: buffer too short: need 9 bytes, have 0 (while parsing FLV header)")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
flv	start	zero	at=0	err=Some("bad FLV signature: [00, 00, 00] (expected \"FLV\")")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
flv	start	random	at=0	err=Some("bad FLV signature: [C8, 53, D8] (expected \"FLV\")")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
flv	start	delete	at=0	err=Some("bad FLV signature: [25, 00, 00] (expected \"FLV\")")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
flv	start	insert	at=0	err=Some("bad FLV signature: [95, CF, 99] (expected \"FLV\")")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
flv	first-unit	truncate	at=6247	err=None	del=8	before=8	after=0	lost=198	dropped_tracks=0	extra=0	mono=true	findings=0
flv	first-unit	zero	at=6247	err=None	del=206	before=79	after=126	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	first-unit	random	at=6247	err=None	del=206	before=79	after=126	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	first-unit	delete	at=6247	err=None	del=205	before=79	after=126	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	first-unit	insert	at=6247	err=None	del=205	before=79	after=126	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	mid	truncate	at=31237	err=None	del=92	before=92	after=0	lost=114	dropped_tracks=0	extra=0	mono=true	findings=0
flv	mid	zero	at=31237	err=None	del=206	before=132	after=73	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	mid	random	at=31237	err=None	del=206	before=132	after=73	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	mid	delete	at=31237	err=None	del=205	before=132	after=73	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	mid	insert	at=31237	err=None	del=205	before=132	after=73	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	last-unit	truncate	at=56227	err=None	del=180	before=180	after=0	lost=26	dropped_tracks=0	extra=0	mono=true	findings=0
flv	last-unit	zero	at=56227	err=None	del=206	before=198	after=7	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	last-unit	random	at=56227	err=None	del=206	before=198	after=7	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	last-unit	delete	at=56227	err=None	del=205	before=198	after=7	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	last-unit	insert	at=56227	err=None	del=205	before=198	after=7	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride1	truncate	at=7809	err=None	del=13	before=13	after=0	lost=193	dropped_tracks=0	extra=0	mono=true	findings=0
flv	stride1	zero	at=7809	err=None	del=206	before=82	after=123	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	stride1	random	at=7809	err=None	del=206	before=82	after=123	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	stride1	delete	at=7809	err=None	del=205	before=82	after=123	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride1	insert	at=7809	err=None	del=205	before=82	after=123	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride2	truncate	at=15618	err=None	del=45	before=45	after=0	lost=161	dropped_tracks=0	extra=0	mono=true	findings=0
flv	stride2	zero	at=15618	err=None	del=205	before=102	after=103	lost=1	dropped_tracks=0	extra=0	mono=true	findings=0
flv	stride2	random	at=15618	err=None	del=205	before=102	after=103	lost=1	dropped_tracks=0	extra=0	mono=true	findings=0
flv	stride2	delete	at=15618	err=None	del=205	before=102	after=103	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride2	insert	at=15618	err=None	del=205	before=102	after=103	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride3	truncate	at=23427	err=None	del=66	before=66	after=0	lost=140	dropped_tracks=0	extra=0	mono=true	findings=0
flv	stride3	zero	at=23427	err=None	del=206	before=156	after=49	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	stride3	random	at=23427	err=None	del=206	before=156	after=49	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	stride3	delete	at=23427	err=None	del=205	before=156	after=49	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride3	insert	at=23427	err=None	del=205	before=156	after=49	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride4	truncate	at=31237	err=None	del=92	before=92	after=0	lost=114	dropped_tracks=0	extra=0	mono=true	findings=0
flv	stride4	zero	at=31237	err=None	del=206	before=132	after=73	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	stride4	random	at=31237	err=None	del=206	before=132	after=73	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	stride4	delete	at=31237	err=None	del=205	before=132	after=73	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride4	insert	at=31237	err=None	del=205	before=132	after=73	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride5	truncate	at=39046	err=None	del=124	before=124	after=0	lost=82	dropped_tracks=0	extra=0	mono=true	findings=0
flv	stride5	zero	at=39046	err=None	del=206	before=152	after=53	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	stride5	random	at=39046	err=None	del=206	before=152	after=53	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	stride5	delete	at=39046	err=None	del=205	before=152	after=53	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride5	insert	at=39046	err=None	del=205	before=152	after=53	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride6	truncate	at=46855	err=None	del=141	before=141	after=0	lost=65	dropped_tracks=0	extra=0	mono=true	findings=0
flv	stride6	zero	at=46855	err=None	del=206	before=163	after=42	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	stride6	random	at=46855	err=None	del=206	before=163	after=42	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	stride6	delete	at=46855	err=None	del=205	before=163	after=42	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride6	insert	at=46855	err=None	del=205	before=163	after=42	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride7	truncate	at=54664	err=None	del=173	before=173	after=0	lost=33	dropped_tracks=0	extra=0	mono=true	findings=0
flv	stride7	zero	at=54664	err=None	del=206	before=183	after=22	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	stride7	random	at=54664	err=None	del=206	before=183	after=22	lost=1	dropped_tracks=0	extra=1	mono=true	findings=0
flv	stride7	delete	at=54664	err=None	del=205	before=183	after=22	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	stride7	insert	at=54664	err=None	del=205	before=183	after=22	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	mid	field-zero	at=31357	err=None	del=205	before=166	after=39	lost=1	dropped_tracks=0	extra=0	mono=true	findings=1
flv	mid	field-max	at=31357	err=None	del=93	before=93	after=0	lost=113	dropped_tracks=0	extra=0	mono=true	findings=0
flv	mid	field-past	at=31357	err=None	del=93	before=93	after=0	lost=113	dropped_tracks=0	extra=0	mono=true	findings=0

## mkv clean samples=206 tracks=2 findings=0 error=None
mkv	start	truncate	at=0	err=None	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	start	zero	at=0	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=false	findings=1
mkv	start	random	at=0	err=None	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=1
mkv	start	delete	at=0	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=false	findings=1
mkv	start	insert	at=0	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=false	findings=7
mkv	first-unit	truncate	at=6082	err=None	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	first-unit	zero	at=6082	err=None	del=206	before=76	after=129	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	first-unit	random	at=6082	err=None	del=206	before=76	after=129	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	first-unit	delete	at=6082	err=Some("matroska: integer payload width 71 is outside 1..=8 octets")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	first-unit	insert	at=6082	err=None	del=206	before=76	after=129	lost=1	dropped_tracks=0	extra=1	mono=false	findings=2
mkv	mid	truncate	at=30410	err=None	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	mid	zero	at=30410	err=None	del=206	before=165	after=40	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	mid	random	at=30410	err=None	del=206	before=165	after=40	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	mid	delete	at=30410	err=Some("invalid input: MatroskaDemux was fed after Eof")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	mid	insert	at=30410	err=None	del=206	before=165	after=40	lost=1	dropped_tracks=0	extra=1	mono=false	findings=2
mkv	last-unit	truncate	at=54739	err=None	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	last-unit	zero	at=54739	err=None	del=206	before=198	after=7	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	last-unit	random	at=54739	err=None	del=206	before=198	after=7	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	last-unit	delete	at=54739	err=Some("invalid input: MatroskaDemux was fed after Eof")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	last-unit	insert	at=54739	err=None	del=206	before=198	after=7	lost=1	dropped_tracks=0	extra=1	mono=false	findings=2
mkv	stride1	truncate	at=7602	err=None	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride1	zero	at=7602	err=None	del=206	before=81	after=124	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	stride1	random	at=7602	err=None	del=206	before=81	after=124	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	stride1	delete	at=7602	err=Some("invalid input: MatroskaDemux was fed after Eof")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride1	insert	at=7602	err=None	del=206	before=81	after=124	lost=1	dropped_tracks=0	extra=1	mono=false	findings=2
mkv	stride2	truncate	at=15205	err=None	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride2	zero	at=15205	err=None	del=206	before=101	after=104	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	stride2	random	at=15205	err=None	del=206	before=101	after=104	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	stride2	delete	at=15205	err=Some("invalid input: MatroskaDemux was fed after Eof")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride2	insert	at=15205	err=Some("matroska: limit max_element_bytes exceeded: 5718868811")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride3	truncate	at=22807	err=None	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride3	zero	at=22807	err=None	del=206	before=156	after=49	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	stride3	random	at=22807	err=None	del=206	before=156	after=49	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	stride3	delete	at=22807	err=Some("invalid input: MatroskaDemux was fed after Eof")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride3	insert	at=22807	err=None	del=206	before=156	after=49	lost=1	dropped_tracks=0	extra=1	mono=false	findings=2
mkv	stride4	truncate	at=30410	err=None	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride4	zero	at=30410	err=None	del=206	before=165	after=40	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	stride4	random	at=30410	err=None	del=206	before=165	after=40	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	stride4	delete	at=30410	err=Some("invalid input: MatroskaDemux was fed after Eof")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride4	insert	at=30410	err=None	del=206	before=165	after=40	lost=1	dropped_tracks=0	extra=1	mono=false	findings=2
mkv	stride5	truncate	at=38013	err=None	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride5	zero	at=38013	err=None	del=206	before=177	after=28	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	stride5	random	at=38013	err=None	del=206	before=177	after=28	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	stride5	delete	at=38013	err=Some("invalid input: MatroskaDemux was fed after Eof")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride5	insert	at=38013	err=None	del=206	before=177	after=28	lost=1	dropped_tracks=0	extra=1	mono=false	findings=2
mkv	stride6	truncate	at=45615	err=None	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride6	zero	at=45615	err=Some("buffer too short: need 143 bytes, have 42 (while parsing NAL payload)")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride6	random	at=45615	err=None	del=205	before=183	after=22	lost=1	dropped_tracks=0	extra=0	mono=false	findings=1
mkv	stride6	delete	at=45615	err=Some("invalid input: MatroskaDemux was fed after Eof")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride6	insert	at=45615	err=Some("buffer too short: need 2049031193 bytes, have 42 (while parsing NAL payload)")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride7	truncate	at=53218	err=None	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride7	zero	at=53218	err=None	del=206	before=183	after=22	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	stride7	random	at=53218	err=None	del=206	before=183	after=22	lost=1	dropped_tracks=0	extra=1	mono=false	findings=0
mkv	stride7	delete	at=53218	err=Some("invalid input: MatroskaDemux was fed after Eof")	del=0	before=0	after=0	lost=206	dropped_tracks=2	extra=0	mono=true	findings=0
mkv	stride7	insert	at=53218	err=None	del=206	before=183	after=22	lost=1	dropped_tracks=0	extra=1	mono=false	findings=2
mkv	mid	field-zero	at=40048	err=None	del=133	before=133	after=0	lost=73	dropped_tracks=0	extra=0	mono=false	findings=1
mkv	mid	field-max	at=40048	err=None	del=206	before=206	after=0	lost=0	dropped_tracks=0	extra=0	mono=false	findings=1
mkv	mid	field-past	at=40048	err=None	del=133	before=133	after=0	lost=73	dropped_tracks=0	extra=0	mono=false	findings=1
```

