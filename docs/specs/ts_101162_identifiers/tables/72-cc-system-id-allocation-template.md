# Table 72: CC_System_ID allocation template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  CC_System_ID | Bit of cc_system_id_bitm ask | Description  |
| --- | --- | --- |
|  1 | b0 | Allocated for the CI Plus LLP PKI as defined in CI Plus Specification 1.3 [i.38] (CI Plus Root of Trust).  |
|  2 | b1 | Allocated for the CI Plus LLP PKI as defined in CI Plus Specification 1.4 [i.39] (CI Plus 2nd Root of Trust).  |
|  3 to 7 | b2 to b6 | Reserved for registration through the DVB Project Office (see http://www.dvbservices.com).  |
|  n/a | b7 | Reserved for a future extension mechanism for additional CC_System_IDs.  |

# 13.2.5 CC_System_ID domain names

Table 73 lists the names, under which the CC_System_ID is used in different DVB specifications.
