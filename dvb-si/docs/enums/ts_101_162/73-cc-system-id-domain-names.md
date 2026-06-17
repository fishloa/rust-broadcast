# Table 73: CC_System_ID domain names

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Name | Domain | Description  |
| --- | --- | --- |
|  CC_System_ID | Registration Domain | Constituted by the present document.  |
|  CC system ID | DVB-CI | ETSI TS 103 205 [i.37]  |


---


# Annex A (informative):

# Example Scenarios for the Utilization of network_id and original_network_id

# A.1 Re-transmission of a satellite signal in terrestrial networks

A service operator A-TV transmits his transport stream to satellite X-SAT. The signal is re-transmitted by the terrestrial network A-NET in country A with modifications to the content. The signal is re-transmitted by the terrestrial network in country B without modifications to the content:

A-TV has the unique original_network_id 0x1234.
- Another television network B-TV (original_network_id = 0x5678) is using the same satellite for the contribution to A-Net in country A and to B-Net in country B.
- The original_network_id of a DVB-T network is likely to have been allocated for the country according to clause 5.7. The originating service operator and its original_network_id in this case do not occur in the NIT of terrestrial networks.
X-SAT has the network_id 0x0200 (in range of unique satellite networks).
A-NET and B-Net share the re-usable terrestrial network_id range of 0x3300 to 0x334F.

![img-0.jpeg](img-0.jpeg)
Figure A.1

The satellite NIT contains the original_network_id of A-TV and the network_id of X-SAT.



On the terrestrial network the original_network_id has always the value that has been allocated for a certain country as defined in clause 5.6. The network_id is replaced by one of the network_ids of country A that could be re-used in country B if it has the same colour in the colour-map.

# A.2 Re-transmission of a satellite signal in cable networks

The same scheme as above applies. Cable networks generally use re-usable network_ids because there is no risk that IRDs are connected to two cable networks sharing the same network_id at the same time.

The satellite serves different cable networks in L-Town and in E-Town. They can use the same network_id because they are physically separated.

A special case is the transmission of cable network NITs as "foreign" NITs on a satellite. In this case the cable network_ids have to be in the unique range of values since a collision on other networks using the same re-usable network_id cannot be guaranteed. Note that this method is not recommended since the number of unique network_ids is limited.

![img-1.jpeg](img-1.jpeg)
Figure A.2



History

|  Document history  |   |   |
| --- | --- | --- |
|  Edition 1 | October 1995 | Publication as ETSI ETR 162  |
|  V1.2.1 | July 2009 | Publication  |
|  V1.3.1 | December 2010 | Publication  |
|  V1.4.1 | May 2011 | Publication  |
|  V1.5.1 | January 2012 | Publication  |
|  V1.6.1 | November 2013 | Publication  |
|  V1.7.1 | February 2014 | Publication  |
|  V1.8.1 | January 2017 | Publication  |
|  V1.9.1 | July 2020 | Publication  |
