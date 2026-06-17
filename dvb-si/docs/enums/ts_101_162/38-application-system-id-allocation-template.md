# Table 38: Application_System_ID allocation template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Application_System_ID |   | Data broadcast specification  |
| --- | --- | --- |
|  0x0000 to 0x001F |   | Reserved for registration to DVB specifications  |
|  |   |   |
|  0x0020 to 0x7FFF |   | Reserved for registration to standardized systems through the DVB Project Office (see http://www.dvbservices.com)  |
|  0x8000 to 0xFFFF |   | Reserved for general registration through the DVB Project Office (see http://www.dvbservices.com)  |

In the standardized systems registration range, allocations shall only be made for application systems which are defined and/or adopted as such by DVB, and which are fully described in a publicly available document from a recognized standardization body.

In the general registration range, separate allocations for different versions of the same application system specification shall only be made if and when a receiver would otherwise not be able to detect the version used from the contents of the application system streams themselves, or from private data carried in GSE LLC descriptors bearing an application_system_id field. Application system specifiers should thus design their specifications such that receivers can detect the version used without the use of separate Application_System_ID values.



# 7.2.3 Application_System_ID domain names

Table 39 lists the names, under which the Application_System_ID is used in different DVB specifications.
