# Table 29: Data_Broadcast_ID allocation template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Data_Broadcast_ID | Data broadcast specification  |
| --- | --- |
|  0x0000 | Reserved for future use  |
|  0x0001 to 0x007F | Reserved for registration to DVB data broadcasting - exclusive range (see note)  |
|  0x0001 | Data pipe  |
|  0x0002 | Asynchronous data stream  |
|  0x0003 | Synchronous data stream  |
|  0x0004 | Synchronized data stream  |
|  0x0005 | Multi-protocol encapsulation  |
|  0x0006 | Data Carousel  |
|  0x0007 | Object Carousel  |
|  0x0008 | DVB ATM streams  |
|  0x0009 | Higher Protocols based on asynchronous data streams  |
|  0x000A | System Software Update service [i.13]  |
|  0x000B | IP/MAC Notification service [i.3]  |
|  0x000C | Synchronized Auxiliary Data [i.29]  |
|  0x000D | Downloadable Font Info Table [i.35]  |
|  0x000E | Single Illumination System metadata [i.40]  |
|  0x0080 to 0x00EF | Reserved for registration to DVB data broadcasting - combined range (see note)  |
|  |   |
|  0x00F0 to 0x00FF | Reserved for registration to MHP data broadcasting  |
|  0x00F0 | MHP Object Carousel  |
|  0x00F1 | MHP Multiprotocol Encapsulation  |
|  0x00F2 | MHP application presence  |
|  0x0100 to 0xFFFE | Reserved for general registration through the DVB Project Office (see http://www.dvbservices.com)  |
|  0xFFFF | Reserved for future use  |
|  NOTE: See clauses 4.2.6.4 and 4.2.7.3 of ETSI TS 101 211 [i.2].  |   |

In the general registration range separate allocations for different versions of the same data broadcast specification shall only be made if and when a receiver would otherwise not be able to detect the version used from the contents of the data broadcast streams themselves or from private data carried in DVB-SI descriptors bearing a data_broadcast_id field. Data broadcast specifiers should thus design their specifications such that receivers can detect the version used without the use of separate Data_Broadcast_ID values.

# 6.1.3 Data_Broadcast_ID domain names

Table 30 lists the names, under which the Data_Broadcast_ID is used in different DVB specifications.
