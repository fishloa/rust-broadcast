# Table 35: Protocol_Type_ID allocation template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Protocol_Type_ID | Description  |
| --- | --- |
|  0x00 | Generic Stream Encapsulation [i.8] and [i.14]  |
|  0x01 | Generic Stream Encapsulation with error detection adaptation layer [i.8] and [i.14] (see note)  |
|  0x02 to 0xB8 | Reserved for registration to standardized protocols through the DVB Project Office (see http://www.dvbservices.com)  |
|  0xB9 to 0xFF | User private  |
|  NOTE: For details of the error detection adaptation layer see the clauses specific to each physical layer in ETSI TS 102 771 [i.14].  |   |

# 7.1.3 Protocol_Type_ID domain names

Table 36 lists the names, under which the Protocol_Type_ID is used in different DVB specifications.
