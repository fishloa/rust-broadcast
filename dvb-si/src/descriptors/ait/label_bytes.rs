//! Opaque label bytes shared by the AIT `application_recording_descriptor`
//! (`label_char`, ETSI TS 102 809 V1.3.1 §5.3.5.4 Table 21) and the
//! `prefetch_descriptor` (`label_byte`, ETSI TS 102 727 V1.1.1 Table 82).
//!
//! Neither spec makes a label text. TS 102 809 (rendered p.41): "label_char:
//! These 8-bit fields carry an array of bytes that label a part of the
//! application within its transport protocol." and NOTE 3: "The present
//! document does not define which parts of applications can be labelled or the
//! form of the label (if any). Platform specifications that wish to use this
//! mechanism need to define the format of labels." The prefetch label is
//! matched byte-for-byte against DII `label_descriptor`s. So the type is a
//! byte string, deliberately not `DvbText` (no Annex A coding is specified).

/// A label: an opaque byte array compared byte-for-byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LabelBytes<'a>(&'a [u8]);

impl<'a> LabelBytes<'a> {
    /// Wrap label bytes.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }

    /// The label bytes, exactly as carried on the wire.
    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.0
    }

    /// Number of bytes in the label (`label_length`).
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Is the label empty?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'a> From<&'a [u8]> for LabelBytes<'a> {
    fn from(b: &'a [u8]) -> Self {
        Self(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_for_byte_equality() {
        assert_eq!(LabelBytes::new(b"img"), LabelBytes::from(&b"img"[..]));
        assert_ne!(LabelBytes::new(b"img"), LabelBytes::new(b"IMG"));
        assert!(LabelBytes::default().is_empty());
        assert_eq!(LabelBytes::new(b"ab").len(), 2);
    }
}
