# Table 5: Subtitling type usage

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Subtitling type in the subtitling_descriptor (see ETSI EN 300 468 [2]) | ETSI EN 300 743 version compliance | Indicative service compatibility | Features that are not recommended for the subtitle service  |
| --- | --- | --- | --- |
|  0x10-0x13, 0x20-0x23 | V1.1.1, V1.2.1 | SDTV | DDS, DSS, ACS, ODS object coding type = '2'  |
|  0x14, 0x24 | V1.3.1 | HDTV, UHDTV 1 | DSS, ACS, ODS object coding type = '2'  |
|  0x15, 0x25 | V1.4.1, V1.5.1 | 3DTV | ACS, ODS object coding type = '2'  |
|  0x16, 0x26 | V1.6.1 | HDTV 2, UHDTV | None  |
|  NOTE 1: The subtitle service may use only the CLUT definition segment (CDS) to define the available subtitle colours within the Recommendation ITU-R BT.601 [3] colour system. NOTE 2: The subtitle service may use ODS object coding type = '2' but in that case decoders compliant with V1.5.1 or earlier of the present document will not be able to decode the subtitles.  |   |   |   |

The subtitling_descriptor shall indicate the page id values of the segments needed to decode that subtitle service. The page id of segments with data specific to that service is referred to as the **composition page id**, while the page id of segments with shared data is referred to as the **ancillary page id**.

Version 1.6.1 of the present document introduces two new features that could, in principle, also be used with non-UHDTV service types. These features are progressive-scan bitmap objects and the alternative CLUT segment. The principle of decoder compatibility implies that if the service provider intends to maintain interoperability with existing decoders supporting an earlier version of the present document, then the new features of the later version of the present document shall not be used.

In other words, a DVB service may include subtitles with capabilities signalled with a subtitling_type that indicates a lower level of indicative service compatibility than would be expected with the associated service.

For example, a UHDTV service could include subtitle streams that do not use the new features introduced in V1.6.1, and can therefore be signalled using subtitling types 0x14 and/or 0x24, if the service provider chooses to target UHDTV IRDs with subtitle decoders that are compliant with ETSI EN 300 743 (V1.3.1) [6], ETSI EN 300 743 (V1.4.1) [7] or ETSI EN 300 743 (V1.5.1) [8] of the present document. However the service provider should bear in mind that there might be unpredictable results with the positioning of such subtitles on the screen with some UHDTV IRDs.

Conversely, if a service provider wishes to deploy progressively-coded subtitles (with ODS object coding type = '2'), subtitling type 0x16 or 0x26 shall be signalled, even if the service is not a UHDTV service.

# 7 Subtitling service data specification

## 7.1 Introduction

The present clause contains the specification of the syntax and semantics of the subtitling segment, and all subtitling segment types, in clause 7.2.

Clause 7.3 contains the specification of interoperability points for subtitle services and decoders.

## 7.2 Syntax and semantics of the subtitling segment

### 7.2.0 General

#### 7.2.0.1 Segment syntax

The basic syntactical element of subtitle streams is the "segment". It forms the common format shared amongst all elements of this subtitling specification. A segment shall be encoded as described in table 6.
