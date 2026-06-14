# Table 69: Private_Resource_Definer_ID allocation template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Registration Authority (see note) | Private_Resource_Definer_ID | Description  |
| --- | --- | --- |
|  0 | 0x000 to 0x0FF | Organizations that have a CA_System_ID (see clause 5.2) are automatically allocated a private definer where the least significant byte of the definer is the most significant byte of CA_System_ID.  |
|   |  0x100 to 0xFFF | Reserved for general registration through the DVB Project Office (see http://www.dvbservices.com)  |
|  1 to 15 | 0x000 to 0xFFF | reserved for future use by ETSI  |
|  NOTE: See clause 12.1.  |   |   |

### 12.2.3 Private_Resource_Definer_ID domain names

Table 70 lists the names, under which the Private_Resource_Definer_ID is used in different DVB specifications.
