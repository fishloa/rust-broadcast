# Table 15: Encoding_type_id domain names

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Name | Domain | Description  |
| --- | --- | --- |
|  Encoding_Type_ID | Registration Domain | Constituted by the present document  |
|  encoding_type_id | DVB-SI | ETSI EN 300 468 [i.1] ETSI TS 101 211 [i.2]  |

# 5.6 Network_ID

# 5.6.0 Network_ID registration principles

Network_ID values shall be allocated to broadcasters and network operators to identify networks within the application area of ETSI EN 300 468 [i.1], by insertion in the network_id field.



A network is defined as a collection of MPEG 2 Transport Stream (TS) multiplexes transmitted on a single delivery system, e.g. all digital channels on a specific cable system. Network_IDs are unique within the geographical region defined by the Country Code:

- For satellite networks, this is a region spanning many countries.
- For a cable network, this is a single country.
- For terrestrial networks, this is a single country also, but it is important that two adjacent countries shall not have the same block of Network IDs. Hence the concept of colour coding countries was introduced.

# 5.6.1 Network_ID registration template

To register a Network_ID, applicants shall supply at least the information labelled as "required" in the registration template below.
