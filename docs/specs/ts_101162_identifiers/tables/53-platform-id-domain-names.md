# Table 53: Platform_id domain names

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Name | Domain | Description  |
| --- | --- | --- |
|  Payload_ID | Registration Domain | Constituted by the present document  |
|  Payload ID | DVB-IPTV | ETSI TS 102 034 [i.12]  |
|  payloadId |  | ETSI TS 102 539 [i.15]  |
|  PayloadId |  | ETSI TS 102 824 [i.16]  |

# 10 IP Datacast over DVB (DVB-IPDC) identifiers

# 10.1 IPDC_Operator_ID

# 10.1.0 IPDC_Operator_ID registration principles

An IPDC Operator is a network entity managing IPDC key streams. It is uniquely identified by a pair of two DVB identifiers:

an IPDC_Operator_ID value; and
a CA_System_ID value (see clause 5.2).

IPDC_Operator_ID values shall be allocated to IPDC operators to construct - under the scope of a CA_system_ID value - the unique identification of an IPDC operator [i.18].

For CA_system_ID values in the range of 0x0001 to 0x00FF (standardized CA systems), associated IPDC_Operator_ID values shall be registered through the DVB Project Office.



# 10.1.1 IPDC_Operator_ID registration template

To register an IPDC_Operator_ID, applicants shall supply at least the information labelled as "required" in the registration template below.
