## Table 1 — System configurations and application areas
_§4.3, PDF pp. 16-16_

| System configurations |  | Broadcast services | Interactive services | DSNG | Professional services |
|---|---|---|---|---|---|
| QPSK | 1/4, 1/3, 2/5 | O | N | N | N |
|  | 1/2, 3/5, 2/3, 3/4, 4/5, 5/6, 8/9, 9/10 | N | N | N | N |
| 8PSK | 3/5, 2/3, 3/4, 5/6, 8/9, 9/10 | N | N | N | N |
| 16APSK | 2/3, 3/4, 4/5, 5/6, 8/9, 9/10 | O | N | N | N |
| 32APSK | 3/4, 4/5, 5/6, 8/9, 9/10 | O | N | N | N |
| CCM |  | N | N (see note 1) | N | N |
| VCM |  | O | O | O | O |
| ACM |  | NA | N (see note 2) | O | O |
| FECFRAME (normal) | 64 800 (bits) | N | N | N | N |
| FECFRAME (short) | 16 200 (bits) | NA | N | O | N |
| Single Transport Stream |  | N | N (see note 1) | N | N |
| Multiple Transport Streams |  | O | O (see note 2) | O | O |
| Single Generic Stream |  | NA | O (see note 2) | NA | O |
| Multiple Generic Streams |  | NA | O (see note 2) | NA | O |
| Roll-off 0,35, 0,25 and 0,20 |  | N | N | N | N |
| Input Stream Synchronizer |  | NA except (see note 3) | O (see note 3) | O (see note 3) | O (see note 3) |
| Null Packet Deletion |  | NA except (see note 3) | O (see note 3) | O (see note 3) | O (see note 3) |
| Dummy Frame insertion |  | NA except (see note 3) | N | N | N |
| Wide-band mode | (see annex M) | O | O | O | O |

N = normative, O = optional, NA = not applicable.

NOTE 1: Interactive service receivers shall implement CCM and Single Transport Stream.

NOTE 2: Interactive Service Receivers shall implement ACM at least in one of the two options: Multiple Transport Streams or Generic Stream (single/multiple input).

NOTE 3: Normative for single/multiple TS input stream(s) combined with ACM/VCM or for multiple TS input streams combined with CCM.

---

