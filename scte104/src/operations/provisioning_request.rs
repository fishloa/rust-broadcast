//! provisioning_request_data() — ANSI/SCTE 104 2023 §10.5.1, Table 10-3
//! (opID 0x000B). PAMS ==> AS.
//!
//! Basic request. The PAMS notifies the Automation System of all Injectors
//! ready for use in DPI service: a loop of services, each with a nested loop
//! of DPI PID entries and an optional `injector_component_list()`
//! (§10.8.1, Table 10-9).
//!
//! Transcribed in `docs/ansi_scte_104/pams_operations.md` (verified via
//! `pdf2md` against ANSI/SCTE 104 2023 pp. 75-76 and p. 80-81).

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for provisioning_request (§8.3, Table 8-3).
pub const OP_ID: u16 = 0x000B;

/// Fixed byte width of the `service_name` / `injector_service_name` string
/// fields (Table 10-3 §10.5.1 / Table 10-5 §10.6.1) — NUL-terminated,
/// zero-padded to this width.
pub const SERVICE_NAME_LEN: usize = 32;

/// Fixed wire size of one `DPI_PID_index`/`shared_PID`/`event_id_compliance_flag`
/// loop entry (Table 10-3).
pub const DPI_PID_ENTRY_LEN: usize = 4;

/// One entry in the `provisioning_request_data()` DPI-PID loop — Table 10-3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DpiPidEntry {
    /// `DPI_PID_index` — 2 bytes. PID index for a specific DPI service (§8.2.1).
    pub dpi_pid_index: u16,
    /// `shared_PID` — 1 byte. Zero = unique; one = intentionally duplicated.
    pub shared_pid: u8,
    /// `event_id_compliance_flag` — 1 byte.
    pub event_id_compliance_flag: u8,
}

impl DpiPidEntry {
    fn parse_one(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < DPI_PID_ENTRY_LEN {
            return Err(Error::BufferTooShort {
                need: DPI_PID_ENTRY_LEN,
                have: bytes.len(),
                what: "provisioning_request_data DPI PID entry",
            });
        }
        Ok(Self {
            dpi_pid_index: u16::from_be_bytes([bytes[0], bytes[1]]),
            shared_pid: bytes[2],
            event_id_compliance_flag: bytes[3],
        })
    }

    fn write_one(&self, buf: &mut [u8]) {
        buf[0..2].copy_from_slice(&self.dpi_pid_index.to_be_bytes());
        buf[2] = self.shared_pid;
        buf[3] = self.event_id_compliance_flag;
    }
}

/// `injector_component_list()` — §10.8.1, Table 10-9. Referenced
/// conditionally from `provisioning_request_data()` when `component_mode != 0`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InjectorComponentList {
    /// `video_component_tag` — 1 byte. `component_tag` of the video stream.
    pub video_component_tag: u8,
    /// `audio_component_tag` loop — one entry per audio stream.
    pub audio_component_tags: Vec<u8>,
    /// `data_component_tag` loop — one entry per data service.
    pub data_component_tags: Vec<u8>,
}

impl InjectorComponentList {
    /// Parse from the front of `bytes`; returns the value and the number of
    /// bytes consumed (the caller has more fields possibly following).
    fn parse_prefix(bytes: &[u8]) -> Result<(Self, usize)> {
        if bytes.len() < 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: bytes.len(),
                what: "injector_component_list header",
            });
        }
        let video_component_tag = bytes[0];
        let n_audio = bytes[1] as usize;
        let mut pos = 2;
        if bytes.len() < pos + n_audio + 1 {
            return Err(Error::BufferTooShort {
                need: pos + n_audio + 1,
                have: bytes.len(),
                what: "injector_component_list audio_component_tags",
            });
        }
        let audio_component_tags = bytes[pos..pos + n_audio].to_vec();
        pos += n_audio;
        let n_data = bytes[pos] as usize;
        pos += 1;
        if bytes.len() < pos + n_data {
            return Err(Error::BufferTooShort {
                need: pos + n_data,
                have: bytes.len(),
                what: "injector_component_list data_component_tags",
            });
        }
        let data_component_tags = bytes[pos..pos + n_data].to_vec();
        pos += n_data;
        Ok((
            Self {
                video_component_tag,
                audio_component_tags,
                data_component_tags,
            },
            pos,
        ))
    }

    fn serialized_len(&self) -> usize {
        2 + self.audio_component_tags.len() + 1 + self.data_component_tags.len()
    }

    /// Write into the front of `buf`; returns bytes written.
    fn write_prefix(&self, buf: &mut [u8]) -> usize {
        let n_audio = self.audio_component_tags.len();
        let n_data = self.data_component_tags.len();
        buf[0] = self.video_component_tag;
        buf[1] = n_audio as u8;
        let mut pos = 2;
        buf[pos..pos + n_audio].copy_from_slice(&self.audio_component_tags);
        pos += n_audio;
        buf[pos] = n_data as u8;
        pos += 1;
        buf[pos..pos + n_data].copy_from_slice(&self.data_component_tags);
        pos += n_data;
        pos
    }
}

/// One service entry within `provisioning_request_data()` — Table 10-3.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProvisioningService {
    /// `injector_IP_address` — 4 bytes. Zero if not using TCP/IP.
    pub injector_ip_address: u32,
    /// `injector_socket_number` — 2 bytes. Zero if not using TCP/IP.
    pub injector_socket_number: u16,
    /// `service_name` — 32-byte NUL-terminated string field.
    pub service_name: [u8; SERVICE_NAME_LEN],
    /// `DPI_PID_index`/`shared_PID`/`event_id_compliance_flag` loop.
    pub dpi_pids: Vec<DpiPidEntry>,
    /// `component_mode` — 1 byte. Acts as the presence flag for
    /// `injector_component_list()` per the syntax table's
    /// `if (component_mode != 0)` condition (see the module doc note on the
    /// prose/syntax-table discrepancy for this field).
    pub component_mode: u8,
    /// `injector_component_list()`, present iff `component_mode != 0`.
    pub injector_component_list: Option<InjectorComponentList>,
}

impl Default for ProvisioningService {
    fn default() -> Self {
        Self {
            injector_ip_address: 0,
            injector_socket_number: 0,
            service_name: [0; SERVICE_NAME_LEN],
            dpi_pids: Vec::new(),
            component_mode: 0,
            injector_component_list: None,
        }
    }
}

impl ProvisioningService {
    fn parse_prefix(bytes: &[u8]) -> Result<(Self, usize)> {
        const HEAD_LEN: usize = 4 + 2 + SERVICE_NAME_LEN + 1; // up to number_of_DPI_PIDs
        if bytes.len() < HEAD_LEN {
            return Err(Error::BufferTooShort {
                need: HEAD_LEN,
                have: bytes.len(),
                what: "provisioning_request_data service header",
            });
        }
        let injector_ip_address = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let injector_socket_number = u16::from_be_bytes([bytes[4], bytes[5]]);
        let mut service_name = [0u8; SERVICE_NAME_LEN];
        service_name.copy_from_slice(&bytes[6..6 + SERVICE_NAME_LEN]);
        let number_of_dpi_pids = bytes[6 + SERVICE_NAME_LEN] as usize;
        let mut pos = HEAD_LEN;

        let mut dpi_pids = Vec::with_capacity(number_of_dpi_pids);
        for _ in 0..number_of_dpi_pids {
            let entry = DpiPidEntry::parse_one(&bytes[pos..])?;
            dpi_pids.push(entry);
            pos += DPI_PID_ENTRY_LEN;
        }

        if bytes.len() < pos + 1 {
            return Err(Error::BufferTooShort {
                need: pos + 1,
                have: bytes.len(),
                what: "provisioning_request_data component_mode",
            });
        }
        let component_mode = bytes[pos];
        pos += 1;

        let injector_component_list = if component_mode != 0 {
            let (list, consumed) = InjectorComponentList::parse_prefix(&bytes[pos..])?;
            pos += consumed;
            Some(list)
        } else {
            None
        };

        Ok((
            Self {
                injector_ip_address,
                injector_socket_number,
                service_name,
                dpi_pids,
                component_mode,
                injector_component_list,
            },
            pos,
        ))
    }

    fn serialized_len(&self) -> usize {
        let mut len = 4 + 2 + SERVICE_NAME_LEN + 1 + self.dpi_pids.len() * DPI_PID_ENTRY_LEN + 1;
        if let Some(list) = &self.injector_component_list {
            len += list.serialized_len();
        }
        len
    }

    fn write_prefix(&self, buf: &mut [u8]) -> usize {
        buf[0..4].copy_from_slice(&self.injector_ip_address.to_be_bytes());
        buf[4..6].copy_from_slice(&self.injector_socket_number.to_be_bytes());
        buf[6..6 + SERVICE_NAME_LEN].copy_from_slice(&self.service_name);
        buf[6 + SERVICE_NAME_LEN] = self.dpi_pids.len() as u8;
        let mut pos = 4 + 2 + SERVICE_NAME_LEN + 1;
        for entry in &self.dpi_pids {
            entry.write_one(&mut buf[pos..pos + DPI_PID_ENTRY_LEN]);
            pos += DPI_PID_ENTRY_LEN;
        }
        buf[pos] = self.component_mode;
        pos += 1;
        if let Some(list) = &self.injector_component_list {
            pos += list.write_prefix(&mut buf[pos..]);
        }
        pos
    }
}

/// provisioning_request_data() — §10.5.1, Table 10-3. Variable-length: a
/// `service_count`-prefixed loop of [`ProvisioningService`] entries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProvisioningRequest {
    /// The `service_count`-prefixed loop of services.
    pub services: Vec<ProvisioningService>,
}

impl<'a> Parse<'a> for ProvisioningRequest {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "provisioning_request_data service_count",
            });
        }
        let service_count = bytes[0] as usize;
        let mut pos = 1;
        let mut services = Vec::with_capacity(service_count);
        for _ in 0..service_count {
            let (service, consumed) = ProvisioningService::parse_prefix(&bytes[pos..])?;
            services.push(service);
            pos += consumed;
        }
        Ok(Self { services })
    }
}

impl Serialize for ProvisioningRequest {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        1 + self
            .services
            .iter()
            .map(ProvisioningService::serialized_len)
            .sum::<usize>()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0] = self.services.len() as u8;
        let mut pos = 1;
        for service in &self.services {
            pos += service.write_prefix(&mut buf[pos..]);
        }
        Ok(pos)
    }
}

impl OperationDef<'_> for ProvisioningRequest {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "PROVISIONING_REQUEST";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn sample_name(s: &str) -> [u8; SERVICE_NAME_LEN] {
        let mut name = [0u8; SERVICE_NAME_LEN];
        name[..s.len()].copy_from_slice(s.as_bytes());
        name
    }

    #[test]
    fn round_trip_no_services() {
        let op = ProvisioningRequest::default();
        let bytes = op.to_bytes();
        assert_eq!(bytes, vec![0]);
        let back = ProvisioningRequest::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn round_trip_one_service_no_component_list() {
        let op = ProvisioningRequest {
            services: vec![ProvisioningService {
                injector_ip_address: 0xC0A8_0001,
                injector_socket_number: 5167,
                service_name: sample_name("svc-1"),
                dpi_pids: vec![DpiPidEntry {
                    dpi_pid_index: 1,
                    shared_pid: 0,
                    event_id_compliance_flag: 1,
                }],
                component_mode: 0,
                injector_component_list: None,
            }],
        };
        let bytes = op.to_bytes();
        let back = ProvisioningRequest::parse(&bytes).unwrap();
        assert_eq!(op, back);
        let b2 = back.to_bytes();
        assert_eq!(bytes, b2);
    }

    #[test]
    fn round_trip_two_services_with_component_list() {
        let op = ProvisioningRequest {
            services: vec![
                ProvisioningService {
                    injector_ip_address: 0x0A00_0001,
                    injector_socket_number: 8000,
                    service_name: sample_name("svc-a"),
                    dpi_pids: vec![
                        DpiPidEntry {
                            dpi_pid_index: 1,
                            shared_pid: 0,
                            event_id_compliance_flag: 1,
                        },
                        DpiPidEntry {
                            dpi_pid_index: 2,
                            shared_pid: 1,
                            event_id_compliance_flag: 0,
                        },
                    ],
                    component_mode: 1,
                    injector_component_list: Some(InjectorComponentList {
                        video_component_tag: 0x10,
                        audio_component_tags: vec![0x20, 0x21],
                        data_component_tags: vec![0x30],
                    }),
                },
                ProvisioningService {
                    injector_ip_address: 0,
                    injector_socket_number: 0,
                    service_name: sample_name("svc-b"),
                    dpi_pids: vec![],
                    component_mode: 0,
                    injector_component_list: None,
                },
            ],
        };
        let bytes = op.to_bytes();
        let back = ProvisioningRequest::parse(&bytes).unwrap();
        assert_eq!(op, back);
        let b2 = back.to_bytes();
        assert_eq!(bytes, b2);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = ProvisioningRequest {
            services: vec![ProvisioningService {
                injector_ip_address: 1,
                injector_socket_number: 1,
                service_name: sample_name("svc"),
                dpi_pids: vec![],
                component_mode: 0,
                injector_component_list: None,
            }],
        };
        let bytes = op.to_bytes();
        let mut op2 = op.clone();
        op2.services[0].injector_ip_address = 999;
        assert_ne!(op2.to_bytes(), bytes);
    }

    #[test]
    fn truncated_buffer_rejected() {
        let op = ProvisioningRequest {
            services: vec![ProvisioningService {
                injector_ip_address: 1,
                injector_socket_number: 1,
                service_name: sample_name("svc"),
                dpi_pids: vec![DpiPidEntry {
                    dpi_pid_index: 1,
                    shared_pid: 0,
                    event_id_compliance_flag: 0,
                }],
                component_mode: 0,
                injector_component_list: None,
            }],
        };
        let bytes = op.to_bytes();
        assert!(matches!(
            ProvisioningRequest::parse(&bytes[..bytes.len() - 1]),
            Err(Error::BufferTooShort { .. })
        ));
    }
}
