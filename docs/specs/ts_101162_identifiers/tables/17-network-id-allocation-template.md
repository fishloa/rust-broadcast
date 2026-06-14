# Table 17: Network_ID allocation template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Network_ID | Classification | Network Type | Country code(s) of validity | Description  |
| --- | --- | --- | --- | --- |
|  0x0000 | Reserved | all | all | Reserved  |
|  0x0001 to 0x2000 | Unique satellite | Satellite | all | 4 096 values reserved for registration through the DVB Project Office (see http://www.dvbservices.com)  |
|  0x2001 to 0x3000 | Unique terrestrial | Terrestrial | all | 4 096 values reserved for registration through the DVB Project Office (see http://www.dvbservices.com)  |
|  0x3001 to 0x3600 | Re-useable terrestrial | Terrestrial | as registered | 1 536 values reserved for registration through the DVB Project Office (see http://www.dvbservices.com)  |
|  0x3001 to 0x3100 | Countries of colour A | Terrestrial | as registered | 256 values  |
|  0x3101 to 0x3200 | Countries of colour B | Terrestrial | as registered | 256 values  |
|  0x3201 to 0x3300 | Countries of colour C | Terrestrial | as registered | 256 values  |
|  0x3301 to 0x3400 | Countries of colour D | Terrestrial | as registered | 256 values  |
|  0x3401 to 0x3500 | Countries of colour E | Terrestrial | as registered | 256 values (to be used only in case of collision)  |
|  0x3501 to 0x3600 | Countries of colour F | Terrestrial | as registered | 256 values (to be used only in case of collision)  |
|  0x3601 to 0xA000 | Reserved for future use | Terrestrial | to be defined | 27 136 values reserved for registration through the DVB Project Office (see http://www.dvbservices.com)  |
|  0xA001 to 0xB000 | Re-useable cable | Cable | as registered | 4 096 values reserved for registration through the DVB Project Office (see http://www.dvbservices.com)  |
|  0xB001 to 0xF000 | Reserved for future use | Cable | to be defined | 16 384 values reserved for registration through the DVB Project Office (see http://www.dvbservices.com)  |
|  0xF001 to 0xFF00 | Unique cable | Cable | all | 3 840 values reserved for registration through the DVB Project Office (see http://www.dvbservices.com)  |
|  0xFEC0 to 0xFF00 | Network Interface Modules | DVB Common Interface [i.10] | all | 64 values for local use by DVB-CI modules  |
|  0xFF01 to 0xFFFF | Temporary private use | Not defined | all | 255 values for temporary private use  |

# 5.6.3 Network_ID domain names

Table 18 lists the names, under which the Network_ID is used in different DVB specifications.
