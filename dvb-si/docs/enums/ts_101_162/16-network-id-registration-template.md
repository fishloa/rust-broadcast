# Table 16: Network_ID registration template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Registration field | Required | Description  |
| --- | --- | --- |
|  Network Type | required | Satellite, terrestrial or cable  |
|  Network Name | required | Name of the Network (e.g. "ACME Cable")  |
|  Network Country Code | required | Country code where the network is unique (e.g. North America)  |
|  Network Operator | required | Name of organization which operates the network (e.g. "ACME Pay-TV, Inc.")  |
|  Network Legal Contact | required | Name and e-mail of authorized legal signatory of "Network Operator"  |
|  Network Technical Contact | required | Name and e-mail of technical contact of "Network Operator"  |
|  Network Notes | optional | Notes on the application, e.g. last revised and what revisions were made  |

The rules for the allocation of Network_IDs are as follows:

1) Network_IDs will be allocated on a geographical basis such that no conflict of network ids occurs in any geographical region. (Satellite network ids will be unique world-wide).
2) Network_IDs are a scarce resource and their allocation is under responsibility of DVB. Application of multiple Network_IDs is subject to exhaustive verification and is discouraged.
3) 256 Network_ID values are reserved for private/temporary use. Their allocation is not subject of the present document.
4) Network_IDs will be allocated according to clause 5.6.2.
5) Network_IDs for the terrestrial delivery medium will be made available to the appropriate national telecommunications regulator and their allocation in each country is under responsibility of this regulator.
6) In order to avoid the uneconomical use of Network_IDs, the values will be given in blocks of 256 values on a country by country basis. Non-allocated Network_IDs will be kept reserved.
7) The allocation of terrestrial network ids shall be based on a 4-colour-map approach. Two blocks of 256 values are reserved for the eventual case of collision.
8) If 256 values are not sufficient for a country, a new block of 256 colours will be allocated. This block can be used by all countries with the same colour in the colour map.

NOTE: Due to the re-usable allocation of all types of Network_ID values (satellite, cable and terrestrial), no link between Network_ID and Original_Network_ID exists.

# 5.6.2 Network_ID allocation template

The scheme and values given in table 17 shall be used for the allocation of Network_ID values.
