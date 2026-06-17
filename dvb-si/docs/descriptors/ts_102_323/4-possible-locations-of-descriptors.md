## Table 4 — Possible locations of descriptors
_§5.3.3.1, PDF pp. 19-19_

| Descriptor | Tag value | Tag ext | Clause | NIT 1 | NIT 2 | BAT 1 | BAT 2 | SDT | PMT 2 | EIT s | EIT pf | RNT 1 | RNT 2 | RNT 3 | RCT 1 | RCT 2 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| metadata pointer descriptor | 0x25 | - | 5.3.3 | * | | * | | * | | | | * | * | * | | * |
| metadata descriptor | 0x26 | - | 5.3.4 | | | | | | * | | | | | | | |
| RAR over DVB stream descriptor | 0x40 | - | 5.3.5 | | | | | | | | | | | * | | |
| RAR over IP descriptor | 0x41 | - | 5.3.6 | | | | | | | | | | | * | | |
| RNT scan descriptor | 0x42 | - | 5.3.7 | | | | | | | | | * | | | | |
| content labelling descriptor | 0x24 | - | 5.3.8 | | | | | | * | | | | | | | |
| default authority descriptor | 0x73 | - | 6.3.3 | * | * | * | * | * | | | | | | | | |
| related content descriptor | 0x74 | - | 10.3 | | | | | | * | | | | | | | |
| TVA_id descriptor | 0x75 | - | 11.2 | | | | | | | | * | | | | | |
| content identifier descriptor | 0x76 | - | 12.1 | | | | | | | * | * | | | | | |
| image icon descriptor | 0x7F | 0x00 | 10.4.3 | | | | | | | | | | | | * | * |
| NOTE 1: NIT 1: common (outer) descriptor loop of the NIT. NIT 2: transport stream descriptor loop of the NIT. BAT 1: common descriptor loop of the BAT. BAT 2: transport stream descriptor loop of the BAT. PMT 2: elementary stream descriptor loop of the PMT. EIT s: the descriptor loop of the EIT schedule. EIT pf: the descriptor loop of the EIT present/following. RNT 1: common descriptor loop of the RNT. RNT 2: resolution provider descriptor loop of the RNT. RNT 3: CRID authority descriptor loop of the RNT. RCT 1: common descriptor loop of the RCT. RCT 2: descriptor loop in the link info structure of the RCT. | | | | | | | | | | | | | | | | |
| NOTE 2: The descriptor tag values 0x40-0x42, inclusive, in the above table, override the definition of the meaning of those descriptor tags in EN 300 468 [1], when they are used in the RNT table only. | | | | | | | | | | | | | | | | |

