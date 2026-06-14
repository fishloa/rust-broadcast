# Table 41: MHP_AIT_Descriptor allocation template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  MHP_AIT_Descriptor | Description  |
| --- | --- |
|  0x00 to 0x5F | Reserved for DVB-MHP  |
|  0x00 | application_descriptor  |
|  0x01 | application_name_descriptor  |
|  0x02 | transport_protocol_descriptor  |
|  0x03 | dvb_j_application_descriptor  |
|  0x04 | dvb_j_application_location_descriptor  |
|  0x05 | external_application_authorization_descriptor  |
|  0x08 | dvb_html_application_descriptor  |
|  0x09 | dvb_html_application_location_descriptor  |
|  0x0A | dvb_html_application_boundary_descriptor  |
|  0x0B | application_icons_descriptor  |
|  0x0C | prefetch_descriptor  |
|  0x0D | DII_location_descriptor  |
|  0x0E | delegated_application_descriptor  |
|  0x0F | plug-in_descriptor  |
|  0x10 | application_storage_descriptor  |
|  0x11 | ip_signalling_descriptor  |
|  0x12 | provider_export_descriptor  |
|  0x13 | provider_usage_descriptor  |
|  0x14 | graphics Constraints_descriptor  |
|  0x5F | private_data_specifier_descriptor  |
|  0x60 to 0x7F | Reserved for registration to standardized descriptors through the DVB Project Office (see http://www.dvbservices.com)  |
|  0x80 to 0xFF | Reserved for future use  |

# 8.1.3 MHP_AIT_Descriptor domain names

Table 42 lists the names, under which the MHP_AIT_Descriptor is used in different DVB specifications.
