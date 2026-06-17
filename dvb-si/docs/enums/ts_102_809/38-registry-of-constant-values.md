## Table 38 — Registry of constant values
_§5.5, PDF pp. 60-61_

| Where used | Type | Value | Where Defined | Scope |
|---|---|---|---|---|
| Private data specifier descriptor | descriptor tag | 0x5F | PSI and SI tables | SI |
| Data broadcast id descriptor | | 0x66 | PMT | |
| Application Signalling Descriptor | | 0x6F | PMT | |
| Service identifier descriptor | | 0x71 | SDT | |
| Caching priority descriptor | descriptor tag | 0x71 | DII moduleInfo userInfo | ETSI EN 301 192 [2] - DVB specification for data broadcasting |
| Content type descriptor | | 0x72 | BIOP objectInfo (note 1) | |
| Reserved to DVB for future OC descriptors | | 0x73 to 0x7F | OC | |
| reserved to DVB for future use | table ID on AIT PID | 0x00 to 0x73 | | The present document |
| Application Information Table | | 0x74 | | |
| Reserved to DVB for future use | | 0x75 to 0x7F | | |
| Reserved for private use | | 0x80 to 0xFF | | |
| Application descriptor | descriptor tag | 0x00 | AIT | The present document |
| Application name descriptor | | 0x01 | | |
| Transport protocol descriptor | | 0x02 | | |
| Reserved to DVB for future use | | 0x03, 0x04 | | |
| External application authorization descriptor | | 0x05 | | |
| Application recording descriptor | | 0x06 | | |
| Reserved to DVB for future use | | 0x07 to 0x0A | | |
| Application icons descriptor | | 0x0B | | |
| Reserved to DVB for future use | | 0x0C to 0x0F | | |
| Application storage descriptor | | 0x10 | | |
| Reserved to DVB for future use | | 0x11 to 0x13 | | |
| Graphics constraints descriptor | | 0x14 | | |
| Simple application location descriptor | | 0x15 | | |
| Application usage descriptor | | 0x16 | | |
| Simple application boundary descriptor | | 0x17 | | |
| reserved to DVB for future use | | 0x18 to 0x5E | | |
| Private data specifier descriptor (note 2) | | 0x5F | | |
| Subject to registration at http://www.dvb.org | | 0x60 to 0x7F | | |
| User defined (note 3) | | 0x80 to 0xFE | | |
| DVB Object Carousel | data broadcast id | 0x00F0 | PMT, AIT | SI |
| reserved | | 0x00F1 | | |
| DVB application presence | | 0x00F2 | EIT, SDT | SI |
| Reserved to DVB for future use | | 0x00F3 to 0x00FE | PMT, AIT | SI |
| NOTE 1: Strictly MessageSubHeader::ObjectInfo in the file message and the bound object info in a file binding of a directory or service gateway message. NOTE 2: The DVB SI private data specifier descriptor is defined for use in the Application Information Table to introduce private descriptors. NOTE 3: All user defined descriptors shall be within the scope of a private data specifier descriptor. | | | | |

