//! component_mode_DPI_request_data() — ANSI/SCTE 104 2023 §9.5.1, Table 9-11 (opID 0x0106).
//!
//! Supplemental usage. Specifies per-component splice timing for component
//! mode DPI. Carries a variable-length loop of `(component_tag, component_preroll)`.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for component_mode_DPI_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0106;

/// One component entry in `component_mode_DPI_request_data()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ComponentModeEntry {
    /// `component_tag` — 1 byte.
    pub component_tag: u8,
    /// `component_preroll` — 2 bytes, milliseconds.
    pub component_preroll: u16,
}

/// Fixed wire size per component entry.
pub const ENTRY_LEN: usize = 3;

/// component_mode_DPI_request_data() — §9.5.1, Table 9-11.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ComponentModeDpi {
    /// Loop of per-component entries.
    pub components: Vec<ComponentModeEntry>,
}

impl<'a> Parse<'a> for ComponentModeDpi {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // No explicit count — parse all available entries
        let count = bytes.len() / ENTRY_LEN;
        if bytes.len() % ENTRY_LEN != 0 {
            return Err(Error::BufferTooShort {
                need: (count + 1) * ENTRY_LEN,
                have: bytes.len(),
                what: "component_mode_DPI data",
            });
        }
        let mut components = Vec::with_capacity(count);
        for i in 0..count {
            let off = i * ENTRY_LEN;
            components.push(ComponentModeEntry {
                component_tag: bytes[off],
                component_preroll: u16::from_be_bytes([bytes[off + 1], bytes[off + 2]]),
            });
        }
        Ok(Self { components })
    }
}

impl Serialize for ComponentModeDpi {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.components.len() * ENTRY_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        for (i, c) in self.components.iter().enumerate() {
            let off = i * ENTRY_LEN;
            buf[off] = c.component_tag;
            buf[off + 1..off + 3].copy_from_slice(&c.component_preroll.to_be_bytes());
        }
        Ok(need)
    }
}

impl<'a> OperationDef<'a> for ComponentModeDpi {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "COMPONENT_MODE_DPI";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = ComponentModeDpi {
            components: alloc::vec![
                ComponentModeEntry {
                    component_tag: 1,
                    component_preroll: 5000,
                },
                ComponentModeEntry {
                    component_tag: 2,
                    component_preroll: 3000,
                },
            ],
        };
        let bytes = op.to_bytes();
        assert_eq!(bytes.len(), 6);
        let back = ComponentModeDpi::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = ComponentModeDpi {
            components: alloc::vec![ComponentModeEntry {
                component_tag: 1,
                component_preroll: 1000,
            }],
        };
        let bytes = op.to_bytes();
        let mut op2 = op.clone();
        op2.components[0].component_preroll = 999;
        assert_ne!(op2.to_bytes(), bytes);
    }

    #[test]
    fn empty_round_trip() {
        let op = ComponentModeDpi::default();
        let bytes = op.to_bytes();
        assert!(bytes.is_empty());
        assert_eq!(ComponentModeDpi::parse(&bytes).unwrap(), op);
    }
}
