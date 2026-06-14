# Table 32: Platform_ID allocation template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Platform_ID | Description  |
| --- | --- |
|  0x000000 | Reserved  |
|  0x000001 to 0xFFFFFF | Reserved for general registration through the DVB Project Office (see http://www.dvbservices.com). These platform_id values are globally unique.  |
|  0xFFFF000 to 0xFFFFFE | Managed by the network operator, and may be used for IP/MAC Platforms supporting services only within a single DVB network. These platform_id values are unique within a network_id only.  |
|  0xFFFFFF | Reserved  |

## 6.2.3 Platform_ID domain names

Table 33 lists the names, under which the Platform_ID is used in different DVB specifications.
