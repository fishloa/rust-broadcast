# Table 58: IPDC_Notification_Type allocation template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  IPDC_Notification_Type | MIME Type | Description  |
| --- | --- | --- |
|  0x0000 to 0x00FF | Reserved for registration to standardized applications through the DVB Project Office (see http://www.dvbservices.com)  |   |
|  0x0000 |  | Reserved for specific IPDC signalling  |
|  0x0001 | text/xml | ESG update message  |
|  0x0002 | application/octet-stream | Notification application inside the smartcard, invoked by the OMA Smart Card Web Server  |
|  0x0100 to 0xFFFF | User defined (dynamically assigned in the scope of an IP platform)  |   |

# 10.2.3 IPDC_Notification_Type domain names

Table 59 lists the names, under which the IPDC_Notification_Type is used in different DVB specifications.
