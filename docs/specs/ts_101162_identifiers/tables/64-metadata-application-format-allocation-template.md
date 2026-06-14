# Table 64: Metadata Application Format allocation template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Metadata Application Format | Description  |
| --- | --- |
|  0x0000 to 0x00FF | Reserved for allocation by ISO/IEC 13818-1 [i.28]  |
|  0x0100 to 0x027F | Reserved for registration to standardized applications through the DVB Project Office (see http://www.dvbservices.com)  |
|  0x0100 | DVB profile of TV-Anytime [i.11]  |
|  0x0101 | UK DTG profile of TV-Anytime  |
|  0x0280 to 0x03FF | Reserved for general registration through the DVB Project Office (see http://www.dvbservices.com)  |
|  0x0400 to 0xFFE | User defined  |
|  0xFFFF | Defined by the metadata application format identifier field [i.28]  |



# 11.1.3 Metadata Application Format domain names

Table 65 lists the names, under which the Metadata Application Format is used in different DVB specifications.
