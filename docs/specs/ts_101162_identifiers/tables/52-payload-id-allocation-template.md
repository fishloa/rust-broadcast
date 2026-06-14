# Table 52: Payload_ID allocation template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Payload_ID | Description  |
| --- | --- |
|  0x00 | Reserved  |
|  0x01 to 0EF | Reserved for payload formats defined by DVB [i.12], [i.15] and [i.16]  |
|  0x01 | SD&S Service Provider Discovery Information  |
|  0x02 | SD&S Broadcast Discovery Information  |
|  0x03 | SD&S CoD Discovery Information  |
|  0x04 | SD&S Services from other SPs  |
|  0x05 | SD&S Package Discovery Information  |
|  0x06 | SD&S BCG Discovery Information  |
|  0x07 | SD&S Regionalization Discovery Information  |
|  0x08 | FUS Stub file and SD&S RMS-FUS record  |
|  0x09 | SRM delivery over DVBSTP  |
|  0xA1 to 0xAF | BCG Payload_ID values (defined in ETSI TS 102 539 [i.15])  |
|  0xB1 | CDS XML download session description (defined in ETSI TS 102 539 [i.15])  |
|  0xB2 | RMS-FUS Firmware Update Announcements (defined in ETSI TS 102 824 [i.16])  |
|  0xC1 | Application Discovery Information  |
|  0xF0 to 0xFF | User defined  |

# 9.1.3 Payload_ID domain names

Table 53 lists the names, under which the Payload_ID is used in different DVB specifications.
