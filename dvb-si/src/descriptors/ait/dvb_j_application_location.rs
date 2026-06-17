//! DVB-J Application Location Descriptor — ETSI TS 102 727 §10.9.2, Table 85
//! (AIT tag 0x04).
//!
//! Carried in the AIT per-application descriptor loop. Contains the base
//! directory, classpath extension, and initial class name for a DVB-J application.

use crate::descriptors::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for dvb_j_application_location_descriptor (AIT namespace).
pub const TAG: u8 = 0x04;
const HEADER_LEN: usize = 2;

/// DVB-J Application Location Descriptor (AIT tag 0x04).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DvbJApplicationLocationDescriptor<'a> {
    /// Base directory bytes (e.g. "/").
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub base_directory: &'a [u8],
    /// Classpath extension bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub classpath_extension: &'a [u8],
    /// Initial class bytes (consumes the remainder of the descriptor).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub initial_class: &'a [u8],
}

impl<'a> Parse<'a> for DvbJApplicationLocationDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "DvbJApplicationLocationDescriptor",
            "unexpected tag for dvb_j_application_location_descriptor",
        )?;
        if body.is_empty() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "dvb_j_application_location_descriptor body is empty",
            });
        }
        let base_dir_len = body[0] as usize;
        let end = 1 + base_dir_len;
        if body.len() < end {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "dvb_j_application_location_descriptor base_directory overruns body",
            });
        }
        let base_directory = &body[1..end];
        let rest = &body[end..];
        if rest.is_empty() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "dvb_j_application_location_descriptor missing classpath_extension",
            });
        }
        let cp_len = rest[0] as usize;
        let cp_end = 1 + cp_len;
        if rest.len() < cp_end {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "dvb_j_application_location_descriptor classpath_extension overruns body",
            });
        }
        let classpath_extension = &rest[1..cp_end];
        let initial_class = &rest[cp_end..];
        Ok(Self {
            base_directory,
            classpath_extension,
            initial_class,
        })
    }
}

impl Serialize for DvbJApplicationLocationDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + 1
            + self.base_directory.len()
            + 1
            + self.classpath_extension.len()
            + self.initial_class.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.base_directory.len() > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "dvb_j_application_location_descriptor base_directory exceeds 255 bytes",
            });
        }
        if self.classpath_extension.len() > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason:
                    "dvb_j_application_location_descriptor classpath_extension exceeds 255 bytes",
            });
        }
        let body_len = self.serialized_len() - HEADER_LEN;
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "dvb_j_application_location_descriptor body exceeds 255 bytes",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        let mut pos = HEADER_LEN;
        buf[pos] = self.base_directory.len() as u8;
        pos += 1;
        buf[pos..pos + self.base_directory.len()].copy_from_slice(self.base_directory);
        pos += self.base_directory.len();
        buf[pos] = self.classpath_extension.len() as u8;
        pos += 1;
        buf[pos..pos + self.classpath_extension.len()].copy_from_slice(self.classpath_extension);
        pos += self.classpath_extension.len();
        buf[pos..pos + self.initial_class.len()].copy_from_slice(self.initial_class);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for DvbJApplicationLocationDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DVB_J_APPLICATION_LOCATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full() {
        let bytes = [
            TAG, 10, // descriptor_length = 10
            1, b'/', // base_directory_len=1, base_directory="/"
            5, b'l', b'i', b'b', b'/', b';', // classpath_extension_len=5, "lib/;"
            b'A', b'B', // initial_class = "AB" (no length prefix — consumes rest)
        ];
        let d = DvbJApplicationLocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.base_directory, b"/");
        assert_eq!(d.classpath_extension, b"lib/;");
        assert_eq!(d.initial_class, b"AB");
    }

    #[test]
    fn parse_no_initial_class() {
        // Per spec, initial_class is the REST — it can be empty
        let bytes = [
            TAG, 4, // descriptor_length = 4
            1, b'/', // base_directory_len=1, "/"
            1,
            b';', // classpath_extension_len=1, ";"
                  // initial_class = empty
        ];
        let d = DvbJApplicationLocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.base_directory, b"/");
        assert_eq!(d.classpath_extension, b";");
        assert!(d.initial_class.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = DvbJApplicationLocationDescriptor {
            base_directory: b"/apps",
            classpath_extension: b"classes/",
            initial_class: b"com.example.Main",
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = DvbJApplicationLocationDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_byte_identical() {
        let bytes = [
            TAG, 9, // descriptor_length = 9
            1, b'/', // base_directory = "/"
            4, b'a', b';', b'b', b';', // classpath_extension = "a;b;"
            b'c', b'd', // initial_class = "cd" (no length prefix — consumes rest)
        ];
        let d = DvbJApplicationLocationDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
    }
}
