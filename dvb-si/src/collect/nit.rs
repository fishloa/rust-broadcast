use crate::descriptors::DescriptorRegistry;
use crate::tables::nit;

use super::{CompleteSectionSet, ParsedDescriptorLoop};

/// Transport-stream entry in a complete NIT.
#[derive(Debug)]
pub struct CompleteNitTransportStream<'a> {
    /// transport_stream_id of the described TS.
    pub transport_stream_id: u16,
    /// original_network_id of the described TS.
    pub original_network_id: u16,
    /// Typed descriptor loop for this transport stream.
    pub descriptors: ParsedDescriptorLoop<'a>,
}

/// Complete logical Network Information Table.
#[derive(Debug)]
pub struct CompleteNit<'a> {
    /// Variant discriminator.
    pub kind: nit::NitKind,
    /// Network identifier.
    pub network_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// Network-wide descriptors from section 0.
    pub network_descriptors: ParsedDescriptorLoop<'a>,
    /// Transport-stream loop entries from all sections in wire order.
    pub transport_streams: Vec<CompleteNitTransportStream<'a>>,
}

impl<'a> CompleteNit<'a> {
    pub(crate) fn parse(
        set: &'a CompleteSectionSet,
        registry: Option<&'a DescriptorRegistry>,
    ) -> crate::Result<Self> {
        let sections: Vec<nit::NitSection<'a>> = set.parse_sections()?;
        let first = sections.first().ok_or(crate::Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "CompleteNit sections",
        })?;
        let mut transport_streams = Vec::new();
        for section in &sections {
            transport_streams.extend(section.transport_streams.iter().map(|ts| {
                CompleteNitTransportStream {
                    transport_stream_id: ts.transport_stream_id,
                    original_network_id: ts.original_network_id,
                    descriptors: ParsedDescriptorLoop::parse(ts.descriptors, registry),
                }
            }));
        }
        Ok(Self {
            kind: first.kind,
            network_id: first.network_id,
            version_number: first.version_number,
            current_next_indicator: first.current_next_indicator,
            // The network descriptor loop is carried in section 0; completed
            // sets are stored in section-number order, so `first` is
            // authoritative for table-wide descriptors.
            network_descriptors: ParsedDescriptorLoop::parse(first.network_descriptors, registry),
            transport_streams,
        })
    }
}
