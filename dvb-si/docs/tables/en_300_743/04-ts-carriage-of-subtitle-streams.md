# Table 4: TS carriage of subtitle streams

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  stream_type in the PMT | Set to '0x06' indicating "PES packets containing private data".  |
| --- | --- |

For each subtitle service a subtitling_descriptor as defined in ETSI EN 300 468 [2] shall signal the properties of the subtitle service in the PMT of the Transport Stream carrying that subtitle service.

The subtitling_type field in the subtitling_descriptor shall be set according to the subtitle service properties and features used in the subtitle service, as shown in table 5. The value of subtitling_type implicitly signals the version of the present document with which the subtitle service is compliant.

The subtitling_type value shall be set to the same value as the component_type value of a DVB component descriptor as defined in ETSI EN 300 468 [2] when the stream_content field of that descriptor is equal to '0x3'. Due to the evolution of the present document, features have been added to each new version. Obviously, features introduced in any version of the present document will not be supported by IRDs that were designed to be compliant with an earlier version of the specification, hence the subtitle service shall use a value of subtitling_type corresponding to the associated service, and should use only those features, i.e. segment types and ODS coding types, that were specified in the corresponding version of the present document. Subtitle services that choose not to follow this recommendation could face issues of incompatibility with legacy subtitle decoders that might not be robust against the presence of unknown or unsupported subtitling features in the subtitle service.

IRDs shall ignore subtitle services signalled with a subtitling_type that they do not support.

NOTE: It is known that some early implementations of subtitle decoders might not ignore nor be robust against the presence of unsupported subtitling_types in subtitle bitstreams.

Table 5 lists the features of the present document that are not recommended to be used in subtitle services that are provided in accordance with a particular version of the present document, which is implicitly signalled by the subtitling_type field in the subtitling_descriptor in the PMT.
