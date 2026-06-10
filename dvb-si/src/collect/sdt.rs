use crate::descriptors::DescriptorRegistry;
use crate::tables::sdt;

use super::{CompleteSectionSet, ParsedDescriptorLoop};

/// Service entry in a complete SDT.
#[derive(Debug)]
pub struct CompleteSdtService<'a> {
    /// service_id.
    pub service_id: u16,
    /// EIT schedule flag.
    pub eit_schedule_flag: bool,
    /// EIT present/following flag.
    pub eit_present_following_flag: bool,
    /// 3-bit running status.
    pub running_status: u8,
    /// free_CA_mode.
    pub free_ca_mode: bool,
    /// Typed descriptor loop for this service.
    pub descriptors: ParsedDescriptorLoop<'a>,
}

/// Complete logical Service Description Table.
#[derive(Debug)]
pub struct CompleteSdt<'a> {
    /// Variant discriminator.
    pub kind: sdt::SdtKind,
    /// transport_stream_id of the described TS.
    pub transport_stream_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// original_network_id of the described TS.
    pub original_network_id: u16,
    /// Services from all sections in wire order.
    pub services: Vec<CompleteSdtService<'a>>,
}

impl<'a> CompleteSdt<'a> {
    pub(crate) fn parse(
        set: &'a CompleteSectionSet,
        registry: Option<&'a DescriptorRegistry>,
    ) -> crate::Result<Self> {
        let sections: Vec<sdt::SdtSection<'a>> = set.parse_sections()?;
        let first = sections.first().ok_or(crate::Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "CompleteSdt sections",
        })?;
        let mut services = Vec::new();
        for section in &sections {
            services.extend(section.services.iter().map(|svc| CompleteSdtService {
                service_id: svc.service_id,
                eit_schedule_flag: svc.eit_schedule_flag,
                eit_present_following_flag: svc.eit_present_following_flag,
                running_status: svc.running_status,
                free_ca_mode: svc.free_ca_mode,
                descriptors: ParsedDescriptorLoop::parse(svc.descriptors, registry),
            }));
        }
        Ok(Self {
            kind: first.kind,
            transport_stream_id: first.transport_stream_id,
            version_number: first.version_number,
            current_next_indicator: first.current_next_indicator,
            original_network_id: first.original_network_id,
            services,
        })
    }
}
