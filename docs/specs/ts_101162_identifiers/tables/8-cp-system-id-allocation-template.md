# Table 8: CP_System_ID allocation template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  CP_System_ID |   | CP system specifier  |
| --- | --- | --- |
|  0x0000 to 0x00FF |   | Reserved for registration to systems defined by DVB  |
|   | 0x0000 | DVB CPCM Content Licence  |
|   |  0x0001 | DVB CPCM Auxiliary Data  |
|   |  0x0002 | DVB CPCM Revocation List  |
|  0x0100 to 0xFFFF |   | Reserved for general registration through the DVB Project Office (see http://www.dvbservices.com)  |

In the general registration range, allocations shall only be made to bona fide Copy Protection system vendors. Applicants need to demonstrate that the vendor is proposing a registration for a legitimate Copy Protection product.

# 5.3.3 CP_System_ID domain names

Table 9 lists the names, under which the CP_System_ID is used in different DVB specifications.
