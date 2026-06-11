use crate::descriptors::DescriptorRegistry;
use crate::tables::bat;

use super::{CompleteSectionSet, ParsedDescriptorLoop};

/// Transport-stream entry in a complete BAT.
#[derive(Debug)]
#[non_exhaustive]
pub struct CompleteBatTransportStream<'a> {
    /// transport_stream_id of the described TS.
    pub transport_stream_id: u16,
    /// original_network_id of the described TS.
    pub original_network_id: u16,
    /// Typed descriptor loop for this transport stream.
    pub descriptors: ParsedDescriptorLoop<'a>,
}

/// Complete logical Bouquet Association Table.
#[derive(Debug)]
#[non_exhaustive]
pub struct CompleteBat<'a> {
    /// Bouquet identifier.
    pub bouquet_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// Bouquet descriptors from section 0.
    pub bouquet_descriptors: ParsedDescriptorLoop<'a>,
    /// Transport-stream loop entries from all sections in wire order.
    pub transport_streams: Vec<CompleteBatTransportStream<'a>>,
}

impl<'a> CompleteBat<'a> {
    pub(crate) fn parse(
        set: &'a CompleteSectionSet,
        registry: Option<&'a DescriptorRegistry>,
    ) -> crate::Result<Self> {
        let sections: Vec<bat::BatSection<'a>> = set.parse_sections()?;
        let first = sections.first().ok_or(crate::Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "CompleteBat sections",
        })?;
        let mut transport_streams = Vec::new();
        for section in &sections {
            transport_streams.extend(section.transport_streams.iter().map(|ts| {
                CompleteBatTransportStream {
                    transport_stream_id: ts.transport_stream_id,
                    original_network_id: ts.original_network_id,
                    descriptors: ParsedDescriptorLoop::parse(ts.descriptors, registry),
                }
            }));
        }
        Ok(Self {
            bouquet_id: first.bouquet_id,
            version_number: first.version_number,
            current_next_indicator: first.current_next_indicator,
            bouquet_descriptors: ParsedDescriptorLoop::parse(first.bouquet_descriptors, registry),
            transport_streams,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::SectionSetCollector;
    use crate::descriptors::DescriptorLoop;
    use crate::tables::bat::BatSection;

    fn build_bat_section(
        bouquet_id: u16,
        version: u8,
        section_number: u8,
        last_section_number: u8,
        bouquet_desc: &[u8],
        ts_entries: &[(u16, u16, &[u8])],
    ) -> Vec<u8> {
        use dvb_common::Serialize;
        let bat = BatSection {
            bouquet_id,
            version_number: version,
            current_next_indicator: true,
            section_number,
            last_section_number,
            bouquet_descriptors: DescriptorLoop::new(bouquet_desc),
            transport_streams: ts_entries
                .iter()
                .map(
                    |(tsid, onid, desc)| crate::tables::bat::BatTransportStream {
                        transport_stream_id: *tsid,
                        original_network_id: *onid,
                        descriptors: DescriptorLoop::new(desc),
                    },
                )
                .collect(),
        };
        let mut buf = vec![0u8; bat.serialized_len()];
        bat.serialize_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn complete_bat_preserves_bouquet_descriptors_and_ts_entries() {
        let bouquet_name = [0x47u8, 0x03, b'F', b'o', b'o'];
        let ts_desc = [0x43u8, 0x01, 0x01];

        let sec = build_bat_section(
            0x1234,
            3,
            0,
            0,
            &bouquet_name,
            &[(0x1001, 0x2002, &ts_desc)],
        );

        let mut collector = SectionSetCollector::new();
        let set = collector.push_section(&sec).unwrap().unwrap();
        let complete = set.bat().unwrap();

        assert_eq!(complete.bouquet_id, 0x1234);
        assert_eq!(complete.version_number, 3);
        assert!(complete.current_next_indicator);
        assert_eq!(complete.transport_streams.len(), 1);
        assert_eq!(complete.transport_streams[0].transport_stream_id, 0x1001);
        assert_eq!(complete.transport_streams[0].original_network_id, 0x2002);
    }
}
