use crate::descriptors::DescriptorRegistry;
use crate::tables::nit;
use alloc::vec::Vec;

use super::{CompleteSectionSet, CompleteTransportStream, ParsedDescriptorLoop};

/// Complete logical Network Information Table.
#[derive(Debug)]
#[non_exhaustive]
pub struct CompleteNit<'a> {
    /// Variant discriminator.
    pub kind: nit::NitKind,
    /// Network identifier.
    pub network_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// Network-wide descriptors, concatenated across every section that
    /// carries a `network_descriptors_loop` (EN 300 468 §5.2.1: each section
    /// has its own loop, so a network name/linkage split across sections is
    /// not lost).
    pub network_descriptors: ParsedDescriptorLoop<'a>,
    /// Transport-stream loop entries from all sections in wire order.
    pub transport_streams: Vec<CompleteTransportStream<'a>>,
}

impl<'a> CompleteNit<'a> {
    pub(crate) fn parse(
        set: &'a CompleteSectionSet,
        registry: Option<&'a DescriptorRegistry>,
    ) -> crate::Result<Self> {
        let sections: Vec<nit::NitSection<'a>> = set.parse_sections()?;
        let first = sections.first().ok_or(crate::Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 1,
                have: 0,
                what: "CompleteNit sections",
            },
        ))?;
        let transport_streams = CompleteTransportStream::collect(
            sections.iter().flat_map(|section| {
                section.transport_streams.iter().map(|ts| {
                    (
                        ts.transport_stream_id,
                        ts.original_network_id,
                        ts.descriptors,
                    )
                })
            }),
            registry,
        );
        Ok(Self {
            kind: first.kind,
            network_id: first.network_id,
            version_number: first.version_number,
            current_next_indicator: first.current_next_indicator,
            network_descriptors: ParsedDescriptorLoop::parse_all(
                sections.iter().map(|section| section.network_descriptors),
                registry,
            ),
            transport_streams,
        })
    }
}
