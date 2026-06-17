# Table 19: Original_Network_ID registration template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Registration field | Required | Description  |
| --- | --- | --- |
|  Original Network Name | required | Name of the Network (e.g. "ACME TV")  |
|  Original Network Operator | required | Name of organization which operates network (e.g. "ACME Broadcast Corp.")  |
|  Original Network Legal Contact | required | Name and e-mail of authorized legal signatory of "Original Network Operator"  |
|  Original Network Technical Contact | required | Name and e-mail of technical contact of "Original Network Operator"  |
|  Original Network Notes | optional | Notes on the application, e.g. last revised and what revisions were made  |

The rules for the allocation of Original_Network_IDs are as follows:

1) In principle only one Original_Network_ID should be assigned to each network operator, broadcaster or content producer.
2) Original_Network_IDs are a scarce resource and their allocation is under responsibility of DVB. Application of multiple Original_Network_IDs is subject to exhaustive verification and discouraged.
3) 256 Original_Network_ID values are reserved for private/temporary use. Their allocation is not subject of the present document.

Since terrestrial and cable networks have in most cases a clearly identified geographical region of validity, the re-usage of Network_IDs is possible. However, Original_Network_IDs shall be unique independent of geographical region, since they are used to uniquely identify the transport streams and services.

In terrestrial networks, however it is recommended that all operators within a country use the same

Original_Network_ID. This implies that broadcasters and operators within a country would need to coordinate the allocation of transport_stream_ids and service_ids between them. The registrar is recommended to allocate

Original_Network_ID values for terrestrial operators on the basis of Country Code + 0x2000. This will help receivers to discriminate broadcasts from multiple countries in cases where the target region descriptor is not used.

Some examples on the use of Network_ID and Original_Network_ID are given in annex A.

# 5.7.2 Original_Network_ID allocation template

The scheme and values given in table 20 shall be used for the allocation of Original_Network_ID values.
