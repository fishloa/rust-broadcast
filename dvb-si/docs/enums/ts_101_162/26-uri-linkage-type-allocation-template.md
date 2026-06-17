# Table 26: URI_Linkage_Type allocation template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  URI_Linkage_Type |   | URI linkage specification  |
| --- | --- | --- |
|  0x00 to 0x5F |   | Reserved for registration to DVB specifications  |
|  0x00 |   | Online SDT (OSDT) for CI Plus [i.9]  |
|  0x01 |   | DVB-IPTV SD&S [i.12]  |
|  0x02 |   | Material Resolution Server (MRS) for companion screen applications [i.10]  |
|  0x03 |   | DVB-I [i.41]  |
|  0x60 to 0x7F |   | Reserved for registration to standardized systems through the DVB Project Office (see http://www.dvbservices.com)  |
|  0x80 to 0xFF |   | User defined  |



In the standardized systems registration range, allocations shall only be made for standardized systems which are fully described in a publicly available document from a standardization body recognized by DVB. Separate allocations for different versions of the same standardized system specification shall only be made if and when a receiver would otherwise not be able to detect the version used from the contents of the standardized system streams themselves. Standardized system specifiers should thus design their specifications such that receivers can detect the version used without the use of separate URI_Linkage_Type values.

## 5.9.4 URI_Linkage_Type domain names

Table 27 lists the names, under which the URI_Linkage_Type is used in different DVB specifications.
