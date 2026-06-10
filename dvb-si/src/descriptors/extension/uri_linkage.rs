use super::*;

impl super::sealed::Sealed for UriLinkage<'_> {}
impl ExtensionBodyDef for UriLinkage<'_> {
    const TAG_EXTENSION: u8 = 0x13;
    const NAME: &'static str = "URI_LINKAGE";
}

// ===========================================================================
//  Section 0x13 — URI_linkage_descriptor (Table 159, §6.4.16.1)
// ---------------------------------------------------------------------------
//  uri_linkage_type, length-delimited URI, an optional min_polling_interval
//  (only for types 0x00/0x01), then trailing private_data. All typed.
// ===========================================================================
/// URI_linkage body (Table 159).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct UriLinkage<'a> {
    /// uri_linkage_type(8).
    pub uri_linkage_type: u8,
    /// Length-delimited URI bytes.
    pub uri: &'a [u8],
    /// min_polling_interval(16), present iff `uri_linkage_type` is 0x00 or 0x01.
    pub min_polling_interval: Option<u16>,
    /// Trailing private_data_byte run.
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for UriLinkage<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < 2 {
            return Err(invalid("URI_linkage: header truncated"));
        }
        let uri_linkage_type = sel[0];
        let uri_length = sel[1] as usize;
        let mut pos = 2;
        if sel.len() < pos + uri_length {
            return Err(invalid("URI_linkage: uri overruns body"));
        }
        let uri = &sel[pos..pos + uri_length];
        pos += uri_length;
        let min_polling_interval = if uri_linkage_type == 0x00 || uri_linkage_type == 0x01 {
            if sel.len() < pos + 2 {
                return Err(invalid("URI_linkage: min_polling_interval truncated"));
            }
            let v = u16::from_be_bytes([sel[pos], sel[pos + 1]]);
            pos += 2;
            Some(v)
        } else {
            None
        };
        Ok(UriLinkage {
            uri_linkage_type,
            uri,
            min_polling_interval,
            private_data: &sel[pos..],
        })
    }
}

impl Serialize for UriLinkage<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        2 + self.uri.len()
            + if self.min_polling_interval.is_some() {
                2
            } else {
                0
            }
            + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = self.uri_linkage_type;
        buf[1] = self.uri.len() as u8;
        let mut p = 2;
        buf[p..p + self.uri.len()].copy_from_slice(self.uri);
        p += self.uri.len();
        if let Some(mpi) = self.min_polling_interval {
            buf[p..p + 2].copy_from_slice(&mpi.to_be_bytes());
            p += 2;
        }
        buf[p..p + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};

    #[test]
    fn parse_uri_linkage_with_polling() {
        let uri = b"http://x";
        let mut sel = vec![0x00, uri.len() as u8];
        sel.extend_from_slice(uri);
        sel.extend_from_slice(&0x1234u16.to_be_bytes());
        sel.push(0xFE); // private
        let bytes = wrap(0x13, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::UriLinkage(b) => {
                assert_eq!(b.uri_linkage_type, 0x00);
                assert_eq!(b.uri, uri);
                assert_eq!(b.min_polling_interval, Some(0x1234));
                assert_eq!(b.private_data, &[0xFE]);
            }
            other => panic!("expected UriLinkage, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_uri_linkage_no_polling() {
        // type 0x02 ⇒ no min_polling_interval
        let uri = b"dvb:";
        let mut sel = vec![0x02, uri.len() as u8];
        sel.extend_from_slice(uri);
        let bytes = wrap(0x13, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::UriLinkage(b) => {
                assert_eq!(b.min_polling_interval, None);
                assert!(b.private_data.is_empty());
            }
            other => panic!("expected UriLinkage, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_uri_linkage_rejects_overrun() {
        let sel = [0x02, 0x10, 0xAA]; // uri_length 16 but 1 byte present
        let bytes = wrap(0x13, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::InvalidDescriptor {
                tag: super::TAG,
                ..
            }
        ));
    }
}
