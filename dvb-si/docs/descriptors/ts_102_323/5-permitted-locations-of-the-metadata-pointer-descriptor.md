## Table 5 — Permitted locations of the metadata pointer descriptor
_§5.3.3.2, PDF pp. 20-20_

| Scope | Linkage location | Example metadata fragment types |
|---|---|---|
| Global | BAT, NIT, common descriptor loop of the RNT, resolution provider descriptor loop of the RNT | BroadcastEvent, Schedule, ServiceInformation, ProgramInformation, GroupInformation, Review, SegmentInformation, SegmentGroupInformation, PersonName, OrganizationName. |
| DVB Service | Service loop of the SDT | BroadcastEvent, Schedule, ServiceInformation. |
| CRID authority | CRID authority descriptor loop of the RNT | ProgramInformation, GroupInformation, Review, SegmentInformation, SegmentGroupInformation, PersonName, OrganizationName. |

