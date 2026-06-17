## Table 48 — MHP object carousel constraints
_§9.2.2, PDF pp. 57-57_

| MHP Section | | |
|---|---|---|
| B.2.2.4.1 | Label descriptor | Not used by the present document (see note). |
| B.2.2.4.2 | Caching priority descriptor | Not used by the present document (see note). |
| B.2.3.4 | Content type descriptor | Not used by the present document (see note). |
| B.2.3.7.2 | LiteOptionsProfileBody | All containers of a metadata service shall be carried in a single object carousel. Therefore, LiteOptionsProfileBody may not form part of the reference to any such container. However, external assets, e.g. images, audio clips, referenced by the metadata service may be delivered in a different object carousel; the LiteOptionsProfileBody may be used as part of the reference to any such external assets. |
| B.2.3.8 | BIOP StreamMessage | Not used by the present document (see note). |
| B.2.3.9 | BIOP StreamEventMessage | Not used by the present document (see note). |
| B.2.4 | Stream Events | Not relevant to the present document. |
| B.2.10.2 | DVB-J mounting of an object carousel | Not relevant to the present document. |
| B.3.2 | DSM-CC association_tags to DVB component_tags | All DILs and DDBs used in the delivery of a metadata service shall be carried in elementary streams that are listed in the PMT that carries the metadata descriptor for that metadata service (see clause 5.3.5). Therefore, use of the deferred_association_tags descriptor is not required. |
| B.3.1.2 | TapUse is BIOP_PROGRAM_USE | Not used by the present document (see note). |
| B.5 | Caching | Informative for receiver manufacturers. |
| NOTE: | Metadata services shall not include such information in the object carousel. Receivers may ignore the presence of such information in the object carousel when accessing metadata services. | |

