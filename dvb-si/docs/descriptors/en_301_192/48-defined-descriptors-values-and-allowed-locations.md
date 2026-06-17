## Table 48 — Defined descriptors, values and allowed locations
_§10.2.2, PDF pp. 64-64_

| Descriptor | Tag | DII - | DSI - | Short description |
|---|---|---|---|---|
|  | value | moduleInfo | groupinfo |  |
| Reserved | 0x00 |  |  |  |
| Type | 0x01 | + | + | type descriptor of data |
| Name | 0x02 | + | + | name descriptor of data |
| Info | 0x03 | + | + | textual description |
| module_link | 0x04 | + |  | concatenated data module |
| CRC32 | 0x05 | + |  | Cyclic Redundancy Code (CRC) |
| Location | 0x06 | + | + | location of data |
| est_download_time | 0x07 | + | + | estimated download time |
| group_link | 0x08 |  | + | links DII messages describing a group |
| compressed_module | 0x09 | + |  | indicates compression structure |
| SSU_module_type | 0x0A | + |  | refer to ETSI TS 102 006 [18] (DVB SSU) |
| subgroup_association | 0x0B |  | + | refer to ETSI TS 102 006 [18] (DVB SSU) |

